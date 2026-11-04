use super::*;
use crate::Resource;

#[derive(Debug, PartialEq, Eq)]
struct ArchiveState {
    path: Vec<u8>,
    identity: u32,
    physical: PathBuf,
    entries: BTreeMap<(Vec<u8>, u64, u64), usize>,
}

mod original {
    use super::{ArchiveState, BTreeMap, Context, Index, Path, PathBuf, Result, ensure};
    use sims_sim::pose::clothing::name_map::Files;

    struct Archive {
        identity: u32,
        physical: PathBuf,
        entries: BTreeMap<(Vec<u8>, u64, u64), usize>,
    }

    pub(super) struct Sources<'a> {
        index: &'a Index,
        archives: BTreeMap<Vec<u8>, Archive>,
    }

    impl<'a> Sources<'a> {
        pub fn new(index: &'a Index) -> Self {
            Self {
                index,
                archives: BTreeMap::new(),
            }
        }

        pub fn bind_archive(&mut self, virtual_path: &[u8], physical: &Path) -> Result<()> {
            let path = key(virtual_path)?;
            let base = &path[..path
                .iter()
                .rposition(|&b| b == b'\\')
                .context("archive binding needs a directory prefix")?
                + 1];
            ensure!(
                !self.archives.contains_key(&path),
                "archive path already bound"
            );
            let identity = u32::try_from(self.archives.len() + 1)?;
            let mut entries = BTreeMap::new();
            for (id, resource) in self.index.resources.iter().enumerate() {
                if !resource.archived || resource.source != physical {
                    continue;
                }
                let (name, _, invalid) = encoding_rs::WINDOWS_1252.encode(&resource.name);
                ensure!(!invalid, "archive filename is not Windows-1252");
                let full = key(&[base, name.as_ref()].concat())?;
                let location = (full, resource.offset, resource.unpacked_size);
                if let Some(&old) = entries.get(&location) {
                    let old: &crate::Resource = &self.index.resources[old];
                    ensure!(old.size == resource.size, "ambiguous archive source range");
                } else {
                    entries.insert(location, id);
                }
            }
            self.archives.insert(
                path,
                Archive {
                    identity,
                    physical: physical.into(),
                    entries,
                },
            );
            Ok(())
        }

        pub fn snapshot(&self) -> Vec<ArchiveState> {
            self.archives
                .iter()
                .map(|(path, archive)| ArchiveState {
                    path: path.clone(),
                    identity: archive.identity,
                    physical: archive.physical.clone(),
                    entries: archive.entries.clone(),
                })
                .collect()
        }
    }

    fn key(path: &[u8]) -> Result<Vec<u8>> {
        Files::<()>::key(path).map_err(anyhow::Error::msg)
    }
}

struct Differential<'a> {
    actual: Sources<'a>,
    reference: original::Sources<'a>,
}

impl<'a> Differential<'a> {
    fn new(index: &'a Index) -> Self {
        Self {
            actual: Sources::new(index),
            reference: original::Sources::new(index),
        }
    }

    fn bind(&mut self, virtual_path: &[u8], physical: &Path) -> Result<(), String> {
        let actual = self
            .actual
            .bind_archive(virtual_path, physical)
            .map_err(|error| format!("{error:#}"));
        let reference = self
            .reference
            .bind_archive(virtual_path, physical)
            .map_err(|error| format!("{error:#}"));
        assert_eq!(actual, reference, "{virtual_path:?} -> {physical:?}");
        let actual = self
            .actual
            .archives
            .iter()
            .map(|(path, archive)| ArchiveState {
                path: path.clone(),
                identity: archive.identity,
                physical: archive.physical.clone(),
                entries: archive.entries.clone(),
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, self.reference.snapshot());
        reference
    }
}

fn resource(source: &Path, name: &str, archived: bool, offset: u64, size: u64) -> Resource {
    Resource {
        locator: format!("{}::{name}", source.display()),
        name: name.into(),
        source: source.into(),
        offset,
        size,
        unpacked_size: size,
        archived,
        owned: None,
    }
}

fn synthetic_index(resources: Vec<Resource>) -> Index {
    Index {
        root: "synthetic-avatar-mount".into(),
        archives: resources
            .iter()
            .filter(|resource| resource.archived)
            .map(|resource| resource.source.as_path())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        resources,
        diagnostics: vec![],
    }
}

#[test]
fn archive_source_index_matches_interleaved_sources_and_request_order() {
    let first = Path::new("physical/z.far");
    let second = Path::new("physical/a.far");
    let unused = Path::new("physical/unused.far");
    let index = synthetic_index(vec![
        resource(first, "Shared.cmx", true, 32, 8),
        resource(first, "loose\0\u{1f980}", false, 0, 1),
        resource(second, "Shared.cmx", true, 64, 9),
        resource(first, "Zed.bcf", true, 80, 4),
        resource(unused, "unrequested\0\u{1f980}", true, 0, 1),
        resource(second, "Caf\u{e9}.bcf", true, 96, 5),
        resource(first, "shared.CMX", true, 32, 8),
        resource(first, "First.cmx", true, 120, 1),
    ]);
    for reverse in [false, true] {
        let mut sources = Differential::new(&index);
        let mut requests = [
            (b"./Skins/Z.far".as_slice(), first),
            (b"./Skins/A.far", second),
        ];
        if reverse {
            requests.reverse();
        }
        for (position, (virtual_path, physical)) in requests.into_iter().enumerate() {
            sources.bind(virtual_path, physical).unwrap();
            assert_eq!(
                sources.actual.archives[&key(virtual_path).unwrap()].identity,
                position as u32 + 1
            );
        }
        let buckets = sources.actual.archive_sources.as_ref().unwrap();
        assert_eq!(buckets.len(), 3);
        assert_eq!(buckets[first], [0, 3, 6, 7]);
        assert_eq!(buckets[second], [2, 5]);
        assert_eq!(buckets[unused], [4]);
        assert!(std::ptr::eq(
            *buckets.get_key_value(first).unwrap().0,
            index.resources[0].source.as_path()
        ));
        let first_archive = &sources.actual.archives[b".\\skins\\z.far".as_slice()];
        assert_eq!(first_archive.entries.len(), 3);
        assert_eq!(
            first_archive.entries[&(b".\\skins\\shared.cmx".to_vec(), 32, 8)],
            0
        );
        let second_archive = &sources.actual.archives[b".\\skins\\a.far".as_slice()];
        assert_eq!(
            second_archive.entries[&(b".\\skins\\caf\xe9.bcf".to_vec(), 96, 5)],
            5
        );
    }
}

#[test]
fn archive_source_index_preserves_duplicate_identities_and_distinct_ranges() {
    let physical = Path::new("physical/ranges.far");
    let mut compressed = resource(physical, "same.BIN", true, 0, 4);
    compressed.unpacked_size = 9;
    let index = synthetic_index(vec![
        resource(physical, "Same.bin", true, 0, 4),
        resource(physical, "same.BIN", true, 0, 4),
        resource(physical, "SAME.bin", true, 8, 4),
        compressed,
    ]);
    let mut sources = Differential::new(&index);
    sources.bind(b"./Skins/First.far", physical).unwrap();
    sources.bind(b"./Other/Second.far", physical).unwrap();
    for (directory, archive) in [(b"./Skins/".as_slice(), 1), (b"./Other/", 2)] {
        for (offset, size, expected) in [(0, 4, Some(0)), (8, 4, Some(2))] {
            assert_eq!(
                sources
                    .actual
                    .source(
                        directory,
                        b"same.bin",
                        &Metadata {
                            archive,
                            archived: 1,
                            offset,
                            size,
                            ..Default::default()
                        }
                    )
                    .unwrap(),
                expected
            );
        }
        assert!(
            sources
                .actual
                .source(
                    directory,
                    b"same.bin",
                    &Metadata {
                        archive,
                        archived: 1,
                        offset: 0,
                        size: 9,
                        ..Default::default()
                    }
                )
                .unwrap_err()
                .to_string()
                .starts_with("compressed FAR source is unsupported:")
        );
    }
    assert_eq!(
        sources
            .bind(b".\\SKINS\\FIRST.FAR", Path::new("not-the-original.far"))
            .unwrap_err(),
        "archive path already bound"
    );
    assert_eq!(sources.actual.archives.len(), 2);
}

#[test]
fn archive_source_index_preserves_conflict_and_filename_error_precedence() {
    let physical = Path::new("physical/conflict.far");
    let valid = resource(physical, "same.bin", true, 0, 4);
    let mut conflict = resource(physical, "SAME.BIN", true, 0, 5);
    conflict.unpacked_size = 4;
    let unicode = resource(physical, "bad\u{1f980}.bin", true, 0, 4);
    let nul = resource(physical, "bad\0.bin", true, 0, 4);
    for (resources, expected) in [
        (
            vec![valid.clone(), conflict.clone(), unicode.clone()],
            "ambiguous archive source range",
        ),
        (
            vec![unicode.clone(), valid.clone(), conflict.clone()],
            "archive filename is not Windows-1252",
        ),
        (
            vec![valid.clone(), nul.clone(), conflict.clone()],
            "native file-map name contains an embedded NUL",
        ),
        (
            vec![nul, unicode, valid.clone(), conflict],
            "native file-map name contains an embedded NUL",
        ),
        (
            vec![resource(physical, "both\0\u{1f980}", true, 0, 4), valid],
            "archive filename is not Windows-1252",
        ),
    ] {
        let index = synthetic_index(resources);
        let mut sources = Differential::new(&index);
        assert_eq!(
            sources.bind(b"./Skins/Bad.far", physical).unwrap_err(),
            expected
        );
        assert!(sources.actual.archives.is_empty());
        assert!(sources.actual.archive_sources.is_some());
        assert_eq!(
            sources.bind(b"./Skins/Again.far", physical).unwrap_err(),
            expected
        );
        sources
            .bind(b"./Skins/Bad.far", Path::new("physical/missing.far"))
            .unwrap();
        assert_eq!(
            sources.actual.archives[b".\\skins\\bad.far".as_slice()].identity,
            1
        );
        assert_eq!(
            sources.bind(b"./Skins/Bad.far", physical).unwrap_err(),
            "archive path already bound"
        );
    }
}

#[test]
fn archive_source_index_defers_validation_of_unrequested_and_loose_names() {
    let good = Path::new("physical/good.far");
    let unicode = Path::new("physical/unicode.far");
    let nul = Path::new("physical/nul.far");
    let index = synthetic_index(vec![
        resource(unicode, "bad\u{1f980}.bin", true, 0, 1),
        resource(nul, "bad\0.bin", true, 0, 1),
        resource(good, "loose\u{1f980}\0.bin", false, 0, 1),
        resource(good, "good.bin", true, 0, 1),
    ]);
    let mut sources = Differential::new(&index);
    assert!(sources.actual.archive_sources.is_none());
    sources.bind(b"./Skins/Good.far", good).unwrap();
    assert_eq!(sources.actual.archive_sources.as_ref().unwrap()[good], [3]);
    assert_eq!(
        sources.bind(b"./Skins/Unicode.far", unicode).unwrap_err(),
        "archive filename is not Windows-1252"
    );
    assert_eq!(
        sources.bind(b"./Skins/Nul.far", nul).unwrap_err(),
        "native file-map name contains an embedded NUL"
    );
    sources.bind(b"./Other/Good.far", good).unwrap();
    assert_eq!(
        sources.actual.archives[b".\\other\\good.far".as_slice()].identity,
        2
    );
}

#[test]
fn archive_source_index_refuses_virtual_paths_before_lazy_construction() {
    let physical = Path::new("physical/invalid.far");
    let index = synthetic_index(vec![resource(physical, "bad\u{1f980}\0.bin", true, 0, 1)]);
    let mut sources = Differential::new(&index);
    for (virtual_path, expected) in [
        (
            b"bare.far".as_slice(),
            "archive binding needs a directory prefix",
        ),
        (b"", "archive binding needs a directory prefix"),
        (
            b"./Bad\0/Bad.far",
            "native file-map name contains an embedded NUL",
        ),
        (
            b"bare\0.far",
            "native file-map name contains an embedded NUL",
        ),
    ] {
        assert_eq!(sources.bind(virtual_path, physical).unwrap_err(), expected);
        assert!(sources.actual.archive_sources.is_none());
    }
    assert_eq!(
        sources.bind(b"./Skins/Bad.far", physical).unwrap_err(),
        "archive filename is not Windows-1252"
    );
    assert!(sources.actual.archive_sources.is_some());
}

#[test]
fn archive_source_index_binds_missing_empty_and_invalid_physical_paths_without_reads() {
    for resources in [
        vec![],
        vec![resource(
            Path::new("physical/empty.far"),
            "loose\0.bin",
            false,
            0,
            1,
        )],
    ] {
        let index = synthetic_index(resources);
        let mut sources = Differential::new(&index);
        for (virtual_path, physical) in [
            (
                b"./Skins/Missing.far".as_slice(),
                Path::new("physical/missing.far"),
            ),
            (b"./Skins/Empty.far", Path::new("physical/empty.far")),
            (b"./Skins/EmptyPath.far", Path::new("")),
            (b"./Skins/NulPath.far", Path::new("physical/invalid\0.far")),
        ] {
            sources.bind(virtual_path, physical).unwrap();
            let archive = &sources.actual.archives[&key(virtual_path).unwrap()];
            assert!(archive.entries.is_empty());
            assert_eq!(archive.physical.as_os_str(), physical.as_os_str());
        }
        assert!(sources.actual.archive_sources.as_ref().unwrap().is_empty());
        assert_eq!(sources.actual.archives.len(), 4);
    }
    for physical in [Path::new(""), Path::new("physical/invalid\0.far")] {
        let index = synthetic_index(vec![resource(physical, "valid.bin", true, 0, 1)]);
        let mut sources = Differential::new(&index);
        sources.bind(b"./Skins/Indexed.far", physical).unwrap();
        assert_eq!(
            sources.actual.archives[b".\\skins\\indexed.far".as_slice()]
                .entries
                .len(),
            1
        );
    }
}

#[test]
fn archive_source_index_uses_path_equality_without_canonicalizing_aliases() {
    let physical = Path::new("physical/first.far");
    let dotted = Path::new("physical/./first.far");
    let repeated = Path::new("physical//first.far");
    let parent = Path::new("physical/../physical/first.far");
    let leading = Path::new("./physical/first.far");
    let case = Path::new("physical/FIRST.far");
    assert_eq!(physical, dotted);
    assert_eq!(physical, repeated);
    assert_ne!(physical, parent);
    assert_ne!(physical, leading);
    assert_ne!(physical, case);
    let index = synthetic_index(vec![
        resource(physical, "base.bin", true, 0, 1),
        resource(dotted, "dot.bin", true, 0, 1),
        resource(repeated, "repeat.bin", true, 0, 1),
        resource(parent, "parent.bin", true, 0, 1),
        resource(leading, "leading.bin", true, 0, 1),
        resource(case, "case.bin", true, 0, 1),
    ]);
    let mut sources = Differential::new(&index);
    for (virtual_path, requested, expected) in [
        (
            b"./Skins/Alias.far".as_slice(),
            Path::new("physical//./first.far/"),
            vec![0, 1, 2],
        ),
        (b"./Skins/Parent.far", parent, vec![3]),
        (b"./Skins/Leading.far", leading, vec![4]),
        (b"./Skins/Case.far", case, vec![5]),
    ] {
        sources.bind(virtual_path, requested).unwrap();
        let archive = &sources.actual.archives[&key(virtual_path).unwrap()];
        assert_eq!(archive.physical.as_os_str(), requested.as_os_str());
        assert_eq!(
            archive.entries.values().copied().collect::<Vec<_>>(),
            expected
        );
    }
    assert_eq!(sources.actual.archive_sources.as_ref().unwrap().len(), 4);
}

#[cfg(unix)]
#[test]
fn archive_source_index_preserves_nonunicode_physical_paths() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let physical = PathBuf::from(OsString::from_vec(b"physical/nonunicode-\xff.far".to_vec()));
    let index = synthetic_index(vec![resource(&physical, "valid.bin", true, 0, 1)]);
    let mut sources = Differential::new(&index);
    sources.bind(b"./Skins/Nonunicode.far", &physical).unwrap();
    let archive = &sources.actual.archives[b".\\skins\\nonunicode.far".as_slice()];
    assert_eq!(archive.physical.as_os_str(), physical.as_os_str());
    assert_eq!(archive.entries[&(b".\\skins\\valid.bin".to_vec(), 0, 1)], 0);
}

#[test]
fn archive_source_index_reuses_buckets_only_within_its_borrowed_index() {
    let physical = Path::new("physical/reused.far");
    let index = synthetic_index(vec![resource(physical, "first.bin", true, 0, 1)]);
    let before = serde_json::to_vec(&index.resources).unwrap();
    let mut first = Differential::new(&index);
    let second = Differential::new(&index);
    first.bind(b"./Skins/First.far", physical).unwrap();
    assert!(second.actual.archive_sources.is_none());
    let allocation = first.actual.archive_sources.as_ref().unwrap()[physical].as_ptr();
    for virtual_path in [b"./Skins/Second.far".as_slice(), b"./Other/First.far"] {
        first.bind(virtual_path, physical).unwrap();
        assert_eq!(
            first.actual.archive_sources.as_ref().unwrap()[physical].as_ptr(),
            allocation
        );
    }
    assert_eq!(
        first.bind(b"./Skins/First.far", physical).unwrap_err(),
        "archive path already bound"
    );
    assert_eq!(
        first.actual.archive_sources.as_ref().unwrap()[physical].as_ptr(),
        allocation
    );
    drop(first);
    drop(second);
    assert_eq!(serde_json::to_vec(&index.resources).unwrap(), before);
    let mut fresh = Differential::new(&index);
    assert!(fresh.actual.archive_sources.is_none());
    fresh.bind(b"./Skins/Fresh.far", physical).unwrap();
    assert_eq!(
        fresh.actual.archives[b".\\skins\\fresh.far".as_slice()].identity,
        1
    );
    let replacement = synthetic_index(vec![resource(physical, "replacement.bin", true, 8, 2)]);
    let mut replacement_sources = Differential::new(&replacement);
    assert!(replacement_sources.actual.archive_sources.is_none());
    replacement_sources
        .bind(b"./Skins/Fresh.far", physical)
        .unwrap();
    let archive = &replacement_sources.actual.archives[b".\\skins\\fresh.far".as_slice()];
    assert_eq!(archive.entries.len(), 1);
    assert_eq!(
        archive.entries[&(b".\\skins\\replacement.bin".to_vec(), 8, 2)],
        0
    );
    assert_eq!(serde_json::to_vec(&index.resources).unwrap(), before);
}

fn enumeration(sources: &mut Sources<'_>) -> Vec<(Vec<u8>, u32)> {
    let mut search = DirectoryHost::first(sources, b".\\supplied\\*.*")
        .unwrap()
        .unwrap();
    let mut entries = vec![];
    while let Some(found) = DirectoryHost::next(sources, &mut search).unwrap() {
        entries.push((found.name, found.attributes));
    }
    DirectoryHost::close(sources, search).unwrap();
    entries
}

#[test]
fn archive_source_index_leaves_loose_bindings_discovery_and_failures_untouched() {
    let physical = Path::new("physical/archive.far");
    let index = synthetic_index(vec![
        resource(Path::new("physical/loose.bin"), "loose.bin", false, 0, 1),
        resource(physical, "archived.bin", true, 0, 1),
    ]);
    let mut sources = Differential::new(&index);
    sources
        .actual
        .bind_loose(b"./Supplied/Loose.bin", 0)
        .unwrap();
    sources
        .actual
        .set_directory(
            b"./Supplied/",
            [
                (b".".as_slice(), 16),
                (b"..", 16),
                (b"Z.far", 0),
                (b"Loose.bin", 1),
                (b"A.far", 0),
            ]
            .into_iter()
            .map(|(name, attributes)| Found {
                name: name.to_vec(),
                attributes,
            })
            .collect(),
        )
        .unwrap();
    ArchiveHost::open_failed(&mut sources.actual, b".\\supplied\\absent.far").unwrap();
    assert!(
        ArchiveHost::open(&mut sources.actual, b".\\supplied\\unbound.far")
            .unwrap()
            .is_none()
    );
    let before = enumeration(&mut sources.actual);
    assert_eq!(
        before
            .iter()
            .map(|(name, _)| name.as_slice())
            .collect::<Vec<_>>(),
        [b"..".as_slice(), b"Z.far", b"Loose.bin", b"A.far"]
    );
    assert!(sources.actual.archive_sources.is_none());
    let loose = sources.actual.loose.clone();
    let failures = sources.actual.open_failures.clone();
    sources.bind(b"./Supplied/Z.far", physical).unwrap();
    sources.bind(b"./Supplied/A.far", physical).unwrap();
    assert_eq!(enumeration(&mut sources.actual), before);
    assert_eq!(sources.actual.loose, loose);
    assert_eq!(sources.actual.open_failures, failures);
    assert_eq!(
        sources
            .actual
            .source(b"./Supplied/", b"Loose.bin", &Metadata::default())
            .unwrap(),
        Some(0)
    );
    assert!(DirectoryHost::first(&mut sources.actual, b".\\absent\\*.*").is_err());
}
