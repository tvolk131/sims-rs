use super::*;
use sims_formats::avatar::index::Kind;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub(crate) fn retained(memo: &Memo) -> bool {
    memo.entry.is_some()
}

pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
    pub(crate) index: Arc<Index>,
}

fn string(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.push(value.len() as u8);
    bytes.extend(value);
}

fn appearances(names: &[&[u8]]) -> Vec<u8> {
    let mut bytes = 0i32.to_le_bytes().to_vec();
    bytes.extend((names.len() as i32).to_le_bytes());
    for name in names {
        string(&mut bytes, name);
        for word in [7u32, 0, 0] {
            bytes.extend(word.to_le_bytes());
        }
    }
    bytes.extend(0i32.to_le_bytes());
    bytes
}

fn chunk(tag: &[u8; 4], id: u16, payload: &[u8]) -> Vec<u8> {
    let mut bytes = tag.to_vec();
    bytes.extend(((76 + payload.len()) as u32).to_be_bytes());
    bytes.extend(id.to_be_bytes());
    bytes.extend(0u16.to_be_bytes());
    bytes.resize(76, 0);
    bytes.extend(payload);
    bytes
}

fn object(chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1".to_vec();
    bytes.resize(64, 0);
    for chunk in chunks {
        bytes.extend(chunk);
    }
    bytes
}

fn archive(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = b"FAR!byAZ".to_vec();
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(0u32.to_le_bytes());
    let mut manifest = (entries.len() as i32).to_le_bytes().to_vec();
    for (name, payload) in entries {
        for word in [payload.len(), payload.len(), bytes.len(), name.len()] {
            manifest.extend((word as u32).to_le_bytes());
        }
        manifest.extend(name.as_bytes());
        bytes.extend(payload);
    }
    let offset = bytes.len() as u32;
    bytes[12..16].copy_from_slice(&offset.to_le_bytes());
    bytes.extend(manifest);
    bytes
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "sims-startup-memo-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        for binding in complete_collection_asset_bindings(&root) {
            fs::create_dir_all(binding.physical).unwrap();
        }
        for directory in ["Global", "Objects", "Skins", "Animation"] {
            fs::create_dir_all(root.join("GameData_ALL_Recursive").join(directory)).unwrap();
        }
        fs::write(
            root.join("GameData_ALL_Recursive/Global/Global.iff"),
            object(&[]),
        )
        .unwrap();
        let mut definition = vec![0; 216];
        definition[..4].copy_from_slice(&136u32.to_le_bytes());
        definition[28..32].copy_from_slice(&17u32.to_le_bytes());
        fs::write(
            root.join("GameData_ALL_Recursive/Objects/Objects.far"),
            archive(&[
                ("object.iff", object(&[chunk(b"OBJD", 1, &definition)])),
                ("ignored.bmp", vec![0x61; 129]),
            ]),
        )
        .unwrap();
        fs::write(
            root.join("GameData_ALL_Recursive/Skins/startup.cmx.bcf"),
            appearances(&[b"eager"]),
        )
        .unwrap();
        let mut ndx = 0i32.to_le_bytes().to_vec();
        ndx.extend(2i32.to_le_bytes());
        for name in [b"first".as_slice(), b"neighbor"] {
            string(&mut ndx, name);
            string(&mut ndx, b"lazy.cmx");
        }
        ndx.extend(0i32.to_le_bytes());
        fs::write(
            root.join("GameData_ALL_Recursive/Animation/startup.ndx"),
            ndx,
        )
        .unwrap();
        fs::write(
            root.join("GameData_ALL_Recursive/Animation/lazy.cmx.bcf"),
            appearances(&[b"first", b"neighbor", b"extra"]),
        )
        .unwrap();
        let index = Arc::new(Index::scan(&root).unwrap());
        Self { root, index }
    }

    pub(crate) fn write(&self, relative: &str, bytes: &[u8]) {
        fs::write(self.root.join(relative), bytes).unwrap();
    }

    fn baseline(&self) -> (Loaded, Directories) {
        crate::clothing_catalog::installed_catalog(&self.index).unwrap()
    }

    fn assert_matches(&self, result: &(Loaded, Directories)) {
        assert_eq!(format!("{result:?}"), format!("{:?}", self.baseline()));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn cold_warm_disabled_evicted_and_other_index_match_complete_startup_state() {
    let fixture = Fixture::new();
    let mut memo = Memo::new(fixture.index.clone());
    let cold = memo.installed_catalog(&fixture.index).unwrap();
    fixture.assert_matches(&cold);
    assert!(memo.entry.is_some());
    assert!(cold.0.catalog.get(Kind::Appearance, b"eager").is_some());
    assert!(
        cold.0
            .catalog
            .get(Kind::Appearance, b"first")
            .unwrap()
            .asset
            .is_none()
    );
    for _ in 0..3 {
        fixture.assert_matches(&memo.installed_catalog(&fixture.index).unwrap());
    }
    memo.entry = None;
    fixture.assert_matches(&memo.installed_catalog(&fixture.index).unwrap());
    memo.entry = None;
    memo.limit = 0;
    fixture.assert_matches(&memo.installed_catalog(&fixture.index).unwrap());
    assert!(memo.entry.is_none());
    let other = Fixture::new();
    other.assert_matches(&memo.installed_catalog(&other.index).unwrap());
    assert!(Arc::ptr_eq(&memo.index, &fixture.index));
    assert!(memo.entry.is_none());
}

#[test]
fn returned_registries_do_not_share_lazy_resolution_attempts_assets_or_faults() {
    let fixture = Fixture::new();
    let mut memo = Memo::new(fixture.index.clone());
    let (mut first, directories) = memo.installed_catalog(&fixture.index).unwrap();
    let first_id = first.catalog.get(Kind::Appearance, b"first").unwrap().asset;
    assert!(first_id.is_none());
    first
        .catalog
        .resolve(&fixture.index, &directories, Kind::Appearance, b"first")
        .unwrap()
        .unwrap();
    assert!(first.catalog.get(Kind::Appearance, b"extra").is_some());
    assert!(
        first
            .catalog
            .get(Kind::Appearance, b"neighbor")
            .unwrap()
            .asset
            .is_some()
    );
    let (mut next, next_directories) = memo.installed_catalog(&fixture.index).unwrap();
    fixture.assert_matches(&(next.clone(), next_directories.clone()));
    assert!(next.catalog.get(Kind::Appearance, b"extra").is_none());
    fixture.write("GameData_ALL_Recursive/Animation/lazy.cmx.bcf", &[0; 2]);
    let error = next.catalog.resolve(
        &fixture.index,
        &next_directories,
        Kind::Appearance,
        b"first",
    );
    assert!(error.is_err() || error.unwrap().is_none());
    fixture.assert_matches(&memo.installed_catalog(&fixture.index).unwrap());
}

#[test]
fn changed_startup_bytes_with_restored_mtime_rebuild_the_exact_new_template() {
    let fixture = Fixture::new();
    let mut memo = Memo::new(fixture.index.clone());
    memo.installed_catalog(&fixture.index).unwrap();
    let filename = fixture
        .root
        .join("GameData_ALL_Recursive/Skins/startup.cmx.bcf");
    let modified = filename.metadata().unwrap().modified().unwrap();
    fixture.write(
        "GameData_ALL_Recursive/Skins/startup.cmx.bcf",
        &appearances(&[b"fresh"]),
    );
    fs::OpenOptions::new()
        .write(true)
        .open(&filename)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let result = memo.installed_catalog(&fixture.index).unwrap();
    fixture.assert_matches(&result);
    assert!(result.0.catalog.get(Kind::Appearance, b"eager").is_none());
    assert!(result.0.catalog.get(Kind::Appearance, b"fresh").is_some());
    fixture.assert_matches(&memo.installed_catalog(&fixture.index).unwrap());
}

#[test]
fn namespace_changes_cannot_authorize_a_hit_or_hide_unindexed_sources() {
    let fixture = Fixture::new();
    let mut memo = Memo::new(fixture.index.clone());
    memo.installed_catalog(&fixture.index).unwrap();
    let empty = fixture.root.join("GameData_ALL_Recursive/Skins/empty");
    fs::create_dir(&empty).unwrap();
    fixture.assert_matches(&memo.installed_catalog(&fixture.index).unwrap());
    fs::remove_dir(empty).unwrap();
    fixture.assert_matches(&memo.installed_catalog(&fixture.index).unwrap());
    fixture.write("GameData_ALL_Recursive/Skins/unindexed.bmp", b"new source");
    let baseline = crate::clothing_catalog::installed_catalog(&fixture.index).unwrap_err();
    let actual = memo.installed_catalog(&fixture.index).unwrap_err();
    assert_eq!(format!("{actual:#}"), format!("{baseline:#}"));
    assert!(actual.to_string().contains("not indexed"));
    assert!(memo.entry.is_none());
    fs::remove_file(
        fixture
            .root
            .join("GameData_ALL_Recursive/Skins/unindexed.bmp"),
    )
    .unwrap();
    fixture.assert_matches(&memo.installed_catalog(&fixture.index).unwrap());
}

#[test]
fn early_decode_error_keeps_precedence_over_a_later_truncated_index_source() {
    let fixture = Fixture::new();
    let mut memo = Memo::new(fixture.index.clone());
    memo.installed_catalog(&fixture.index).unwrap();
    let mut bytes = appearances(&[b"eager"]);
    bytes[..4].copy_from_slice(&65i32.to_le_bytes());
    fixture.write("GameData_ALL_Recursive/Skins/startup.cmx.bcf", &bytes);
    fixture.write("GameData_ALL_Recursive/Animation/startup.ndx", &[]);
    let expected = crate::clothing_catalog::installed_catalog(&fixture.index).unwrap_err();
    let actual = memo.installed_catalog(&fixture.index).unwrap_err();
    assert_eq!(format!("{actual:#}"), format!("{expected:#}"));
    assert!(actual.to_string().contains("catalog group"));
    assert!(memo.entry.is_none());
}

#[cfg(unix)]
#[test]
fn fresh_namespace_rejects_symlinks_and_case_collisions_before_tape_validation() {
    let fixture = Fixture::new();
    let mut memo = Memo::new(fixture.index.clone());
    memo.installed_catalog(&fixture.index).unwrap();
    let link = fixture.root.join("GameData_ALL_Recursive/Skins/linked");
    std::os::unix::fs::symlink(fixture.root.join("GameData_ALL_Recursive/Global"), &link).unwrap();
    let expected = crate::clothing_catalog::installed_catalog(&fixture.index).unwrap_err();
    assert_eq!(
        format!("{:#}", memo.installed_catalog(&fixture.index).unwrap_err()),
        format!("{expected:#}")
    );
    fs::remove_file(link).unwrap();
    fs::create_dir(fixture.root.join("GameData_ALL_Recursive/Skins/Empty")).unwrap();
    if let Err(error) = fs::create_dir(fixture.root.join("GameData_ALL_Recursive/Skins/empty")) {
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        return;
    }
    let expected = crate::clothing_catalog::installed_catalog(&fixture.index).unwrap_err();
    assert_eq!(
        format!("{:#}", memo.installed_catalog(&fixture.index).unwrap_err()),
        format!("{expected:#}")
    );
}

#[test]
fn every_read_frontier_matches_the_native_controller_without_reobserving_the_prefix() {
    use std::{
        collections::BTreeMap,
        io::{Read, Seek, SeekFrom, Write},
        path::Path,
        sync::Mutex,
    };

    type Log = Arc<Mutex<Vec<(PathBuf, u64)>>>;

    #[derive(Debug, Clone, Copy)]
    enum Mutation {
        Flip,
        Truncate,
        Remove,
    }

    fn observer(
        target: usize,
        mutation: Mutation,
        log: Log,
    ) -> impl FnMut(usize, &Path, u64) + Send + 'static {
        move |position, filename, offset| {
            log.lock().unwrap().push((filename.into(), offset));
            if position == target {
                if matches!(mutation, Mutation::Remove) {
                    fs::remove_file(filename).unwrap();
                    return;
                }
                let mut file = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(filename)
                    .unwrap();
                if matches!(mutation, Mutation::Truncate) {
                    file.set_len(offset).unwrap();
                    return;
                }
                file.seek(SeekFrom::Start(offset)).unwrap();
                let mut byte = [0];
                file.read_exact(&mut byte).unwrap();
                file.seek(SeekFrom::Start(offset)).unwrap();
                file.write_all(&[byte[0] ^ 1]).unwrap();
            }
        }
    }

    fn outcome(result: Result<(Loaded, Directories)>) -> String {
        match result {
            Ok(state) => format!("ok: {state:?}"),
            Err(error) => format!("error: {error:#}"),
        }
    }

    let fixture = Fixture::new();
    let mut memo = Memo::new(fixture.index.clone());
    memo.installed_catalog(&fixture.index).unwrap();
    let entry = memo.entry.take().unwrap();
    let unchanged_log = Log::default();
    assert!(matches!(
        entry
            .tape
            .validate_observing(
                MAX_BYTES,
                observer(usize::MAX, Mutation::Flip, unchanged_log.clone())
            )
            .unwrap(),
        Validation::Unchanged
    ));
    let requests = unchanged_log.lock().unwrap().clone();
    assert!(requests.len() >= 4);
    let originals: BTreeMap<_, _> = requests
        .iter()
        .map(|(filename, _)| (filename.clone(), fs::read(filename).unwrap()))
        .collect();
    for mutation in [Mutation::Flip, Mutation::Truncate, Mutation::Remove] {
        for target in 0..requests.len() {
            for (filename, bytes) in &originals {
                fs::write(filename, bytes).unwrap();
            }
            let expected_log = Log::default();
            let inputs = Session::capture(MAX_BYTES);
            inputs.observe_before(observer(target, mutation, expected_log.clone()));
            let installation = Installation::new(
                &fixture.index,
                complete_collection_asset_bindings(&fixture.root),
            )
            .unwrap();
            let expected = outcome(crate::clothing_catalog::installed_catalog_from(
                &fixture.index,
                installation,
                Some(&inputs),
            ));
            inputs.finish().unwrap();
            for (filename, bytes) in &originals {
                fs::write(filename, bytes).unwrap();
            }
            memo.entry = Some(Entry {
                namespace: entry.namespace.clone(),
                tape: entry.tape.clone(),
                template: entry.template.clone(),
            });
            let actual_log = Log::default();
            let capture_log = actual_log.clone();
            let validate_log = actual_log.clone();
            let actual = outcome(memo.installed_catalog_using(
                &fixture.index,
                move |limit| {
                    let inputs = Session::capture(limit);
                    inputs.observe_before(observer(target, mutation, capture_log));
                    Ok(inputs)
                },
                move |tape, limit| {
                    tape.validate_observing(limit, observer(target, mutation, validate_log))
                },
            ));
            assert_eq!(
                actual, expected,
                "native outcome at observation {target}, {mutation:?}"
            );
            assert_eq!(
                *actual_log.lock().unwrap(),
                *expected_log.lock().unwrap(),
                "source observation order at frontier {target}, {mutation:?}"
            );
        }
    }
}

#[test]
fn unchanged_hit_validates_the_whole_transcript_without_reconstruction() {
    use std::sync::Mutex;
    let fixture = Fixture::new();
    let mut memo = Memo::new(fixture.index.clone());
    memo.installed_catalog(&fixture.index).unwrap();
    let reference = memo.entry.as_ref().unwrap().tape.clone();
    let expected_log = Arc::new(Mutex::new(Vec::new()));
    let expected_output = expected_log.clone();
    assert!(matches!(
        reference
            .validate_observing(MAX_BYTES, move |position, filename, offset| {
                expected_output
                    .lock()
                    .unwrap()
                    .push((position, filename.to_path_buf(), offset));
            })
            .unwrap(),
        Validation::Unchanged
    ));
    let actual_log = Arc::new(Mutex::new(Vec::new()));
    let actual_output = actual_log.clone();
    let actual = memo
        .installed_catalog_using(
            &fixture.index,
            |_| anyhow::bail!("unchanged hit attempted reconstruction"),
            move |tape, limit| {
                let validation =
                    tape.validate_observing(limit, move |position, filename, offset| {
                        actual_output.lock().unwrap().push((
                            position,
                            filename.to_path_buf(),
                            offset,
                        ));
                    })?;
                assert!(matches!(validation, Validation::Unchanged));
                Ok(validation)
            },
        )
        .unwrap();
    fixture.assert_matches(&actual);
    assert!(memo.entry.as_ref().unwrap().tape.same_reference(&reference));
    assert_eq!(*expected_log.lock().unwrap(), *actual_log.lock().unwrap());
}
