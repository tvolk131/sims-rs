//! Public storage contract negatives. Every mutation is inside a test TempDir;
//! these checks do not execute game state or claim native compatibility.
use sims_fixtures::{BlobRef, Compression, Store};
use std::{
    fs,
    io::{self, Read},
};
use tempfile::TempDir;

fn store() -> (TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::create(temp.path().join("store")).unwrap();
    (temp, store)
}

fn corrupt(store: &Store, reference: &BlobRef, mutate: impl FnOnce(&mut Vec<u8>)) {
    let path = store.blob_path(reference).unwrap();
    let mut encoded = fs::read(&path).unwrap();
    mutate(&mut encoded);
    fs::write(path, encoded).unwrap();
}

#[test]
fn codecs_share_decoded_identity_and_preserve_opaque_bytes() {
    let payload: Vec<_> = (0..=255).chain((0..=255).rev()).collect();
    let (_temp, store) = store();
    let raw = store.put_bytes(&payload, Compression::Raw).unwrap();
    let gzip = store.put_bytes(&payload, Compression::Gzip).unwrap();
    assert_eq!(raw.sha256, gzip.sha256);
    assert_eq!(raw.bytes, gzip.bytes);
    assert_eq!(raw.bytes, payload.len() as u64);
    assert_eq!(
        store.read_bytes(&gzip, payload.len() as u64).unwrap(),
        payload
    );
    store.verify_blob(&raw).unwrap();
}

#[test]
fn zero_length_payloads_roundtrip_in_both_codecs() {
    for compression in [Compression::Raw, Compression::Gzip] {
        let (_temp, store) = store();
        let reference = store.put_bytes(&[], compression).unwrap();
        assert_eq!(reference.bytes, 0);
        assert_eq!(store.read_bytes(&reference, 0).unwrap(), Vec::<u8>::new());
        store.verify_blob(&reference).unwrap();
    }
}

#[test]
fn caller_length_and_decode_limit_are_enforced() {
    let (_temp, store) = store();
    let reference = store.put_bytes(&[7; 4096], Compression::Gzip).unwrap();
    assert!(store.read_bytes(&reference, 4095).is_err());
    for bytes in [0, 4095, 4097, u64::MAX] {
        let wrong = BlobRef {
            sha256: reference.sha256.clone(),
            bytes,
        };
        assert!(
            store.verify_blob(&wrong).is_err(),
            "accepted caller length {bytes}"
        );
    }
    assert_eq!(store.read_bytes(&reference, 4096).unwrap(), vec![7; 4096]);
}

#[test]
fn malformed_hashes_cannot_be_used_as_paths() {
    let (_temp, store) = store();
    for sha256 in [
        "../escape".to_owned(),
        "a/".repeat(32),
        "g".repeat(64),
        "A".repeat(64),
        "a".repeat(63),
        "a".repeat(65),
    ] {
        let reference = BlobRef { sha256, bytes: 0 };
        assert!(store.blob_path(&reference).is_err());
        assert!(store.read_bytes(&reference, 0).is_err());
        assert!(store.verify_blob(&reference).is_err());
    }
}

#[test]
fn corrupted_existing_blob_is_not_trusted_by_put() {
    for compression in [Compression::Raw, Compression::Gzip] {
        let (_temp, store) = store();
        let reference = store
            .put_bytes(b"existing content identity", compression)
            .unwrap();
        corrupt(&store, &reference, |bytes| {
            *bytes.last_mut().unwrap() ^= 0x55;
        });
        assert!(store.verify_blob(&reference).is_err());
        assert!(
            store
                .put_bytes(b"existing content identity", compression)
                .is_err()
        );
    }
}

#[test]
fn truncated_blobs_fail_verification() {
    for compression in [Compression::Raw, Compression::Gzip] {
        let (_temp, store) = store();
        let reference = store
            .put_bytes(b"payload that must reach its exact end", compression)
            .unwrap();
        corrupt(&store, &reference, |bytes| {
            bytes.pop();
        });
        assert!(store.verify_blob(&reference).is_err());
        assert!(store.read_bytes(&reference, 1024).is_err());
    }
}

#[test]
fn trailing_bytes_are_not_ignored_in_either_codec() {
    for compression in [Compression::Raw, Compression::Gzip] {
        let (_temp, store) = store();
        let reference = store.put_bytes(b"one exact payload", compression).unwrap();
        corrupt(&store, &reference, |bytes| {
            bytes.extend_from_slice(b"trailing garbage")
        });
        assert!(store.verify_blob(&reference).is_err());
        assert!(store.read_bytes(&reference, 1024).is_err());
    }
}

#[test]
fn a_second_empty_gzip_member_is_not_an_invisible_trailer() {
    let (_temp, store) = store();
    let reference = store
        .put_bytes(b"one gzip member only", Compression::Gzip)
        .unwrap();
    let trailer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default())
        .finish()
        .unwrap();
    corrupt(&store, &reference, |bytes| {
        bytes.extend_from_slice(&trailer)
    });
    assert!(store.verify_blob(&reference).is_err());
    assert!(store.read_bytes(&reference, 1024).is_err());
}

#[test]
fn unknown_codec_and_header_digest_mismatch_are_rejected() {
    for offset in [0, 8, 17] {
        let (_temp, store) = store();
        let reference = store
            .put_bytes(
                b"header must describe its filename identity",
                Compression::Raw,
            )
            .unwrap();
        corrupt(&store, &reference, |bytes| bytes[offset] ^= 0x55);
        assert!(
            store.verify_blob(&reference).is_err(),
            "accepted header mutation at {offset}"
        );
    }
}

struct FailingReader {
    sent: bool,
}
impl Read for FailingReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.sent {
            return Err(io::Error::other("authored interrupted input"));
        }
        self.sent = true;
        let payload = b"partial input";
        let n = output.len().min(payload.len());
        output[..n].copy_from_slice(&payload[..n]);
        Ok(n)
    }
}

#[test]
fn interrupted_input_does_not_publish_a_partial_blob() {
    let (_temp, store) = store();
    assert!(
        store
            .put_reader(FailingReader { sent: false }, Compression::Gzip)
            .is_err()
    );
    let expected = BlobRef {
        sha256: sims_fixtures::digest(b"partial input"),
        bytes: 13,
    };
    assert!(!store.blob_path(&expected).unwrap().exists());
    let complete = store
        .put_bytes(b"partial input", Compression::Gzip)
        .unwrap();
    store.verify_blob(&complete).unwrap();
}

#[test]
fn copy_blob_returns_exact_decoded_bytes() {
    let (_temp, store) = store();
    let payload = b"\0\xff\r\n  native bytes without newline";
    let reference = store.put_bytes(payload, Compression::Gzip).unwrap();
    let mut copied = Vec::new();
    store.copy_blob(&reference, &mut copied).unwrap();
    assert_eq!(copied, payload);
}

#[cfg(unix)]
#[test]
fn symlinked_blob_is_rejected_even_when_target_content_is_valid() {
    use std::os::unix::fs::symlink;
    let (temp, store) = store();
    let reference = store
        .put_bytes(b"must stay contained", Compression::Raw)
        .unwrap();
    let path = store.blob_path(&reference).unwrap();
    let outside = temp.path().join("outside-blob");
    fs::rename(&path, &outside).unwrap();
    symlink(&outside, &path).unwrap();
    assert!(store.verify_blob(&reference).is_err());
    assert!(store.read_bytes(&reference, 1024).is_err());
}

#[cfg(unix)]
#[test]
fn open_rejects_a_symlinked_store_root() {
    use std::os::unix::fs::symlink;
    let (temp, store) = store();
    let link = temp.path().join("store-alias");
    symlink(store.root(), &link).unwrap();
    assert!(Store::open(&link).is_err());
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ExactBinary {
    singles: Vec<f32>,
    doubles: Vec<f64>,
    #[serde(with = "serde_bytes")]
    bytes: Vec<u8>,
    signed: i64,
    unsigned: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    omitted: Option<String>,
    after_omission: u32,
}

#[test]
fn named_values_preserve_nan_payloads_signed_zero_subnormals_and_bytes() {
    let (_temp, store) = store();
    let input = ExactBinary {
        singles: [
            0, 0x80000000, 1, 0x007fffff, 0x7f800000, 0xff800000, 0x7fc12345, 0xffc54321,
        ]
        .into_iter()
        .map(f32::from_bits)
        .collect(),
        doubles: [
            0,
            0x8000000000000000,
            1,
            0x000fffffffffffff,
            0x7ff0000000000000,
            0xfff0000000000000,
            0x7ff8123456789abc,
            0xfff8987654321abc,
        ]
        .into_iter()
        .map(f64::from_bits)
        .collect(),
        bytes: (0..=255).collect(),
        signed: i64::MIN,
        unsigned: u64::MAX,
        omitted: None,
        after_omission: 17,
    };
    let reference = store.put_value(&input).unwrap();
    let output: ExactBinary = store.get_value(&reference).unwrap();
    assert_eq!(
        output
            .singles
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        input
            .singles
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        output
            .doubles
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        input
            .doubles
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>()
    );
    assert_eq!(output.bytes, input.bytes);
    assert_eq!(
        (output.signed, output.unsigned, output.after_omission),
        (i64::MIN, u64::MAX, 17)
    );
    assert!(output.omitted.is_none());
}

#[test]
fn a_valid_blob_with_two_messagepack_values_is_not_one_typed_value() {
    let (_temp, store) = store();
    let mut bytes = rmp_serde::to_vec_named(&serde_json::json!({"field": 7})).unwrap();
    bytes.extend_from_slice(&rmp_serde::to_vec(&serde_json::Value::Null).unwrap());
    let reference = store.put_bytes(&bytes, Compression::Gzip).unwrap();
    store.verify_blob(&reference).unwrap();
    assert!(store.get_value::<serde_json::Value>(&reference).is_err());
}

fn case(store: &Store, id: &str) -> sims_fixtures::CaseManifest {
    let source = store
        .put_bytes(b"exact source\r\n  untouched", Compression::Gzip)
        .unwrap();
    sims_fixtures::CaseManifest::minimal_import(
        id,
        "Authored storage test",
        sims_fixtures::Artifact::self_contained(source, sims_fixtures::ArtifactFormat::Raw, 1),
        vec![],
    )
}

#[test]
fn case_publication_is_idempotent_but_never_replaces_different_bytes() {
    let (_temp, store) = store();
    let original = case(&store, "collision");
    store.publish_case(&original).unwrap();
    store.publish_case(&original).unwrap();
    let mut changed = original.clone();
    changed.notes.push("different case bytes".into());
    assert!(store.publish_case(&changed).is_err());
    assert_eq!(
        serde_json::to_value(store.load_case("collision").unwrap()).unwrap(),
        serde_json::to_value(original).unwrap()
    );
}

#[test]
fn missing_dependency_fails_before_case_publication_and_keeps_existing_blobs() {
    let (_temp, store) = store();
    let mut c = case(&store, "missing-dependency");
    let source = c.source.blob.clone();
    c.source.dependency_mode = sims_fixtures::DependencyMode::Declared;
    c.source.dependencies.push(BlobRef {
        sha256: sims_fixtures::digest(b"not in this store"),
        bytes: 17,
    });
    assert!(store.publish_case(&c).is_err());
    assert!(store.load_case(&c.id).is_err());
    assert!(store.list_cases().unwrap().is_empty());
    assert_eq!(
        store.read_bytes(&source, 100).unwrap(),
        b"exact source\r\n  untouched"
    );
    let preview = store.gc_preview().unwrap();
    assert!(preview.dry_run);
    assert!(preview.unreferenced.iter().any(|x| x.blob == source));
    store.verify_blob(&source).unwrap();
}

#[test]
fn invalid_case_ids_cannot_escape_the_cases_directory() {
    let (temp, store) = store();
    let outside = temp.path().join("outside");
    fs::write(&outside, b"must not change").unwrap();
    for id in ["../outside", "a/b", "/absolute", "..", "."] {
        let c = case(&store, id);
        assert!(store.publish_case(&c).is_err());
    }
    assert_eq!(fs::read(outside).unwrap(), b"must not change");
    assert!(store.list_cases().unwrap().is_empty());
}

#[test]
fn unknown_case_schema_blocks_verification_and_gc_claims() {
    let (_temp, store) = store();
    let mut c = case(&store, "unknown-schema");
    c.schema = u32::MAX;
    assert!(store.publish_case(&c).is_err());
    c.schema = sims_fixtures::CASE_SCHEMA;
    store.publish_case(&c).unwrap();
    let path = store.root().join("cases").join(&c.id).join("manifest.json");
    let mut value = serde_json::to_value(c).unwrap();
    value["schema"] = serde_json::json!(u32::MAX);
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(store.load_case("unknown-schema").is_err());
    assert!(store.verify_all().is_err());
    assert!(store.gc_preview().is_err());
}

#[test]
fn exported_case_is_independent_after_source_blob_mutation() {
    let (temp, source_store) = store();
    let mut c = case(&source_store, "independent");
    let dependency = source_store
        .put_bytes(b"required dependency", Compression::Raw)
        .unwrap();
    c.source.dependency_mode = sims_fixtures::DependencyMode::Declared;
    c.source.dependencies.push(dependency.clone());
    source_store.publish_case(&c).unwrap();
    let orphan = source_store
        .put_bytes(b"not part of this case", Compression::Raw)
        .unwrap();
    let destination = temp.path().join("exported");
    source_store.export_case(&c.id, &destination).unwrap();
    let exported = Store::open(&destination).unwrap();
    exported.verify_all().unwrap();
    assert_eq!(
        exported.read_bytes(&dependency, 100).unwrap(),
        b"required dependency"
    );
    assert!(exported.read_bytes(&orphan, 100).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let a = fs::metadata(source_store.blob_path(&dependency).unwrap()).unwrap();
        let b = fs::metadata(exported.blob_path(&dependency).unwrap()).unwrap();
        assert_ne!(
            (a.dev(), a.ino()),
            (b.dev(), b.ino()),
            "export must not hardlink source blobs"
        );
    }
    corrupt(&source_store, &dependency, |bytes| {
        *bytes.last_mut().unwrap() ^= 0x55;
    });
    assert!(source_store.verify_all().is_err());
    exported.verify_all().unwrap();
    assert_eq!(
        exported.read_bytes(&dependency, 100).unwrap(),
        b"required dependency"
    );
}

#[test]
fn export_refuses_existing_empty_or_populated_destinations() {
    let (temp, store) = store();
    let c = case(&store, "existing-destination");
    store.publish_case(&c).unwrap();
    for (name, populated) in [("empty", false), ("populated", true)] {
        let destination = temp.path().join(name);
        fs::create_dir(&destination).unwrap();
        if populated {
            fs::write(destination.join("sentinel"), b"unchanged").unwrap();
        }
        assert!(store.export_case(&c.id, &destination).is_err());
        assert!(!destination.join("store.json").exists());
        if populated {
            assert_eq!(
                fs::read(destination.join("sentinel")).unwrap(),
                b"unchanged"
            );
        }
    }
}

#[test]
fn concurrent_exports_to_one_destination_have_exactly_one_publisher() {
    use std::sync::{Arc, Barrier};
    let (temp, store) = store();
    let c = case(&store, "concurrent-export");
    store.publish_case(&c).unwrap();
    let destination = temp.path().join("exported-once");
    let start = Arc::new(Barrier::new(3));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let store = store.clone();
        let destination = destination.clone();
        let id = c.id.clone();
        let start = start.clone();
        threads.push(std::thread::spawn(move || {
            start.wait();
            store.export_case(&id, &destination).is_ok()
        }));
    }
    start.wait();
    assert_eq!(
        threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .filter(|ok| *ok)
            .count(),
        1
    );
    let exported = Store::open(&destination).unwrap();
    exported.verify_all().unwrap();
    assert_eq!(exported.list_cases().unwrap(), vec![c.id]);
}

#[test]
fn export_cannot_nest_a_second_store_inside_its_source() {
    let (_temp, store) = store();
    let c = case(&store, "no-nested-export");
    store.publish_case(&c).unwrap();
    let destination = store.root().join("nested");
    assert!(store.export_case(&c.id, &destination).is_err());
    assert!(!destination.exists());
    store.verify_all().unwrap();
}

#[test]
fn trace_index_requires_its_complete_declared_chunk_closure() {
    use sims_fixtures::trace::{
        RecordKind, TRACE_INDEX_SCHEMA, TraceOptions, TraceRecordMeta, TraceWriter,
    };
    use sims_fixtures::{Artifact, ArtifactFormat, Boundary};
    let (_temp, store) = store();
    let mut writer = TraceWriter::new(
        &store,
        TraceOptions {
            max_records_per_chunk: 1,
            ..TraceOptions::default()
        },
    )
    .unwrap();
    for i in 0..3 {
        writer
            .append(
                &serde_json::json!({"value": i}),
                TraceRecordMeta {
                    kind: RecordKind::Observation,
                    boundary: Boundary::Unspecified,
                    source: None,
                },
            )
            .unwrap();
    }
    let index = writer.finish().unwrap();
    assert_eq!(index.blob_refs().len(), 3);
    let index_blob = store.put_value(&index).unwrap();
    let mut c = case(&store, "trace-closure");
    c.artifacts.push(Artifact::declared(
        index_blob,
        ArtifactFormat::TraceIndex,
        TRACE_INDEX_SCHEMA,
        index.blob_refs()[..2].to_vec(),
    ));
    assert!(store.publish_case(&c).is_err());
    assert!(store.list_cases().unwrap().is_empty());
    c.artifacts[0].dependencies = index.blob_refs();
    store.publish_case(&c).unwrap();
    let preview = store.gc_preview().unwrap();
    for chunk in index.blob_refs() {
        assert!(
            !preview
                .unreferenced
                .iter()
                .any(|candidate| candidate.blob == chunk)
        );
    }
    store.verify_all().unwrap();
}

fn archive_case(
    store: &Store,
    id: &str,
) -> (
    sims_fixtures::CaseManifest,
    sims_fixtures::archive::ArchiveInventory,
) {
    use sims_fixtures::archive::{ARCHIVE_SCHEMA, ArchiveFile, ArchiveInventory, OriginalMetadata};
    use sims_fixtures::{Artifact, ArtifactFormat};

    let payload = store
        .put_bytes(b"\0\xff exact archived bytes\r\n", Compression::Gzip)
        .unwrap();
    let inventory = ArchiveInventory {
        schema: ARCHIVE_SCHEMA,
        source_locator: Some("inert original locator".into()),
        total_bytes: payload.bytes,
        files: vec![ArchiveFile {
            relative_path: "nested/raw.bin".into(),
            blob: payload,
            metadata: OriginalMetadata {
                modified_unix: None,
                readonly: false,
                unix_mode: None,
                unix_device: None,
                unix_inode: None,
                unix_links: None,
            },
        }],
    };
    inventory.validate().unwrap();
    let reference = store.put_value(&inventory).unwrap();
    let case = sims_fixtures::CaseManifest::minimal_import(
        id,
        "Authored archive storage contract",
        Artifact::declared(
            reference,
            ArtifactFormat::ArchiveInventory,
            ARCHIVE_SCHEMA,
            inventory.blob_refs(),
        ),
        vec![],
    );
    (case, inventory)
}

#[test]
fn archive_inventory_requires_exact_declared_file_closure_before_publication() {
    let (_temp, store) = store();
    let (valid, inventory) = archive_case(&store, "archive-closure");
    let file = inventory.files[0].blob.clone();
    let mut missing = valid.clone();
    missing.source.dependencies.clear();
    let error = store.publish_case(&missing).unwrap_err();
    assert!(format!("{error:#}").contains("closure"));
    assert!(store.list_cases().unwrap().is_empty());
    store.verify_blob(&file).unwrap();
    store.verify_blob(&valid.source.blob).unwrap();

    let extra = store
        .put_bytes(b"not an archive file", Compression::Raw)
        .unwrap();
    let mut surplus = valid.clone();
    surplus.source.dependencies.push(extra.clone());
    let error = store.publish_case(&surplus).unwrap_err();
    assert!(format!("{error:#}").contains("closure"));
    assert!(store.list_cases().unwrap().is_empty());

    store.publish_case(&valid).unwrap();
    store.load_case(&valid.id).unwrap();
    store.verify_all().unwrap();
    let preview = store.gc_preview().unwrap();
    assert!(preview.unreferenced.iter().any(|entry| entry.blob == extra));
    assert!(!preview.unreferenced.iter().any(|entry| entry.blob == file));
}

#[test]
fn archive_artifact_and_payload_versions_fail_closed_in_publication_and_gc() {
    let (_temp, store) = store();
    let (valid, mut inventory) = archive_case(&store, "archive-schema");
    let mut future_artifact = valid.clone();
    future_artifact.source.schema = u32::MAX;
    let error = store.publish_case(&future_artifact).unwrap_err();
    assert!(format!("{error:#}").contains("schema"));

    inventory.schema = u32::MAX;
    let mut future_payload = valid.clone();
    future_payload.source.blob = store.put_value(&inventory).unwrap();
    let error = store.publish_case(&future_payload).unwrap_err();
    assert!(format!("{error:#}").contains("schema"));
    assert!(store.list_cases().unwrap().is_empty());

    store.publish_case(&valid).unwrap();
    let manifest_path = store
        .root()
        .join("cases")
        .join(&valid.id)
        .join("manifest.json");
    let mut changed = serde_json::to_value(&valid).unwrap();
    changed["source"]["blob"] = serde_json::to_value(future_payload.source.blob).unwrap();
    fs::write(manifest_path, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(store.load_case(&valid.id).is_err());
    assert!(store.verify_all().is_err());
    assert!(store.gc_preview().is_err());
    store.verify_blob(&valid.source.blob).unwrap();
    store.verify_blob(&valid.source.dependencies[0]).unwrap();
}
