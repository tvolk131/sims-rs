//! Window-only observations of completed Load requests. Never load authority.
use std::path::{Path, PathBuf};

pub(super) const MAX_ENTRIES: usize = 8;
pub(super) const MAX_PATH_BYTES: usize = 4096;
pub(super) const MAX_TOTAL_BYTES: usize = 32768;

/// Skip paths the existing UTF-8 editor cannot represent exactly. Allocation
/// refusal is an absent convenience entry, never a failed household operation.
pub(super) fn copy_path(path: &Path) -> Option<PathBuf> {
    let text = path.to_str()?;
    if text.is_empty() || text.contains('\0') || path.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES {
        return None;
    }
    let mut copy = PathBuf::new();
    copy.try_reserve_exact(path.as_os_str().len()).ok()?;
    copy.push(path);
    Some(copy)
}

#[derive(Default)]
pub(super) struct History {
    paths: Vec<PathBuf>,
}

impl History {
    /// Call only at the adapter's confirmed successful input-commit boundary.
    /// OsStr equality intentionally retains dots, separators and Unicode form.
    pub(super) fn record(&mut self, path: &Path) {
        if let Some(index) = self.paths.iter().position(|p| p.as_os_str() == path.as_os_str()) {
            self.paths[..=index].rotate_right(1);
            return;
        }
        let Some(path) = copy_path(path) else { return; };
        if self.paths.try_reserve_exact(MAX_ENTRIES.saturating_sub(self.paths.len())).is_err() {
            return;
        }
        let bytes = path.as_os_str().as_encoded_bytes().len();
        while self.paths.len() >= MAX_ENTRIES || self.retained_bytes() + bytes > MAX_TOTAL_BYTES {
            self.paths.pop();
        }
        self.paths.insert(0, path);
    }

    /// A picker owns an immutable snapshot for its entire lifetime. Its sealed
    /// picker identity plus entry ordinal therefore names exactly one path.
    pub(super) fn snapshot(&self) -> Self {
        let mut snapshot = Self::default();
        for path in self.paths.iter().rev() { snapshot.record(path); }
        snapshot
    }

    pub(super) fn paths(&self) -> &[PathBuf] { &self.paths }

    fn retained_bytes(&self) -> usize {
        self.paths.iter().map(|p| p.as_os_str().as_encoded_bytes().len()).sum()
    }
}

#[cfg(test)]
#[path = "recent_loads/tests.rs"]
mod tests;
