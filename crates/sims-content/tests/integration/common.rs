//! Synthetic install trees and archives for tests, and the real install's
//! index built once per run. No game data.
#![allow(dead_code)]

use sims_content::{Index, Install};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Numbers temporary trees. Every module's tests now run in one process,
/// so the process ID no longer keeps their paths apart; this names folders
/// and holds no test state.
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// A temporary directory removed on drop.
pub struct Tree(pub PathBuf);

impl Tree {
    pub fn new(name: &str) -> Self {
        let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("sims-content-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    pub fn file(&self, relative: &str, bytes: &[u8]) -> &Self {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
        self
    }
    pub fn dir(&self, relative: &str) -> &Self {
        std::fs::create_dir_all(self.0.join(relative)).unwrap();
        self
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A FAR 1a archive holding the given members.
pub fn far(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut bytes = b"FAR!byAZ".to_vec();
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(0u32.to_le_bytes());
    let mut offsets = Vec::new();
    for (_, data) in members {
        offsets.push(bytes.len() as u32);
        bytes.extend(*data);
    }
    let manifest = bytes.len() as u32;
    bytes[12..16].copy_from_slice(&manifest.to_le_bytes());
    bytes.extend((members.len() as u32).to_le_bytes());
    for ((name, data), offset) in members.iter().zip(offsets) {
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend(offset.to_le_bytes());
        bytes.extend((name.len() as u32).to_le_bytes());
        bytes.extend(name.as_bytes());
    }
    bytes
}

/// An IFF 2.5 file without a resource map, holding the given chunks.
pub fn iff(chunks: &[(&[u8; 4], u16, &[u8])]) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
    for (tag, id, data) in chunks {
        bytes.extend(*tag);
        bytes.extend((76 + data.len() as u32).to_be_bytes());
        bytes.extend(id.to_be_bytes());
        bytes.extend(0u16.to_be_bytes());
        bytes.extend([0; 64]);
        bytes.extend(*data);
    }
    bytes
}

/// An OBJD payload with the given GUID and nothing else set.
pub fn objd(guid: u32) -> Vec<u8> {
    let mut bytes = vec![0; 216];
    bytes[..4].copy_from_slice(&138u32.to_le_bytes());
    bytes[28..32].copy_from_slice(&guid.to_le_bytes());
    bytes
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
