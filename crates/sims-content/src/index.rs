use crate::file_table::{Table, key};
use crate::install::resolve;
use crate::{Error, Install, Mount, Result};
use sims_formats::far;
use std::collections::BTreeMap;
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
    /// Lists the install's mounts in the order the original scans them for
    /// objects.
    ///
    /// Within a directory, entries come in the order of the original's file
    /// table (see `file_table`): by the hash of each name, not by name. A
    /// subdirectory is searched where its own name falls in that order, and
    /// an archive's members are entries of the directory the archive is in.
    /// A mount that does not exist contributes nothing.
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

    /// Adds a neighborhood's `Characters` directory as a mount named
    /// `Characters`. The original scans the neighborhood's people after the
    /// user objects and before Downloads (0x43c609), so the mount and its
    /// resources go there, not at the end.
    pub fn add_characters(&mut self, path: &Path) -> Result<()> {
        let number = self
            .mounts
            .iter()
            .position(|m| m.path == Install::USER_OBJECTS)
            .map_or(self.mounts.len(), |n| n + 1);
        self.mounts.insert(
            number,
            Mount {
                path: "Characters".to_owned(),
                recursive: true,
            },
        );
        let at = self
            .resources
            .iter()
            .position(|r| r.mount >= number)
            .unwrap_or(self.resources.len());
        let later = self.resources.split_off(at);
        if path.is_dir() {
            self.walk(number, path, true)?;
        }
        self.resources.extend(later.into_iter().map(|mut r| {
            r.mount += 1;
            r
        }));
        Ok(())
    }

    /// Resources whose own name matches, ignoring ASCII case, in load
    /// order, with their positions in [`Index::resources`].
    pub fn named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = (usize, &'a Resource)> {
        self.resources
            .iter()
            .enumerate()
            .filter(move |(_, r)| r.name().eq_ignore_ascii_case(name))
    }

    /// Lists one mount: gathers its directories, then walks them.
    fn walk(&mut self, mount: usize, directory: &Path, recursive: bool) -> Result<()> {
        let mut directories = Directories::new();
        self.scan(&mut directories, &[], directory, recursive)?;
        self.list(mount, &directories, &[]);
        Ok(())
    }

    /// Gathers entries the way the original's directory scan does
    /// (0x5ba5d8): each entry the file system lists goes into its
    /// directory, a subdirectory is scanned as soon as it is reached, and
    /// so is an archive. `at` is the directory's key: its path below the
    /// mount, in key form, ending in `\`.
    fn scan(
        &mut self,
        directories: &mut Directories,
        at: &[u8],
        directory: &Path,
        recursive: bool,
    ) -> Result<()> {
        directories.entry(at.to_vec()).or_default();
        for path in sorted_entries(directory)? {
            let name = key(path.file_name().unwrap_or_default().as_encoded_bytes());
            let here = directories.get_mut(at).expect("inserted above");
            if path.is_dir() {
                // A file or directory on disk replaces an archive member of
                // the same name, keeping the member's place in the order.
                here.put(name.clone(), Entry::Directory);
                if recursive {
                    let child = [at, &name, b"\\"].concat();
                    self.scan(directories, &child, &path, true)?;
                }
            } else if name.ends_with(b"far") {
                // The original tests the last three letters, with no dot.
                here.add(name, Entry::Archive);
                match far_members(&path) {
                    Ok(members) => mount_archive(directories, at, &path, members),
                    // An unreadable archive is skipped, so one bad download
                    // does not stop the game loading.
                    Err(error) => self.diagnostics.push(error.to_string()),
                }
            } else {
                here.put(name, Entry::File(path));
            }
        }
        Ok(())
    }

    /// Lists a directory in the order the object scan walks its table
    /// (0x5ba9bd), going into each subdirectory where its name falls.
    fn list(&mut self, mount: usize, directories: &Directories, at: &[u8]) {
        let Some(directory) = directories.get(at) else {
            // A subdirectory of a mount that is not searched recursively.
            return;
        };
        for name in directory.table().iter() {
            match directory.entries.get(name) {
                // The object scan skips directories named `*.dir` (0x43a436).
                Some(Entry::Directory) => {
                    if !name.ends_with(b".dir") {
                        self.list(mount, directories, &[at, name, b"\\"].concat());
                    }
                }
                Some(Entry::File(path)) => self.resources.push(Resource {
                    mount,
                    path: path.clone(),
                    member: None,
                }),
                Some(Entry::Member(path, member)) => self.resources.push(Resource {
                    mount,
                    path: path.clone(),
                    member: Some(member.clone()),
                }),
                // An archive file itself.
                Some(Entry::Archive) | None => {}
            }
        }
    }
}

/// Every directory of one mount, by key: its path below the mount in key
/// form, ending in `\`. The mount itself has the empty key.
type Directories = BTreeMap<Vec<u8>, Directory>;

#[derive(Default)]
struct Directory {
    /// Names in the order they were first added.
    order: Vec<Vec<u8>>,
    entries: BTreeMap<Vec<u8>, Entry>,
}

impl Directory {
    /// Adds an entry unless the name is taken. Returns whether it was added.
    fn add(&mut self, name: Vec<u8>, entry: Entry) -> bool {
        if self.entries.contains_key(&name) {
            return false;
        }
        self.order.push(name.clone());
        self.entries.insert(name, entry);
        true
    }

    /// Adds an entry, or replaces the one of that name where it stands.
    fn put(&mut self, name: Vec<u8>, entry: Entry) {
        if !self.entries.contains_key(&name) {
            self.order.push(name.clone());
        }
        self.entries.insert(name, entry);
    }

    /// The original's table for this directory.
    fn table(&self) -> Table {
        let mut table = Table::default();
        for name in &self.order {
            table.insert(name.clone());
        }
        table
    }
}

/// What a name in a directory stands for.
enum Entry {
    Directory,
    File(PathBuf),
    Member(PathBuf, Member),
    /// An archive file. Its members are entries of their own.
    Archive,
}

/// Adds an archive's members (0x5b9dbc). A member's name may hold a path,
/// which places it in a subdirectory of the archive's own directory `at`,
/// whether or not that subdirectory exists on disk. A member never
/// replaces an entry that is already there.
fn mount_archive(directories: &mut Directories, at: &[u8], archive: &Path, members: Vec<Member>) {
    let split = |path: &[u8]| path.iter().rposition(|&b| b == b'\\').map_or(0, |n| n + 1);
    for member in members {
        let full = [at, &key(member.name.as_bytes())].concat();
        let (directory, name) = full.split_at(split(&full));
        let entry = Entry::Member(archive.to_path_buf(), member);
        if !directories
            .entry(directory.to_vec())
            .or_default()
            .add(name.to_vec(), entry)
        {
            continue;
        }
        // Each directory on the member's path becomes an entry of its
        // parent, up to the archive's own directory.
        let mut child = directory;
        while child.len() > at.len() {
            let trimmed = &child[..child.len() - 1];
            let (parent, name) = trimmed.split_at(split(trimmed));
            directories
                .entry(parent.to_vec())
                .or_default()
                .add(name.to_vec(), Entry::Directory);
            child = parent;
        }
    }
}

/// A directory's entries in NTFS name order: ASCII case folded to upper case
/// (NTFS's upcase table agrees with ASCII there), then raw bytes. This
/// stands in for the order Windows lists a directory in. That order decides
/// which of two names sharing a bucket of the original's file table comes
/// first, and which archive supplies a member two archives both hold.
/// **Unverified**: the capture the catalog order is checked against holds
/// no such pair.
pub(crate) fn sorted_entries(directory: &Path) -> Result<Vec<PathBuf>> {
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
