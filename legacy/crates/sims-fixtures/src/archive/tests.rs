use super::*;
use crate::Coverage;
use tempfile::TempDir;

fn setup() -> (TempDir, Store, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::create(temp.path().join("store")).unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    (temp, store, source)
}
fn options() -> ArchiveOptions {
    ArchiveOptions {
        id: "archive-case".into(),
        title: "Exact authored files".into(),
        source_locator: Some("authored-test-input".into()),
        compression: Compression::Gzip,
        commands: vec![vec!["archive".into(), "inert argument".into()]],
        notes: vec![],
    }
}
fn input(root: &Path, name: &str, bytes: &[u8]) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn inventory(files: &[&str]) -> ArchiveInventory {
    ArchiveInventory {
        schema: ARCHIVE_SCHEMA,
        source_locator: None,
        total_bytes: files.len() as u64,
        files: files
            .iter()
            .map(|name| ArchiveFile {
                relative_path: (*name).into(),
                blob: BlobRef {
                    sha256: crate::digest(b"x"),
                    bytes: 1,
                },
                metadata: OriginalMetadata {
                    modified_unix: None,
                    readonly: false,
                    unix_mode: None,
                    unix_device: None,
                    unix_inode: None,
                    unix_links: None,
                },
            })
            .collect(),
    }
}
#[test]
fn path_validation_rejects_aliases_and_nonadjacent_ancestors() {
    for names in [
        vec!["a", "a-b", "a/child"],
        vec!["A/child", "a"],
        vec!["a", "A"],
        vec!["A/x", "a/y"],
        vec!["a", "a"],
    ] {
        assert!(inventory(&names).validate().is_err(), "accepted {names:?}");
    }
    for bad in [
        "",
        "/absolute",
        "../parent",
        "a/../b",
        "a/./b",
        "a//b",
        "a/",
        "C:relative",
        "C:/absolute",
        r"a\b",
        "a\0b",
        "a:stream",
        "a.",
        "a ",
        "CON",
        "nul.txt",
        "x/LPT9.log",
        "x/<name>",
    ] {
        assert!(inventory(&[bad]).validate().is_err(), "accepted {bad:?}");
    }
    inventory(&["a-b", "a/child", "ordinary name.json", "é.txt"])
        .validate()
        .unwrap();
}
#[test]
fn inventory_schema_totals_and_closure_are_explicit() {
    let valid = inventory(&["a", "b"]);
    valid.validate().unwrap();
    assert_eq!(valid.blob_refs().len(), 1);
    let index = BlobRef {
        sha256: crate::digest(b"index"),
        bytes: 5,
    };
    let artifact = Artifact::declared(
        index,
        ArtifactFormat::ArchiveInventory,
        ARCHIVE_SCHEMA,
        valid.blob_refs(),
    );
    valid.validate_artifact(&artifact).unwrap();
    let mut wrong = valid.clone();
    wrong.schema += 1;
    assert!(wrong.validate().is_err());
    let mut wrong = valid.clone();
    wrong.total_bytes += 1;
    assert!(wrong.validate().is_err());
    let mut wrong = valid.clone();
    wrong.files[1].blob.bytes = 2;
    wrong.total_bytes = 3;
    assert!(wrong.validate().is_err());
    let mut wrong = artifact.clone();
    wrong.dependencies.clear();
    assert!(valid.validate_artifact(&wrong).is_err());
    let mut wrong = artifact;
    wrong.dependency_mode = DependencyMode::SelfContained;
    assert!(wrong.validate().is_err());
}
#[test]
fn exact_bytes_full_hash_metadata_and_independent_export_restore() {
    let (temp, store, source) = setup();
    let raw = b" \r\n{opaque: deliberately not JSON}\0\xff\n";
    input(&source, "nested/state.json", raw);
    input(&source, "empty.bin", b"");
    let report = archive_files(
        &store,
        &source,
        &["nested/state.json".into(), "empty.bin".into()],
        &options(),
    )
    .unwrap();
    assert_eq!(report.original_bytes, raw.len() as u64);
    let case = store.load_case("archive-case").unwrap();
    assert!(matches!(case.coverage, Coverage::Unqualified { .. }));
    assert!(case.references.is_empty());
    assert_eq!(case.provenance.commands, options().commands);
    let inv: ArchiveInventory = store.get_value(&case.source.blob).unwrap();
    assert_eq!(inv.files[0].blob.sha256, crate::digest(raw));
    assert_eq!(inv.files[0].blob.bytes, raw.len() as u64);
    assert_eq!(
        inv.files[0].metadata,
        original_metadata(&fs::metadata(source.join("nested/state.json")).unwrap())
    );
    let exported = temp.path().join("exported");
    store.export_case("archive-case", &exported).unwrap();
    // Remove original paths from reach without deleting either source tree.
    fs::rename(&source, temp.path().join("source-unavailable")).unwrap();
    fs::rename(store.root(), temp.path().join("store-unavailable")).unwrap();
    let portable = Store::open(&exported).unwrap();
    portable.verify_all().unwrap();
    let destination = temp.path().join("restored");
    let restored = restore_archive(&portable, "archive-case", &destination).unwrap();
    assert_eq!(
        fs::read(restored.payload_root.join("nested/state.json")).unwrap(),
        raw
    );
    assert_eq!(
        fs::read(restored.payload_root.join("empty.bin")).unwrap(),
        b""
    );
    fs::rename(exported, temp.path().join("export-unavailable")).unwrap();
    verify_restored(&destination).unwrap();
    let changed = destination.join("files/nested/state.json");
    fs::write(&changed, b"corrupt").unwrap();
    assert!(verify_restored(&destination).is_err());
    assert_eq!(
        fs::read(temp.path().join("source-unavailable/nested/state.json")).unwrap(),
        raw
    );
}
#[cfg(unix)]
#[test]
fn distinct_hardlink_names_are_retained_and_restores_are_independent_inodes() {
    use std::os::unix::fs::MetadataExt;
    let (temp, store, source) = setup();
    input(&source, "one", b"same original inode");
    fs::hard_link(source.join("one"), source.join("two")).unwrap();
    archive_files(&store, &source, &["one".into(), "two".into()], &options()).unwrap();
    let (inv, _) = load_inventory(&store, "archive-case").unwrap();
    assert_eq!(inv.files.len(), 2);
    assert_eq!(inv.blob_refs().len(), 1);
    assert_eq!(
        inv.files[0].metadata.unix_inode,
        inv.files[1].metadata.unix_inode
    );
    assert_eq!(
        inv.files[0].metadata.unix_device,
        inv.files[1].metadata.unix_device
    );
    assert_eq!(inv.files[0].metadata.unix_links, Some(2));
    let dest = temp.path().join("restored");
    restore_archive(&store, "archive-case", &dest).unwrap();
    let original = fs::metadata(source.join("one")).unwrap();
    let one = fs::metadata(dest.join("files/one")).unwrap();
    let two = fs::metadata(dest.join("files/two")).unwrap();
    assert_ne!((original.dev(), original.ino()), (one.dev(), one.ino()));
    assert_ne!((one.dev(), one.ino()), (two.dev(), two.ino()));
    assert_eq!(one.nlink(), 1);
    assert_eq!(two.nlink(), 1);
    fs::write(dest.join("files/one"), b"independent").unwrap();
    assert_eq!(
        fs::read(source.join("one")).unwrap(),
        b"same original inode"
    );
    assert_eq!(
        fs::read(dest.join("files/two")).unwrap(),
        b"same original inode"
    );
}
#[test]
fn mutation_during_copy_or_before_publication_never_publishes_case() {
    for late in [false, true] {
        let (_temp, store, source) = setup();
        input(&source, "one", b"before");
        input(&source, "two", b"before");
        let first = source.join("one");
        let mut seen = 0;
        let result = archive_files_inner(
            &store,
            &source,
            &["one".into(), "two".into()],
            &options(),
            &mut |path| {
                seen += 1;
                if (!late && seen == 1) || (late && seen == 2) {
                    fs::write(if late { &first } else { path }, b"change")?;
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(store.list_cases().unwrap().is_empty());
    }
}
#[cfg(unix)]
#[test]
fn symlink_root_parent_file_and_store_self_archive_are_rejected() {
    use std::os::unix::fs::symlink;
    let (temp, store, source) = setup();
    input(&source, "good/file", b"bytes");
    let linked = temp.path().join("linked-root");
    symlink(&source, &linked).unwrap();
    assert!(archive_files(&store, &linked, &["good/file".into()], &options()).is_err());
    symlink(source.join("good"), source.join("linked-parent")).unwrap();
    assert!(archive_files(&store, &source, &["linked-parent/file".into()], &options()).is_err());
    symlink(source.join("good/file"), source.join("linked-file")).unwrap();
    assert!(archive_files(&store, &source, &["linked-file".into()], &options()).is_err());
    assert!(
        archive_files(
            &store,
            temp.path(),
            &["store/store.json".into()],
            &options()
        )
        .is_err()
    );
    assert!(store.list_cases().unwrap().is_empty());
}
#[test]
fn existing_destinations_and_corrupt_blobs_do_not_publish_partial_success() {
    let (temp, store, source) = setup();
    input(&source, "a", b"payload");
    archive_files(&store, &source, &["a".into()], &options()).unwrap();
    let existing = temp.path().join("existing");
    fs::create_dir(&existing).unwrap();
    assert!(restore_archive(&store, "archive-case", &existing).is_err());
    assert!(fs::read_dir(&existing).unwrap().next().is_none());
    let file = temp.path().join("file");
    fs::write(&file, b"preserve").unwrap();
    assert!(restore_archive(&store, "archive-case", &file).is_err());
    assert_eq!(fs::read(file).unwrap(), b"preserve");
    let (inv, _) = load_inventory(&store, "archive-case").unwrap();
    fs::write(store.blob_path(&inv.files[0].blob).unwrap(), b"corrupt").unwrap();
    let dest = temp.path().join("never-published");
    assert!(restore_archive(&store, "archive-case", &dest).is_err());
    assert!(!dest.exists());
}
#[test]
fn staged_payload_inventory_and_extra_files_are_verified_before_reservation() {
    for damage in 0..3 {
        let (temp, store, source) = setup();
        input(&source, "a", b"payload");
        archive_files(&store, &source, &["a".into()], &options()).unwrap();
        let dest = temp.path().join("must-not-publish");
        let result = restore_inner(&store, "archive-case", &dest, &mut |stage| {
            match damage {
                0 => fs::write(stage.join("files/a"), b"corrupt")?,
                1 => fs::write(stage.join("inventory.msgpack"), b"corrupt")?,
                _ => fs::write(stage.join("files/unexpected"), b"extra")?,
            };
            Ok(())
        });
        assert!(result.is_err());
        assert!(!dest.exists());
        assert!(fs::read_dir(temp.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".sims-archive-stage-")
        }));
    }
}
#[cfg(unix)]
#[test]
fn privileged_permissions_are_audit_only_not_recreated() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, store, source) = setup();
    input(&source, "a", b"payload");
    fs::set_permissions(source.join("a"), fs::Permissions::from_mode(0o4755)).unwrap();
    archive_files(&store, &source, &["a".into()], &options()).unwrap();
    let (mut inv, _) = load_inventory(&store, "archive-case").unwrap();
    // Filesystems may strip setuid. Retain exactly what the host reported.
    let actual_mode = fs::metadata(source.join("a")).unwrap().permissions().mode() & 0o7777;
    assert_eq!(inv.files[0].metadata.unix_mode, Some(actual_mode));
    let dest = temp.path().join("restored");
    restore_archive(&store, "archive-case", &dest).unwrap();
    assert_eq!(
        fs::metadata(dest.join("files/a"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o600
    );
    // Separately authored metadata proves that special bits in an inventory
    // never cause chmod, without requiring a privileged host filesystem.
    inv.files[0].metadata.unix_mode = Some(0o4755);
    let blob = store.put_value(&inv).unwrap();
    let case = CaseManifest::minimal_import(
        "authored-mode",
        "Authored special permission metadata",
        Artifact::declared(
            blob,
            ArtifactFormat::ArchiveInventory,
            ARCHIVE_SCHEMA,
            inv.blob_refs(),
        ),
        vec!["Authored mode04755 metadata; not an observed source permission claim".into()],
    );
    store.publish_case(&case).unwrap();
    let authored = temp.path().join("authored-mode-restored");
    restore_archive(&store, "authored-mode", &authored).unwrap();
    assert_eq!(
        fs::metadata(authored.join("files/a"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o600
    );
}
#[test]
fn restored_bundle_rejects_missing_marker_and_unlisted_entries() {
    let (temp, store, source) = setup();
    input(&source, "a", b"payload");
    archive_files(&store, &source, &["a".into()], &options()).unwrap();
    let dest = temp.path().join("restored");
    restore_archive(&store, "archive-case", &dest).unwrap();
    fs::write(dest.join("unexpected"), b"extra").unwrap();
    assert!(verify_restored(&dest).is_err());
    fs::rename(
        dest.join("unexpected"),
        temp.path().join("extra-moved-away"),
    )
    .unwrap();
    verify_restored(&dest).unwrap();
    fs::rename(
        dest.join(COMPLETION_FILE),
        temp.path().join("marker-moved-away"),
    )
    .unwrap();
    assert!(verify_restored(&dest).is_err());
}
