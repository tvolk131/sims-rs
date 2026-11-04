//! 43c609 object catalog phases and 4127b2/4125a2/412227 folder traversal.
//! File-manager enumeration and per-IFF registration are explicit services.
//! This scanner supplies their order, category and writable/People mode inputs.
use crate::pose::clothing::{catalog_mount::Manager, name_map::Files};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paths {
    pub objects: Vec<u8>,
    pub user_objects: Vec<u8>,
    pub people: Vec<u8>,
    pub downloads: Vec<u8>,
    /// ExpansionPack, 2, 3, 4, Deluxe, 5, 6, 7.
    pub expansions: [Vec<u8>; 8],
    pub shared: Vec<u8>,
    pub npcs: Vec<u8>,
}
pub trait PhaseHost {
    fn count(&self) -> usize;
    /// A native enumeration miss is not an error here: 43c609 ignores -1.
    /// Err denotes an unresolved host operation, not a native scan return.
    fn scan(&mut self, path: &[u8], category: u32, people: u8) -> Result<(), String>;
    fn changed(&mut self) -> Result<(), String>;
}
pub fn phases(
    paths: &Paths,
    expansions: i16,
    people: u8,
    loaded: bool,
    host: &mut impl PhaseHost,
) -> Result<(), String> {
    let count = host.count();
    let mut scan = |path: &[u8], category, mode| -> Result<(), String> {
        counted(path)?;
        if !path.is_empty() {
            host.scan(path, category, mode)?;
        }
        Ok(())
    };
    scan(&paths.objects, 0, people)?;
    scan(&paths.user_objects, 3, people)?;
    scan(&paths.people, 1, 1)?;
    scan(&paths.downloads, 2, people)?;
    for i in 0..7 {
        if expansions & (1 << i) != 0 {
            scan(&paths.expansions[i], 4 << i, people)?;
        }
    }
    if expansions > 3 {
        scan(&paths.shared, 0x200, people)?;
        scan(&paths.npcs, 0x400, people)?;
    }
    if expansions & 0x80 != 0 {
        scan(&paths.expansions[7], 0x800, people)?;
    }
    if host.count() != count && loaded {
        host.changed()?;
    }
    Ok(())
}

fn counted(path: &[u8]) -> Result<(), String> {
    if path.len() >= 260 {
        return Err("object scan path exceeds CFString260".into());
    }
    Ok(())
}
fn cstring(path: &[u8]) -> &[u8] {
    &path[..path.iter().position(|&b| b == 0).unwrap_or(path.len())]
}
/// 43a436/412196 find the extension in counted storage, then copy it through
/// C-string storage for the comparison. A dot after NUL still affects selection.
pub fn recurse_directory(path: &[u8]) -> Result<bool, String> {
    counted(path)?;
    let basename = path
        .rsplit(|b| matches!(b, b'/' | b'\\' | b':'))
        .next()
        .unwrap();
    let extension = basename
        .iter()
        .rposition(|&b| b == b'.')
        .map_or(&[][..], |i| &basename[i..]);
    Ok(!cstring(extension).eq_ignore_ascii_case(b".dir"))
}
/// 43a4df compares the case-insensitive search result with len-4. A three-byte
/// string therefore passes even without an extension: both values are -1.
pub fn object_file(path: &[u8]) -> Result<bool, String> {
    counted(path)?;
    Ok(
        path.len() == 3
            || (path.len() >= 4 && path[path.len() - 4..].eq_ignore_ascii_case(b".iff")),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub directory: Vec<u8>,
    pub name: Vec<u8>,
    pub directory_flag: u8,
    pub writable: u8,
}

/// 5ba9bd/5baac8 for the scanner's '*' query. Keep a current key rather than
/// snapshotting the map: subsequent steps read retained node order/metadata.
pub struct MountedSearch {
    directory: Vec<u8>,
    current: Option<Vec<u8>>,
}
impl MountedSearch {
    pub fn first(manager: &Manager, pattern: &[u8]) -> Result<(Self, Option<Entry>), String> {
        let pattern = Files::<()>::key(cstring(pattern))?;
        let at = pattern
            .iter()
            .rposition(|&b| b == b'\\')
            .map_or(0, |i| i + 1);
        let directory = pattern[..at].to_vec();
        let mut search = Self {
            directory,
            current: None,
        };
        // Native skips even the map lookup when the directory part is empty.
        if at == 0 {
            return Ok((search, None));
        }
        if &pattern[at..] != b"*" {
            return Err("object iterator needs its native all-files pattern".into());
        }
        let entry = manager
            .directory(&search.directory)?
            .and_then(|files| files.iter().next())
            .map(|(name, metadata)| Entry {
                directory: search.directory.clone(),
                name: name.to_vec(),
                directory_flag: metadata.directory,
                writable: metadata.writable,
            });
        search.current = entry.as_ref().map(|e| e.name.clone());
        Ok((search, entry))
    }
    pub fn next(&mut self, manager: &Manager) -> Result<Option<Entry>, String> {
        let Some(current) = &self.current else {
            return Ok(None);
        };
        let files = manager
            .directory(&self.directory)?
            .ok_or("object iterator directory disappeared")?;
        let next = files
            .successor(current)
            .map_err(|()| "object iterator current node disappeared")?
            .map(|(name, metadata)| Entry {
                directory: self.directory.clone(),
                name: name.to_vec(),
                directory_flag: metadata.directory,
                writable: metadata.writable,
            });
        self.current = next.as_ref().map(|e| e.name.clone());
        Ok(next)
    }
}

#[cfg(test)]
mod successor_tests;
pub trait Host {
    type Manager: Copy;
    type Search;
    fn global(&mut self) -> Self::Manager;
    fn registered(&mut self, path: &[u8]) -> Result<bool, String>;
    fn temporary(&mut self) -> Result<Self::Manager, String>;
    fn register(&mut self, manager: Self::Manager, path: &[u8]) -> Result<(), String>;
    fn clear(&mut self, manager: Self::Manager) -> Result<(), String>;
    fn dispose(&mut self, manager: Self::Manager) -> Result<(), String>;
    /// Both traversal variants use '*'. Directory mounting itself has a
    /// separate '*.*' OS search; that is not this file-manager iterator.
    fn first(
        &mut self,
        manager: Self::Manager,
        pattern: &[u8],
    ) -> Result<(Self::Search, Option<Entry>), String>;
    fn next(
        &mut self,
        manager: Self::Manager,
        search: &mut Self::Search,
    ) -> Result<Option<Entry>, String>;
    fn close(&mut self, search: Self::Search) -> Result<(), String>;
    /// Optional observation of the directory-filter callback, before recursion.
    fn directory(&mut self, _path: &[u8]) -> Result<(), String> {
        Ok(())
    }
    fn file(&mut self, path: &[u8], category: u32, writable: u8) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scanner {
    pub category: u32,
    /// Updated for every file, even when its name fails the IFF filter.
    pub writable: u8,
}
impl Scanner {
    /// True corresponds to native return 0; false to -1 (no first entry).
    pub fn scan(&mut self, path: &[u8], host: &mut impl Host) -> Result<bool, String> {
        counted(path)?;
        let path: Vec<_> = cstring(path)
            .iter()
            .map(|&b| if b == b'/' { b'\\' } else { b })
            .collect();
        let mounted = host.registered(&path)?;
        self.walk(&path, mounted, host)
    }
    fn walk(&mut self, path: &[u8], mounted: bool, host: &mut impl Host) -> Result<bool, String> {
        let manager = if mounted {
            host.global()
        } else {
            host.temporary()?
        };
        let result = (|| {
            if !mounted {
                // Original ignores the registration return. The host supplies
                // its effects; native recursive=false, mount_archives=true.
                host.register(manager, cstring(path))?;
            }
            let pattern = [path, b"*"].concat();
            let (mut search, mut entry) = host.first(manager, cstring(&pattern))?;
            let result = (|| {
                let found = entry.is_some();
                while let Some(row) = entry {
                    let full = [row.directory, row.name].concat();
                    let callback = cstring(&full);
                    let callback = &callback[..callback.len().min(259)];
                    if row.directory_flag != 0 {
                        host.directory(callback)?;
                        if recurse_directory(callback)? {
                            // Recurse before the parent's next entry. The
                            // initial mode is inherited, without another
                            // registered-directory query for the child.
                            let child = [full.as_slice(), b"\\"].concat();
                            let _ = self.walk(&child, mounted, host)?;
                        }
                    } else {
                        self.writable = row.writable;
                        if object_file(callback)? {
                            host.file(callback, self.category, self.writable)?;
                        }
                    }
                    entry = host.next(manager, &mut search)?;
                }
                Ok(found)
            })();
            let closed = host.close(search);
            result.and_then(|found| closed.map(|()| found))
        })();
        if mounted {
            return result;
        }
        // An empty temporary search skips clear but still destroys its manager.
        // On an unsupported host error, dispose ownership without inventing a
        // successful scan or continuing the parent iterator.
        let result = match result {
            Ok(true) => host.clear(manager).map(|()| true),
            other => other,
        };
        let disposed = host.dispose(manager);
        result.and_then(|found| disposed.map(|()| found))
    }
}
