use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "sims-content-scan-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn resource(source: &Path, offset: u64, size: u64) -> Resource {
    Resource {
        locator: format!("archive.far::{offset}"),
        name: "entry.cmx.bcf".into(),
        source: source.into(),
        offset,
        size,
        unpacked_size: size,
        archived: true,
        owned: None,
    }
}
fn compare(scan: &mut Scan, r: &Resource) {
    let expected = r.read().map_err(|e| format!("{e:#}"));
    assert_eq!(scan.read(r).map_err(|e| format!("{e:#}")), expected);
}

#[test]
fn scan_reads_current_ranges_and_owned_bytes_with_identical_failures() {
    let dir = Directory::new();
    let path = dir.path("archive");
    fs::write(&path, b"first-second-third").unwrap();
    let mut scan = Scan::default();
    for offset in [0, 6, 13, 6, 0] {
        compare(&mut scan, &resource(&path, offset, 5));
    }
    #[cfg(unix)]
    assert_eq!(scan.opens, 1);
    let mut r = resource(&path, 0, 5);
    // Locator/name changes and ID reordering do not select a cached payload.
    r.name = "different.cmx".into();
    r.locator = "another-provider".into();
    compare(&mut scan, &r);
    r.offset = 13;
    compare(&mut scan, &r);
    for (offset, size, unpacked) in [
        (u64::MAX, 2, 2),
        (16, 5, 5),
        (0, 2, 3),
        (0, crate::MAX_RESOURCE + 1, crate::MAX_RESOURCE + 1),
    ] {
        let mut r = resource(&path, offset, size);
        r.unpacked_size = unpacked;
        compare(&mut scan, &r);
    }
    r.offset = 0;
    r.owned = Some(Arc::from(&b"owned"[..]));
    compare(&mut scan, &r);
    r.owned = Some(Arc::from(&b"short!"[..]));
    compare(&mut scan, &r);
    compare(&mut scan, &resource(&dir.0, 0, 1));
    compare(&mut scan, &resource(&dir.path("missing"), 0, 1));
}

#[test]
fn scan_observes_in_place_writes_replacement_removal_and_recovery() {
    use std::io::{Seek, Write};
    let dir = Directory::new();
    let path = dir.path("archive");
    fs::write(&path, b"before").unwrap();
    let r = resource(&path, 0, 6);
    let mut scan = Scan::default();
    compare(&mut scan, &r);
    let mut same_inode = fs::OpenOptions::new().write(true).open(&path).unwrap();
    same_inode.seek(std::io::SeekFrom::Start(0)).unwrap();
    same_inode.write_all(b"change").unwrap();
    compare(&mut scan, &r);
    assert_eq!(scan.read(&r).unwrap(), b"change");
    let new = dir.path("replacement");
    fs::write(&new, b"newest").unwrap();
    fs::rename(&new, &path).unwrap();
    compare(&mut scan, &r);
    assert_eq!(scan.read(&r).unwrap(), b"newest");
    fs::remove_file(&path).unwrap();
    compare(&mut scan, &r);
    fs::write(&path, b"retry!").unwrap();
    compare(&mut scan, &r);
    // A failed range does not poison a subsequent valid resource or retain bytes.
    compare(&mut scan, &resource(&path, 5, 2));
    compare(&mut scan, &r);
    fs::write(&path, b"a").unwrap();
    compare(&mut scan, &r);
    fs::write(&path, b"longer").unwrap();
    compare(&mut scan, &r);
}

#[cfg(unix)]
#[test]
fn scan_revalidates_symlink_target_and_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = Directory::new();
    let a = dir.path("a");
    let b = dir.path("b");
    let link = dir.path("link");
    fs::write(&a, b"aaaa").unwrap();
    fs::write(&b, b"bbbb").unwrap();
    symlink(&a, &link).unwrap();
    let r = resource(&link, 0, 4);
    let mut scan = Scan::default();
    compare(&mut scan, &r);
    fs::remove_file(&link).unwrap();
    symlink(&b, &link).unwrap();
    compare(&mut scan, &r);
    assert_eq!(scan.read(&r).unwrap(), b"bbbb");
    assert_eq!(scan.opens, 2);
    fs::set_permissions(&b, fs::Permissions::from_mode(0)).unwrap();
    compare(&mut scan, &r);
    fs::set_permissions(&b, fs::Permissions::from_mode(0o600)).unwrap();
    compare(&mut scan, &r);
}

#[cfg(unix)]
#[test]
fn scan_handles_are_bounded_and_end_with_the_query() {
    let dir = Directory::new();
    let resources: Vec<_> = (0..CAPACITY + 3)
        .map(|n| {
            let p = dir.path(&n.to_string());
            fs::write(&p, [n as u8]).unwrap();
            resource(&p, 0, 1)
        })
        .collect();
    let mut scan = Scan::default();
    for r in &resources {
        compare(&mut scan, r);
        assert!(scan.files.len() <= CAPACITY);
    }
    assert_eq!(scan.opens, resources.len());
    compare(&mut scan, &resources[0]);
    assert_eq!(scan.opens, resources.len() + 1);
    drop(scan);
    let mut next = Scan::default();
    assert!(next.files.is_empty());
    compare(&mut next, &resources[0]);
    assert_eq!(next.opens, 1);
}

#[cfg(unix)]
#[test]
#[ignore = "private installed descriptor read-count/timing diagnostic"]
fn installed_descriptor_reads() -> Result<()> {
    use sha2::{Digest, Sha256};
    use std::time::Instant;
    let index = crate::Index::scan(&PathBuf::from(std::env::var("SIMS_SCAN_PROFILE_ROOT")?))?;
    let out = PathBuf::from(std::env::var("SIMS_SCAN_PROFILE_OUTPUT")?);
    fs::create_dir(&out)?;
    let mut rows = vec![];
    for appearance in [false, true] {
        let resources: Vec<_> = index
            .resources
            .iter()
            .filter(|r| {
                let name = r.name.to_ascii_lowercase();
                if appearance {
                    !r.locator.to_ascii_lowercase().starts_with("creator_files/")
                        && (name.ends_with(".cmx") || name.ends_with(".cmx.bcf"))
                } else {
                    name.ends_with(".cmx.bcf")
                }
            })
            .collect();
        for repetition in 0..3 {
            let before = Instant::now();
            let expected = resources
                .iter()
                .map(|r| r.read())
                .collect::<Result<Vec<_>>>()?;
            let direct_ms = before.elapsed().as_secs_f64() * 1000.;
            let mut scan = Scan::default();
            let before = Instant::now();
            let actual = resources
                .iter()
                .map(|r| scan.read(r))
                .collect::<Result<Vec<_>>>()?;
            let scan_ms = before.elapsed().as_secs_f64() * 1000.;
            assert_eq!(actual, expected);
            let mut hash = Sha256::new();
            for bytes in &actual {
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(bytes);
            }
            rows.push(serde_json::json!({
                "appearance":appearance,"repetition":repetition,"resources":resources.len(),
                "bytes":actual.iter().map(Vec::len).sum::<usize>(),
                "direct_opens":resources.iter().filter(|r|r.owned.is_none()).count(),
                "scan_opens":scan.opens + resources.iter().filter(|r|r.owned.is_none() && !r.archived).count(),
                "peak_handle_limit":CAPACITY,"retained_handles":scan.files.len(),
                "direct_ms":direct_ms,"scan_ms":scan_ms,"sha256":format!("{:x}",hash.finalize())
            }));
        }
    }
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&rows)?)?;
    Ok(())
}
