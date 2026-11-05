//! Read-only sources for the native file manager. Virtual installation paths,
//! directory entry order and Windows attributes are supplied explicitly. This
//! adapter does not infer installer destinations or Windows enumeration order
//! from the alphabetically sorted content index.
use crate::{Index, avatar_catalog::Directories};
use anyhow::{Context, Result, ensure};
use sims_sim::pose::clothing::{
    catalog_mount::{ArchiveHost, DirectoryHost, Found, Manager, Metadata, Mounted, Opened},
    name_map::Files,
};
use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Debug)]
struct Archive {
    identity: u32,
    physical: PathBuf,
    // A source identity must match the selected manifest entry, not merely a
    // same-named entry in another archive or at another offset.
    entries: BTreeMap<(Vec<u8>, u64, u64), usize>,
}

pub struct Sources<'a> {
    index: &'a Index,
    archive_sources: Option<BTreeMap<&'a Path, Vec<usize>>>,
    archives: BTreeMap<Vec<u8>, Archive>,
    loose: BTreeMap<Vec<u8>, usize>,
    directories: BTreeMap<Vec<u8>, Vec<Found>>,
    pub open_failures: Vec<Vec<u8>>,
}
impl<'a> Sources<'a> {
    pub fn new(index: &'a Index) -> Self {
        Self {
            index,
            archive_sources: None,
            archives: BTreeMap::new(),
            loose: BTreeMap::new(),
            directories: BTreeMap::new(),
            open_failures: vec![],
        }
    }
    pub fn bind_archive(&mut self, virtual_path: &[u8], physical: &Path) -> Result<()> {
        let path = key(virtual_path)?;
        let base = &path[..path
            .iter()
            .rposition(|&b| b == b'\\')
            .context("archive binding needs a directory prefix")?
            + 1];
        ensure!(
            !self.archives.contains_key(&path),
            "archive path already bound"
        );
        let identity = u32::try_from(self.archives.len() + 1)?;
        let mut entries = BTreeMap::new();
        let index = self.index;
        let archive_sources = self.archive_sources.get_or_insert_with(|| {
            let mut sources: BTreeMap<&Path, Vec<usize>> = BTreeMap::new();
            for (id, resource) in index.resources.iter().enumerate() {
                if resource.archived {
                    sources
                        .entry(resource.source.as_path())
                        .or_default()
                        .push(id);
                }
            }
            sources
        });
        for &id in archive_sources.get(physical).into_iter().flatten() {
            let resource = &self.index.resources[id];
            let (name, _, invalid) = encoding_rs::WINDOWS_1252.encode(&resource.name);
            ensure!(!invalid, "archive filename is not Windows-1252");
            let full = key(&[base, name.as_ref()].concat())?;
            let location = (full, resource.offset, resource.unpacked_size);
            // Identical manifest entries may share the same readable bytes.
            // Distinct offsets remain distinct source identities.
            if let Some(&old) = entries.get(&location) {
                let old: &crate::Resource = &self.index.resources[old];
                ensure!(old.size == resource.size, "ambiguous archive source range");
            } else {
                entries.insert(location, id);
            }
        }
        self.archives.insert(
            path,
            Archive {
                identity,
                physical: physical.into(),
                entries,
            },
        );
        Ok(())
    }
    pub fn bind_loose(&mut self, virtual_path: &[u8], source: usize) -> Result<()> {
        let resource = self
            .index
            .resources
            .get(source)
            .context("loose source absent")?;
        ensure!(!resource.archived, "loose binding points into an archive");
        let path = key(virtual_path)?;
        ensure!(!self.loose.contains_key(&path), "loose path already bound");
        self.loose.insert(path, source);
        Ok(())
    }
    /// The complete OS result list, including its first two entries. No dot
    /// entries are fabricated here. Empty means FindFirst fails; an unknown
    /// directory is an unresolved host input and returns an error instead.
    pub fn set_directory(&mut self, path: &[u8], entries: Vec<Found>) -> Result<()> {
        let path = key(path)?;
        ensure!(
            path.ends_with(b"\\"),
            "directory input needs a trailing separator"
        );
        self.directories.insert(path, entries);
        Ok(())
    }
    pub fn scan(
        &mut self,
        manager: &mut Manager,
        path: &[u8],
        recursive: bool,
        mount: bool,
    ) -> Result<()> {
        manager
            .scan(path, recursive, mount, self)
            .map_err(anyhow::Error::msg)
    }
    pub fn mount(&mut self, manager: &mut Manager, path: &[u8]) -> Result<Mounted> {
        manager.mount(path, self).map_err(anyhow::Error::msg)
    }
    pub fn catalog_directories(&self, manager: &Manager) -> Result<Directories> {
        Directories::from_mounted(manager, |directory, name, metadata| {
            self.source(directory, name, metadata)
        })
    }
    fn source(&self, directory: &[u8], name: &[u8], metadata: &Metadata) -> Result<Option<usize>> {
        if metadata.directory != 0 {
            return Ok(None);
        }
        let full = key(&[directory, name].concat())?;
        if metadata.archived != 0 {
            let archive = self
                .archives
                .values()
                .find(|a| a.identity == metadata.archive)
                .context("mounted archive identity has no source binding")?;
            let id = *archive
                .entries
                .get(&(full, metadata.offset.into(), metadata.size.into()))
                .context("mounted archive entry has no matching indexed source range")?;
            let resource = &self.index.resources[id];
            ensure!(
                resource.size == resource.unpacked_size,
                "compressed FAR source is unsupported: {}",
                resource.locator
            );
            return Ok(Some(id));
        }
        // FAR container entries have zero/default metadata, unless an earlier
        // archive supplied that name. They still occupy a file-map node.
        if name.ends_with(b"far") {
            return Ok(None);
        }
        // A loose override may retain a stale archive word. Only `archived`
        // selects that source; never use a nonzero archive word as the test.
        Ok(Some(*self.loose.get(&full).with_context(|| {
            format!(
                "loose source binding absent: {}",
                String::from_utf8_lossy(&full)
            )
        })?))
    }
}
fn key(path: &[u8]) -> Result<Vec<u8>> {
    Files::<()>::key(path).map_err(anyhow::Error::msg)
}
fn read_archive(input: &mut (impl Read + Seek), identity: u32) -> std::io::Result<Opened> {
    let mut header = Vec::new();
    input.by_ref().take(16).read_to_end(&mut header)?;
    let mut directory = Vec::new();
    if let Some(offset) = header.get(12..16) {
        let offset = u32::from_le_bytes(offset.try_into().unwrap());
        if (offset as usize) < header.len() {
            directory.extend_from_slice(&header[offset as usize..]);
        } else {
            input.seek(SeekFrom::Start(u64::from(offset)))?;
        }
        input.read_to_end(&mut directory)?;
    }
    Ok(Opened::from_directory(identity, header, directory))
}
impl ArchiveHost for Sources<'_> {
    fn open(&mut self, path: &[u8]) -> Result<Option<Opened>, String> {
        let Some(archive) = self.archives.get(path) else {
            return Ok(None);
        };
        let result = std::fs::File::open(&archive.physical)
            .and_then(|mut input| read_archive(&mut input, archive.identity));
        match result {
            Ok(opened) => Ok(Some(opened)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!(
                "read archive {}: {error}",
                archive.physical.display()
            )),
        }
    }
    fn open_failed(&mut self, path: &[u8]) -> Result<(), String> {
        self.open_failures.push(path.to_vec());
        Ok(())
    }
}
pub struct Search {
    entries: std::vec::IntoIter<Found>,
}
impl DirectoryHost for Sources<'_> {
    type Search = Search;
    fn first(&mut self, pattern: &[u8]) -> Result<Option<Search>, String> {
        let directory = pattern
            .strip_suffix(b"*.*")
            .ok_or("unexpected directory pattern")?;
        let entries = self.directories.get(directory).ok_or_else(|| {
            format!(
                "directory enumeration not supplied: {}",
                String::from_utf8_lossy(directory)
            )
        })?;
        let mut entries = entries.clone().into_iter();
        Ok(entries.next().map(|_| Search { entries }))
    }
    fn next(&mut self, search: &mut Search) -> Result<Option<Found>, String> {
        Ok(search.entries.next())
    }
    fn close(&mut self, _: Search) -> Result<(), String> {
        Ok(())
    }
    fn recurse(
        &mut self,
        manager: &mut Manager,
        path: &[u8],
        recursive: bool,
        mount: bool,
    ) -> Result<(), String> {
        manager.scan(path, recursive, mount, self)
    }
    fn mount(&mut self, manager: &mut Manager, path: &[u8]) -> Result<bool, String> {
        Ok(manager.mount(path, self)?.success)
    }
}

#[cfg(test)]
#[path = "avatar_mount_tests.rs"]
mod tests;
