//! `sims-antics test` end to end: the repository's own `open-pack/tests/*.test.toml` specs
//! pass through the built binary, and failing or broken specs fail it with a summary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn sims_antics(args: &[&Path]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sims-antics"))
        .arg("test")
        .args(args)
        .output()
        .expect("run sims-antics")
}

fn open_pack() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../open-pack")
}

// Sorted before use, so the order never depends on the filesystem.
#[allow(clippy::disallowed_methods)]
fn specs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(open_pack().join("tests"))
        .expect("open-pack/tests")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.to_string_lossy().ends_with(".test.toml"))
        .collect();
    out.sort();
    out
}

/// A scratch directory for this process (left in place: tests never delete).
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("antics-test-runner-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn every_open_pack_spec_passes() {
    let files = specs();
    assert!(files.len() >= 4, "{files:?}");
    for file in &files {
        let out = sims_antics(&[file]);
        let text = stdout(&out);
        assert!(out.status.success(), "{}:\n{text}\n{out:?}", file.display());
        assert!(!text.contains("FAIL"), "{text}");
        // Every open-pack Main loops, so no case runs under an assumption.
        assert!(!text.contains("ASSUME"), "{text}");
        let summary = text.lines().last().unwrap_or_default();
        assert!(summary.ends_with(" passed, 0 failed"), "{summary}");
        let passed: usize = summary.split(' ').next().unwrap().parse().unwrap();
        assert_eq!(
            passed,
            text.lines().filter(|l| l.starts_with("PASS ")).count()
        );
        assert!(passed > 0, "{text}");
    }
}

/// A Main that returns faults the lot update (error 0x3e9), so `idle_main` runs the cases
/// with an idle loop in its place and says so before every case. The open-pack objects all
/// loop (v2's default Main idles), so the subject here is the mailbox given a returning Main.
#[test]
fn idle_main_prints_an_assume_line_with_every_case() {
    let dir = scratch("idle-main");
    let pack = open_pack();
    let source = std::fs::read_to_string(pack.join("objects/open-mailbox.antics")).unwrap();
    let at = source.find("    interaction ").expect("an interaction");
    let returning = format!(
        "{}    main {{\n        return true;\n    }}\n\n{}",
        &source[..at],
        &source[at..]
    );
    let object = dir.join("returning-mailbox.antics");
    std::fs::write(&object, returning).unwrap();
    let spec_text = std::fs::read_to_string(pack.join("tests/open-mailbox.test.toml")).unwrap();
    let spec_text = spec_text
        .replace(
            "antics = \"../objects/open-mailbox.antics\"",
            &format!("antics = {:?}", object.display().to_string()),
        )
        // The mailbox imports nothing, so its copy needs no pack.
        .replace("pack = \"..\"", "idle_main = true")
        // Its Main returns at once now, as the real Main of a case runs.
        .replace(
            "turns = 4\nexpect = { result = \"yield\" }",
            "expect = { result = true, turns = 0 }",
        );
    let file = dir.join("returning-mailbox.test.toml");
    std::fs::write(&file, spec_text).unwrap();
    let out = sims_antics(&[&file]);
    let text = stdout(&out);
    assert!(out.status.success(), "{text}\n{out:?}");
    let lines: Vec<&str> = text.lines().collect();
    let cases: Vec<usize> = (0..lines.len())
        .filter(|&i| lines[i].starts_with("PASS ") || lines[i].starts_with("FAIL "))
        .collect();
    assert!(!cases.is_empty(), "{text}");
    for i in cases {
        assert!(
            i > 0
                && lines[i - 1].starts_with("ASSUME ")
                && lines[i - 1].contains("returning-mailbox.test.toml: subject Main (OBJD #")
                && lines[i - 1].ends_with(") replaced by an idle loop (idle_main)"),
            "{text}"
        );
    }
}

#[test]
fn a_directory_runs_every_spec_below_it() {
    let out = sims_antics(&[&open_pack().join("tests")]);
    let text = stdout(&out);
    assert!(out.status.success(), "{text}\n{out:?}");
    for file in specs() {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        assert!(text.contains(&name), "{name} missing from:\n{text}");
    }
    let total: usize = text
        .lines()
        .last()
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(total >= 25, "{text}");
}

#[test]
fn failures_print_field_expected_actual_and_exit_nonzero() {
    let dir = scratch("fail");
    let pack = open_pack();
    let spec = dir.join("tv.test.toml");
    std::fs::write(
        &spec,
        format!(
            r#"[subject]
antics = "{tv}"
pack = "{pack}"

[sim]
motives = {{ fun = 0 }}

[[test]]
name = "right"
guard = "Turn On"
expect = {{ result = true }}

[[test]]
name = "wrong"
action = "Turn On"
expect = {{ result = false, attrs = {{ power = 0 }}, sounds = [] }}
"#,
            tv = pack.join("objects/open-tv.antics").display(),
            pack = pack.display(),
        ),
    )
    .unwrap();
    let out = sims_antics(&[&spec]);
    let text = stdout(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("PASS ") && text.contains(": right"), "{text}");
    assert!(text.contains("FAIL ") && text.contains(": wrong"), "{text}");
    assert!(
        text.contains("    result: expected false, got true"),
        "{text}"
    );
    assert!(
        text.contains("    attrs.power (attr[1]): expected 0, got 1"),
        "{text}"
    );
    assert!(
        text.contains("    sounds: expected [], got [\"open_tv_click\"]"),
        "{text}"
    );
    assert_eq!(text.lines().last(), Some("1 passed, 1 failed"), "{text}");
}

#[test]
fn a_broken_spec_is_reported_and_fails_the_run() {
    let dir = scratch("broken");
    let spec = dir.join("broken.test.toml");
    std::fs::write(
        &spec,
        "[subject]\nantics = \"nowhere.antics\"\n\n[[test]]\nname = \"t\"\ntree = 1\n",
    )
    .unwrap();
    let bad_toml = dir.join("bad.test.toml");
    std::fs::write(&bad_toml, "[subject]\nantics = \"x.antics\"\nbogus = 1\n").unwrap();
    let out = sims_antics(&[&dir]);
    let text = stdout(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("ERROR ") && text.contains("nowhere.antics"),
        "{text}"
    );
    assert!(text.contains("unknown key 'bogus'"), "{text}");
    assert_eq!(
        text.lines().last(),
        Some("0 passed, 0 failed, 2 file(s) could not run"),
        "{text}"
    );
}

#[test]
fn an_empty_directory_is_an_error() {
    let dir = scratch("empty");
    let out = sims_antics(&[&dir]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no *.test.toml files"),
        "{out:?}"
    );
}

#[test]
fn the_synthetic_stub_spec_passes() {
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test_runner/stub-probe.test.toml");
    let out = sims_antics(&[&spec]);
    let text = stdout(&out);
    assert!(out.status.success(), "{text}\n{out:?}");
    assert!(text.contains(": pushing Poke on the Sim"), "{text}");
    assert_eq!(text.lines().last(), Some("5 passed, 0 failed"), "{text}");
}
