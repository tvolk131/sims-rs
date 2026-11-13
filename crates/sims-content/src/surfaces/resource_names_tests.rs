use super::*;
use crate::Resource;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

// Independent pre-optimization implementation, including duplicate-read order.
fn reference(index: &Index, name: &str) -> Result<usize> {
    let candidates: Vec<_> = index
        .resources
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.name
                .rsplit(['/', '\\'])
                .next()
                .is_some_and(|s| s.eq_ignore_ascii_case(name))
        })
        .collect();
    let &(first, res) = candidates
        .first()
        .with_context(|| format!("surface file missing: {name}"))?;
    let bytes = res.read()?;
    for (_, other) in candidates.iter().skip(1) {
        ensure!(
            other.read()? == bytes,
            "ambiguous surface file {name}: {} / {}",
            res.locator,
            other.locator
        );
    }
    Ok(first)
}

fn owned(name: &str, locator: &str, bytes: &[u8]) -> Resource {
    Resource {
        name: name.into(),
        locator: locator.into(),
        source: "/unused-owned-surface".into(),
        offset: 0,
        size: bytes.len() as u64,
        unpacked_size: bytes.len() as u64,
        archived: false,
        owned: Some(Arc::from(bytes)),
    }
}
fn index(resources: Vec<Resource>) -> Index {
    Index {
        root: "/unused-surface-root".into(),
        resources,
        archives: 0,
        diagnostics: vec![],
    }
}
fn compare(index: &Index, lookup: &ResourceNames<'_>, name: &str) {
    assert_eq!(
        lookup.resource(name).map_err(|e| format!("{e:#}")),
        reference(index, name).map_err(|e| format!("{e:#}")),
        "{name:?}"
    );
}

#[test]
fn names_match_original_basename_case_order_duplicates_and_errors() {
    let mut resources = vec![
        owned("dir/BUILD.iff", "first-build", b"same"),
        owned("other\\build.iff", "second-build", b"same"),
        owned("dir/Wall.iff", "first-wall", b"wall"),
        owned("WALL.IFF", "second-wall", b"different"),
        owned("dir/Ä.iff", "unicode", b"unicode"),
        owned("dir/", "empty-basename", b"empty"),
        owned("", "other-empty", b"empty"),
        owned("dir/İ.iff", "unicode-i", b"i"),
    ];
    let mut malformed = owned("bad.iff", "bad-owned-size", b"bad");
    malformed.size += 1;
    malformed.unpacked_size += 1;
    resources.push(malformed);
    for rotation in 0..resources.len() {
        let mut entries = resources.clone();
        entries.rotate_left(rotation);
        let index = index(entries);
        let lookup = ResourceNames::new(&index);
        for name in [
            "build.iff",
            "BuIlD.iFf",
            "wall.iff",
            "Ä.iff",
            "ä.iff",
            "İ.iff",
            "i.iff",
            "",
            "bad.iff",
            "missing",
            "dir/BUILD.iff",
            "other\\build.iff",
        ] {
            compare(&index, &lookup, name);
        }
    }
}

#[test]
fn names_bucket_only_matching_ascii_folded_basenames_in_source_order() {
    let mut resources = (0..10_000)
        .map(|id| owned(&format!("folder/{id:08}.iff"), "unrelated", b"other"))
        .collect::<Vec<_>>();
    resources.insert(12, owned("dir/Floors.iff", "first", b"floor"));
    resources.insert(250, owned("other\\floors.IFF", "second", b"floor"));
    let index = index(resources);
    let lookup = ResourceNames::new(&index);
    assert_eq!(lookup.names.get("floors.iff"), Some(&vec![12, 250]));
    compare(&index, &lookup, "FLOORS.IFF");
    assert_eq!(lookup.resource("floors.iff").unwrap(), 12);
}

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(std::path::PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn names_never_cache_file_bytes_or_duplicate_validation() {
    let dir = Directory(std::env::temp_dir().join(format!(
        "sims-surface-names-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir(&dir.0).unwrap();
    let first = dir.0.join("first");
    let second = dir.0.join("second");
    let mut a = owned("BUILD.IFF", "first-provider", b"same");
    a.source = first.clone();
    a.owned = None;
    let mut b = owned("dir/build.iff", "second-provider", b"same");
    b.source = second.clone();
    b.owned = None;
    let index = index(vec![a, b]);
    let lookup = ResourceNames::new(&index);
    for (left, right) in [
        (Some(&b"same"[..]), Some(&b"same"[..])),
        (Some(&b"same"[..]), Some(&b"diff"[..])),
        (Some(&b"x"[..]), None),
        (None, Some(&b"same"[..])),
        (Some(&b"same"[..]), None),
        (Some(&b"same"[..]), Some(&b"same"[..])),
    ] {
        for (path, bytes) in [(&first, left), (&second, right)] {
            if let Some(bytes) = bytes {
                std::fs::write(path, bytes).unwrap();
            } else if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
        }
        compare(&index, &lookup, "BuIlD.iFf");
    }
    assert_eq!(lookup.resource("build.iff").unwrap(), 0);
}
