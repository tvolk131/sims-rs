//! Integration tests for the stdout-only `sims-antics lift` CLI inspector.
//!
//! Uses only clean-room `open-pack/` and synthetic in-memory `.iff` / `.far`
//! fixtures (zero commercial assets; runs in CI without `SIMS_INSTALL`).

use sims_antics::encode::encode_bhav;
use sims_antics::ir::BhavChunk;
use sims_antics::{
    EquivOptions, EquivResult, check_bhav_equivalent, compile_antics, compile_pack_dir, package_far,
};
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

    let cwd = tmp.path().join("cwd");
    std::fs::create_dir_all(&cwd).expect("create empty cwd");
    let before_files = list_dir_entries(tmp.path());
    let before_cwd = list_dir_entries(&cwd);
    let before_far_bytes = std::fs::read(&far_path).expect("read far before");
    let before_fridge_bytes = std::fs::read(&fridge_path).expect("read fridge before");

    // 1. `lift <open-pack.far> --list` lists all 20 `.iff` entries in sorted order.
    let list_far = Command::new(cli_bin())
        .current_dir(&cwd)
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
        .current_dir(&cwd)
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
        .current_dir(&cwd)
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
            && tv_table.contains("WatchTV")
            && tv_table.contains("structured"),
        "{tv_table}"
    );
    let pos_4096 = tv_table.find("4096").expect("4096 row");
    let pos_4103 = tv_table.find("4103").expect("4103 row");
    assert!(pos_4096 < pos_4103, "BHAV table must be sorted by ID");

    // 3b. `--list --filter` on BHAVs matches both name and hex ID.
    let filter_tv = Command::new(cli_bin())
        .current_dir(&cwd)
        .args(["lift"])
        .arg(&far_path)
        .args(["--entry", "open_tv.iff", "--list", "--filter", "0x1007"])
        .output()
        .expect("run lift --list --filter on BHAVs");
    assert!(filter_tv.status.success());
    let filter_tv_out = String::from_utf8_lossy(&filter_tv.stdout);
    assert!(filter_tv_out.contains("WatchTV"), "{filter_tv_out}");
    assert!(!filter_tv_out.contains("Init"), "{filter_tv_out}");

    // 4. `lift <open-pack.far> --entry open_tv.iff --tree 4103 --verify-equiv`
    let watch_tv = Command::new(cli_bin())
        .current_dir(&cwd)
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
        .current_dir(&cwd)
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
        .current_dir(&cwd)
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
    let raw_stdout = String::from_utf8(fridge_raw.stdout).expect("utf8 stdout");
    assert!(
        raw_stdout.contains("tree 4099 \"CookMeal\""),
        "{raw_stdout}"
    );
    assert!(raw_stdout.contains("@n0:"), "{raw_stdout}");
    assert!(raw_stdout.contains("op 0x"), "{raw_stdout}");
    let raw_recompiled = compile_antics(&raw_stdout).expect("raw fridge tree compiles");
    let orig_fridge_4099 = encode_bhav(&fridge.object.behaviors[&4099]).expect("encode orig 4099");
    let rt_fridge_4099 =
        encode_bhav(&raw_recompiled.behaviors[&4099]).expect("encode roundtripped 4099");
    assert_eq!(
        rt_fridge_4099, orig_fridge_4099,
        "raw lift of open_fridge.iff #4099 must round-trip byte-for-byte"
    );

    // 5b. Lifting all BHAVs of `open_tv.iff` with `--verify-equiv` succeeds with 0 raw ops.
    let all_tv = Command::new(cli_bin())
        .current_dir(&cwd)
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
        .current_dir(&cwd)
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

    // 6. Verify stdout-only guarantee: zero files created in temp dir or cwd, and inputs unchanged.
    let after_files = list_dir_entries(tmp.path());
    let after_cwd = list_dir_entries(&cwd);
    assert_eq!(
        before_files, after_files,
        "lift must never write files to fixture dir"
    );
    assert_eq!(
        before_cwd, after_cwd,
        "lift must never write files to working directory"
    );
    assert!(after_cwd.is_empty(), "cwd must remain empty");
    assert_eq!(
        before_far_bytes,
        std::fs::read(&far_path).expect("read far after"),
        "lift must never modify input .far"
    );
    assert_eq!(
        before_fridge_bytes,
        std::fs::read(&fridge_path).expect("read fridge after"),
        "lift must never modify input .iff"
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

#[test]
fn lift_raw_preserves_trailing_bytes_out_of_range_targets_and_escapes_control_labels() {
    let orig = BhavChunk {
        id: 4096,
        label: "Raw\x1b[31mTest\x07\x7f0m".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 2,
        locals: 3,
        revision: 0x1234_5678,
        instructions: vec![Instruction {
            opcode: 0x0042,
            // Out-of-range node target (5 >= 1) and reserved byte 251 (0xfb).
            on_true: 0x05,
            on_false: 0xfb,
            operands: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
        }],
        trailing: vec![0xDE, 0xAD, 0xBE, 0xEF],
    };
    let orig_payload = encode_bhav(&orig).expect("encode orig BHAV");
    let iff_bytes = synthetic_bhav_iff(std::slice::from_ref(&orig));

    let tmp = CleanupDir::new("raw-roundtrip");
    let iff_path = tmp.path().join("raw_exact.iff");
    std::fs::write(&iff_path, &iff_bytes).expect("write iff");

    let out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .args(["--raw", "--tree", "4096"])
        .output()
        .expect("run lift --raw");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    // Must not emit raw ASCII ESC/BEL/DEL control bytes to stdout.
    assert!(
        !stdout.contains('\x1b'),
        "raw ESC leaked to stdout: {stdout}"
    );
    assert!(
        !stdout.contains('\x07'),
        "raw BEL leaked to stdout: {stdout}"
    );
    assert!(
        !stdout.contains('\x7f'),
        "raw DEL leaked to stdout: {stdout}"
    );
    assert!(stdout.contains("\\x1b"), "{stdout}");
    assert!(stdout.contains("raw(0x05)"), "{stdout}");
    assert!(stdout.contains("raw(0xfb)"), "{stdout}");
    assert!(
        stdout.contains("trailing: [0xde, 0xad, 0xbe, 0xef]"),
        "{stdout}"
    );

    // Recompile stdout in memory and compare encoded BHAV payload byte-for-byte.
    let compiled = compile_antics(&stdout).expect("raw lift output must compile cleanly");
    let roundtripped = compiled.behaviors.get(&4096).expect("compiled tree 4096");
    assert_eq!(
        roundtripped.label, orig.label,
        "label must round-trip losslessly"
    );
    let roundtripped_payload = encode_bhav(roundtripped).expect("encode roundtripped BHAV");
    assert_eq!(
        roundtripped_payload, orig_payload,
        "raw lifted output must re-encode byte-for-byte identically"
    );

    // Also verify Unicode C1 CSI (`U+009B`) is escaped in `format_bhav_antics` and round-trips.
    let c1_tree = BhavChunk {
        label: "C1\u{009b}31m".to_owned(),
        ..orig
    };
    let c1_src = sims_antics::lift::format_bhav_antics(&c1_tree);
    assert!(!c1_src.contains('\u{009b}'), "{c1_src}");
    let c1_rt = compile_antics(&c1_src).expect("C1 escaped source compiles");
    assert_eq!(c1_rt.behaviors[&4096].label, c1_tree.label);
}

#[test]
fn lift_unicode_windows1252_case_folding_and_ambiguity() {
    let make_iff = |label: &str, val: u8| {
        synthetic_bhav_iff(&[BhavChunk {
            id: 4096,
            label: label.to_owned(),
            format: 0x8002,
            kind: 0,
            args: 0,
            locals: 0,
            revision: 0,
            instructions: vec![Instruction {
                opcode: 2,
                on_true: 0xFE,
                on_false: 0xFE,
                operands: [0, 0, val, 0, 0, 5, 0, 7],
            }],
            trailing: Vec::new(),
        }])
    };

    let sink_iff = make_iff("EvierInit", 11);
    let cafe_a = make_iff("CafeA", 21);
    let cafe_b = make_iff("CafeB", 22);

    let far_bytes = package_far(&[
        ("Objets\\Évier.iff", &sink_iff),
        ("Objets\\Alpha\\Café.iff", &cafe_a),
        ("Objets\\Beta\\café.iff", &cafe_b),
    ])
    .expect("package_far with Windows-1252 names");

    let tmp = CleanupDir::new("win1252-case");
    let far_path = tmp.path().join("french.far");
    std::fs::write(&far_path, &far_bytes).expect("write french.far");

    // Case-insensitive exact match across `É` / `é` and `\` / `/`.
    let exact_ci = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .args(["--entry", "objets/évier.iff", "--tree", "4096"])
        .output()
        .expect("case-folded accented exact match");
    assert!(
        exact_ci.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&exact_ci.stderr)
    );
    assert!(String::from_utf8_lossy(&exact_ci.stdout).contains("my.attr[0] := 11"));

    // Unique basename match across `É` / `é`.
    let basename_ci = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .args(["--entry", "évier.iff", "--tree", "4096"])
        .output()
        .expect("case-folded accented basename match");
    assert!(
        basename_ci.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&basename_ci.stderr)
    );
    assert!(String::from_utf8_lossy(&basename_ci.stdout).contains("my.attr[0] := 11"));

    // Ambiguous accented match (`Café.iff` vs `café.iff` queried as `CAFÉ.IFF`).
    let ambig = Command::new(cli_bin())
        .args(["lift"])
        .arg(&far_path)
        .args(["--entry", "CAFÉ.IFF", "--tree", "4096"])
        .output()
        .expect("ambiguous accented match");
    assert!(!ambig.status.success());
    let stderr = String::from_utf8_lossy(&ambig.stderr);
    assert!(stderr.contains("ambiguous"), "{stderr}");
    assert!(stderr.contains("Objets\\Alpha\\Café.iff"), "{stderr}");
    assert!(stderr.contains("Objets\\Beta\\café.iff"), "{stderr}");
}

#[test]
fn lift_0x8002_high_byte_locals_matches_vm_frame_dimensions() {
    // Format 0x8002 with locals = 0x0100: `sims-vm` truncates locals to u8 (0 locals),
    // so assigning `local[0] := 1` traps at runtime. Structured lifting must NOT promote
    // this to `locals: 255`, and the equivalence checker must detect divergence against
    // a 1-local tree.
    let orig = BhavChunk {
        id: 4096,
        label: "HighByteLocals".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0x0100,
        revision: 0,
        instructions: vec![Instruction {
            opcode: 2,
            on_true: 0xFE,
            on_false: 0xFF,
            // local[0] := 1 (scope 25 = local, op 5 = Set, scope 7 = literal 1)
            operands: [0, 0, 1, 0, 0, 5, 25, 7],
        }],
        trailing: Vec::new(),
    };
    let one_local = BhavChunk {
        locals: 1,
        ..orig.clone()
    };

    // Equivalence checker must report Counterexample (0 effective locals vs 1 local).
    let eq = check_bhav_equivalent(&orig, &one_local, &EquivOptions::default());
    assert!(
        matches!(eq, EquivResult::Counterexample { .. }),
        "expected Counterexample when original has 0x0100 (0 effective) locals vs 1 local, got {eq:?}"
    );

    // CLI `lift --verify-equiv` must fall back to raw instead of falsely lifting + reporting Equivalent.
    let tmp = CleanupDir::new("high-byte-locals");
    let iff_path = tmp.path().join("high_locals.iff");
    std::fs::write(&iff_path, synthetic_bhav_iff(&[orig])).expect("write iff");

    let out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .args(["--tree", "4096", "--verify-equiv"])
        .output()
        .expect("run lift on 0x0100 locals");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("structured lift fallback"), "{stdout}");
    assert!(
        !stdout.contains("// verify-equiv: Equivalent"),
        "must not falsely report Equivalent on 0x0100 locals:\n{stdout}"
    );
}

#[test]
fn lift_bulk_returns_nonzero_when_any_bhav_is_malformed() {
    let valid = BhavChunk {
        id: 4096,
        label: "ValidTree".to_owned(),
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
    };
    let iff_bytes = write_iff(&[
        ChunkData {
            tag: *b"BHAV",
            id: valid.id,
            flags: 0,
            label: valid.label.clone(),
            data: encode_bhav(&valid).expect("encode valid"),
        },
        ChunkData {
            tag: *b"BHAV",
            id: 4097,
            flags: 0,
            label: "TruncatedTree".to_owned(),
            data: vec![0x02, 0x80, 0x01],
        },
    ])
    .expect("write mixed iff");

    let tmp = CleanupDir::new("bulk-malformed");
    let iff_path = tmp.path().join("mixed.iff");
    std::fs::write(&iff_path, &iff_bytes).expect("write mixed.iff");

    let out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .output()
        .expect("run bulk lift on mixed iff");
    assert!(
        !out.status.success(),
        "bulk lift must fail when a BHAV fails to parse"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ValidTree"),
        "valid BHAV should still be emitted:\n{stdout}"
    );
    assert!(
        stdout.contains("failed to parse BHAV"),
        "malformed BHAV comment should be emitted:\n{stdout}"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("failed to parse 1 BHAV"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// Subprocess hang-watchdog timeout in a CLI integration test (not simulation logic).
#[cfg(unix)]
#[test]
#[allow(clippy::disallowed_methods)]
fn lift_rejects_unix_fifo_without_blocking() {
    use std::time::{Duration, Instant};

    let tmp = CleanupDir::new("fifo");
    let fifo_path = tmp.path().join("pipe.iff");
    let status = Command::new("mkfifo")
        .arg(&fifo_path)
        .status()
        .expect("mkfifo should be available on Unix");
    assert!(status.success(), "mkfifo failed");

    let mut child = Command::new(cli_bin())
        .args(["lift"])
        .arg(&fifo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sims-antics lift on fifo");

    let deadline = Instant::now() + Duration::from_secs(5);
    let exited = loop {
        if let Some(st) = child.try_wait().expect("try_wait") {
            break Some(st);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    let st = exited.expect("sims-antics lift blocked on FIFO instead of rejecting it");
    assert!(!st.success(), "FIFO input must exit non-zero");
    let mut stderr = String::new();
    if let Some(mut err_pipe) = child.stderr.take() {
        let _ = err_pipe.read_to_string(&mut stderr);
    }
    assert!(stderr.contains("not a regular file"), "stderr: {stderr}");
}

#[cfg(unix)]
#[test]
fn lift_handles_non_utf8_unix_path_without_panicking() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let tmp = CleanupDir::new("non-utf8");
    let mut raw_path = tmp.path().as_os_str().as_encoded_bytes().to_vec();
    raw_path.push(b'/');
    raw_path.extend_from_slice(b"bad_\xff_name.iff");
    let os_path = OsString::from_vec(raw_path);

    // On filesystems that allow non-UTF-8 filenames (e.g. Linux ext4), write a real IFF and
    // verify `lift` succeeds; on filesystems that reject invalid UTF-8 (e.g. macOS APFS),
    // verify `lift` returns a clean error and does NOT panic.
    let valid_iff = synthetic_bhav_iff(&[BhavChunk {
        id: 4096,
        label: "NonUtf8PathTree".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0,
        revision: 0,
        instructions: vec![Instruction {
            opcode: 2,
            on_true: 0xFE,
            on_false: 0xFE,
            operands: [0, 0, 7, 0, 0, 5, 0, 7],
        }],
        trailing: Vec::new(),
    }]);
    let created = std::fs::write(&os_path, &valid_iff).is_ok();

    let out = Command::new(cli_bin())
        .arg("lift")
        .arg(&os_path)
        .arg("--tree")
        .arg("4096")
        .output()
        .expect("run lift with non-UTF-8 path");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked"),
        "CLI panicked on non-UTF-8 path: {stderr}"
    );
    if created {
        assert!(out.status.success(), "stderr: {stderr}");
        assert!(String::from_utf8_lossy(&out.stdout).contains("my.attr[0] := 7"));
    } else {
        assert!(!out.status.success());
    }
}

#[test]
fn lift_raw_roundtrips_252_instruction_bhav_byte_for_byte() {
    // 252 instructions: node 0 returns true immediately, nodes 1..=251 are trailing raw records.
    let mut instructions = Vec::with_capacity(252);
    instructions.push(Instruction {
        opcode: 2,
        on_true: 0xFE,
        on_false: 0xFE,
        operands: [0, 0, 1, 0, 0, 5, 0, 7],
    });
    for idx in 1..252u16 {
        instructions.push(Instruction {
            opcode: 2,
            on_true: if idx == 251 { 0xfb } else { 0xFE },
            on_false: 0xFF,
            operands: [0, 0, (idx & 0xFF) as u8, 0, 0, 5, 0, 7],
        });
    }
    let orig = BhavChunk {
        id: 4096,
        label: "Long252Tree".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0,
        revision: 0,
        instructions,
        trailing: vec![0xCA, 0xFE],
    };
    let orig_payload = encode_bhav(&orig).expect("encode 252-ins BHAV");

    let tmp = CleanupDir::new("raw-252");
    let iff_path = tmp.path().join("long252.iff");
    std::fs::write(&iff_path, synthetic_bhav_iff(std::slice::from_ref(&orig))).expect("write iff");

    let out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .args(["--raw", "--tree", "4096"])
        .output()
        .expect("run lift --raw on 252-ins BHAV");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert!(stdout.contains("raw(0xfb)"), "{stdout}");
    let compiled = compile_antics(&stdout).expect("252-instruction raw lift must recompile");
    let rt_payload = encode_bhav(&compiled.behaviors[&4096]).expect("encode roundtripped");
    assert_eq!(rt_payload, orig_payload);
}

#[test]
fn lift_bulk_raw_compiles_with_duplicate_empty_labels_and_high_flag_variant() {
    let t1 = BhavChunk {
        id: 4096,
        label: String::new(),
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
    };
    let t2 = BhavChunk {
        id: 4097,
        label: String::new(),
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
    };
    let t1_variant = BhavChunk {
        id: 4096,
        label: "HighFlagVariant".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0,
        revision: 0,
        instructions: vec![Instruction {
            opcode: 2,
            on_true: 0xFF,
            on_false: 0xFF,
            operands: [0, 0, 9, 0, 0, 5, 0, 7],
        }],
        trailing: Vec::new(),
    };

    let iff_bytes = write_iff(&[
        ChunkData {
            tag: *b"BHAV",
            id: 4096,
            flags: 0x0100, // high-flag variant sharing ID 4096
            label: t1_variant.label.clone(),
            data: encode_bhav(&t1_variant).expect("encode variant"),
        },
        ChunkData {
            tag: *b"BHAV",
            id: 4096,
            flags: 0, // canonical flags high byte 0
            label: t1.label.clone(),
            data: encode_bhav(&t1).expect("encode t1"),
        },
        ChunkData {
            tag: *b"BHAV",
            id: 4097,
            flags: 0,
            label: t2.label.clone(),
            data: encode_bhav(&t2).expect("encode t2"),
        },
    ])
    .expect("write iff");

    let tmp = CleanupDir::new("dup-labels");
    let iff_path = tmp.path().join("dup_labels.iff");
    std::fs::write(&iff_path, &iff_bytes).expect("write iff");

    let out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .arg("--raw")
        .output()
        .expect("run bulk lift --raw");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let compiled =
        compile_antics(&stdout).expect("bulk raw lift with duplicate labels must compile");
    assert_eq!(compiled.behaviors.len(), 2);
    assert_eq!(
        encode_bhav(&compiled.behaviors[&4096]).unwrap(),
        encode_bhav(&t1).unwrap(),
        "must pick canonical high-flag-byte 0 resource for ID 4096"
    );
    assert_eq!(
        encode_bhav(&compiled.behaviors[&4097]).unwrap(),
        encode_bhav(&t2).unwrap()
    );
}

#[test]
fn lift_verify_equiv_ignores_stray_global_id_arity_in_object_iff() {
    // An object IFF contains a stray BHAV #300 (in the global range 256..=4095) claiming
    // `args: 0`, plus local BHAV #4096 that makes an inline call to global #300 followed by
    // a dead instruction. If `--verify-equiv` trusted stray #300's arity (0 args), it would
    // skip trailing-suffix checks on the inline call and report `Equivalent` even though the
    // real global #300 could read >4 inline args from the dropped dead instruction!
    let stray_global = BhavChunk {
        id: 300,
        label: "StrayGlobal300".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0,
        revision: 0,
        instructions: vec![Instruction {
            opcode: 2,
            on_true: 0xFE,
            on_false: 0xFE,
            operands: [0, 0, 0, 0, 0, 5, 0, 7],
        }],
        trailing: Vec::new(),
    };
    let caller = BhavChunk {
        id: 4096,
        label: "CallsGlobal300".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0,
        revision: 0,
        instructions: vec![
            Instruction {
                // Inline call to global #300 (non-0xFF operands) returning true/false.
                opcode: 300,
                on_true: 0xFE,
                on_false: 0xFF,
                operands: [1, 0, 2, 0, 3, 0, 4, 0],
            },
            // Dead instruction right after node 0 — structured lifter strips unreachable nodes,
            // changing trailing bytes that a >4-arg global #300 could read!
            Instruction {
                opcode: 2,
                on_true: 0xFE,
                on_false: 0xFF,
                operands: [0x99, 0, 0, 0, 0, 5, 0, 7],
            },
        ],
        trailing: Vec::new(),
    };

    let tmp = CleanupDir::new("stray-global-arity");
    let iff_path = tmp.path().join("stray_global.iff");
    std::fs::write(&iff_path, synthetic_bhav_iff(&[stray_global, caller])).expect("write iff");

    let out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .args(["--tree", "4096", "--verify-equiv"])
        .output()
        .expect("run lift --verify-equiv");
    assert!(
        out.status.success(),
        "Inconclusive should exit 0 while reporting unknown callee arity;\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("// verify-equiv: Equivalent"),
        "must NOT report Equivalent when stray global #300 arity would mask trailing-suffix divergence:\n{stdout}"
    );
    assert!(
        stdout.contains("// verify-equiv: Inconclusive(")
            && stdout.contains("unknown callee arity"),
        "expected Inconclusive(unknown callee arity) comment:\n{stdout}"
    );

    // Conversely, a local BHAV #4097 (in 4096..=8191) declaring `args: 5` IS resolved
    // from this IFF, so stripping the trailing dead instruction after an inline call to
    // #4097 produces a real `Counterexample` and a non-zero exit code!
    let local_5arg = BhavChunk {
        id: 4097,
        label: "LocalFiveArgs".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 5,
        locals: 0,
        revision: 0,
        instructions: vec![Instruction {
            opcode: 2,
            on_true: 0xFE,
            on_false: 0xFE,
            operands: [0, 0, 0, 0, 0, 5, 0, 7],
        }],
        trailing: Vec::new(),
    };
    let caller_local = BhavChunk {
        id: 4098,
        label: "CallsLocal5Arg".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0,
        revision: 0,
        instructions: vec![
            Instruction {
                opcode: 4097,
                on_true: 0xFE,
                on_false: 0xFF,
                operands: [1, 0, 2, 0, 3, 0, 4, 0],
            },
            Instruction {
                opcode: 2,
                on_true: 0xFE,
                on_false: 0xFF,
                operands: [0x99, 0, 0, 0, 0, 5, 0, 7],
            },
        ],
        trailing: Vec::new(),
    };
    let local_iff_path = tmp.path().join("local_5arg.iff");
    std::fs::write(
        &local_iff_path,
        synthetic_bhav_iff(&[local_5arg, caller_local]),
    )
    .expect("write local_5arg.iff");

    let ce_out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&local_iff_path)
        .args(["--tree", "4098", "--verify-equiv"])
        .output()
        .expect("run lift --verify-equiv expecting Counterexample");
    assert!(
        !ce_out.status.success(),
        "Counterexample must exit non-zero;\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&ce_out.stdout),
        String::from_utf8_lossy(&ce_out.stderr)
    );
    let ce_stdout = String::from_utf8_lossy(&ce_out.stdout);
    assert!(
        ce_stdout.contains("// verify-equiv: Counterexample("),
        "expected Counterexample comment:\n{ce_stdout}"
    );
}

#[cfg(unix)]
#[test]
fn lift_sanitizes_newlines_and_escapes_in_input_filenames() {
    let empty_iff = write_iff(&[]).expect("write empty iff");
    let tmp = CleanupDir::new("sanitize-filename");
    let weird_path = tmp.path().join("evil\n\x1b[31m_file.iff");
    std::fs::write(&weird_path, &empty_iff).expect("write weird filename iff");

    let out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&weird_path)
        .output()
        .expect("run lift on weird filename");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert!(
        !stdout.contains('\x1b'),
        "raw ESC leaked in stdout: {stdout}"
    );
    assert_eq!(
        stdout.lines().count(),
        1,
        "newline in filename must not break out of comment line:\n{stdout}"
    );
    assert!(
        stdout.starts_with("// no BHAV chunks found in "),
        "{stdout}"
    );
    assert!(stdout.contains("\\n\\x1b[31m_file.iff"), "{stdout}");
}
