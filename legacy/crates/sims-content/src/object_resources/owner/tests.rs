#![cfg(unix)]

use super::*;
use crate::{Resource, avatar_installation::Binding};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    index: Index,
}

fn iff(payload: &[u8]) -> Vec<u8> {
    let mut bytes = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1".to_vec();
    bytes.resize(64, 0);
    let mut chunk = b"TEST".to_vec();
    chunk.extend(((76 + payload.len()) as u32).to_be_bytes());
    chunk.extend(1u16.to_be_bytes());
    chunk.extend(0u16.to_be_bytes());
    chunk.resize(76, 0);
    chunk.extend(payload);
    bytes.extend(chunk);
    bytes
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "sims-mounted-scan-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("Objects.far");
        let first = iff(b"first");
        let second = iff(b"other");
        std::fs::write(&source, [&first[..], &second[..]].concat()).unwrap();
        let resources = [
            ("first.iff", 0, first.len()),
            ("second.iff", first.len(), second.len()),
        ]
        .into_iter()
        .map(|(name, offset, size)| Resource {
            locator: format!("Objects.far::{name}"),
            name: name.into(),
            source: source.clone(),
            offset: offset as u64,
            size: size as u64,
            unpacked_size: size as u64,
            archived: true,
            owned: None,
        })
        .collect();
        Self {
            index: Index {
                root: root.clone(),
                resources,
                archives: 1,
                diagnostics: vec![],
            },
            root,
        }
    }

    fn resources(&self) -> Resources<'_> {
        let installation = crate::avatar_installation::Installation::new(
            &self.index,
            vec![Binding {
                directory: b"./Base/".to_vec(),
                physical: self.root.clone(),
            }],
        )
        .unwrap();
        let mut directories = Directories::default();
        for (source, resource) in self.index.resources.iter().enumerate() {
            directories
                .insert_first(b"./Base/", resource.name.as_bytes(), source)
                .unwrap();
        }
        Resources::new(&self.index, directories, installation.directory_search())
    }

    fn replace(&self, first: &[u8], second: &[u8], rename: bool) {
        let path = self
            .root
            .join(if rename { "replacement" } else { "Objects.far" });
        std::fs::write(&path, [iff(first), iff(second)].concat()).unwrap();
        if rename {
            std::fs::rename(path, self.root.join("Objects.far")).unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn read(resources: &mut Resources<'_>, name: &[u8]) -> Vec<u8> {
    let aggregate = resources.acquire(name, 0).unwrap().unwrap();
    let chunk = resources
        .by_id(aggregate, u32::from_be_bytes(*b"TEST"), 1, 0)
        .unwrap();
    let bytes = resources.data(chunk).unwrap().to_vec();
    resources.release(aggregate).unwrap();
    assert_eq!(resources.live_counts(), (0, 0, 0));
    bytes
}

#[test]
fn components_share_bounded_archive_handles_without_sharing_payloads_or_owners() {
    let fixture = Fixture::new();
    let mut resources = fixture.resources();
    assert_eq!(read(&mut resources, b"./Base/first"), b"first");
    assert_eq!(read(&mut resources, b"./Base/second"), b"other");
    assert_eq!(read(&mut resources, b"./Base/first"), b"first");
    assert_eq!(
        resources.owner.components.scan.lock().unwrap().open_count(),
        1
    );
    let mut independent = fixture.resources();
    assert!(!Arc::ptr_eq(
        &resources.owner.components.scan,
        &independent.owner.components.scan
    ));
    assert_eq!(read(&mut independent, b"./Base/second"), b"other");
    assert_eq!(
        independent
            .owner
            .components
            .scan
            .lock()
            .unwrap()
            .open_count(),
        1
    );
}

#[test]
fn warm_mounted_reads_observe_current_writes_and_atomic_replacement() {
    let fixture = Fixture::new();
    let mut resources = fixture.resources();
    assert_eq!(read(&mut resources, b"./Base/first"), b"first");
    fixture.replace(b"fresh", b"later", false);
    assert_eq!(read(&mut resources, b"./Base/second"), b"later");
    assert_eq!(read(&mut resources, b"./Base/first"), b"fresh");
    fixture.replace(b"again", b"spare", true);
    assert_eq!(read(&mut resources, b"./Base/first"), b"again");
    assert_eq!(read(&mut resources, b"./Base/second"), b"spare");
    assert_eq!(
        resources.owner.components.scan.lock().unwrap().open_count(),
        3
    );
}

#[test]
fn warm_mounted_reads_preserve_truncation_and_missing_source_failures() {
    for remove in [false, true] {
        let fixture = Fixture::new();
        let mut resources = fixture.resources();
        assert_eq!(read(&mut resources, b"./Base/first"), b"first");
        let source = fixture.root.join("Objects.far");
        if remove {
            std::fs::remove_file(source).unwrap();
        } else {
            std::fs::write(source, b"short").unwrap();
        }
        let expected = fixture.index.resources[0].read().unwrap_err().to_string();
        let actual = resources.acquire(b"./Base/first", 0).unwrap_err();
        assert_eq!(actual, expected);
        assert!(resources.acquire(b"./Base/second", 0).is_err());
    }
}

#[test]
fn mounted_components_evict_archive_handles_after_eight_distinct_sources() {
    let mut fixture = Fixture::new();
    let bytes = iff(b"first");
    fixture.index.resources = (0..9)
        .map(|archive| {
            let source = fixture.root.join(format!("Objects{archive}.far"));
            let name = format!("entry{archive}.iff");
            std::fs::write(&source, &bytes).unwrap();
            Resource {
                locator: format!("Objects{archive}.far::{name}"),
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
    fixture.index.archives = 9;
    let mut resources = fixture.resources();
    for archive in 0..9 {
        assert_eq!(
            read(&mut resources, format!("./Base/entry{archive}").as_bytes()),
            b"first"
        );
    }
    assert_eq!(
        resources.owner.components.scan.lock().unwrap().open_count(),
        9
    );
    assert_eq!(read(&mut resources, b"./Base/entry0"), b"first");
    assert_eq!(
        resources.owner.components.scan.lock().unwrap().open_count(),
        10
    );
    assert_eq!(read(&mut resources, b"./Base/entry8"), b"first");
    assert_eq!(
        resources.owner.components.scan.lock().unwrap().open_count(),
        10
    );
}
