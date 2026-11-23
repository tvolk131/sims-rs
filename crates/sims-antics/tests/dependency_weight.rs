//! Compiler-only builds (`--no-default-features`) must not pull in the VM or the
//! networking and storage crates it depends on; only the `install-catalog` feature
//! (on by default, used by `lift`) does.

use std::process::Command;

#[test]
fn compiler_only_build_skips_the_vm_and_networking() {
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "--offline",
            "--manifest-path",
            concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
            "-p",
            "sims-antics",
            "--no-default-features",
            "-e",
            "normal",
            "--prefix",
            "none",
        ])
        .output()
        .expect("run cargo tree");
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8_lossy(&output.stdout);
    let heavy: Vec<&str> = tree
        .lines()
        .filter(|line| {
            let name = line.split_whitespace().next().unwrap_or_default();
            matches!(
                name,
                "sims-vm" | "sims-content" | "sims-net" | "iroh" | "tokio" | "redb"
            )
        })
        .collect();
    assert!(heavy.is_empty(), "compiler-only build depends on {heavy:?}");
}
