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
use std::ffi::OsStr;
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
    // Row IDs in table order (skipping the `// arity sources` header and column titles).
    let row_ids: Vec<u16> = tv_table
        .lines()
        .filter_map(|line| line.split_whitespace().next()?.parse().ok())
        .collect();
    let pos_4096 = row_ids.iter().position(|&id| id == 4096).expect("4096 row");
    let pos_4103 = row_ids.iter().position(|&id| id == 4103).expect("4103 row");
    assert!(pos_4096 < pos_4103, "BHAV table must be sorted by ID");
    assert!(
        row_ids.windows(2).all(|w| w[0] < w[1]),
        "BHAV table must be sorted by ID: {row_ids:?}"
    );

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
fn lift_keeps_structured_flow_with_inline_raw_op_and_decodes_irreducible_cfg() {
    // Tree 4096: reducible CFG containing one unmodeled opcode (0x0042) inside an `if`.
    // Must lift structurally with inline `op 0x0042 (...)` instead of falling back to raw!
    let inline_raw_tree = BhavChunk {
        id: 4096,
        label: "InlineRawTree".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0,
        revision: 0,
        instructions: vec![
            Instruction {
                // my.attr[0] == 1 ? node 1 : return false
                opcode: 2,
                on_true: 1,
                on_false: 0xFF,
                operands: [0, 0, 1, 0, 0, 2, 0, 7],
            },
            Instruction {
                // Unmodeled opcode 0x0042 branching true -> node 2, false -> return false
                opcode: 0x0042,
                on_true: 2,
                on_false: 0xFF,
                operands: [1, 2, 3, 4, 5, 6, 7, 8],
            },
            Instruction {
                // my.attr[1] := 7 -> return true
                opcode: 2,
                on_true: 0xFE,
                on_false: 0xFE,
                operands: [1, 0, 7, 0, 0, 5, 0, 7],
            },
        ],
        trailing: Vec::new(),
    };

    // Tree 4097: genuinely irreducible 2-entry loop (0 -> {1, 2}, 1 <-> 2).
    // Must fall back to decoded labeled-CFG `.air` (keeping @n0: labels + readable expressions!).
    let irreducible_tree = BhavChunk {
        id: 4097,
        label: "IrreducibleTree".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0,
        revision: 0,
        instructions: vec![
            Instruction {
                // my.attr[0] == 0 ? @n1 : @n2
                opcode: 2,
                on_true: 1,
                on_false: 2,
                operands: [0, 0, 0, 0, 0, 2, 0, 7],
            },
            Instruction {
                // my.attr[1] == 0 ? @n2 : true
                opcode: 2,
                on_true: 2,
                on_false: 0xFE,
                operands: [1, 0, 0, 0, 0, 2, 0, 7],
            },
            Instruction {
                // my.attr[2] == 0 ? @n1 : false
                opcode: 2,
                on_true: 1,
                on_false: 0xFF,
                operands: [2, 0, 0, 0, 0, 2, 0, 7],
            },
        ],
        trailing: Vec::new(),
    };

    let iff_bytes = synthetic_bhav_iff(&[inline_raw_tree, irreducible_tree.clone()]);
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
        list_stdout.contains("structured (1 inline raw op)"),
        "{list_stdout}"
    );
    assert!(
        list_stdout.contains("labeled-cfg (unstructured CFG)"),
        "{list_stdout}"
    );

    // Tree 4096 lifts structurally with inline `op 0x0042` and verifies Equivalent!
    let lift_4096 = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .args(["--tree", "4096", "--verify-equiv"])
        .output()
        .expect("lift inline raw op tree");
    assert!(
        lift_4096.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&lift_4096.stderr)
    );
    let stdout_4096 = String::from_utf8_lossy(&lift_4096.stdout);
    assert!(
        stdout_4096.contains("if my.attr[0] == 1") || stdout_4096.contains("if !my.attr[0] == 1"),
        "{stdout_4096}"
    );
    assert!(
        stdout_4096.contains("if op 0x0042") || stdout_4096.contains("if !op 0x0042"),
        "{stdout_4096}"
    );
    assert!(stdout_4096.contains("my.attr[1] := 7;"), "{stdout_4096}");
    assert!(
        stdout_4096.contains("// verify-equiv: Equivalent"),
        "{stdout_4096}"
    );

    // Tree 4097 falls back to decoded labeled-CFG `.air` (readable `expr(...)`, zero `op 0x`)
    // and round-trips byte-for-byte identically!
    let lift_4097 = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .args(["--tree", "4097"])
        .output()
        .expect("lift irreducible tree");
    assert!(
        lift_4097.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&lift_4097.stderr)
    );
    let stdout_4097 = String::from_utf8_lossy(&lift_4097.stdout);
    assert!(
        stdout_4097.contains("structured lift fallback (unstructured CFG)"),
        "{stdout_4097}"
    );
    assert!(stdout_4097.contains("@n0:"), "{stdout_4097}");
    assert!(
        stdout_4097.contains("expr(my.attr[0] == 0) -> true: @n1, false: @n2;"),
        "{stdout_4097}"
    );
    assert!(
        !stdout_4097.contains("op 0x"),
        "all modeled instructions in decoded labeled-CFG should be decoded:\n{stdout_4097}"
    );
    let compiled_4097 = compile_antics(&stdout_4097).expect("decoded labeled CFG compiles");
    assert_eq!(
        encode_bhav(&compiled_4097.behaviors[&4097]).unwrap(),
        encode_bhav(&irreducible_tree).unwrap()
    );
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

    // CLI `lift --verify-equiv` keeps effective `locals: 0` (never promoting to 255!) and
    // preserves the trapping instruction via inline `op 0x0002 (...)` so it stays Equivalent.
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
    assert!(
        stdout.contains("locals: 0"),
        "must preserve effective 0 locals (not promote to 255):\n{stdout}"
    );
    assert!(
        stdout.contains("op 0x0002"),
        "out-of-bounds local[0] with locals: 0 must stay as inline raw op:\n{stdout}"
    );
    assert!(
        stdout.contains("// verify-equiv: Equivalent"),
        "inline raw op with locals: 0 is genuinely trace-equivalent to 0x0100 locals:\n{stdout}"
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

    // Decoded labeled-CFG counterpart (without `--raw`): must also decode instructions,
    // preserve all 252 records + trailing bytes + `raw(0xfb)`, and round-trip byte-for-byte!
    let dec_out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .args(["--tree", "4096", "--verify-equiv"])
        .output()
        .expect("run decoded lift on 252-ins BHAV");
    assert!(
        dec_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&dec_out.stderr)
    );
    let dec_stdout = String::from_utf8(dec_out.stdout).expect("utf8 dec stdout");
    assert!(dec_stdout.contains("expr(my.attr[0] := 1)"), "{dec_stdout}");
    assert!(dec_stdout.contains("raw(0xfb)"), "{dec_stdout}");
    assert!(
        dec_stdout.contains("trailing: [0xca, 0xfe]"),
        "{dec_stdout}"
    );
    assert!(
        dec_stdout.contains("// verify-equiv: Equivalent"),
        "expected --verify-equiv on labeled-CFG fallback output:\n{dec_stdout}"
    );
    let dec_compiled =
        compile_antics(&dec_stdout).expect("252-instruction decoded labeled lift must recompile");
    let dec_rt_payload =
        encode_bhav(&dec_compiled.behaviors[&4096]).expect("encode decoded roundtripped");
    assert_eq!(dec_rt_payload, orig_payload);
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
                // Inline call to global #300 (non-0xFF operands) jumping to node 2 then node 1,
                // so structured lifting reorders reachable nodes 1 and 2 after the call!
                opcode: 300,
                on_true: 2,
                on_false: 0xFF,
                operands: [1, 0, 2, 0, 3, 0, 4, 0],
            },
            Instruction {
                opcode: 2,
                on_true: 0xFE,
                on_false: 0xFE,
                operands: [0, 0, 1, 0, 0, 5, 0, 7],
            },
            Instruction {
                opcode: 2,
                on_true: 1,
                on_false: 1,
                operands: [0, 0, 2, 0, 0, 5, 0, 7],
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
        .args(["--tree", "4096", "--relax-padding", "--verify-equiv"])
        .output()
        .expect("run lift --verify-equiv");
    assert!(
        out.status.success(),
        "unknown-arity global #300 must fall back to protected labeled-CFG and verify Equivalent;\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("structured lift fallback"),
        "must NOT trust stray global #300 arity=0 to structurally reorder nodes after call #300:\n{stdout}"
    );
    assert!(
        stdout.contains("// verify-equiv: Equivalent"),
        "protected labeled-CFG fallback must prove Equivalent:\n{stdout}"
    );

    // Conversely, a local BHAV #4097 (in 4096..=8191) declaring `args: 7` IS resolved
    // from this IFF, so structured lifting refuses to drop/reorder the following extended-arg
    // record (even under `--relax-padding`!), falls back to decoded labeled-CFG preserving the
    // extended argument bytes, and `--verify-equiv` verifies it `Equivalent`!
    let local_7arg = BhavChunk {
        id: 4097,
        label: "LocalSevenArgs".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 7,
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
        label: "CallsLocal7Arg".to_owned(),
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
                // Unreachable NotifyIdle carrying extended arg 7 in operand byte 0 (`0x99`)!
                opcode: 49,
                on_true: 0xFE,
                on_false: 0xFF,
                operands: [0x99, 0, 0, 0, 0, 0, 0, 0],
            },
        ],
        trailing: Vec::new(),
    };
    let local_iff_path = tmp.path().join("local_7arg.iff");
    std::fs::write(
        &local_iff_path,
        synthetic_bhav_iff(&[local_7arg, caller_local]),
    )
    .expect("write local_7arg.iff");

    let ext_out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&local_iff_path)
        .args(["--tree", "4098", "--relax-padding", "--verify-equiv"])
        .output()
        .expect("run lift --relax-padding --verify-equiv on 7-arg local call");
    assert!(
        ext_out.status.success(),
        "7-arg local call fallback must preserve extended args and prove Equivalent;\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&ext_out.stdout),
        String::from_utf8_lossy(&ext_out.stderr)
    );
    let ext_stdout = String::from_utf8_lossy(&ext_out.stdout);
    assert!(
        ext_stdout.contains("structured lift fallback") && ext_stdout.contains("op 0x0031 (0x99"),
        "expected labeled-CFG fallback preserving arg-7 byte 0x99:\n{ext_stdout}"
    );
    assert!(
        ext_stdout.contains("// verify-equiv: Equivalent"),
        "expected Equivalent on protected labeled-CFG fallback:\n{ext_stdout}"
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

#[test]
fn lift_relax_padding_cli_flag_and_raw_conflict() {
    let dirty = BhavChunk {
        id: 4096,
        label: "DirtyNotify".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0,
        revision: 0,
        instructions: vec![Instruction {
            // NotifyIdle (opcode 49) with dirty Edith padding byte 3 = 0x42.
            opcode: 49,
            on_true: 0xFE,
            on_false: 0xFD,
            operands: [0, 0, 0, 0x42, 0, 0, 0, 0],
        }],
        trailing: Vec::new(),
    };
    let tmp = CleanupDir::new("relax-padding");
    let iff_path = tmp.path().join("dirty.iff");
    std::fs::write(&iff_path, synthetic_bhav_iff(&[dirty])).expect("write iff");

    // Default (strict_padding: true): preserves dirty padding via inline `op 0x0031`.
    let strict_out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .args(["--tree", "4096", "--verify-equiv"])
        .output()
        .expect("run strict lift");
    assert!(strict_out.status.success());
    let strict_stdout = String::from_utf8_lossy(&strict_out.stdout);
    assert!(strict_stdout.contains("op 0x0031"), "{strict_stdout}");
    assert!(
        strict_stdout.contains("// verify-equiv: Equivalent"),
        "{strict_stdout}"
    );

    // `--relax-padding`: strips verified-ignored padding, emits `notify_idle();`, still Equivalent!
    let relaxed_out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .args(["--tree", "4096", "--relax-padding", "--verify-equiv"])
        .output()
        .expect("run relaxed lift");
    assert!(relaxed_out.status.success());
    let relaxed_stdout = String::from_utf8_lossy(&relaxed_out.stdout);
    assert!(
        relaxed_stdout.contains("notify_idle();"),
        "{relaxed_stdout}"
    );
    assert!(!relaxed_stdout.contains("op 0x"), "{relaxed_stdout}");
    assert!(
        relaxed_stdout.contains("// verify-equiv: Equivalent"),
        "{relaxed_stdout}"
    );

    // `--raw --relax-padding` is rejected cleanly.
    let conflict = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .args(["--raw", "--relax-padding"])
        .output()
        .expect("run conflicting --raw --relax-padding");
    assert!(!conflict.status.success());
    assert!(
        String::from_utf8_lossy(&conflict.stderr).contains("--relax-padding"),
        "stderr: {}",
        String::from_utf8_lossy(&conflict.stderr)
    );
}

#[test]
fn lift_emits_symbol_comments_by_default_and_no_comments_disables_them() {
    use sims_antics::encode::{encode_bcon, encode_string_table};
    use sims_antics::ir::{BconChunk, StringEntry, StringFormat, StringTableChunk};

    let src = r#"
        tree 4096 "EatSnack" (args: 0, locals: 0) {
            my.motive[7] += bcon[0];
            if my.person[58] >= 18 {
                action_string(table: 300, index: 1);
                call 4097(0, 0, 0, 0);
            }
            return true;
        }
        tree 4097 "open" (args: 0, locals: 0) {
            return true;
        }
    "#;
    let compiled = compile_antics(src).expect("compile");
    let bcon = BconChunk {
        id: 4096,
        label: "FridgeTuning".to_owned(),
        flags: 0,
        values: vec![35],
        trailing: Vec::new(),
    };
    let str300 = StringTableChunk::new(
        *b"STR#",
        300,
        "Actions",
        StringFormat::Localized,
        vec![StringEntry::english("Have a Snack\n\x1b[31mNow")],
    );
    let iff_bytes = write_iff(&[
        ChunkData {
            tag: *b"BHAV",
            id: 4096,
            flags: 0,
            label: "EatSnack".to_owned(),
            data: encode_bhav(&compiled.behaviors[&4096]).unwrap(),
        },
        ChunkData {
            tag: *b"BHAV",
            id: 4097,
            flags: 0,
            label: "open".to_owned(),
            data: encode_bhav(&compiled.behaviors[&4097]).unwrap(),
        },
        ChunkData {
            tag: *b"BCON",
            id: 4096,
            flags: 0,
            label: bcon.label.clone(),
            data: encode_bcon(&bcon).unwrap(),
        },
        ChunkData {
            tag: *b"STR#",
            id: 300,
            flags: 0,
            label: str300.label.clone(),
            data: encode_string_table(&str300).unwrap(),
        },
    ])
    .expect("write iff");

    let tmp = CleanupDir::new("symbol-comments");
    let iff_path = tmp.path().join("annotated.iff");
    std::fs::write(&iff_path, &iff_bytes).expect("write iff");

    // Default CLI `lift --verify-equiv`: comments enabled, control chars sanitized, compiles & proves Equivalent!
    let with_comments = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .args(["--tree", "4096", "--verify-equiv"])
        .output()
        .expect("run lift with default comments");
    assert!(
        with_comments.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&with_comments.stderr)
    );
    let stdout = String::from_utf8(with_comments.stdout).expect("utf8 stdout");
    assert!(
        !stdout.contains('\x1b'),
        "control char leaked into comment:\n{stdout}"
    );
    assert!(
        stdout.contains("// Hunger, bcon[0:0] = 35 (\"FridgeTuning\")"),
        "{stdout}"
    );
    assert!(stdout.contains("// Age"), "{stdout}");
    assert!(stdout.contains("// \"Have a Snack [31mNow\""), "{stdout}");
    assert!(stdout.contains("// \"open\""), "{stdout}");
    assert!(stdout.contains("// verify-equiv: Equivalent"), "{stdout}");

    // `--no-comments`: suppresses inline symbol comments while preserving tree header & verify-equiv.
    let without_comments = Command::new(cli_bin())
        .args(["lift"])
        .arg(&iff_path)
        .args(["--tree", "4096", "--no-comments", "--verify-equiv"])
        .output()
        .expect("run lift --no-comments");
    assert!(without_comments.status.success());
    let plain_stdout = String::from_utf8(without_comments.stdout).expect("utf8 stdout");
    assert!(!plain_stdout.contains("// Hunger"), "{plain_stdout}");
    assert!(!plain_stdout.contains("// Age"), "{plain_stdout}");
    assert!(!plain_stdout.contains("// \"open\""), "{plain_stdout}");
    assert!(
        plain_stdout.contains("// verify-equiv: Equivalent"),
        "{plain_stdout}"
    );
}

fn one_node_bhav(id: u16, args: u8) -> BhavChunk {
    BhavChunk {
        id,
        label: format!("Callee{id}"),
        format: 0x8002,
        kind: 0,
        args,
        locals: 0,
        revision: 0,
        instructions: vec![Instruction {
            opcode: 2,
            on_true: 0xFE,
            on_false: 0xFE,
            operands: [0, 0, 0, 0, 0, 5, 0, 7],
        }],
        trailing: Vec::new(),
    }
}

/// A private tree making an inline call to `first` and then to `second`.
fn two_call_bhav(id: u16, first: u16, second: u16) -> BhavChunk {
    BhavChunk {
        id,
        label: format!("Caller{id}"),
        format: 0x8002,
        kind: 0,
        args: 0,
        locals: 0,
        revision: 0,
        instructions: vec![
            Instruction {
                opcode: first,
                on_true: 1,
                on_false: 0xFF,
                operands: [1, 0, 2, 0, 3, 0, 4, 0],
            },
            Instruction {
                opcode: second,
                on_true: 0xFE,
                on_false: 0xFF,
                operands: [5, 0, 0, 0, 0, 0, 0, 0],
            },
        ],
        trailing: Vec::new(),
    }
}

/// A synthetic install: `Global.iff` (300: 4 args, 301: 6 args), a `TestGlobals.iff`
/// semi-global (8192: 1 arg), and `Objects/Test.far!Thing.iff` linking to it with
/// callers 4096 (300 then 8192), 4097 (301), and 4098 (302, absent from `Global.iff`).
struct SyntheticInstall {
    _dir: CleanupDir,
    root: PathBuf,
}

impl SyntheticInstall {
    fn new() -> Self {
        let dir = CleanupDir::new("synthetic-install");
        let root = dir.path().join("install");
        let game_data = root.join("GameData");
        std::fs::create_dir_all(game_data.join("Global")).expect("mkdir Global");
        std::fs::create_dir_all(game_data.join("Objects")).expect("mkdir Objects");
        std::fs::write(
            game_data.join("Behavior.iff"),
            write_iff(&[]).expect("empty iff"),
        )
        .expect("write Behavior.iff");
        std::fs::write(
            game_data.join("Global").join("Global.iff"),
            synthetic_bhav_iff(&[one_node_bhav(300, 4), one_node_bhav(301, 6)]),
        )
        .expect("write Global.iff");
        std::fs::write(
            game_data.join("Global").join("TestGlobals.iff"),
            synthetic_bhav_iff(&[one_node_bhav(8192, 1)]),
        )
        .expect("write TestGlobals.iff");

        let thing = synthetic_object_iff(Some(b"TestGlobals"));
        let far = package_far(&[("Thing.iff", &thing)]).expect("package far");
        std::fs::write(game_data.join("Objects").join("Test.far"), far).expect("write far");
        // The same trees as a loose object file without a GLOB link.
        std::fs::write(
            game_data.join("Objects").join("Loose.iff"),
            synthetic_object_iff(None),
        )
        .expect("write Loose.iff");
        Self { _dir: dir, root }
    }

    fn far(&self) -> PathBuf {
        self.root.join("GameData").join("Objects").join("Test.far")
    }

    fn global_dir(&self) -> PathBuf {
        self.root.join("GameData").join("Global")
    }
}

/// The synthetic object's trees (4096..=4098), with a `GLOB` chunk naming `link` if given.
fn synthetic_object_iff(link: Option<&[u8]>) -> Vec<u8> {
    let trees = [
        two_call_bhav(4096, 300, 8192),
        two_call_bhav(4097, 301, 300),
        two_call_bhav(4098, 302, 300),
    ];
    let mut chunks: Vec<ChunkData> = trees
        .iter()
        .map(|bhav| ChunkData {
            tag: *b"BHAV",
            id: bhav.id,
            flags: 0,
            label: bhav.label.clone(),
            data: encode_bhav(bhav).expect("encode_bhav"),
        })
        .collect();
    if let Some(link) = link {
        let mut glob = vec![link.len() as u8];
        glob.extend_from_slice(link);
        chunks.push(ChunkData {
            tag: *b"GLOB",
            id: 128,
            flags: 0,
            label: String::new(),
            data: glob,
        });
    }
    write_iff(&chunks).expect("write object iff")
}

fn run_lift(args: &[&OsStr]) -> (bool, String, String) {
    let out = Command::new(cli_bin())
        .arg("lift")
        .args(args)
        .output()
        .expect("run lift");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn lift_inside_install_uses_catalog_global_and_semi_global_arities() {
    let install = SyntheticInstall::new();
    let far = install.far();
    let (ok, stdout, stderr) = run_lift(&[
        far.as_os_str(),
        OsStr::new("--entry"),
        OsStr::new("Thing.iff"),
        OsStr::new("--tree"),
        OsStr::new("4096"),
        OsStr::new("--verify-equiv"),
    ]);
    assert!(ok, "stderr: {stderr}\nstdout: {stdout}");
    let header = stdout.lines().next().unwrap_or_default();
    assert!(
        header.starts_with("// arity sources: ")
            && header
                .contains("global 256..=4095: 2 from GameData/Global/Global.iff (install catalog)")
            && header.contains("private 4096..=8191: 3 from this file")
            && header.contains(
                "semi-global 8192+: 1 from GameData/Global/TestGlobals.iff (install catalog)"
            ),
        "{stdout}"
    );
    assert!(
        !header.contains(&*install.root.to_string_lossy()),
        "catalog locations are install-relative: {header}"
    );
    assert!(stdout.contains("// .air (Antics IR) tree 4096"), "{stdout}");
    assert!(!stdout.contains("structured lift fallback"), "{stdout}");
    assert!(stdout.contains("// verify-equiv: Equivalent"), "{stdout}");

    // Known 6-arg global and a callee missing from Global.iff fall back, naming the cause.
    let (ok, stdout, stderr) = run_lift(&[
        far.as_os_str(),
        OsStr::new("--entry"),
        OsStr::new("Thing.iff"),
        OsStr::new("--verify-equiv"),
    ]);
    assert!(ok, "stderr: {stderr}\nstdout: {stdout}");
    assert!(
        stdout.contains(
            "tree 4097 \"Caller4097\": structured lift fallback (unsupported header or target: node 0 calls global tree 301 with inline operands and 6 args"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("node 0 calls global tree 302 with inline operands but its arity is unknown; no usable arity for tree 302 in GameData/Global/Global.iff (absent, or duplicate chunks disagree))"),
        "{stdout}"
    );
    assert_eq!(
        stdout.matches("// verify-equiv: Equivalent").count(),
        3,
        "{stdout}"
    );

    // `--list` uses the same arities and a compact reason.
    let (ok, stdout, stderr) = run_lift(&[
        far.as_os_str(),
        OsStr::new("--entry"),
        OsStr::new("Thing.iff"),
        OsStr::new("--list"),
    ]);
    assert!(ok, "stderr: {stderr}");
    let row = |id: &str| {
        stdout
            .lines()
            .find(|l| l.starts_with(id))
            .unwrap_or_default()
            .to_owned()
    };
    assert!(stdout.starts_with("// arity sources: "), "{stdout}");
    assert!(row("4096").contains(" structured "), "{stdout}");
    assert!(
        row("4097").contains("labeled-cfg (calls global 301 with 6 args)"),
        "{stdout}"
    );
    assert!(
        row("4098").contains("labeled-cfg (calls global 302, arity unknown)"),
        "{stdout}"
    );

    // A loose object file without a GLOB link has no semi-global library in the VM, so
    // semi-global arities stay unknown (its own stray 8192+ chunks are not trusted).
    let loose = install
        .root
        .join("GameData")
        .join("Objects")
        .join("Loose.iff");
    let (ok, stdout, stderr) =
        run_lift(&[loose.as_os_str(), OsStr::new("--tree"), OsStr::new("4096")]);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("semi-global 8192+: none (no resolved GLOB link)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("node 1 calls semi-global tree 8192 with inline operands but its arity is unknown; semi-global arities not loaded (no resolved GLOB link); pass --semiglobal <semiglobal.iff>"),
        "{stdout}"
    );
}

#[test]
fn lift_no_catalog_and_explicit_arity_overrides() {
    let install = SyntheticInstall::new();
    let far = install.far();
    let base: [&OsStr; 5] = [
        far.as_os_str(),
        OsStr::new("--entry"),
        OsStr::new("Thing.iff"),
        OsStr::new("--tree"),
        OsStr::new("4096"),
    ];

    // `--no-catalog` keeps the private-only behaviour and explains the fallback.
    let mut args = base.to_vec();
    args.extend([OsStr::new("--no-catalog"), OsStr::new("--verify-equiv")]);
    let (ok, stdout, stderr) = run_lift(&args);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("global 256..=4095: none (--no-catalog)")
            && stdout.contains("semi-global 8192+: none (--no-catalog)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("structured lift fallback (unsupported header or target: node 0 calls global tree 300 with inline operands but its arity is unknown; global arities not loaded (--no-catalog); pass --globals <Global.iff>)"),
        "{stdout}"
    );
    assert!(stdout.contains("// verify-equiv: Equivalent"), "{stdout}");

    // `--globals` alone moves the cause to the semi-global callee.
    let globals = install.global_dir().join("Global.iff");
    let mut args = base.to_vec();
    args.extend([
        OsStr::new("--no-catalog"),
        OsStr::new("--globals"),
        globals.as_os_str(),
    ]);
    let (ok, stdout, stderr) = run_lift(&args);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("global 256..=4095: 2 from ") && stdout.contains("Global.iff (explicit)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("node 1 calls semi-global tree 8192 with inline operands but its arity is unknown; semi-global arities not loaded (--no-catalog); pass --semiglobal <semiglobal.iff>"),
        "{stdout}"
    );

    // `--globals` plus `--semiglobal` (as a FAR member) lifts structurally.
    let tmp = CleanupDir::new("semi-far");
    let semi_bytes =
        std::fs::read(install.global_dir().join("TestGlobals.iff")).expect("read semi");
    let semi_far = tmp.path().join("Semi.far");
    std::fs::write(
        &semi_far,
        package_far(&[("TestGlobals.iff", &semi_bytes)]).expect("package"),
    )
    .expect("write Semi.far");
    let semi_spec = format!("{}!TestGlobals.iff", semi_far.display());
    let mut args = base.to_vec();
    args.extend([
        OsStr::new("--no-catalog"),
        OsStr::new("--globals"),
        globals.as_os_str(),
        OsStr::new("--semiglobal"),
        OsStr::new(&semi_spec),
        OsStr::new("--verify-equiv"),
    ]);
    let (ok, stdout, stderr) = run_lift(&args);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("semi-global 8192+: 1 from ")
            && stdout.contains("Semi.far!TestGlobals.iff (explicit)"),
        "{stdout}"
    );
    assert!(!stdout.contains("structured lift fallback"), "{stdout}");
    assert!(stdout.contains("// verify-equiv: Equivalent"), "{stdout}");

    // A FAR `--semiglobal` must name its member; `--raw` rejects arity flags.
    let mut args = base.to_vec();
    args.extend([OsStr::new("--semiglobal"), semi_far.as_os_str()]);
    let (ok, _, stderr) = run_lift(&args);
    assert!(!ok && stderr.contains("name a member"), "stderr: {stderr}");
    let mut args = base.to_vec();
    args.extend([OsStr::new("--raw"), OsStr::new("--no-catalog")]);
    let (ok, _, stderr) = run_lift(&args);
    assert!(
        !ok && stderr.contains("cannot be combined with --raw"),
        "stderr: {stderr}"
    );
}

/// `SIMS_INSTALL`-gated: the CLI path on `Objects.far!Fridges.iff` uses the catalog
/// arities the corpus run uses, lifting all 51 trees structurally and proving each one
/// `Equivalent` (strict and relaxed padding). Prints metadata only.
#[test]
fn install_fridges_cli_lift_is_fully_structured_and_equivalent() {
    let Ok(root) = std::env::var("SIMS_INSTALL") else {
        eprintln!("install_fridges_cli_lift: skipped (SIMS_INSTALL unset)");
        return;
    };
    if root.trim().is_empty() {
        eprintln!("install_fridges_cli_lift: skipped (SIMS_INSTALL empty)");
        return;
    }
    let install = sims_content::Install::detect(Path::new(&root))
        .expect("SIMS_INSTALL did not resolve to a valid Sims installation");
    let objects = install
        .file("GameData/Objects/Objects.far")
        .expect("Objects.far in SIMS_INSTALL");
    for extra in [None, Some("--relax-padding")] {
        let mut args: Vec<&OsStr> = vec![
            objects.as_os_str(),
            OsStr::new("--entry"),
            OsStr::new("Fridges.iff"),
            OsStr::new("--verify-equiv"),
        ];
        if let Some(flag) = extra {
            args.push(OsStr::new(flag));
        }
        let (ok, stdout, _) = run_lift(&args);
        assert!(ok, "lift Fridges.iff ({extra:?}) failed");
        let structured = stdout.matches("// .air (Antics IR) tree ").count();
        let fallbacks = stdout.matches("structured lift fallback").count();
        let equivalent = stdout.matches("// verify-equiv: Equivalent").count();
        let header = stdout.lines().next().unwrap_or_default();
        assert!(
            header.contains("Global.iff (install catalog)"),
            "global arities must come from the install catalog"
        );
        eprintln!(
            "install_fridges_cli_lift ({extra:?}): {structured} structured, {fallbacks} labeled-CFG, {equivalent} Equivalent"
        );
        assert_eq!((structured, fallbacks, equivalent), (51, 0, 51));
    }
}

#[test]
fn lift_owner_dependent_ranges_stay_unknown_for_global_and_semi_global_files() {
    let install = SyntheticInstall::new();
    // Global.iff runs under whichever object called it: private and semi-global callees
    // depend on that owner, so only its own global range is known.
    let global = install.global_dir().join("Global.iff");
    let (ok, stdout, stderr) = run_lift(&[global.as_os_str(), OsStr::new("--list")]);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("global 256..=4095: 2 from GameData/Global/Global.iff (install catalog)")
            && stdout.contains(
                "private 4096..=8191: none (owner-dependent: Global.iff runs under the calling object)"
            )
            && stdout.contains(
                "semi-global 8192+: none (owner-dependent: Global.iff runs under the calling object)"
            ),
        "{stdout}"
    );
    // A semi-global file knows its own semi-global range (every owner reaching it links
    // to it) but not the calling object's private trees.
    let semi = install.global_dir().join("TestGlobals.iff");
    let (ok, stdout, stderr) = run_lift(&[semi.as_os_str(), OsStr::new("--list")]);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains(
            "private 4096..=8191: none (owner-dependent: semi-global file runs under the calling object)"
        ) && stdout.contains("semi-global 8192+: 1 from this file (linked as a semi-global)"),
        "{stdout}"
    );
}

#[test]
fn lift_arity_override_specs_are_strict_and_validated() {
    let install = SyntheticInstall::new();
    let far = install.far();
    let tmp = CleanupDir::new("override-specs");
    let globals_bytes = std::fs::read(install.global_dir().join("Global.iff")).expect("read");

    // `--globals Lib.far` defaults to `Global.iff` and never suffix-matches `NotGlobal.iff`.
    let lib = tmp.path().join("Lib.far");
    std::fs::write(
        &lib,
        package_far(&[("NotGlobal.iff", &globals_bytes)]).expect("package"),
    )
    .expect("write Lib.far");
    let (ok, _, stderr) = run_lift(&[
        far.as_os_str(),
        OsStr::new("--entry"),
        OsStr::new("Thing.iff"),
        OsStr::new("--globals"),
        lib.as_os_str(),
    ]);
    assert!(
        !ok && stderr.contains("--globals: FAR member 'Global.iff' not found"),
        "stderr: {stderr}"
    );

    // A member name containing `!` is addressable: the spec splits at the first `.far!`.
    let bang = tmp.path().join("Bang.far");
    std::fs::write(
        &bang,
        package_far(&[("Food!Prep.iff", &globals_bytes)]).expect("package"),
    )
    .expect("write Bang.far");
    let spec = format!("{}!Food!Prep.iff", bang.display());
    let (ok, stdout, stderr) = run_lift(&[
        far.as_os_str(),
        OsStr::new("--entry"),
        OsStr::new("Thing.iff"),
        OsStr::new("--tree"),
        OsStr::new("4096"),
        OsStr::new("--no-catalog"),
        OsStr::new("--globals"),
        OsStr::new(&spec),
    ]);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("Bang.far!Food!Prep.iff (explicit)"),
        "{stdout}"
    );

    // Arity flags with an archive-only listing are rejected rather than ignored.
    let (ok, _, stderr) = run_lift(&[
        far.as_os_str(),
        OsStr::new("--list"),
        OsStr::new("--globals"),
        OsStr::new("missing.iff"),
    ]);
    assert!(!ok && stderr.contains("need --entry"), "stderr: {stderr}");

    // A missing tree is reported before a bad override is read.
    let (ok, _, stderr) = run_lift(&[
        far.as_os_str(),
        OsStr::new("--entry"),
        OsStr::new("Thing.iff"),
        OsStr::new("--tree"),
        OsStr::new("4999"),
        OsStr::new("--globals"),
        OsStr::new("missing.iff"),
    ]);
    assert!(!ok && stderr.contains("BHAV #4999"), "stderr: {stderr}");
}

/// An object reached through a symlink in install A that points into install B uses A's
/// catalog (the install the VM would boot), not B's.
#[cfg(unix)]
#[test]
fn lift_through_symlink_uses_the_install_it_was_reached_from() {
    let a = SyntheticInstall::new();
    let b = SyntheticInstall::new();
    // In B, global #300 takes 6 args, so B's arities would force a labeled-CFG fallback.
    std::fs::write(
        b.global_dir().join("Global.iff"),
        synthetic_bhav_iff(&[one_node_bhav(300, 6), one_node_bhav(301, 6)]),
    )
    .expect("rewrite B Global.iff");
    let alias = a.root.join("GameData").join("Objects").join("Alias.far");
    std::os::unix::fs::symlink(b.far(), &alias).expect("symlink");

    let (ok, stdout, stderr) = run_lift(&[
        alias.as_os_str(),
        OsStr::new("--entry"),
        OsStr::new("Thing.iff"),
        OsStr::new("--tree"),
        OsStr::new("4096"),
        OsStr::new("--verify-equiv"),
    ]);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains(
            "semi-global 8192+: 1 from GameData/Global/TestGlobals.iff (install catalog)"
        ),
        "the symlinked member must match A's index entry and keep its GLOB link:\n{stdout}"
    );
    assert!(
        !stdout.contains("structured lift fallback"),
        "A's 4-arg #300 must apply, not B's 6-arg #300:\n{stdout}"
    );
    assert!(stdout.contains("// verify-equiv: Equivalent"), "{stdout}");
}

/// Index matching is by file identity: a `..` path finds its entry; a file reached
/// through an outside symlink and indexed twice (as `Objects/Loose.iff` and a
/// `Global/Shadow.iff` alias) resolves when both entries agree; an unindexed copy under
/// the install root gets private arities only.
#[cfg(unix)]
#[test]
fn lift_index_matching_uses_file_identity() {
    let install = SyntheticInstall::new();
    let objects = install.root.join("GameData").join("Objects");
    let loose = objects.join("Loose.iff");
    std::os::unix::fs::symlink(&loose, install.global_dir().join("Shadow.iff"))
        .expect("symlink Shadow.iff");
    std::fs::create_dir_all(objects.join("sub")).expect("mkdir sub");

    let dotted = objects.join("sub").join("..").join("Loose.iff");
    let (ok, stdout, stderr) = run_lift(&[dotted.as_os_str(), OsStr::new("--list")]);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("private 4096..=8191: 3 from this file")
            && stdout.contains("semi-global 8192+: none (no resolved GLOB link)"),
        "a `..` path must match Objects/Loose.iff itself:\n{stdout}"
    );

    let outside = CleanupDir::new("outside-link");
    let via = outside.path().join("via.iff");
    std::os::unix::fs::symlink(&loose, &via).expect("symlink via.iff");
    let (ok, stdout, stderr) = run_lift(&[via.as_os_str(), OsStr::new("--list")]);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("global 256..=4095: 2 from GameData/Global/Global.iff (install catalog)")
            && stdout.contains("private 4096..=8191: 3 from this file")
            && stdout.contains("semi-global 8192+: none (no resolved GLOB link)"),
        "both entries resolve the same way:\n{stdout}"
    );

    let scratch = install.root.join("scratch");
    std::fs::create_dir_all(&scratch).expect("mkdir scratch");
    let copy = scratch.join("Copy.iff");
    std::fs::copy(&loose, &copy).expect("copy");
    let (ok, stdout, stderr) = run_lift(&[copy.as_os_str(), OsStr::new("--list")]);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("global 256..=4095: none (file not in install index)")
            && stdout.contains("private 4096..=8191: 3 from this file"),
        "{stdout}"
    );
}

/// A FIFO elsewhere in the install must not block catalog discovery.
// Subprocess hang-watchdog timeout in a CLI integration test (not simulation logic).
#[cfg(unix)]
#[test]
#[allow(clippy::disallowed_methods)]
fn lift_install_discovery_skips_fifos_in_mounts() {
    use std::time::{Duration, Instant};
    let install = SyntheticInstall::new();
    let downloads = install.root.join("Downloads");
    std::fs::create_dir_all(&downloads).expect("mkdir Downloads");
    let status = Command::new("mkfifo")
        .arg(downloads.join("pipe.iff"))
        .arg(downloads.join("pipe.far"))
        .status()
        .expect("mkfifo should be available on Unix");
    assert!(status.success(), "mkfifo failed");

    let mut child = Command::new(cli_bin())
        .arg("lift")
        .arg(install.far())
        .args(["--entry", "Thing.iff", "--tree", "4096"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn lift");
    let deadline = Instant::now() + Duration::from_secs(20);
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
    let st = exited.expect("lift blocked on a FIFO in the install's Downloads mount");
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut stdout)
        .expect("read stdout");
    assert!(st.success(), "{stdout}");
    assert!(
        stdout.contains("GameData/Global/Global.iff (install catalog)"),
        "{stdout}"
    );
}

#[test]
fn lift_override_ambiguity_and_bang_directories() {
    let install = SyntheticInstall::new();
    let far = install.far();
    let tmp = CleanupDir::new("override-ambiguity");
    let globals_bytes = std::fs::read(install.global_dir().join("Global.iff")).expect("read");

    let lib = tmp.path().join("Lib.far");
    std::fs::write(
        &lib,
        package_far(&[
            ("a/Global.iff", &globals_bytes),
            ("b/Global.iff", &globals_bytes),
        ])
        .expect("package"),
    )
    .expect("write Lib.far");
    let (ok, _, stderr) = run_lift(&[
        far.as_os_str(),
        OsStr::new("--entry"),
        OsStr::new("Thing.iff"),
        OsStr::new("--globals"),
        lib.as_os_str(),
    ]);
    assert!(
        !ok && stderr.contains("--globals: ambiguous FAR member 'Global.iff' matched 2"),
        "stderr: {stderr}"
    );

    // A loose IFF under a directory whose name contains `.far!` is used as-is.
    let staging = tmp.path().join("Patch.far! staging");
    std::fs::create_dir_all(&staging).expect("mkdir staging");
    let staged = staging.join("Global.iff");
    std::fs::write(&staged, &globals_bytes).expect("write staged");
    let (ok, stdout, stderr) = run_lift(&[
        far.as_os_str(),
        OsStr::new("--entry"),
        OsStr::new("Thing.iff"),
        OsStr::new("--tree"),
        OsStr::new("4096"),
        OsStr::new("--no-catalog"),
        OsStr::new("--globals"),
        staged.as_os_str(),
    ]);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("Patch.far! staging/Global.iff (explicit)"),
        "{stdout}"
    );
}

/// `Objects/link/../Loose.iff` opens the `Loose.iff` beside the link's target, not
/// `Objects/Loose.iff`: the catalog entry must be the file actually opened, or none.
#[cfg(unix)]
#[test]
fn lift_symlink_then_dotdot_uses_the_opened_file_not_the_lexical_entry() {
    let install = SyntheticInstall::new();
    let objects = install.root.join("GameData").join("Objects");

    // Target inside the install: the opened file is indexed and has its own GLOB link,
    // while the lexical entry (Objects/Loose.iff) has none.
    std::fs::create_dir_all(objects.join("actual").join("sub")).expect("mkdir actual/sub");
    std::fs::write(
        objects.join("actual").join("Loose.iff"),
        synthetic_object_iff(Some(b"TestGlobals")),
    )
    .expect("write actual/Loose.iff");
    std::os::unix::fs::symlink(objects.join("actual").join("sub"), objects.join("link"))
        .expect("symlink link");
    let via = objects.join("link").join("..").join("Loose.iff");
    let (ok, stdout, stderr) = run_lift(&[via.as_os_str(), OsStr::new("--list")]);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains(
            "semi-global 8192+: 1 from GameData/Global/TestGlobals.iff (install catalog)"
        ),
        "must resolve through the opened actual/Loose.iff:\n{stdout}"
    );

    // Target outside the install: the opened file is not indexed, so only its own
    // private range is known, and the header says why.
    let outside = CleanupDir::new("dotdot-outside");
    std::fs::create_dir_all(outside.path().join("sub")).expect("mkdir outside/sub");
    std::fs::write(
        outside.path().join("Loose.iff"),
        synthetic_object_iff(Some(b"TestGlobals")),
    )
    .expect("write outside/Loose.iff");
    std::os::unix::fs::symlink(outside.path().join("sub"), objects.join("away"))
        .expect("symlink away");
    let via = objects.join("away").join("..").join("Loose.iff");
    let (ok, stdout, stderr) = run_lift(&[via.as_os_str(), OsStr::new("--list")]);
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("global 256..=4095: none (file not in install index)")
            && stdout.contains("private 4096..=8191: 3 from this file")
            && stdout.contains("semi-global 8192+: none (file not in install index)"),
        "{stdout}"
    );
}

/// `Objects/Global.iff` is a hard link to `Global/Global.iff`, and `Objects/link` points
/// at `Global/sub`. Whether the file is opened directly or as `Objects/link/../Global.iff`
/// (which names the object alias lexically), it is one file with two roles, so its
/// owner-dependent ranges stay unknown and the header names both entries.
#[cfg(unix)]
#[test]
fn lift_file_indexed_under_two_roles_is_ambiguous() {
    let install = SyntheticInstall::new();
    let objects = install.root.join("GameData").join("Objects");
    let global = install.global_dir().join("Global.iff");
    std::fs::hard_link(&global, objects.join("Global.iff")).expect("hard link Global.iff");
    std::fs::create_dir_all(install.global_dir().join("sub")).expect("mkdir Global/sub");
    std::os::unix::fs::symlink(install.global_dir().join("sub"), objects.join("link"))
        .expect("symlink link");

    let why = "ambiguous: the same file is indexed as GameData/Global/Global.iff and GameData/Objects/Global.iff, with different code-owner roles";
    for input in [
        global.clone(),
        objects.join("link").join("..").join("Global.iff"),
    ] {
        let (ok, stdout, stderr) = run_lift(&[input.as_os_str(), OsStr::new("--list")]);
        assert!(ok, "stderr: {stderr}");
        assert!(
            stdout
                .contains("global 256..=4095: 2 from GameData/Global/Global.iff (install catalog)")
                && stdout.contains(&format!("private 4096..=8191: none ({why})"))
                && stdout.contains(&format!("semi-global 8192+: none ({why})")),
            "{}:\n{stdout}",
            input.display()
        );
    }
}

/// An atomic save replaces the file at a path with a new inode. The CLI's test hook
/// renames a staged file over the input after the CLI has read it and before the
/// catalog lookup; the lookup uses the identity of the CLI's own read handle, so the
/// replaced file finds no index entry and the new file's role is not borrowed.
#[cfg(unix)]
#[test]
fn lift_catalog_lookup_never_matches_a_file_replaced_after_reading() {
    let install = SyntheticInstall::new();
    let objects = install.root.join("GameData").join("Objects");
    let loose = objects.join("Loose.iff");
    let staged = objects.join("Loose.iff.saving");
    std::fs::write(&staged, synthetic_object_iff(Some(b"TestGlobals"))).expect("stage");

    let out = Command::new(cli_bin())
        .arg("lift")
        .arg(&loose)
        .arg("--list")
        .env("SIMS_ANTICS_TEST_REPLACE_INPUT_AFTER_READ", &staged)
        .output()
        .expect("run lift");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!staged.exists(), "the hook must have replaced the input");
    assert!(
        stdout.contains("global 256..=4095: none (file not in install index)")
            && stdout.contains("semi-global 8192+: none (file not in install index)"),
        "the replaced file must not take the new file's GLOB link:\n{stdout}"
    );
}

/// An index path that cannot be `stat`ed (here: in a directory without search
/// permission) might be the lifted file under another role, so owner-dependent ranges
/// are unknown and the header names it.
#[cfg(unix)]
#[test]
fn lift_with_an_unidentifiable_index_path_keeps_owner_ranges_unknown() {
    use std::os::unix::fs::PermissionsExt;

    struct RestoreMode(PathBuf);
    impl Drop for RestoreMode {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    let install = SyntheticInstall::new();
    let objects = install.root.join("GameData").join("Objects");
    let locked = objects.join("locked");
    std::fs::create_dir_all(&locked).expect("mkdir locked");
    std::fs::write(locked.join("Hidden.iff"), synthetic_object_iff(None)).expect("write");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    let _restore = RestoreMode(locked.clone());
    if std::fs::metadata(locked.join("Hidden.iff")).is_ok() {
        eprintln!("skipping: permissions are not enforced for this user");
        return;
    }

    let loose = objects.join("Loose.iff");
    let (ok, stdout, stderr) = run_lift(&[loose.as_os_str(), OsStr::new("--list")]);
    assert!(ok, "stderr: {stderr}");
    let why = "unknown code-owner role: 1 install index path(s) could not be identified: GameData/Objects/locked/Hidden.iff (";
    assert!(
        stdout.contains("global 256..=4095: 2 from GameData/Global/Global.iff (install catalog)")
            && stdout.contains(&format!("private 4096..=8191: none ({why}"))
            && stdout.contains(&format!("semi-global 8192+: none ({why}")),
        "{stdout}"
    );
}
