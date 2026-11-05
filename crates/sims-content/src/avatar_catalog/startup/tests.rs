#![cfg(unix)]

use super::*;
use crate::Resource;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture {
    root: PathBuf,
    index: Index,
    directories: Directories,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "sims-startup-scan-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("Catalog.far");
        std::fs::write(&source, b"firstother").unwrap();
        let mut directories = Directories::default();
        let resources = ["one.bcf", "two.ndx"]
            .into_iter()
            .enumerate()
            .map(|(ordinal, name)| {
                directories
                    .insert_first(b"./Catalog/", name.as_bytes(), ordinal)
                    .unwrap();
                Resource {
                    locator: format!("Catalog.far::{name}"),
                    name: name.into(),
                    source: source.clone(),
                    offset: ordinal as u64 * 5,
                    size: 5,
                    unpacked_size: 5,
                    archived: true,
                    owned: None,
                }
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
            directories,
        }
    }

    fn open(
        &self,
        scan: &mut crate::resource_scan::Scan,
        name: &[u8],
    ) -> Result<(u32, Option<Vec<u8>>), String> {
        let mut stream = Stream {
            source: None,
            bytes: None,
            binary: true,
        };
        let status = catalog_open::Host::open(
            &mut Opening {
                index: &self.index,
                directories: &self.directories,
                stream: &mut stream,
                scan,
            },
            name,
        )?;
        Ok((status, stream.bytes))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn startup_streams_reuse_handles_but_read_fresh_bytes_for_every_open() {
    let fixture = Fixture::new();
    let mut scan = crate::resource_scan::Scan::default();
    for (name, expected) in [
        (b"./Catalog/one.bcf".as_slice(), b"first"),
        (b"./Catalog/two.ndx", b"other"),
    ] {
        assert_eq!(
            fixture.open(&mut scan, name).unwrap(),
            (0, Some(expected.to_vec()))
        );
    }
    assert_eq!(scan.open_count(), 1);
    let source = fixture.root.join("Catalog.far");
    std::fs::write(&source, b"freshlater").unwrap();
    assert_eq!(
        fixture.open(&mut scan, b"./Catalog/one.bcf").unwrap(),
        (0, Some(b"fresh".to_vec()))
    );
    let replacement = fixture.root.join("replacement.far");
    std::fs::write(&replacement, b"againspare").unwrap();
    std::fs::rename(replacement, &source).unwrap();
    assert_eq!(
        fixture.open(&mut scan, b"./Catalog/two.ndx").unwrap(),
        (0, Some(b"spare".to_vec()))
    );
    assert_eq!(scan.open_count(), 3);
    let mut independent = crate::resource_scan::Scan::default();
    assert_eq!(
        fixture
            .open(&mut independent, b"./Catalog/one.bcf")
            .unwrap(),
        (0, Some(b"again".to_vec()))
    );
    assert_eq!(independent.open_count(), 1);
}

#[test]
fn startup_warm_reads_preserve_native_missing_status_and_source_errors() {
    let fixture = Fixture::new();
    let mut scan = crate::resource_scan::Scan::default();
    assert_eq!(fixture.open(&mut scan, b"./Catalog/one.bcf").unwrap().0, 0);
    let source = fixture.root.join("Catalog.far");
    std::fs::write(&source, b"short").unwrap();
    let expected = format!("{:#}", fixture.index.resources[1].read().unwrap_err());
    assert_eq!(
        fixture.open(&mut scan, b"./Catalog/two.ndx").unwrap_err(),
        expected
    );
    std::fs::remove_file(&source).unwrap();
    assert_eq!(
        fixture.open(&mut scan, b"./Catalog/one.bcf").unwrap(),
        (5, None)
    );
    assert_eq!(
        fixture.open(&mut scan, b"./Catalog/absent.bcf").unwrap(),
        (5, None)
    );
    std::fs::write(&source, b"newlyvalid").unwrap();
    assert_eq!(
        fixture.open(&mut scan, b"./Catalog/two.ndx").unwrap(),
        (0, Some(b"valid".to_vec()))
    );
}
