//! Helpers shared by the `sims-sim` integration tests. No game data.

use sims_content::{Index, Install};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Numbers temporary paths. Every module's tests now run in one process,
/// so one counter keeps their paths apart; it names files and holds no
/// test state.
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn temp_path(label: &str, extension: &str) -> PathBuf {
    let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "sims-sim-{label}-{}-{n}{extension}",
        std::process::id()
    ))
}

/// A REDB file path in the temporary directory, removed on drop.
pub struct TempDb(pub PathBuf);

impl TempDb {
    pub fn new(label: &str) -> Self {
        let path = temp_path(label, ".redb");
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// An empty directory in the temporary directory, made now and removed on
/// drop. Anything a killed run left at the same path is cleared first.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(label: &str) -> Self {
        let path = temp_path(label, "");
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `Index::build(install)` for the install `SIMS_INSTALL` names, built
/// once per run and cloned for each caller, who may then mount characters
/// on its copy. The tests change only their own temporary folders, never
/// the install, so every build would be the same.
pub fn installed_index(install: &Install) -> Index {
    static INDEX: OnceLock<(PathBuf, Index)> = OnceLock::new();
    let (root, index) =
        INDEX.get_or_init(|| (install.root.clone(), Index::build(install).unwrap()));
    assert_eq!(
        root, &install.root,
        "installed_index serves one install per run"
    );
    index.clone()
}
