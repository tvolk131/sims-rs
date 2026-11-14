//! Every sound file of an install, by the path the sound tables use.
//!
//! The sound tables (`.hot` files) name samples by a path relative to the
//! install, such as `sounddata\phone\Phone_ring.xa`. Most of those files
//! are members of a FAR archive: `SoundData\SoundData.far` holds
//! `Phone\Phone_Ring.XA`. As in the object scan
//! (legacy/crates/sims-content and `sims_content::Index`), an archive's
//! members are entries of the directory the archive is in, so that file's
//! path is `SoundData\Phone\Phone_Ring.XA`.
//!
//! Paths compare ignoring ASCII case, with `/` and `\` alike. A loose file
//! replaces an archive member with the same path, and of two members with
//! one path the first listed is kept, as in `sims_content::Index`. That
//! the sound manager resolves paths through the same rules is
//! **unverified**.

use crate::{Error, Result};
use sims_formats::far;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Extensions of the files the sound system reads: samples, and the
/// sound scripts and their tables.
pub const EXTENSIONS: [&str; 10] = [
    "xa", "wav", "mp3", "utk", "hit", "hsm", "hot", "evt", "fsc", "tlo",
];

/// Where a file's bytes are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Loose(PathBuf),
    Member {
        archive: PathBuf,
        name: String,
        offset: u64,
        size: u64,
    },
}

impl Source {
    /// The first `n` bytes, or all of them for a shorter file.
    pub fn read_head(&self, n: usize) -> Result<Vec<u8>> {
        let (path, offset, size) = match self {
            Self::Loose(path) => (path, 0, u64::MAX),
            Self::Member {
                archive,
                offset,
                size,
                ..
            } => (archive, *offset, *size),
        };
        let io = |e| Error::io(path, e);
        let mut file = File::open(path).map_err(io)?;
        file.seek(SeekFrom::Start(offset)).map_err(io)?;
        let mut bytes = Vec::with_capacity(n);
        file.take(size.min(n as u64))
            .read_to_end(&mut bytes)
            .map_err(io)?;
        Ok(bytes)
    }

    pub fn read(&self) -> Result<Vec<u8>> {
        match self {
            Self::Loose(path) => std::fs::read(path).map_err(|e| Error::io(path, e)),
            Self::Member {
                archive,
                offset,
                size,
                ..
            } => {
                let io = |e| Error::io(archive, e);
                let mut file = File::open(archive).map_err(io)?;
                file.seek(SeekFrom::Start(*offset)).map_err(io)?;
                let mut bytes = vec![0; *size as usize];
                file.read_exact(&mut bytes).map_err(io)?;
                Ok(bytes)
            }
        }
    }
}

/// One file under the install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The path relative to the install with `\` separators, in the case
    /// the disk or archive gives it.
    pub path: String,
    pub source: Source,
}

impl Entry {
    /// The lower-case extension, without the dot.
    pub fn extension(&self) -> String {
        let name = self.path.rsplit('\\').next().unwrap_or_default();
        match name.rsplit_once('.') {
            Some((_, extension)) => extension.to_ascii_lowercase(),
            None => String::new(),
        }
    }

    pub fn read(&self) -> Result<Vec<u8>> {
        self.source.read()
    }
}

/// The comparison form of a path: ASCII lower case, `\` separators, and
/// no leading `.\` or `\`.
pub fn key(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        match c {
            '/' => out.push('\\'),
            c => out.push(c.to_ascii_lowercase()),
        }
    }
    let mut trimmed = out.as_str();
    loop {
        if let Some(rest) = trimmed.strip_prefix(".\\") {
            trimmed = rest;
        } else if let Some(rest) = trimmed.strip_prefix('\\') {
            trimmed = rest;
        } else {
            break;
        }
    }
    trimmed.to_owned()
}

/// Lists every file under `root` and every member of every FAR archive
/// below it, in a fixed order (directory entries sorted by name, an
/// archive's members in its own order). `keep` sees each lower-case
/// extension and decides whether the entry is listed. Unreadable archives
/// are reported in the second value and skipped.
pub fn walk(root: &Path, keep: &dyn Fn(&str) -> bool) -> Result<(Vec<Entry>, Vec<String>)> {
    let mut entries = Vec::new();
    let mut problems = Vec::new();
    visit(root, "", keep, &mut entries, &mut problems)?;
    Ok((entries, problems))
}

fn visit(
    directory: &Path,
    at: &str,
    keep: &dyn Fn(&str) -> bool,
    entries: &mut Vec<Entry>,
    problems: &mut Vec<String>,
) -> Result<()> {
    let listing = std::fs::read_dir(directory).map_err(|e| Error::io(directory, e))?;
    let mut paths = Vec::new();
    for entry in listing {
        paths.push(entry.map_err(|e| Error::io(directory, e))?.path());
    }
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let relative = format!("{at}{name}");
        if path.is_dir() {
            visit(&path, &format!("{relative}\\"), keep, entries, problems)?;
            continue;
        }
        let extension = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        if extension == "far" {
            match members(&path) {
                Ok(members) => {
                    for member in members {
                        let entry = Entry {
                            path: format!("{at}{}", member.name.replace('/', "\\")),
                            source: Source::Member {
                                archive: path.clone(),
                                name: member.name,
                                offset: u64::from(member.offset),
                                size: u64::from(member.stored_size),
                            },
                        };
                        if keep(&entry.extension()) {
                            entries.push(entry);
                        }
                    }
                }
                Err(error) => problems.push(error.to_string()),
            }
        }
        if keep(&extension) {
            entries.push(Entry {
                path: relative,
                source: Source::Loose(path),
            });
        }
    }
    Ok(())
}

/// Reads a FAR archive's directory without reading its members.
fn members(path: &Path) -> Result<Vec<far::Entry>> {
    let io = |e| Error::io(path, e);
    let mut file = File::open(path).map_err(io)?;
    let length = file.metadata().map_err(io)?.len();
    let mut header = [0; 16];
    file.read_exact(&mut header).map_err(io)?;
    let format = |e: sims_formats::Error| Error::format("FAR", 0, e.to_string());
    let offset = far::manifest_offset(&header, length).map_err(format)?;
    file.seek(SeekFrom::Start(u64::from(offset))).map_err(io)?;
    let mut directory = Vec::new();
    file.read_to_end(&mut directory).map_err(io)?;
    let mut entries = far::directory(&directory, length).map_err(format)?;
    for entry in &mut entries {
        if entry.stored_size != entry.unpacked_size {
            return Err(Error::format(
                "FAR",
                0,
                format!("{}: compressed members are not supported", entry.name),
            ));
        }
        entry.name = entry.name.replace('/', "\\");
    }
    Ok(entries)
}

/// The install's sound files, by path.
#[derive(Debug, Clone, Default)]
pub struct SoundFiles {
    pub root: PathBuf,
    files: BTreeMap<String, Entry>,
    /// Entries dropped because another took their path.
    pub shadowed: Vec<Entry>,
    /// Archives that could not be read.
    pub problems: Vec<String>,
}

impl SoundFiles {
    /// Lists the sound files under `root` (those with one of
    /// [`EXTENSIONS`]).
    pub fn scan(root: &Path) -> Result<Self> {
        let (entries, problems) = walk(root, &|e| EXTENSIONS.contains(&e))?;
        let mut files = Self {
            root: root.to_path_buf(),
            problems,
            ..Self::default()
        };
        for entry in entries {
            files.insert(entry);
        }
        Ok(files)
    }

    /// Adds a file. A loose file replaces a member; otherwise the first
    /// entry with a path is kept.
    pub fn insert(&mut self, entry: Entry) {
        let key = key(&entry.path);
        match self.files.get_mut(&key) {
            None => {
                self.files.insert(key, entry);
            }
            Some(old) => {
                let loose = matches!(entry.source, Source::Loose(_));
                let old_loose = matches!(old.source, Source::Loose(_));
                if loose && !old_loose {
                    let old = std::mem::replace(old, entry);
                    self.shadowed.push(old);
                } else {
                    self.shadowed.push(entry);
                }
            }
        }
    }

    /// The file at `path`, compared as [`key`] does.
    pub fn get(&self, path: &str) -> Option<&Entry> {
        self.files.get(&key(path))
    }

    /// Every file, in key order.
    pub fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.files.values()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}
