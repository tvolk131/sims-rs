use super::*;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
    resources: Vec<Resource>,
}

impl Fixture {
    fn new(contents: &[&[u8]]) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "sims-startup-inputs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let resources = contents
            .iter()
            .enumerate()
            .map(|(position, bytes)| {
                let name = format!("resource-{position}");
                let source = directory.join(&name);
                fs::write(&source, bytes).unwrap();
                Resource {
                    locator: name.clone(),
                    name,
                    source,
                    offset: 0,
                    size: bytes.len() as u64,
                    unpacked_size: bytes.len() as u64,
                    archived: true,
                    owned: None,
                }
            })
            .collect();
        Self {
            directory,
            resources,
        }
    }

    fn tape(&self) -> Tape {
        let session = Session::capture(MAX_BYTES);
        {
            let mut scan = Scan::observed(&session).unwrap();
            for resource in &self.resources {
                assert_eq!(scan.read(resource).unwrap(), resource.read().unwrap());
            }
        }
        session.finish().unwrap().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn changed(validation: Validation) -> Session {
    match validation {
        Validation::Changed(session) => session,
        Validation::Unchanged => panic!("source mutation earned unchanged input credit"),
    }
}

#[test]
fn warm_validation_reads_each_repeated_observation_and_detects_same_length_changes() {
    let fixture = Fixture::new(&[b"same"]);
    let resource = &fixture.resources[0];
    let session = Session::capture(MAX_BYTES);
    {
        let mut scan = Scan::observed(&session).unwrap();
        assert_eq!(scan.read(resource).unwrap(), b"same");
        assert_eq!(scan.read(resource).unwrap(), b"same");
    }
    let tape = session.finish().unwrap().unwrap();
    assert!(matches!(
        tape.validate(MAX_BYTES).unwrap(),
        Validation::Unchanged
    ));
    let modified = resource.source.metadata().unwrap().modified().unwrap();
    let mut reads = 0;
    let validation = tape
        .validate_before(MAX_BYTES, |_, event| {
            if matches!(event, Event::Resource { .. }) {
                reads += 1;
                if reads == 2 {
                    fs::write(&resource.source, b"next").unwrap();
                    fs::OpenOptions::new()
                        .write(true)
                        .open(&resource.source)
                        .unwrap()
                        .set_times(fs::FileTimes::new().set_modified(modified))
                        .unwrap();
                }
            }
        })
        .unwrap();
    assert_eq!(reads, 2);
    let replay = changed(validation);
    fs::write(&resource.source, b"late").unwrap();
    {
        let mut scan = Scan::observed(&replay).unwrap();
        assert_eq!(scan.read(resource).unwrap(), b"same");
        assert_eq!(scan.read(resource).unwrap(), b"next");
    }
    assert!(replay.finish().unwrap().is_some());
}

#[test]
fn first_divergence_stops_lookahead_and_prefix_is_not_reread() {
    let fixture = Fixture::new(&[b"old1", b"old2", b"old3"]);
    let tape = fixture.tape();
    fs::write(&fixture.resources[1].source, b"new2").unwrap();
    let mut observed = vec![];
    let validation = tape
        .validate_before(MAX_BYTES, |_, event| {
            if let Event::Resource { resource, .. } = event {
                observed.push(resource.locator.clone());
            }
        })
        .unwrap();
    assert_eq!(observed, ["resource-0", "resource-1"]);
    let replay = changed(validation);
    fs::remove_file(&fixture.resources[0].source).unwrap();
    fs::write(&fixture.resources[1].source, b"late").unwrap();
    fs::remove_file(&fixture.resources[2].source).unwrap();
    {
        let mut scan = Scan::observed(&replay).unwrap();
        assert_eq!(scan.read(&fixture.resources[0]).unwrap(), b"old1");
        assert_eq!(scan.read(&fixture.resources[1]).unwrap(), b"new2");
    }
    let replacement = replay.finish().unwrap().unwrap();
    assert_eq!(replacement.events.len(), 4);
}

#[test]
fn controller_keeps_early_decode_error_ahead_of_later_missing_source() {
    fn controller(fixture: &Fixture, session: &Session) -> Result<()> {
        let mut scan = Scan::observed(session)?;
        let bytes = scan.read(&fixture.resources[0])?;
        ensure!(bytes.as_slice() != b"bad!", "early native support boundary");
        scan.read(&fixture.resources[1])?;
        Ok(())
    }
    let fixture = Fixture::new(&[b"good", b"tail"]);
    let session = Session::capture(MAX_BYTES);
    controller(&fixture, &session).unwrap();
    let tape = session.finish().unwrap().unwrap();
    fs::write(&fixture.resources[0].source, b"bad!").unwrap();
    fs::remove_file(&fixture.resources[1].source).unwrap();
    let mut reads = 0;
    let replay = changed(
        tape.validate_before(MAX_BYTES, |_, event| {
            reads += usize::from(matches!(event, Event::Resource { .. }));
        })
        .unwrap(),
    );
    assert_eq!(reads, 1);
    assert_eq!(
        controller(&fixture, &replay).unwrap_err().to_string(),
        "early native support boundary"
    );
    replay.finish().unwrap();
}

#[cfg(unix)]
#[test]
fn fallback_continues_the_validating_descriptor_scope_at_the_frontier() {
    let mut fixture = Fixture::new(&[b"aaaabbbbcccc"]);
    fixture.resources[0].size = 4;
    fixture.resources[0].unpacked_size = 4;
    let mut middle = fixture.resources[0].clone();
    middle.offset = 4;
    middle.locator = "middle".into();
    let mut last = middle.clone();
    last.offset = 8;
    last.locator = "last".into();
    fixture.resources.extend([middle, last]);
    let tape = fixture.tape();
    let replay = changed(
        tape.validate_before(MAX_BYTES, |_, event| {
            if matches!(event, Event::Resource { resource, .. } if resource.locator == "middle") {
                fs::write(&fixture.resources[0].source, b"aaaaBBBBcccc").unwrap();
            }
        })
        .unwrap(),
    );
    {
        let mut scan = Scan::observed(&replay).unwrap();
        assert_eq!(scan.read(&fixture.resources[0]).unwrap(), b"aaaa");
        assert_eq!(scan.read(&fixture.resources[1]).unwrap(), b"BBBB");
        let opens = replay.0.lock().unwrap().scans[&1].open_count();
        assert_eq!(opens, 2);
        assert_eq!(scan.read(&fixture.resources[2]).unwrap(), b"cccc");
        assert_eq!(replay.0.lock().unwrap().scans[&1].open_count(), opens);
    }
    assert!(replay.0.lock().unwrap().scans.is_empty());
    replay.finish().unwrap();
}

#[test]
fn io_error_at_the_frontier_is_replayed_not_retried() {
    let fixture = Fixture::new(&[b"data"]);
    let tape = fixture.tape();
    fs::remove_file(&fixture.resources[0].source).unwrap();
    let replay = changed(tape.validate(MAX_BYTES).unwrap());
    fs::write(&fixture.resources[0].source, b"late").unwrap();
    {
        let mut scan = Scan::observed(&replay).unwrap();
        let error = scan.read(&fixture.resources[0]).unwrap_err();
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::NotFound
        );
    }
    assert!(replay.finish().unwrap().is_none());
}

#[test]
fn archive_validation_preserves_captured_bytes_and_missing_error() {
    let fixture = Fixture::new(&[]);
    let path = fixture.directory.join("test.far");
    let mut bytes = vec![0; 20];
    bytes[12..16].copy_from_slice(&16u32.to_le_bytes());
    fs::write(&path, &bytes).unwrap();
    let session = Session::capture(MAX_BYTES);
    let expected = session
        .archive(&path, 7, || crate::avatar_mount::read_archive_input(&path))
        .unwrap();
    let tape = session.finish().unwrap().unwrap();
    assert!(matches!(
        tape.validate(MAX_BYTES).unwrap(),
        Validation::Unchanged
    ));
    bytes[0] = 1;
    fs::write(&path, &bytes).unwrap();
    let updated = crate::avatar_mount::read_archive_input(&path).unwrap();
    assert_ne!(expected, updated);
    let replay = changed(tape.validate(MAX_BYTES).unwrap());
    fs::remove_file(&path).unwrap();
    assert_eq!(
        replay
            .archive(&path, 7, || panic!("captured archive was reread"))
            .unwrap(),
        updated
    );
    let tape = replay.finish().unwrap().unwrap();
    let replay = changed(tape.validate(MAX_BYTES).unwrap());
    fs::write(&path, &bytes).unwrap();
    assert_eq!(
        replay
            .archive(&path, 7, || panic!("missing archive was retried"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert!(replay.finish().unwrap().is_none());
}

#[test]
fn exceptional_capture_and_capacity_bypass_do_not_change_live_reads() {
    let fixture = Fixture::new(&[b"data"]);
    for limit in [0, 1, 32] {
        let session = Session::capture(limit);
        {
            let mut scan = Scan::observed(&session).unwrap();
            assert_eq!(scan.read(&fixture.resources[0]).unwrap(), b"data");
            fs::write(&fixture.resources[0].source, b"next").unwrap();
            assert_eq!(scan.read(&fixture.resources[0]).unwrap(), b"next");
            fs::write(&fixture.resources[0].source, b"data").unwrap();
        }
        assert!(session.finish().unwrap().is_none());
    }
    let session = Session::capture(MAX_BYTES);
    {
        let mut scan = Scan::observed(&session).unwrap();
        fs::remove_file(&fixture.resources[0].source).unwrap();
        assert!(scan.read(&fixture.resources[0]).is_err());
        fs::write(&fixture.resources[0].source, b"data").unwrap();
        assert_eq!(scan.read(&fixture.resources[0]).unwrap(), b"data");
    }
    assert!(session.finish().unwrap().is_none());
}

#[test]
fn event_count_boundary_disables_retention_without_disabling_later_reads() {
    let fixture = Fixture::new(&[b"data"]);
    for extra in [0, 1] {
        let session = Session::capture(MAX_BYTES);
        for _ in 0..MAX_EVENTS / 2 + extra {
            let scope = session.begin().unwrap();
            session.end(scope);
        }
        if extra == 0 {
            let tape = session.finish().unwrap().unwrap();
            assert_eq!(tape.statistics().0, MAX_EVENTS);
            assert!(matches!(
                tape.validate(MAX_BYTES).unwrap(),
                Validation::Unchanged
            ));
        } else {
            {
                let mut scan = Scan::observed(&session).unwrap();
                assert_eq!(scan.read(&fixture.resources[0]).unwrap(), b"data");
            }
            assert!(session.finish().unwrap().is_none());
        }
    }
}

#[test]
fn oversized_archives_and_replacement_prefixes_remain_fresh_without_retention() {
    let fixture = Fixture::new(&[b"data", b"tail"]);
    let filename = fixture.directory.join("test.far");
    let mut bytes = vec![0; 20];
    bytes[12..16].copy_from_slice(&16u32.to_le_bytes());
    fs::write(&filename, &bytes).unwrap();
    let session = Session::capture(1);
    let expected = crate::avatar_mount::read_archive_input(&filename).unwrap();
    assert_eq!(
        session
            .archive(&filename, 1, || crate::avatar_mount::read_archive_input(
                &filename
            ))
            .unwrap(),
        expected
    );
    bytes[0] = 1;
    fs::write(&filename, &bytes).unwrap();
    assert_ne!(
        session
            .archive(&filename, 1, || crate::avatar_mount::read_archive_input(
                &filename
            ))
            .unwrap(),
        expected
    );
    assert!(session.finish().unwrap().is_none());
    let tape = fixture.tape();
    fs::write(&fixture.resources[0].source, b"next").unwrap();
    let replay = changed(tape.validate(1).unwrap());
    fs::remove_file(&fixture.resources[0].source).unwrap();
    {
        let mut scan = Scan::observed(&replay).unwrap();
        assert_eq!(scan.read(&fixture.resources[0]).unwrap(), b"next");
        assert_eq!(scan.read(&fixture.resources[1]).unwrap(), b"tail");
    }
    assert!(replay.finish().unwrap().is_none());
}

#[test]
fn returned_buffers_do_not_mutate_retained_evidence() {
    let fixture = Fixture::new(&[b"data"]);
    let session = Session::capture(MAX_BYTES);
    {
        let mut scan = Scan::observed(&session).unwrap();
        let mut bytes = scan.read(&fixture.resources[0]).unwrap();
        bytes.fill(0);
    }
    let tape = session.finish().unwrap().unwrap();
    assert!(matches!(
        tape.validate(MAX_BYTES).unwrap(),
        Validation::Unchanged
    ));
}

#[test]
fn wrong_prefix_identity_and_unclosed_scope_fail_closed() {
    let fixture = Fixture::new(&[b"data"]);
    let tape = fixture.tape();
    fs::write(&fixture.resources[0].source, b"next").unwrap();
    let replay = changed(tape.validate(MAX_BYTES).unwrap());
    {
        let mut scan = Scan::observed(&replay).unwrap();
        let mut wrong = fixture.resources[0].clone();
        wrong.locator.push_str("-other");
        assert!(
            scan.read(&wrong)
                .unwrap_err()
                .to_string()
                .contains("resource changed")
        );
    }
    assert!(replay.finish().is_err());
    let session = Session::capture(MAX_BYTES);
    let scan = Scan::observed(&session).unwrap();
    assert!(session.finish().is_err());
    drop(scan);
}

#[test]
fn separate_scope_lifetimes_and_sessions_remain_independent() {
    let fixture = Fixture::new(&[b"data"]);
    let session = Session::capture(MAX_BYTES);
    for _ in 0..2 {
        let mut scan = Scan::observed(&session).unwrap();
        scan.read(&fixture.resources[0]).unwrap();
    }
    let tape = session.finish().unwrap().unwrap();
    assert!(matches!(
        tape.events.as_ref(),
        [
            Event::Begin(1),
            Event::Resource { scope: 1, .. },
            Event::End(1),
            Event::Begin(2),
            Event::Resource { scope: 2, .. },
            Event::End(2)
        ]
    ));
    assert!(matches!(
        tape.validate(MAX_BYTES).unwrap(),
        Validation::Unchanged
    ));
    let other = Session::capture(MAX_BYTES);
    assert_eq!(other.begin().unwrap(), 1);
    other.end(1);
    assert!(other.finish().unwrap().is_some());
}
