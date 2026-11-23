//! Integration tests for the stdout-only `sims-antics lift` CLI inspector.
//!
//! Uses only clean-room `open-pack/` and synthetic in-memory `.iff` / `.far`
//! fixtures (zero commercial assets; runs in CI without `SIMS_INSTALL`).

use sims_antics::encode::encode_bhav;
use sims_antics::ir::BhavChunk;
use sims_antics::{compile_pack_dir, package_far};
use sims_formats::behavior::Instruction;
use sims_formats::iff::{ChunkData, write as write_iff};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

struct CleanupDir(PathBuf);

impl CleanupDir {
    fn new(label: &str) -> Self {
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "sims-antics-cli-lift-{label}-{}-{seq}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("failed to create temp test dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for CleanupDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open_pack_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("workspace root")
        .join("open-pack")
}

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_sims-antics")
}

// Collected immediately into a sorted `BTreeSet` for deterministic comparison.
#[allow(clippy::disallowed_methods)]
fn list_dir_entries(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .expect("read_dir")
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

fn synthetic_bhav_iff(trees: &[BhavChunk]) -> Vec<u8> {
    let chunks: Vec<ChunkData> = trees
        .iter()
        .map(|bhav| ChunkData {
            tag: *b"BHAV",
            id: bhav.id,
            flags: 0,
            label: bhav.label.clone(),
            data: encode_bhav(bhav).expect("encode_bhav"),
        })
        .collect();
    write_iff(&chunks).expect("write_iff")
}

#[test]
fn lift_far_list_filter_entry_structured_verify_and_raw() {
    let compiled = compile_pack_dir(&open_pack_dir()).expect("compile open-pack");
    let tmp = CleanupDir::new("open-pack");
    let far_path = tmp.path().join("open-pack.far");
    std::fs::write(&far_path, &compiled.far_bytes).expect("write open-pack.far");

    let fridge = compiled
        .objects
        .iter()
        .find(|o| o.iff_name == "open_fridge.iff")
        .expect("open_fridge.iff in open-pack");
    let fridge_path = tmp.path().join("open_fridge.iff");
    std::fs::write(&fridge_path, &fridge.object.iff_bytes).expect("write open_fridge.iff");

    let before_files = list_dir_entries(tmp.path());

    // 1. `lift <open-pack.far> --list` lists all 20 `.iff` entries in sorted order.
    let list_far = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .arg("--list")
        .output()
        .expect("run lift --list on far");
    assert!(
        list_far.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&list_far.stderr)
    );
    let far_lines: Vec<String> = String::from_utf8_lossy(&list_far.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        far_lines.len(),
        20,
        "expected 20 IFF entries: {far_lines:?}"
    );
    let mut sorted_lines = far_lines.clone();
    sorted_lines.sort();
    assert_eq!(far_lines, sorted_lines, "FAR listing must be sorted");
    assert!(far_lines.iter().any(|l| l == "open_tv.iff"));
    assert!(far_lines.iter().any(|l| l == "open_fridge.iff"));

    // 2. `lift <open-pack.far> --list --filter TV` filters case-insensitively.
    let filter_far = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .args(["--list", "--filter", "TV"])
        .output()
        .expect("run lift --list --filter on far");
    assert!(
        filter_far.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&filter_far.stderr)
    );
    let filtered_lines: Vec<String> = String::from_utf8_lossy(&filter_far.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(filtered_lines, vec!["open_tv.iff".to_owned()]);

    // 3. `lift <open-pack.far> --entry open_tv.iff --list` prints sorted BHAV table.
    let list_tv = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .args(["--entry", "open_tv.iff", "--list"])
        .output()
        .expect("run lift --entry open_tv.iff --list");
    assert!(
        list_tv.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&list_tv.stderr)
    );
    let tv_table = String::from_utf8_lossy(&list_tv.stdout);
    assert!(
        tv_table.contains("4096") && tv_table.contains("0x1000"),
        "{tv_table}"
    );
    assert!(
        tv_table.contains("4103")
            && tv_table.contains("0x1007")
            && tv_table.contains("WatchTv")
            && tv_table.contains("structured"),
        "{tv_table}"
    );
    let pos_4096 = tv_table.find("4096").expect("4096 row");
    let pos_4103 = tv_table.find("4103").expect("4103 row");
    assert!(pos_4096 < pos_4103, "BHAV table must be sorted by ID");

    // 3b. `--list --filter` on BHAVs matches both name and hex ID.
    let filter_tv = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .args(["--entry", "open_tv.iff", "--list", "--filter", "0x1007"])
        .output()
        .expect("run lift --list --filter on BHAVs");
    assert!(filter_tv.status.success());
    let filter_tv_out = String::from_utf8_lossy(&filter_tv.stdout);
    assert!(filter_tv_out.contains("WatchTv"), "{filter_tv_out}");
    assert!(!filter_tv_out.contains("Init"), "{filter_tv_out}");

    // 4. `lift <open-pack.far> --entry open_tv.iff --tree 4103 --verify-equiv`
    let watch_tv = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .args(["--entry", "open_tv.iff", "--tree", "4103", "--verify-equiv"])
        .output()
        .expect("run lift --tree 4103 --verify-equiv");
    assert!(
        watch_tv.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&watch_tv.stderr)
    );
    let watch_stdout = String::from_utf8_lossy(&watch_tv.stdout);
    assert!(watch_stdout.contains("tree 4103"), "{watch_stdout}");
    assert!(watch_stdout.contains("while "), "{watch_stdout}");
    assert!(
        !watch_stdout.contains("op 0x"),
        "structured output must contain zero raw op instructions:\n{watch_stdout}"
    );
    assert!(
        watch_stdout.contains("// verify-equiv: Equivalent"),
        "expected Equivalent comment:\n{watch_stdout}"
    );

    // 4b. Hex `--tree 0x1007` produces identical output.
    let watch_hex = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .args([
            "--entry",
            "open_tv.iff",
            "--tree",
            "0x1007",
            "--verify-equiv",
        ])
        .output()
        .expect("run lift --tree 0x1007 --verify-equiv");
    assert!(watch_hex.status.success());
    assert_eq!(watch_hex.stdout, watch_tv.stdout);

    // 5. `lift <open_fridge.iff> --raw --tree 4099` emits low-level byte-exact `.antics`.
    let fridge_raw = Command::new(cli_bin())
        .args(["lift"])
        .arg(&fridge_path)
        .args(["--raw", "--tree", "4099"])
        .output()
        .expect("run lift --raw --tree 4099");
    assert!(
        fridge_raw.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&fridge_raw.stderr)
    );
    let raw_stdout = String::from_utf8_lossy(&fridge_raw.stdout);
    assert!(
        raw_stdout.contains("tree 4099 \"CookMeal\""),
        "{raw_stdout}"
    );
    assert!(raw_stdout.contains("@n0:"), "{raw_stdout}");
    assert!(raw_stdout.contains("op 0x"), "{raw_stdout}");

    // 5b. Lifting all BHAVs of `open_tv.iff` with `--verify-equiv` succeeds with 0 raw ops.
    let all_tv = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .args(["--entry", "open_tv.iff", "--verify-equiv"])
        .output()
        .expect("run lift all trees with --verify-equiv");
    assert!(
        all_tv.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&all_tv.stderr)
    );
    let all_tv_out = String::from_utf8_lossy(&all_tv.stdout);
    assert!(all_tv_out.contains("tree 4096"), "{all_tv_out}");
    assert!(all_tv_out.contains("tree 4103"), "{all_tv_out}");
    assert!(!all_tv_out.contains("op 0x"), "{all_tv_out}");
    assert!(
        all_tv_out.matches("// verify-equiv: Equivalent").count() >= 8,
        "expected all 8 TV BHAVs verified Equivalent:\n{all_tv_out}"
    );

    // 5c. Lifting with `--filter` (without `--list`) lifts only matching trees.
    let filtered_lift = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .args(["--entry", "open_tv.iff", "--filter", "WatchTv"])
        .output()
        .expect("run lift --filter WatchTv");
    assert!(filtered_lift.status.success());
    let filtered_lift_out = String::from_utf8_lossy(&filtered_lift.stdout);
    assert!(
        filtered_lift_out.contains("tree 4103"),
        "{filtered_lift_out}"
    );
    assert!(
        !filtered_lift_out.contains("tree 4096"),
        "{filtered_lift_out}"
    );

    // 6. Verify stdout-only guarantee: zero files created in temp dir.
    let after_files = list_dir_entries(tmp.path());
    assert_eq!(
        before_files, after_files,
        "lift must never write files to disk"
    );
}

#[test]
fn lift_far_entry_resolution_exact_case_slash_suffix_and_ambiguous() {
    let iff_a = synthetic_bhav_iff(&[BhavChunk {
        id: 4096,
        label: "AlphaInit".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0,
        revision: 0,
        instructions: vec![Instruction {
            opcode: 2,
            on_true: 0xFE,
            on_false: 0xFE,
            operands: [0, 0, 1, 0, 0, 5, 0, 7],
        }],
        trailing: Vec::new(),
    }]);
    let iff_b = synthetic_bhav_iff(&[BhavChunk {
        id: 4096,
        label: "BetaInit".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0,
        revision: 0,
        instructions: vec![Instruction {
            opcode: 2,
            on_true: 0xFE,
            on_false: 0xFE,
            operands: [0, 0, 2, 0, 0, 5, 0, 7],
        }],
        trailing: Vec::new(),
    }]);
    let iff_c = synthetic_bhav_iff(&[BhavChunk {
        id: 4096,
        label: "UniqueInit".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0,
        revision: 0,
        instructions: vec![Instruction {
            opcode: 2,
            on_true: 0xFE,
            on_false: 0xFE,
            operands: [0, 0, 3, 0, 0, 5, 0, 7],
        }],
        trailing: Vec::new(),
    }]);

    let far_bytes = package_far(&[
        ("Objects\\Alpha\\shared.iff", &iff_a),
        ("Objects\\Beta\\shared.iff", &iff_b),
        ("Objects\\Single\\unique_item.iff", &iff_c),
        // 5-byte UTF-8 non-IFF entry where `len() - 4` falls mid-character (`é` = 2 UTF-8 bytes)
        ("\u{00e9}abc", b"not-an-iff"),
    ])
    .expect("package_far");

    let tmp = CleanupDir::new("resolve");
    // Extensionless file with FAR magic to verify content-based FAR detection too.
    let far_path = tmp.path().join("archive_without_ext.bin");
    std::fs::write(&far_path, &far_bytes).expect("write archive");

    // Listing `.iff` entries skips non-IFF entries (including multi-byte UTF-8 names) without panicking.
    let list_res = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .arg("--list")
        .output()
        .expect("list archive with utf8 non-iff entry");
    assert!(
        list_res.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&list_res.stderr)
    );
    let listed: Vec<&str> = std::str::from_utf8(&list_res.stdout)
        .expect("utf8 stdout")
        .lines()
        .collect();
    assert_eq!(
        listed,
        vec![
            "Objects\\Alpha\\shared.iff",
            "Objects\\Beta\\shared.iff",
            "Objects\\Single\\unique_item.iff",
        ]
    );

    // Exact match.
    let exact = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .args(["--entry", "Objects\\Alpha\\shared.iff", "--tree", "4096"])
        .output()
        .expect("exact match");
    assert!(
        exact.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&exact.stderr)
    );
    let exact_out = String::from_utf8_lossy(&exact.stdout);
    assert!(exact_out.contains("AlphaInit"), "{exact_out}");
    assert!(exact_out.contains("my.attr[0] := 1"), "{exact_out}");

    // Case-insensitive + forward-slash normalized match.
    let ci_slash = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .args(["--entry", "objects/beta/SHARED.IFF", "--tree", "4096"])
        .output()
        .expect("ci slash match");
    assert!(
        ci_slash.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&ci_slash.stderr)
    );
    let ci_out = String::from_utf8_lossy(&ci_slash.stdout);
    assert!(ci_out.contains("BetaInit"), "{ci_out}");
    assert!(ci_out.contains("my.attr[0] := 2"), "{ci_out}");

    // Unique basename match.
    let unique = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .args(["--entry", "unique_item.iff", "--tree", "4096"])
        .output()
        .expect("unique basename match");
    assert!(
        unique.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&unique.stderr)
    );
    let unique_out = String::from_utf8_lossy(&unique.stdout);
    assert!(unique_out.contains("UniqueInit"), "{unique_out}");
    assert!(unique_out.contains("my.attr[0] := 3"), "{unique_out}");

    // Ambiguous basename match lists both candidates.
    let ambig = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .args(["--entry", "shared.iff", "--tree", "4096"])
        .output()
        .expect("ambiguous match");
    assert!(!ambig.status.success());
    let stderr = String::from_utf8_lossy(&ambig.stderr);
    assert!(stderr.contains("ambiguous"), "{stderr}");
    assert!(stderr.contains("Objects\\Alpha\\shared.iff"), "{stderr}");
    assert!(stderr.contains("Objects\\Beta\\shared.iff"), "{stderr}");
}

#[test]
fn lift_falls_back_to_raw_on_unsupported_opcode_and_reports_status_in_list() {
    let iff_bytes = synthetic_bhav_iff(&[BhavChunk {
        id: 4096,
        label: "UnsupportedTree".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0,
        revision: 0,
        instructions: vec![Instruction {
            // Primitive 0x0042 is outside the structured high-level DSL.
            opcode: 0x0042,
            on_true: 0xFE,
            on_false: 0xFF,
            operands: [1, 2, 3, 4, 5, 6, 7, 8],
        }],
        trailing: Vec::new(),
    }]);

    let tmp = CleanupDir::new("fallback");
    let iff_path = tmp.path().join("unsupported.iff");
    std::fs::write(&iff_path, &iff_bytes).expect("write iff");

    let list_out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .arg("--list")
        .output()
        .expect("list unsupported");
    assert!(list_out.status.success());
    let list_stdout = String::from_utf8_lossy(&list_out.stdout);
    assert!(
        list_stdout.contains("raw-only (unsupported opcode 0x0042)"),
        "{list_stdout}"
    );

    let lift_out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .args(["--tree", "4096"])
        .output()
        .expect("lift fallback");
    assert!(
        lift_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&lift_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&lift_out.stdout);
    assert!(
        stdout.contains("structured lift fallback (unsupported opcode 0x0042)"),
        "{stdout}"
    );
    assert!(stdout.contains("op 0x0042"), "{stdout}");
}

#[test]
fn lift_error_cases_return_nonzero_without_panicking() {
    let compiled = compile_pack_dir(&open_pack_dir()).expect("compile open-pack");
    let tmp = CleanupDir::new("errors");
    let far_path = tmp.path().join("open-pack.far");
    std::fs::write(&far_path, &compiled.far_bytes).expect("write far");
    let tv = compiled
        .objects
        .iter()
        .find(|o| o.iff_name == "open_tv.iff")
        .expect("open_tv.iff");
    let tv_path = tmp.path().join("open_tv.iff");
    std::fs::write(&tv_path, &tv.object.iff_bytes).expect("write iff");
    let corrupt_iff = tmp.path().join("corrupt.iff");
    std::fs::write(&corrupt_iff, b"not an iff").expect("write corrupt iff");
    let corrupt_far = tmp.path().join("corrupt.far");
    std::fs::write(&corrupt_far, b"not a far").expect("write corrupt far");

    // 1. Missing `--entry` and `--list` on `.far`.
    let no_entry = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .output()
        .expect("run lift on far without entry/list");
    assert!(!no_entry.status.success());
    let stderr = String::from_utf8_lossy(&no_entry.stderr);
    assert!(
        stderr.contains("--list") && stderr.contains("--entry"),
        "{stderr}"
    );

    // 2. Unknown `--tree`.
    let bad_tree = Command::new(cli_bin())
        .args(["lift"])
        .arg(&tv_path)
        .args(["--tree", "9999"])
        .output()
        .expect("run lift unknown tree");
    assert!(!bad_tree.status.success());
    assert!(
        String::from_utf8_lossy(&bad_tree.stderr).contains("9999"),
        "{}",
        String::from_utf8_lossy(&bad_tree.stderr)
    );

    // 3. Nonexistent file.
    let missing = Command::new(cli_bin())
        .args(["lift"])
        .arg(tmp.path().join("does_not_exist.iff"))
        .output()
        .expect("run lift missing file");
    assert!(!missing.status.success());
    assert!(
        !String::from_utf8_lossy(&missing.stderr).contains("panicked"),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );

    // 4. Directory passed as input.
    let dir_input = Command::new(cli_bin())
        .args(["lift"])
        .arg(tmp.path())
        .output()
        .expect("run lift on dir");
    assert!(!dir_input.status.success());
    assert!(String::from_utf8_lossy(&dir_input.stderr).contains("not a regular file"));

    // 5. `--entry` on non-FAR file.
    let entry_on_iff = Command::new(cli_bin())
        .args(["lift"])
        .arg(&tv_path)
        .args(["--entry", "open_tv.iff"])
        .output()
        .expect("run lift --entry on iff");
    assert!(!entry_on_iff.status.success());
    assert!(String::from_utf8_lossy(&entry_on_iff.stderr).contains("--entry"));

    // 6. Malformed `.iff` and `.far`.
    for path in [&corrupt_iff, &corrupt_far] {
        let out = Command::new(cli_bin())
            .args(["lift"])
            .arg(path)
            .arg("--list")
            .output()
            .expect("run lift on corrupt file");
        assert!(!out.status.success());
        assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"));
    }

    // 7. Conflicting flag combinations.
    for extra in [
        vec!["--list", "--tree", "4096"],
        vec!["--raw", "--verify-equiv"],
        vec!["--tree", "4096", "--filter", "Init"],
    ] {
        let out = Command::new(cli_bin())
            .args(["lift"])
            .arg(&tv_path)
            .args(extra)
            .output()
            .expect("run conflicting flags");
        assert!(!out.status.success());
    }
}

#[test]
fn lift_handles_broken_pipe_cleanly_without_panicking() {
    // Generate a large IFF (>200 KiB of `.antics` text) so stdout exceeds the OS pipe buffer
    // and reliably hits `BrokenPipe` once the reader closes its end early.
    let mut trees = Vec::new();
    for idx in 0..300u16 {
        let mut instructions = Vec::new();
        for step in 0..20u8 {
            instructions.push(Instruction {
                opcode: 2,
                on_true: if step + 1 < 20 { step + 1 } else { 0xFE },
                on_false: 0xFF,
                operands: [0, 0, i16::from(step).to_le_bytes()[0], 0, 0, 5, 0, 7],
            });
        }
        trees.push(BhavChunk {
            id: 4096 + idx,
            label: format!("Tree_{idx}"),
            format: 0x8002,
            kind: 0,
            args: 0,
            locals: 0,
            revision: 0,
            instructions,
            trailing: Vec::new(),
        });
    }
    let iff_bytes = synthetic_bhav_iff(&trees);
    let tmp = CleanupDir::new("broken-pipe");
    let iff_path = tmp.path().join("many_trees.iff");
    std::fs::write(&iff_path, &iff_bytes).expect("write many_trees.iff");

    let mut child = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sims-antics lift");

    // Read just a few bytes and drop stdout immediately (simulating `| head -n 1`).
    if let Some(mut stdout) = child.stdout.take() {
        let mut buf = [0u8; 32];
        let _ = stdout.read(&mut buf);
        drop(stdout);
    }

    let output = child.wait_with_output().expect("wait child");
    assert!(
        output.status.success(),
        "broken pipe should exit cleanly; status={:?}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "expected empty stderr on broken pipe, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
