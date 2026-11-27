//! Computes this build's co-op build identity (`sims_net::BUILD_ID`).
//!
//! Two builds can share a lockstep session only when every peer runs the
//! same simulation code. The identity is a SHA-256 over the sources of the
//! crates that decide a tick's result: `sims-core`, `sims-formats`,
//! `sims-content`, `sims-sim`, `sims-vm` and this crate. For each crate it
//! covers `Cargo.toml` and every file under `src/`, in path order, each as
//! its path relative to the workspace, a NUL, its length and its bytes with
//! every carriage return dropped, so a checkout with Windows line endings
//! gets the identity of the same commit checked out with Unix ones.
//!
//! The workspace's `Cargo.toml` and `Cargo.lock` are covered too: they
//! pick the dependencies' versions and patches (such as the float maths
//! a tick uses), so a dependency bump is a different build.
//!
//! The interface crates are left out on purpose: a peer whose window or HUD
//! differs still runs the same ticks.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// The crates whose sources the identity covers, by directory name under
/// `crates/`.
const CRATES: [&str; 6] = [
    "sims-core",
    "sims-formats",
    "sims-content",
    "sims-sim",
    "sims-vm",
    "sims-net",
];

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let crates = manifest
        .parent()
        .expect("crates/ holds this crate")
        .to_path_buf();
    let workspace = crates.parent().expect("the workspace holds crates/");
    let mut files = Vec::new();
    for name in ["Cargo.toml", "Cargo.lock"] {
        println!("cargo:rerun-if-changed={}", workspace.join(name).display());
        files.push(workspace.join(name));
    }
    for name in CRATES {
        let dir = crates.join(name);
        println!(
            "cargo:rerun-if-changed={}",
            dir.join("Cargo.toml").display()
        );
        println!("cargo:rerun-if-changed={}", dir.join("src").display());
        files.push(dir.join("Cargo.toml"));
        collect(&dir.join("src"), &mut files);
    }
    let mut named: Vec<(String, PathBuf)> = files
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(workspace)
                .expect("under the workspace")
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            (relative, path)
        })
        .collect();
    named.sort();
    let mut hash = Sha256::new();
    for (relative, path) in &named {
        // A file that cannot be read is hashed as missing, so the build
        // still has an identity (one no clean checkout shares).
        let bytes: Vec<u8> = fs::read(path)
            .map(|bytes| bytes.into_iter().filter(|&b| b != b'\r').collect())
            .unwrap_or_else(|_| b"\0missing".to_vec());
        hash.update(relative.as_bytes());
        hash.update([0]);
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    let digest = hash.finalize();
    let bytes: Vec<String> = digest[..16].iter().map(|b| format!("0x{b:02x}")).collect();
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("set by cargo"));
    fs::write(
        out.join("build_id.rs"),
        format!(
            "/// This build's identity: see `build.rs`.\npub const BUILD_ID: BuildId = BuildId([{}]);\n",
            bytes.join(", ")
        ),
    )
    .expect("write build_id.rs");
}

/// Every file under `dir`, recursively, leaving out hidden entries (a
/// name starting with `.`, such as a desktop's folder metadata) and editor
/// backups (ending in `~`), which no checkout shares. The order does not
/// matter: the caller sorts by path.
#[allow(clippy::disallowed_methods)]
fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name.ends_with('~') {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => collect(&path, out),
            Ok(kind) if kind.is_file() => out.push(path),
            _ => {}
        }
    }
}
