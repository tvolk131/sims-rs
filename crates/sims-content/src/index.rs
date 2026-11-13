use crate::install::resolve;
use crate::{Error, Install, Mount, Result};
use sims_formats::far;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// A member stored inside a FAR archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

/// One file, or one FAR archive member, under a mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    /// Index into [`Index::mounts`].
    pub mount: usize,
    /// The file on disk.
    pub path: PathBuf,
    /// Set when this resource is a member of the archive at `path`.
    pub member: Option<Member>,
}

impl Resource {
    /// The resource's own file name: the member name for archive members.
    pub fn name(&self) -> String {
        match &self.member {
            Some(member) => member.name.clone(),
            None => self
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }

    /// A readable location: the path relative to `root`, with `/member`
    /// appended for archive members.
    pub fn location(&self, root: &Path) -> String {
        let path = self.path.strip_prefix(root).unwrap_or(&self.path);
        let path = path.to_string_lossy().replace('\\', "/");
        match &self.member {
            Some(member) => format!("{path}/{}", member.name),
            None => path,
        }
    }

    pub fn read(&self) -> Result<Vec<u8>> {
        let io = |e| Error::io(&self.path, e);
        match &self.member {
            None => std::fs::read(&self.path).map_err(io),
            Some(member) => {
                let mut file = File::open(&self.path).map_err(io)?;
                file.seek(SeekFrom::Start(member.offset)).map_err(io)?;
                let mut bytes = vec![0; member.size as usize];
                file.read_exact(&mut bytes).map_err(io)?;
                Ok(bytes)
            }
        }
    }
}

/// Every file and archive member under an install's mounts, in load order.
#[derive(Debug, Clone)]
pub struct Index {
    /// The install root the resources were found under.
    pub root: PathBuf,
    pub mounts: Vec<Mount>,
    pub resources: Vec<Resource>,
    /// Problems that do not stop indexing, such as an unreadable archive.
    pub diagnostics: Vec<String>,
}

impl Index {
    /// Lists the install's mounts in the original's order.
    ///
    /// Within a directory, entries are taken in Windows/NTFS name order
    /// (ASCII case-insensitive), and a subdirectory is searched where its
    /// name falls among the files. Archive members keep their stored order.
    /// How the original walks a directory has not been confirmed yet; see
    /// `docs/decisions.md`. A mount that does not exist contributes nothing.
    pub fn build(install: &Install) -> Result<Self> {
        let mut index = Self {
            root: install.root.clone(),
            mounts: install.mounts(),
            resources: Vec::new(),
            diagnostics: Vec::new(),
        };
        for (number, mount) in index.mounts.clone().iter().enumerate() {
            if let Some(path) = resolve(&install.root, &mount.path)?
                && path.is_dir()
            {
                index.walk(number, &path, mount.recursive)?;
            }
        }
        Ok(index)
    }

    /// Resources whose own name matches, ignoring ASCII case, in load
    /// order, with their positions in [`Index::resources`].
    pub fn named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = (usize, &'a Resource)> {
        self.resources
            .iter()
            .enumerate()
            .filter(move |(_, r)| r.name().eq_ignore_ascii_case(name))
    }

    fn walk(&mut self, mount: usize, directory: &Path, recursive: bool) -> Result<()> {
        for path in sorted_entries(directory)? {
            if path.is_dir() {
                if recursive {
                    self.walk(mount, &path, true)?;
                }
            } else if is_far(&path) {
                self.archive(mount, path);
            } else {
                self.resources.push(Resource {
                    mount,
                    path,
                    member: None,
                });
            }
        }
        Ok(())
    }

    /// Adds an archive's members. An unreadable archive is skipped with a
    /// diagnostic, so one bad download does not stop the game loading.
    fn archive(&mut self, mount: usize, path: PathBuf) {
        match far_members(&path) {
            Ok(members) => {
                self.resources
                    .extend(members.into_iter().map(|member| Resource {
                        mount,
                        path: path.clone(),
                        member: Some(member),
                    }));
            }
            Err(error) => self.diagnostics.push(error.to_string()),
        }
    }
}

/// A directory's entries in NTFS name order: ASCII case folded to upper case
/// (NTFS's upcase table agrees with ASCII there), then raw bytes.
fn sorted_entries(directory: &Path) -> Result<Vec<PathBuf>> {
    let entries = std::fs::read_dir(directory).map_err(|e| Error::io(directory, e))?;
    let mut paths = Vec::new();
    for entry in entries {
        paths.push(entry.map_err(|e| Error::io(directory, e))?.path());
    }
    paths.sort_by_cached_key(|path| {
        let name = path.file_name().unwrap_or_default().as_encoded_bytes();
        (name.to_ascii_uppercase(), name.to_vec())
    });
    Ok(paths)
}

fn is_far(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("far"))
}

fn far_members(path: &Path) -> Result<Vec<Member>> {
    let io = |e| Error::io(path, e);
    let mut file = File::open(path).map_err(io)?;
    let len = file.metadata().map_err(io)?.len();
    let mut header = [0; 16];
    file.read_exact(&mut header).map_err(io)?;
    let invalid = |e: sims_formats::Error| {
        Error::io(
            path,
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()),
        )
    };
    let offset = far::manifest_offset(&header, len).map_err(invalid)?;
    file.seek(SeekFrom::Start(u64::from(offset))).map_err(io)?;
    let mut directory = Vec::new();
    file.read_to_end(&mut directory).map_err(io)?;
    let entries = far::directory(&directory, len).map_err(invalid)?;
    Ok(entries
        .into_iter()
        .map(|entry| Member {
            name: entry.name,
            offset: u64::from(entry.offset),
            size: u64::from(entry.stored_size),
        })
        .collect())
}
