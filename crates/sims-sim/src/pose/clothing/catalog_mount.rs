//! 5ba5d8 directory scans and 5b9dbc archive mounting. OS entry order is an
//! explicit input. Metadata updates retain existing file-map node positions.
use super::name_map::Files;
use sims_formats::far::mount::Reader;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    /// Stable host identity for the native archive pointer. A loose override
    /// clears `archived` but retains this word and the reserved word.
    pub archive: u32,
    pub offset: u32,
    pub reserved: u32,
    pub size: u32,
    pub archived: u8,
    pub writable: u8,
    pub directory: u8,
}
#[derive(Debug, Default)]
pub struct Manager {
    directories: BTreeMap<Vec<u8>, Files<Metadata>>,
    archives: BTreeMap<Vec<u8>, u32>,
    pub has_archives: bool,
}
impl Manager {
    /// 5b9c9f checks the normalized directory key, including its separator.
    pub fn registered(&self, path: &[u8]) -> Result<bool, String> {
        let path = path.split(|&b| b == 0).next().unwrap();
        Ok(self.directory(path)?.is_some())
    }
    /// 5b9cfd accepts any mounted metadata entry, regardless of flags.
    pub fn contains_path(&self, path: &[u8]) -> Result<bool, String> {
        let path = path.split(|&b| b == 0).next().unwrap();
        let (directory, filename) = split(path)?;
        self.directory(directory)?
            .map(|files| files.get(filename).map(|m| m.is_some()))
            .unwrap_or(Ok(false))
    }
    /// 5b9b92 consults physical existence only after a mounted lookup misses.
    /// The OS receives the original case/slashes, not the normalized map key.
    pub fn exists(
        &self,
        path: &[u8],
        physical: impl FnOnce(&[u8]) -> Result<bool, String>,
    ) -> Result<bool, String> {
        let path = path.split(|&b| b == 0).next().unwrap();
        if self.contains_path(path)? {
            Ok(true)
        } else {
            physical(path)
        }
    }
    pub fn directory(&self, path: &[u8]) -> Result<Option<&Files<Metadata>>, String> {
        Ok(self.directories.get(&Files::<Metadata>::key(path)?))
    }
    pub fn directory_mut(&mut self, path: &[u8]) -> Result<&mut Files<Metadata>, String> {
        Ok(self
            .directories
            .entry(Files::<Metadata>::key(path)?)
            .or_default())
    }
    pub fn directories(&self) -> impl Iterator<Item = (&[u8], &Files<Metadata>)> {
        self.directories.iter().map(|(k, v)| (k.as_slice(), v))
    }
    pub fn archives(&self) -> impl Iterator<Item = (&[u8], u32)> {
        self.archives.iter().map(|(k, v)| (k.as_slice(), *v))
    }
    /// Recursive scan and mount calls remain explicit host operations. Their
    /// effects occur between enumeration steps, with updated parent metadata.
    pub fn scan(
        &mut self,
        path: &[u8],
        recursive: bool,
        mount: bool,
        host: &mut impl DirectoryHost,
    ) -> Result<(), String> {
        let path = Files::<Metadata>::key(path)?;
        let pattern = [path.as_slice(), b"*.*"].concat();
        let Some(mut search) = host.first(&pattern)? else {
            return Ok(());
        };
        // FindFirst's entry and the first FindNext result are both discarded,
        // without checking that their names really are '.' and '..'.
        let _ = host.next(&mut search)?;
        self.directory_mut(&path)?;
        while let Some(entry) = host.next(&mut search)? {
            if entry.name.len() >= 260 {
                return Err("directory filename exceeds native buffer".into());
            }
            let name = Files::<Metadata>::key(&entry.name)?;
            let files = self.directory_mut(&path)?;
            files.insert(&name, Metadata::default())?;
            let directory = ((entry.attributes >> 4) & 1) as u8;
            let metadata = files.get_mut(&name)?.unwrap();
            if directory != 0 || !name.ends_with(b"far") {
                metadata.archived = 0;
                metadata.offset = 0;
                metadata.size = 0;
                metadata.writable = (!entry.attributes & 1) as u8;
                metadata.directory = directory;
            }
            let full = [path.as_slice(), name.as_slice()].concat();
            if directory != 0 && recursive {
                host.recurse(self, &[full.as_slice(), b"\\"].concat(), recursive, mount)?;
            } else if directory == 0 && name.ends_with(b"far") && mount {
                let _ = host.mount(self, &full)?;
            }
        }
        host.close(search)
    }

    pub fn mount(&mut self, path: &[u8], host: &mut impl ArchiveHost) -> Result<Mounted, String> {
        let path = Files::<Metadata>::key(path)?;
        let base = split(&path)?.0;
        if base.is_empty() {
            return Err(
                "archive mounting needs a directory-prefixed path (native bare-path underflow)"
                    .into(),
            );
        }
        if self.archives.contains_key(&path) {
            return Ok(Mounted::empty(true));
        }
        let Some(opened) = host.open(&path)? else {
            host.open_failed(&path)?;
            return Ok(Mounted::empty(false));
        };
        self.archives.insert(path.clone(), opened.identity);
        let mut reader = Reader::new(&opened.bytes);
        let Some(count) = reader.count() else {
            return Ok(Mounted::read(false, &reader));
        };
        for _ in 0..count {
            let Some(entry) = reader.entry().map_err(|e| e.to_string())? else {
                return Ok(Mounted::read(false, &reader));
            };
            let full = [base, entry.name.as_slice()].concat();
            let (directory, name) = split(&full)?;
            if let Some(files) = self.directory(directory)? {
                if files.get(name)?.is_some() {
                    continue;
                }
            }
            self.directory_mut(directory)?.insert(
                name,
                Metadata {
                    archive: opened.identity,
                    offset: entry.offset,
                    size: entry.size,
                    archived: 1,
                    ..Default::default()
                },
            )?;
            let mut parent = directory;
            loop {
                let trimmed = parent
                    .get(
                        ..parent
                            .len()
                            .checked_sub(1)
                            .ok_or("native ancestor path underflow")?,
                    )
                    .unwrap();
                let (directory, child) = split(trimmed)?;
                if directory.is_empty() {
                    break;
                }
                self.directory_mut(directory)?.insert(
                    child,
                    Metadata {
                        writable: 1,
                        directory: 1,
                        ..Default::default()
                    },
                )?;
                parent = directory;
            }
        }
        if count > 0 {
            self.has_archives = true;
        }
        Ok(Mounted::read(true, &reader))
    }
}
fn split(path: &[u8]) -> Result<(&[u8], &[u8]), String> {
    if path.len() >= 260 || path.contains(&0) {
        return Err("native mount path exceeds bounded C-string support".into());
    }
    let at = path
        .iter()
        .rposition(|c| matches!(c, b'/' | b'\\'))
        .map_or(0, |n| n + 1);
    Ok((&path[..at], &path[at..]))
}
#[derive(Debug, Clone)]
pub struct Found {
    pub name: Vec<u8>,
    pub attributes: u32,
}
pub trait DirectoryHost {
    type Search;
    /// Consume the first entry when opening the search; its contents are ignored.
    fn first(&mut self, pattern: &[u8]) -> Result<Option<Self::Search>, String>;
    fn next(&mut self, search: &mut Self::Search) -> Result<Option<Found>, String>;
    fn close(&mut self, search: Self::Search) -> Result<(), String>;
    fn recurse(
        &mut self,
        manager: &mut Manager,
        path: &[u8],
        recursive: bool,
        mount: bool,
    ) -> Result<(), String>;
    fn mount(&mut self, manager: &mut Manager, path: &[u8]) -> Result<bool, String>;
}
pub struct Opened {
    pub identity: u32,
    pub bytes: Vec<u8>,
}
pub trait ArchiveHost {
    fn open(&mut self, path: &[u8]) -> Result<Option<Opened>, String>;
    /// Report the failed native open and release its allocated file object.
    fn open_failed(&mut self, path: &[u8]) -> Result<(), String>;
}
#[derive(Debug)]
pub struct Mounted {
    pub success: bool,
    pub consumed: usize,
    pub manifest_offset: Option<u32>,
}
impl Mounted {
    fn empty(success: bool) -> Self {
        Self {
            success,
            consumed: 0,
            manifest_offset: None,
        }
    }
    fn read(success: bool, reader: &Reader<'_>) -> Self {
        Self {
            success,
            consumed: reader.position,
            manifest_offset: reader.manifest_offset,
        }
    }
}
