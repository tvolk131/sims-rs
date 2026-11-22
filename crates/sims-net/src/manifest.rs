//! Deterministic game-file and custom-content manifest digest.
//!
//! Per `docs/vision.md`: *"A file manifest check ensures everyone has identical
//! game files and custom content."*
//!
//! Entries are canonicalized to POSIX forward-slash lowercase relative paths,
//! sorted lexicographically, and hashed with SHA-256 so macOS, Linux, and
//! Windows installs with identical game data produce identical digests.

use crate::error::ManifestError;
use crate::protocol::ManifestDigest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

const MANIFEST_DOMAIN: &[u8] = b"sims-net/manifest/v1\0";

/// One hashed file inside a [`FileManifest`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FileManifestEntry {
    /// Canonical lowercase POSIX relative path (e.g. `gamedata/objects/objects.far`).
    pub relative_path: String,
    /// File length in bytes.
    pub size_bytes: u64,
    /// SHA-256 digest of the file's contents.
    pub sha256: [u8; 32],
}

/// Deterministic sorted manifest of game files and custom content, plus its
/// overall [`ManifestDigest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileManifest {
    entries: Vec<FileManifestEntry>,
    digest: ManifestDigest,
}

/// Structured difference between a local [`FileManifest`] and a remote one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestDiff {
    /// Paths present in the remote/expected manifest but missing locally.
    pub missing_locally: Vec<String>,
    /// Paths present locally but absent from the remote/expected manifest.
    pub extra_locally: Vec<String>,
    /// Paths present in both manifests with differing size or SHA-256 digest.
    pub mismatched: Vec<String>,
}

impl ManifestDiff {
    /// Returns whether both manifests are completely identical.
    pub fn is_empty(&self) -> bool {
        self.missing_locally.is_empty()
            && self.extra_locally.is_empty()
            && self.mismatched.is_empty()
    }
}

impl FileManifest {
    /// Constructs a canonical sorted manifest from raw entries, validating
    /// paths and rejecting duplicates after normalization.
    pub fn from_entries(
        entries: impl IntoIterator<Item = FileManifestEntry>,
    ) -> Result<Self, ManifestError> {
        let mut by_path: BTreeMap<String, FileManifestEntry> = BTreeMap::new();
        for mut entry in entries {
            let canonical = canonicalize_relative_path(&entry.relative_path)?;
            entry.relative_path = canonical.clone();
            if by_path.insert(canonical.clone(), entry).is_some() {
                return Err(ManifestError::DuplicatePath(canonical));
            }
        }
        let sorted: Vec<FileManifestEntry> = by_path.into_values().collect();
        let digest = compute_manifest_digest(&sorted);
        Ok(Self {
            entries: sorted,
            digest,
        })
    }

    /// Deterministically hashes all regular files under `root` matching
    /// `include_file` (called with the canonical lowercase POSIX relative path).
    ///
    /// Symlinks are skipped to avoid host-dependent cycles. Directory listings
    /// are sorted lexicographically before traversal.
    pub fn from_directory<F>(root: &Path, mut include_file: F) -> Result<Self, ManifestError>
    where
        F: FnMut(&str) -> bool,
    {
        let mut entries = Vec::new();
        walk_dir(root, root, &mut include_file, &mut entries)?;
        Self::from_entries(entries)
    }

    /// Returns the sorted manifest entries.
    pub fn entries(&self) -> &[FileManifestEntry] {
        &self.entries
    }

    /// Returns the overall SHA-256 [`ManifestDigest`].
    pub const fn digest(&self) -> ManifestDigest {
        self.digest
    }

    /// Computes a deterministic diff comparing `self` (local) against
    /// `expected` (host/remote).
    pub fn diff(&self, expected: &Self) -> ManifestDiff {
        let local_map: BTreeMap<&str, (u64, [u8; 32])> = self
            .entries
            .iter()
            .map(|e| (e.relative_path.as_str(), (e.size_bytes, e.sha256)))
            .collect();
        let expected_map: BTreeMap<&str, (u64, [u8; 32])> = expected
            .entries
            .iter()
            .map(|e| (e.relative_path.as_str(), (e.size_bytes, e.sha256)))
            .collect();

        let mut diff = ManifestDiff::default();
        for (path, expected_meta) in &expected_map {
            match local_map.get(path) {
                None => diff.missing_locally.push((*path).to_string()),
                Some(local_meta) if local_meta != expected_meta => {
                    diff.mismatched.push((*path).to_string());
                }
                Some(_) => {}
            }
        }
        for path in local_map.keys() {
            if !expected_map.contains_key(path) {
                diff.extra_locally.push((*path).to_string());
            }
        }
        diff
    }
}

/// Computes the SHA-256 digest of a byte slice.
pub fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn canonicalize_relative_path(raw: &str) -> Result<String, ManifestError> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(ManifestError::InvalidPath(raw.to_string()));
    }
    // Reject rooted POSIX paths (`/foo`), Windows rooted/UNC paths (`\foo`, `\\server\share`),
    // and Windows drive-qualified paths (`C:foo`, `C:\foo`).
    if raw.starts_with('/') || raw.starts_with('\\') {
        return Err(ManifestError::InvalidPath(raw.to_string()));
    }
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(ManifestError::InvalidPath(raw.to_string()));
    }
    let normalized = raw.replace('\\', "/");
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(ManifestError::InvalidPath(raw.to_string()));
        }
        parts.push(part.to_ascii_lowercase());
    }
    if parts.is_empty() {
        return Err(ManifestError::InvalidPath(raw.to_string()));
    }
    Ok(parts.join("/"))
}

fn compute_manifest_digest(entries: &[FileManifestEntry]) -> ManifestDigest {
    let mut hasher = Sha256::new();
    hasher.update(MANIFEST_DOMAIN);
    hasher.update((entries.len() as u64).to_le_bytes());
    for entry in entries {
        let path_bytes = entry.relative_path.as_bytes();
        hasher.update((path_bytes.len() as u64).to_le_bytes());
        hasher.update(path_bytes);
        hasher.update(entry.size_bytes.to_le_bytes());
        hasher.update(entry.sha256);
    }
    ManifestDigest(hasher.finalize().into())
}

fn walk_dir<F>(
    root: &Path,
    dir: &Path,
    include_file: &mut F,
    out: &mut Vec<FileManifestEntry>,
) -> Result<(), ManifestError>
where
    F: FnMut(&str) -> bool,
{
    #[expect(clippy::disallowed_methods, reason = "entries are sorted before use")]
    let read_dir = std::fs::read_dir(dir).map_err(|err| ManifestError::Io {
        path: dir.display().to_string(),
        message: err.to_string(),
    })?;

    let mut children = Vec::new();
    for item in read_dir {
        let entry = item.map_err(|err| ManifestError::Io {
            path: dir.display().to_string(),
            message: err.to_string(),
        })?;
        children.push(entry.path());
    }
    children.sort();

    for path in children {
        let meta = std::fs::symlink_metadata(&path).map_err(|err| ManifestError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            walk_dir(root, &path, include_file, out)?;
        } else if meta.is_file() {
            let rel = path
                .strip_prefix(root)
                .map_err(|_| ManifestError::InvalidPath(path.display().to_string()))?;
            let rel_str = rel
                .to_str()
                .ok_or_else(|| ManifestError::InvalidPath(path.display().to_string()))?;
            let canonical = canonicalize_relative_path(rel_str)?;
            if !include_file(&canonical) {
                continue;
            }
            let (size_bytes, sha256) = hash_file(&path)?;
            out.push(FileManifestEntry {
                relative_path: canonical,
                size_bytes,
                sha256,
            });
        }
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<(u64, [u8; 32]), ManifestError> {
    let mut file = File::open(path).map_err(|err| ManifestError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(|err| ManifestError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total = total.saturating_add(n as u64);
    }
    Ok((total, hasher.finalize().into()))
}
