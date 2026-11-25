//! `sims-antics test` argument handling, in every feature set; without `vm-replay` the
//! subcommand refuses to run.

use std::path::Path;
use std::process::{Command, Output};

fn sims_antics(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sims-antics"))
        .args(args)
        .output()
        .expect("run sims-antics")
}

fn open_pack_tests() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../open-pack/tests")
        .display()
        .to_string()
}

#[test]
fn test_needs_a_path() {
    let out = sims_antics(&["test"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("test requires at least one"),
        "{out:?}"
    );
}

#[test]
fn test_rejects_unknown_flags() {
    let out = sims_antics(&["test", "--fast", &open_pack_tests()]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unknown flag '--fast'"),
        "{out:?}"
    );
}

#[test]
fn usage_lists_test() {
    let out = sims_antics(&["--help"]);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("sims-antics test <file.test.toml|dir...>"),
        "{out:?}"
    );
}

#[cfg(not(feature = "vm-replay"))]
#[test]
fn test_without_vm_replay_says_so_and_fails() {
    let out = sims_antics(&["test", &open_pack_tests()]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("needs sims-vm") && err.contains("vm-replay"),
        "{err}"
    );
    assert!(out.stdout.is_empty(), "{out:?}");
}
