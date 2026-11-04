//! Authored CLI integration only: no original assets, game, GUI or player saves.
use serde_json::{Value, json};
use sims_fixtures::{
    ArtifactFormat, Boundary, Coverage, HashVerification, Store,
    trace::{RecordKind, TraceIndex},
};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn cli(store: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sims-fixtures"))
        .arg("--store")
        .arg(store)
        .args(args)
        .output()
        .expect("launch authored fixture CLI")
}
fn pass(store: &Path, args: &[&str]) -> Output {
    let output = cli(store, args);
    assert!(
        output.status.success(),
        "args={args:?}\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn setup() -> (TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let store = temp.path().join("store");
    pass(&store, &["init"]);
    (temp, store)
}

fn capture(store: &Path, id: &str, input: &[u8], limit: u64) -> Output {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_sims-fixtures"))
        .arg("--store")
        .arg(store)
        .args(["capture", "--id", id, "--max-bytes", &limit.to_string()])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Invalid preflight may close the pipe without consuming any bytes.
    let _ = child.stdin.take().unwrap().write_all(input);
    child.wait_with_output().unwrap()
}

#[test]
fn stdin_capture_preserves_all_bytes_without_qualifying_eof() {
    let (_temp, root) = setup();
    let raw = b" \r\n{\"tick\":1}\n{\"partial\":\"\xff";
    let result = capture(&root, "stream", raw, 1024);
    assert!(result.status.success(), "{:?}", result);
    let split = result.stdout.iter().position(|b| *b == b'\n').unwrap();
    let ready: Value = serde_json::from_slice(&result.stdout[..split]).unwrap();
    assert_eq!(ready, json!({"ready":true,"case_id":"stream"}));
    let report: Value = serde_json::from_slice(&result.stdout[split + 1..]).unwrap();
    assert_eq!(report["source"]["sha256"], sims_fixtures::digest(raw));
    assert_eq!(report["source"]["bytes"], raw.len());
    let store = Store::open(&root).unwrap();
    let case = store.load_case("stream").unwrap();
    assert_eq!(store.read_bytes(&case.source.blob, 1024).unwrap(), raw);
    assert!(matches!(case.coverage, Coverage::Unqualified { .. }));
    assert!(case.schedule.is_empty() && case.references.is_empty());
    let source = pass(
        &root,
        &[
            "source",
            "stream",
            "--sha256",
            &case.source.blob.sha256,
            "--bytes",
            &raw.len().to_string(),
        ],
    );
    assert_eq!(source.stdout, raw);
}

#[test]
fn stream_limits_preflight_and_source_identity_fail_closed() {
    let (_temp, root) = setup();
    for (id, limit) in [("../invalid", 100), ("zero-limit", 0)] {
        let failed = capture(&root, id, b"bytes", limit);
        assert!(!failed.status.success());
        assert!(
            failed.stdout.is_empty(),
            "no readiness before valid preflight"
        );
    }
    let failed = capture(&root, "too-large", b"12345", 4);
    assert!(!failed.status.success());
    let store = Store::open(&root).unwrap();
    assert!(store.list_cases().unwrap().is_empty());
    assert_eq!(fs::read_dir(root.join("tmp")).unwrap().count(), 0);
    assert_eq!(store.verify_all().unwrap().blobs, 0);
    assert!(capture(&root, "good", b"1234", 4).status.success());
    let manifest = fs::read(root.join("cases/good/manifest.json")).unwrap();
    let failed = capture(&root, "good", b"other", 100);
    assert!(!failed.status.success() && failed.stdout.is_empty());
    assert_eq!(
        manifest,
        fs::read(root.join("cases/good/manifest.json")).unwrap()
    );
    let case = store.load_case("good").unwrap();
    let failed = cli(
        &root,
        &[
            "source",
            "good",
            "--sha256",
            &case.source.blob.sha256,
            "--bytes",
            "3",
        ],
    );
    assert!(!failed.status.success() && failed.stdout.is_empty());
    fs::write(store.blob_path(&case.source.blob).unwrap(), b"corrupt").unwrap();
    let failed = cli(
        &root,
        &[
            "source",
            "good",
            "--sha256",
            &case.source.blob.sha256,
            "--bytes",
            "4",
        ],
    );
    assert!(!failed.status.success() && failed.stdout.is_empty());
}

#[test]
fn independent_capture_ids_deduplicate_and_empty_sources_remain_unqualified() {
    let (_temp, root) = setup();
    for id in ["first", "second"] {
        assert!(capture(&root, id, b"same\r\n", 100).status.success());
    }
    assert!(capture(&root, "empty", b"", 1).status.success());
    let store = Store::open(&root).unwrap();
    let a = store.load_case("first").unwrap();
    let b = store.load_case("second").unwrap();
    assert_eq!(a.source.blob, b.source.blob);
    let empty = store.load_case("empty").unwrap();
    assert_eq!(empty.source.blob.bytes, 0);
    assert!(matches!(empty.coverage, Coverage::Unqualified { .. }));
    let verified = store.verify_all().unwrap();
    assert_eq!(verified.cases, 3);
    assert_eq!(verified.blobs, 2);
}
fn observations(store: &Store, id: &str) -> Vec<(sims_fixtures::trace::TraceRecordMeta, Value)> {
    let case = store.load_case(id).unwrap();
    let artifact = case
        .artifacts
        .iter()
        .find(|a| a.format == ArtifactFormat::TraceIndex)
        .unwrap();
    let index: TraceIndex = store.get_value(&artifact.blob).unwrap();
    let mut values = vec![];
    index
        .reconstruct(store, |meta, value| {
            values.push((meta.clone(), value.clone()));
            Ok(())
        })
        .unwrap();
    values
}
#[test]
fn json_keeps_exact_raw_bytes_and_normalized_value_without_qualification() {
    let (temp, root) = setup();
    let source = temp.path().join("original observation.json");
    let raw = b" \r\n{ \"z\": [1, true, null], \"a\": -0.0 }\t\r\n";
    fs::write(&source, raw).unwrap();
    pass(
        &root,
        &[
            "import-json",
            source.to_str().unwrap(),
            "--id",
            "json-case",
            "--compression",
            "raw",
        ],
    );
    let store = Store::open(&root).unwrap();
    let case = store.load_case("json-case").unwrap();
    assert_eq!(store.read_bytes(&case.source.blob, 1024).unwrap(), raw);
    assert_eq!(case.source.blob.sha256, sims_fixtures::digest(raw));
    let normalized: Value = store.get_value(&case.artifacts[0].blob).unwrap();
    assert_eq!(normalized, serde_json::from_slice::<Value>(raw).unwrap());
    assert!(matches!(case.coverage, Coverage::Unqualified { .. }));
    assert!(case.references.is_empty() && case.schedule.is_empty());
    assert_eq!(
        case.provenance.source_locator.as_deref(),
        Some("original observation.json")
    );
    assert!(
        case.provenance.sources.is_empty(),
        "computed source BlobRef is not an invented whole-capture declaration"
    );
    assert_eq!(case.provenance.commands.len(), 1);
    assert!(
        case.provenance.commands[0]
            .iter()
            .any(|arg| arg == "import-json")
    );
    let got = pass(
        &root,
        &[
            "get",
            "--sha256",
            &case.source.blob.sha256,
            "--bytes",
            &raw.len().to_string(),
        ],
    );
    assert_eq!(got.stdout, raw);
    pass(&root, &["verify"]);
}
#[test]
fn jsonl_prefix_roundtrip_export_and_reimport_reconstructs_every_observation() {
    let (temp, root) = setup();
    let source = temp.path().join("capture.jsonl");
    let lines = [
        b" {\"kind\":\"baseline\",\"tick\":10,\"advanced\":false,\"state\":{\"x\":1}}\r\n"
            .as_slice(),
        b"{\"kind\":\"ui_pass\",\"tick\":10,\"advanced\":false,\"state\":{\"x\":2}}\n".as_slice(),
        b"{\"kind\":\"update\",\"tick\":11,\"advanced\":true,\"state\":{\"x\":3}}\n".as_slice(),
    ];
    let prefix = lines.concat();
    let mut raw = prefix.clone();
    raw.extend_from_slice(b"this unselected suffix is intentionally invalid JSON");
    fs::write(&source, raw).unwrap();
    let result = pass(
        &root,
        &[
            "import-jsonl",
            source.to_str().unwrap(),
            "--id",
            "trace-case",
            "--record-limit",
            "3",
            "--tick-pointer",
            "/tick",
            "--advanced-pointer",
            "/advanced",
        ],
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["selected_records"], 3);
    assert_eq!(report["explicit_advanced_true_records"], 1);
    assert_eq!(report["first_observed_tick"], 10);
    assert_eq!(report["last_observed_tick"], 11);
    assert_eq!(report["eof_observed"], false);
    let store = Store::open(&root).unwrap();
    let case = store.load_case("trace-case").unwrap();
    assert_eq!(store.read_bytes(&case.source.blob, 4096).unwrap(), prefix);
    let streamed = pass(
        &root,
        &[
            "source",
            "trace-case",
            "--sha256",
            &case.source.blob.sha256,
            "--bytes",
            &case.source.blob.bytes.to_string(),
        ],
    );
    assert_eq!(streamed.stdout, prefix);
    let before = observations(&store, "trace-case");
    assert_eq!(before.len(), 3);
    let mut offset = 0;
    for (i, (meta, value)) in before.iter().enumerate() {
        assert_eq!(*value, serde_json::from_slice::<Value>(lines[i]).unwrap());
        assert_eq!(meta.kind, RecordKind::Observation);
        assert_eq!(meta.boundary, Boundary::Unspecified);
        let source = meta.source.as_ref().unwrap();
        assert_eq!(source.line, i as u64 + 1);
        assert_eq!(source.byte_offset, offset);
        assert_eq!(source.byte_length, lines[i].len() as u64);
        offset += source.byte_length;
    }
    let exported = temp.path().join("portable bundle");
    pass(&root, &["export", "trace-case", exported.to_str().unwrap()]);
    pass(&exported, &["verify"]);
    let portable = Store::open(&exported).unwrap();
    assert_eq!(before, observations(&portable, "trace-case"));
    let restored_source = temp.path().join("restored exact prefix.jsonl");
    pass(
        &exported,
        &[
            "get",
            "--sha256",
            &case.source.blob.sha256,
            "--bytes",
            &prefix.len().to_string(),
            "--output",
            restored_source.to_str().unwrap(),
        ],
    );
    let second = temp.path().join("reimported");
    pass(&second, &["init"]);
    pass(
        &second,
        &[
            "import-jsonl",
            restored_source.to_str().unwrap(),
            "--id",
            "reimported",
            "--record-limit",
            "3",
            "--tick-pointer",
            "/tick",
            "--advanced-pointer",
            "/advanced",
        ],
    );
    let second = Store::open(&second).unwrap();
    assert_eq!(before, observations(&second, "reimported"));
    assert_eq!(
        second.load_case("reimported").unwrap().source.blob,
        case.source.blob
    );
    pass(&root, &["verify"]);
}
#[test]
fn jsonl_retains_no_final_newline_and_missing_metadata_stays_unknown() {
    let (temp, root) = setup();
    let source = temp.path().join("no-final-newline.jsonl");
    let raw = b"{\"x\":1}\n {\"x\":2} ";
    fs::write(&source, raw).unwrap();
    let result = pass(
        &root,
        &[
            "import-jsonl",
            source.to_str().unwrap(),
            "--id",
            "tail",
            "--record-limit",
            "10",
            "--tick-pointer",
            "/absent",
            "--advanced-pointer",
            "/absent",
        ],
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["selected_records"], 2);
    assert_eq!(report["eof_observed"], true);
    assert_eq!(report["first_observed_tick"], Value::Null);
    assert_eq!(report["explicit_advanced_true_records"], 0);
    let store = Store::open(&root).unwrap();
    let case = store.load_case("tail").unwrap();
    assert_eq!(store.read_bytes(&case.source.blob, 1024).unwrap(), raw);
    assert!(
        observations(&store, "tail").iter().all(|(m, _)| m
            .source
            .as_ref()
            .unwrap()
            .actual_tick
            .is_none()
            && m.source.as_ref().unwrap().advanced.is_none())
    );
    let at_limit = pass(
        &root,
        &[
            "import-jsonl",
            source.to_str().unwrap(),
            "--id",
            "tail-at-limit",
            "--record-limit",
            "2",
        ],
    );
    let report: Value = serde_json::from_slice(&at_limit.stdout).unwrap();
    assert_eq!(
        report["eof_observed"], true,
        "EOF was read while collecting the unterminated second selected row"
    );
}
#[test]
fn invalid_caps_and_metadata_reject_without_publishing_a_case() {
    let (temp, root) = setup();
    let source = temp.path().join("bounded.jsonl");
    fs::write(&source, b"{\"tick\":1,\"advanced\":1}\n{\"x\":2}\n").unwrap();
    for extra in [
        vec!["--max-line-bytes", "2"],
        vec!["--max-prefix-bytes", "2"],
        vec!["--record-limit", "0"],
        vec!["--record-limit", "2", "--max-records", "1"],
        vec!["--advanced-pointer", "/advanced"],
        vec!["--tick-pointer", "bad-pointer"],
        vec!["--max-trace-record-bytes", "0"],
        vec!["--max-trace-chunk-bytes", "1024"],
        vec![
            "--max-trace-record-bytes",
            "64",
            "--max-trace-chunk-bytes",
            "1088",
        ],
    ] {
        let mut args = vec!["import-jsonl", source.to_str().unwrap(), "--id", "rejected"];
        args.extend(extra);
        assert!(!cli(&root, &args).status.success(), "{args:?}");
        assert!(Store::open(&root).unwrap().list_cases().unwrap().is_empty());
    }
}
#[test]
fn blank_or_invalid_json_lines_are_not_silently_skipped() {
    let (temp, root) = setup();
    let source = temp.path().join("malformed.jsonl");
    for raw in [
        b"\n{\"x\":1}\n".as_slice(),
        b"{\"x\":1}\nnot-json\n".as_slice(),
        b"".as_slice(),
    ] {
        fs::write(&source, raw).unwrap();
        assert!(
            !cli(
                &root,
                &["import-jsonl", source.to_str().unwrap(), "--id", "invalid"]
            )
            .status
            .success()
        );
        assert!(Store::open(&root).unwrap().list_cases().unwrap().is_empty());
    }
}
#[test]
fn declarations_never_promote_import_and_invalid_hash_is_rejected() {
    let (temp, root) = setup();
    let source = temp.path().join("declared.json");
    fs::write(&source, b"{\"observed\":true}").unwrap();
    let coverage = temp.path().join("coverage.json");
    fs::write(
        &coverage,
        br#"{"kind":"claimed_parity","contract":"unverified assertion"}"#,
    )
    .unwrap();
    let declared = "a".repeat(64);
    pass(
        &root,
        &[
            "import-json",
            source.to_str().unwrap(),
            "--id",
            "declared",
            "--origin-sha256",
            &declared,
            "--source-image-sha256",
            &declared,
            "--coverage",
            coverage.to_str().unwrap(),
            "--source-id",
            "logical-original-capture",
        ],
    );
    let case = Store::open(&root).unwrap().load_case("declared").unwrap();
    assert!(matches!(case.coverage, Coverage::Unqualified { .. }));
    assert_eq!(
        case.provenance.source_locator.as_deref(),
        Some("logical-original-capture")
    );
    assert!(
        case.notes
            .iter()
            .any(|n| n.contains("Unverified caller coverage"))
    );
    assert_eq!(case.provenance.sources.len(), 2);
    assert!(
        case.provenance
            .sources
            .iter()
            .all(|source| source.sha256 == declared
                && matches!(source.verification, HashVerification::Provided))
    );
    assert!(
        case.provenance.commands[0]
            .iter()
            .any(|arg| arg == "--origin-sha256")
    );
    assert!(
        !cli(
            &root,
            &[
                "import-json",
                source.to_str().unwrap(),
                "--id",
                "invalid-hash",
                "--origin-sha256",
                "made-up"
            ]
        )
        .status
        .success()
    );
}
#[test]
fn get_export_and_same_id_do_not_overwrite_existing_paths_or_cases() {
    let (temp, root) = setup();
    let source = temp.path().join("one.json");
    fs::write(&source, b"1").unwrap();
    pass(
        &root,
        &["import-json", source.to_str().unwrap(), "--id", "existing"],
    );
    let store = Store::open(&root).unwrap();
    let original = store.load_case("existing").unwrap();
    let target = temp.path().join("output");
    fs::write(&target, b"sentinel").unwrap();
    assert!(
        !cli(
            &root,
            &[
                "get",
                "--sha256",
                &original.source.blob.sha256,
                "--bytes",
                "1",
                "--output",
                target.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    assert_eq!(fs::read(&target).unwrap(), b"sentinel");
    let destination = temp.path().join("occupied");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("sentinel"), b"keep").unwrap();
    assert!(
        !cli(
            &root,
            &["export", "existing", destination.to_str().unwrap()]
        )
        .status
        .success()
    );
    assert_eq!(fs::read(destination.join("sentinel")).unwrap(), b"keep");
    fs::write(&source, b"2").unwrap();
    assert!(
        !cli(
            &root,
            &["import-json", source.to_str().unwrap(), "--id", "existing"]
        )
        .status
        .success()
    );
    assert_eq!(store.load_case("existing").unwrap(), original);
}
#[test]
fn gc_is_explicitly_dry_run_and_does_not_delete_orphan_content() {
    let (_temp, root) = setup();
    let store = Store::open(&root).unwrap();
    let orphan = store
        .put_bytes(b"authored orphan", sims_fixtures::Compression::Gzip)
        .unwrap();
    let path = store.blob_path(&orphan).unwrap();
    let before = fs::read(&path).unwrap();
    assert!(!cli(&root, &["gc"]).status.success());
    let report: Value = serde_json::from_slice(&pass(&root, &["gc", "--dry-run"]).stdout).unwrap();
    assert_eq!(report["dry_run"], true);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(report["unreferenced"].as_array().unwrap().len(), 1);
}
#[test]
fn case_ids_are_checked_before_publication_and_inspect_is_read_only() {
    let (temp, root) = setup();
    let source = temp.path().join("source.json");
    fs::write(&source, b"null").unwrap();
    assert!(
        !cli(
            &root,
            &["import-json", source.to_str().unwrap(), "--id", "../escape"]
        )
        .status
        .success()
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&pass(&root, &["inspect"]).stdout).unwrap(),
        json!([])
    );
    assert!(Store::open(&root).unwrap().list_cases().unwrap().is_empty());
}
#[test]
fn help_works_without_store_and_store_flag_can_follow_subcommand() {
    let help = Command::new(env!("CARGO_BIN_EXE_sims-fixtures"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(
        help.status.success(),
        "{}",
        String::from_utf8_lossy(&help.stderr)
    );
    let missing = Command::new(env!("CARGO_BIN_EXE_sims-fixtures"))
        .arg("verify")
        .output()
        .unwrap();
    assert!(!missing.status.success());
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("global-after-command");
    let created = Command::new(env!("CARGO_BIN_EXE_sims-fixtures"))
        .arg("init")
        .arg("--store")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    Store::open(path).unwrap().verify_all().unwrap();
}

#[cfg(unix)]
#[test]
fn non_utf8_invocation_is_rejected_without_panic_or_lossy_provenance() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let (temp, root) = setup();
    let source = temp.path().join("observation.json");
    fs::write(&source, b"null").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sims-fixtures"))
        .arg("--store")
        .arg(&root)
        .arg("import-json")
        .arg(&source)
        .args(["--id", "non-utf8", "--source-id", "logical-source"])
        .arg("--note")
        .arg(OsString::from_vec(b"unrepresentable-\xff".to_vec()))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("UTF-8"), "{error}");
    assert!(!error.contains("panicked"), "{error}");
    assert!(Store::open(&root).unwrap().list_cases().unwrap().is_empty());
}

#[test]
fn explicit_file_archive_exports_and_restores_exact_opaque_bytes() {
    let (temp, root) = setup();
    let source = temp.path().join("raw-source");
    fs::create_dir_all(source.join("nested")).unwrap();
    let raw = b"not JSON\0\xff\r\n";
    fs::write(source.join("nested/World.json"), raw).unwrap();
    fs::write(source.join("empty"), b"").unwrap();
    let list = temp.path().join("paths.json");
    fs::write(&list, br#"["nested/World.json","empty"]"#).unwrap();
    let output = pass(
        &root,
        &[
            "archive",
            "--source-root",
            source.to_str().unwrap(),
            "--files-from",
            list.to_str().unwrap(),
            "--id",
            "opaque-archive",
            "--source-id",
            "logical-old-cache",
        ],
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["files"], 2);
    assert_eq!(report["original_bytes"], raw.len());
    let store = Store::open(&root).unwrap();
    let case = store.load_case("opaque-archive").unwrap();
    assert_eq!(case.source.format, ArtifactFormat::ArchiveInventory);
    assert!(matches!(case.coverage, Coverage::Unqualified { .. }));
    let inv: sims_fixtures::archive::ArchiveInventory = store.get_value(&case.source.blob).unwrap();
    assert_eq!(inv.files[0].blob.sha256, sims_fixtures::digest(raw));
    pass(&root, &["verify"]);
    let exported = temp.path().join("exported");
    pass(
        &root,
        &["export", "opaque-archive", exported.to_str().unwrap()],
    );
    fs::rename(&root, temp.path().join("store-offline")).unwrap();
    fs::rename(&source, temp.path().join("source-offline")).unwrap();
    let destination = temp.path().join("restored");
    pass(
        &exported,
        &["restore", "opaque-archive", destination.to_str().unwrap()],
    );
    assert_eq!(
        fs::read(destination.join("files/nested/World.json")).unwrap(),
        raw
    );
    fs::rename(exported, temp.path().join("export-offline")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sims-fixtures"))
        .arg("verify-restored")
        .arg(&destination)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        destination
            .join(sims_fixtures::archive::COMPLETION_FILE)
            .is_file()
    );
}

#[test]
fn archive_rejects_path_aliases_and_restore_never_overwrites() {
    let (temp, root) = setup();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("a"), b"bytes").unwrap();
    let list = temp.path().join("list.json");
    for names in [
        json!(["../a"]),
        json!(["a", "A"]),
        json!(["a", "a-b", "a/child"]),
        json!(["A/x", "a/y"]),
        json!(["a:stream"]),
    ] {
        fs::write(&list, serde_json::to_vec(&names).unwrap()).unwrap();
        let output = cli(
            &root,
            &[
                "archive",
                "--source-root",
                source.to_str().unwrap(),
                "--files-from",
                list.to_str().unwrap(),
                "--id",
                "invalid",
            ],
        );
        assert!(!output.status.success());
        assert!(Store::open(&root).unwrap().list_cases().unwrap().is_empty());
    }
    fs::write(&list, br#"["a"]"#).unwrap();
    pass(
        &root,
        &[
            "archive",
            "--source-root",
            source.to_str().unwrap(),
            "--files-from",
            list.to_str().unwrap(),
            "--id",
            "valid",
        ],
    );
    let existing = temp.path().join("existing-empty");
    fs::create_dir(&existing).unwrap();
    let failed = cli(&root, &["restore", "valid", existing.to_str().unwrap()]);
    assert!(!failed.status.success());
    assert!(fs::read_dir(&existing).unwrap().next().is_none());
    let dest = temp.path().join("restored");
    pass(&root, &["restore", "valid", dest.to_str().unwrap()]);
    fs::write(dest.join("files/a"), b"changed independently").unwrap();
    assert!(
        !cli(&root, &["restore", "valid", dest.to_str().unwrap()])
            .status
            .success()
    );
    assert_eq!(
        fs::read(dest.join("files/a")).unwrap(),
        b"changed independently"
    );
    assert_eq!(fs::read(source.join("a")).unwrap(), b"bytes");
}
