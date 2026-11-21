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
    /// The bytes the member takes in the archive, which are its contents:
    /// a member whose unpacked size differs is compressed, which is not
    /// supported, so `far_members` leaves it out.
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

    /// The last component of [`Resource::name`]: an archive member's name
    /// may hold directories, separated by `\\` or `/`.
    pub fn file_name(&self) -> String {
        let name = self.name();
        match name.rfind(['\\', '/']) {
            Some(at) => name[at + 1..].to_owned(),
            None => name,
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

    /// Reads the resource. A resource larger than [`far::MAX_MEMBER`] and a
    /// member that no longer lies inside
    /// its archive (the file changed since it was indexed) fail.
    pub fn read(&self) -> Result<Vec<u8>> {
        let io = |e| Error::io(&self.path, e);
        let invalid = |message: String| {
            Error::io(
                &self.path,
                std::io::Error::new(std::io::ErrorKind::InvalidData, message),
            )
        };
        let mut file = File::open(&self.path).map_err(io)?;
        let len = file.metadata().map_err(io)?.len();
        let (offset, size) = match &self.member {
            None => (0, len),
            Some(member) => {
                if member
                    .offset
                    .checked_add(member.size)
                    .is_none_or(|end| end > len)
                {
                    return Err(invalid(format!("FAR member {} outside file", member.name)));
                }
                (member.offset, member.size)
            }
        };
        if size > far::MAX_MEMBER {
            return Err(invalid(format!(
                "{} exceeds the 128 MiB resource limit",
                self.name()
            )));
        }
        file.seek(SeekFrom::Start(offset)).map_err(io)?;
        let mut bytes = Vec::with_capacity(size as usize);
        file.take(size).read_to_end(&mut bytes).map_err(io)?;
        if bytes.len() as u64 != size {
            return Err(invalid(format!("{} ends early", self.name())));
        }
        Ok(bytes)
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

/// A directory's listing ([`Index::listing`]): each entry's key-form name
/// and resource, and the reasons any entries were skipped.
pub(crate) type Listing = (Vec<(Vec<u8>, Resource)>, Vec<String>);

impl Index {
    /// The name of the mount [`Index::add_characters`] adds.
    pub const CHARACTERS: &'static str = "Characters";

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
                path: Self::CHARACTERS.to_owned(),
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

    /// Resources whose name matches, ignoring ASCII case, in load order,
    /// with their positions in [`Index::resources`]. A name without a
    /// directory matches a resource's file name, the last component of its
    /// name, so `SofaGlobals.iff` finds the member `sub\SofaGlobals.iff`.
    /// A name with a directory (`\` or `/`) matches the whole
    /// [`Resource::name`], as before.
    pub fn named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = (usize, &'a Resource)> {
        let qualified = name.contains(['\\', '/']);
        self.resources.iter().enumerate().filter(move |(_, r)| {
            if qualified {
                r.name().eq_ignore_ascii_case(name)
            } else {
                r.file_name().eq_ignore_ascii_case(name)
            }
        })
    }

    /// The files and archive members in one directory of a mount and every
    /// directory below it, in the order the original's file table walks
    /// them (0x4125a2): a subdirectory is gone into where its name falls
    /// (0x412699), as the pattern catalogs' scan asks for every directory
    /// (0x412863, set at 0x4a06e9). Each comes with its file name in key
    /// form. `directory` is the mount's directory on disk, which is
    /// scanned recursively, as the original mounts it, and `at` the
    /// directory's key below it (empty, or ending in `\`).
    pub(crate) fn listing(directory: &Path, at: &[u8]) -> Result<Listing> {
        let mut files = MountedFiles::default();
        files.scan(&[], directory, true, sorted_entries)?;
        let mut listed = Vec::new();
        Self::list_all(&files.directories, at, &mut listed);
        Ok((listed, files.diagnostics))
    }

    /// [`Index::listing`]'s walk of one directory.
    fn list_all(directories: &Directories, at: &[u8], listed: &mut Vec<(Vec<u8>, Resource)>) {
        let Some(here) = directories.get(at) else {
            return;
        };
        for name in here.table().iter() {
            let resource = |path: &PathBuf, member: Option<&Member>| Resource {
                mount: 0,
                path: path.clone(),
                member: member.cloned(),
            };
            match here.entries.get(name) {
                Some(Entry::File(path)) => listed.push((name.to_vec(), resource(path, None))),
                Some(Entry::Member(path, member)) => {
                    listed.push((name.to_vec(), resource(path, Some(member))))
                }
                Some(Entry::Directory) => {
                    Self::list_all(directories, &[at, name, b"\\"].concat(), listed)
                }
                Some(Entry::Archive) | None => {}
            }
        }
    }

    /// Lists one mount: gathers its directories, then walks them.
    fn walk(&mut self, mount: usize, directory: &Path, recursive: bool) -> Result<()> {
        let mut files = MountedFiles::default();
        files.scan(&[], directory, recursive, sorted_entries)?;
        Self::list(&mut self.resources, mount, &files.directories, &[]);
        self.diagnostics.extend(files.diagnostics);
        Ok(())
    }

    /// Lists a directory in the order the object scan walks its table
    /// (0x5ba9bd), going into each subdirectory where its name falls.
    fn list(resources: &mut Vec<Resource>, mount: usize, directories: &Directories, at: &[u8]) {
        let Some(directory) = directories.get(at) else {
            // A subdirectory of a mount that is not searched recursively.
            return;
        };
        for name in directory.table().iter() {
            match directory.entries.get(name) {
                // The object scan skips directories named `*.dir` (0x43a436).
                Some(Entry::Directory) => {
                    if !name.ends_with(b".dir") {
                        Self::list(resources, mount, directories, &[at, name, b"\\"].concat());
                    }
                }
                Some(Entry::File(path)) => resources.push(Resource {
                    mount,
                    path: path.clone(),
                    member: None,
                }),
                Some(Entry::Member(path, member)) => resources.push(Resource {
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

/// Directory tables shared by the startup mounts and the later avatar paths.
#[derive(Default)]
pub(crate) struct MountedFiles {
    directories: Directories,
    diagnostics: Vec<String>,
}

impl MountedFiles {
    /// Register a root-relative path once, preserving any existing virtual
    /// directory supplied by an earlier FAR mount (0x5ba5d8, 0x5b9dbc).
    pub(crate) fn register(&mut self, root: &Path, path: &str, recursive: bool) -> Result<()> {
        let at = mount_key(path)?;
        if !self.directories.contains_key(&at)
            && let Some(directory) = resolve(root, path)?
            && directory.is_dir()
        {
            self.scan(&at, &directory, recursive, ntfs_entries)?;
            if let Some(reason) = self.diagnostics.first() {
                return Err(Error::Unsupported(format!(
                    "0x5b9dbc wardrobe FAR mount: {reason}"
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn exists(&self, root: &Path, path: &str) -> Result<bool> {
        if let Some((parent, name)) = path.rsplit_once('/') {
            let at = mount_key(parent)?;
            let name = key(&disk_name(name.as_ref()));
            if self
                .directories
                .get(&at)
                .is_some_and(|dir| dir.entries.contains_key(&name))
            {
                return Ok(true);
            }
        }
        Ok(resolve(root, path)?.is_some())
    }

    /// One directory, in its file-map order. Appearance searches at
    /// 0x5c1a7d do not recurse into its subdirectories.
    pub(crate) fn files(&self, path: &str) -> Result<Vec<(Vec<u8>, Resource)>> {
        let Some(dir) = self.directories.get(&mount_key(path)?) else {
            return Ok(Vec::new());
        };
        Ok(dir
            .table()
            .iter()
            .filter_map(|name| {
                let (path, member) = match &dir.entries[name] {
                    Entry::File(path) => (path.clone(), None),
                    Entry::Member(path, member) => (path.clone(), Some(member.clone())),
                    Entry::Directory | Entry::Archive => return None,
                };
                Some((
                    name.to_vec(),
                    Resource {
                        mount: 0,
                        path,
                        member,
                    },
                ))
            })
            .collect())
    }

    pub(crate) fn objects(&self, path: &str) -> Result<Vec<Resource>> {
        let mut resources = Vec::new();
        Index::list(&mut resources, 0, &self.directories, &mount_key(path)?);
        Ok(resources)
    }

    /// Looks up `relative` (raw Windows-1252 bytes) under mounted `directory`,
    /// matching how `0x5a3810` opens an `.ndx` entry's `dir + catalog.bcf`
    /// through the virtual file system (`0x5b8f64` → `0x5b94ae`).
    pub(crate) fn lookup(&self, directory: &str, relative: &[u8]) -> Result<Option<Resource>> {
        let at = mount_key(directory)?;
        let rel = key(relative);
        let mut rel = rel.as_slice();
        while let Some(rest) = rel.strip_prefix(b"\\") {
            rel = rest;
        }
        let full = [at.as_slice(), rel].concat();
        let split = full.iter().rposition(|&b| b == b'\\').map_or(0, |n| n + 1);
        let (dir_key, name) = full.split_at(split);
        let Some(entry) = self
            .directories
            .get(dir_key)
            .and_then(|dir| dir.entries.get(name))
        else {
            return Ok(None);
        };
        Ok(match entry {
            Entry::File(path) => Some(Resource {
                mount: 0,
                path: path.clone(),
                member: None,
            }),
            Entry::Member(path, member) => Some(Resource {
                mount: 0,
                path: path.clone(),
                member: Some(member.clone()),
            }),
            Entry::Directory | Entry::Archive => None,
        })
    }

    /// Gathers entries the way the original's directory scan does
    /// (0x5ba5d8): each entry the file system lists goes into its
    /// directory, a subdirectory is scanned as soon as it is reached, and
    /// so is an archive. `at` is the directory's key: its path below the
    /// mount, in key form, ending in `\`.
    fn scan(
        &mut self,
        at: &[u8],
        directory: &Path,
        recursive: bool,
        entries: fn(&Path) -> Result<Vec<PathBuf>>,
    ) -> Result<()> {
        self.directories.entry(at.to_vec()).or_default();
        for path in entries(directory)? {
            let name = key(&disk_name(path.file_name().unwrap_or_default()));
            let here = self.directories.get_mut(at).expect("inserted above");
            if path.is_dir() {
                // A file or directory on disk replaces an archive member of
                // the same name, keeping the member's place in the order.
                here.put(name.clone(), Entry::Directory);
                if recursive {
                    let child = [at, &name, b"\\"].concat();
                    self.scan(&child, &path, true, entries)?;
                }
            } else if name.ends_with(b"far") {
                // The original tests the last three letters, with no dot.
                here.add(name, Entry::Archive);
                match archive_members(&path) {
                    Ok((members, skipped)) => {
                        self.diagnostics.extend(skipped);
                        mount_archive(&mut self.directories, at, &path, members);
                    }
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
}

fn mount_key(path: &str) -> Result<Vec<u8>> {
    let name = far::encode_name(path).ok_or_else(|| {
        Error::Unsupported(format!(
            "0x5b96c8 mount path is outside Windows-1252: {path}"
        ))
    })?;
    let mut at = key(&name);
    at.push(b'\\');
    Ok(at)
}

/// Replay the startup's NTFS upcase ordering for Windows-1252 filenames.
/// The output is insertion order. Callers enumerate the resulting tables.
fn ntfs_entries(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = sorted_entries(directory)?;
    for path in &paths {
        let name = path.file_name().and_then(std::ffi::OsStr::to_str);
        if name.and_then(far::encode_name).is_none() {
            return Err(Error::Unsupported(format!(
                "0x5ba5d8 filename is outside Windows-1252: {}",
                path.display()
            )));
        }
    }
    paths.sort_by_cached_key(|path| {
        path.file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .chars()
            .map(|ch| {
                let mut upper = ch.to_uppercase();
                let first = upper.next().unwrap();
                if upper.next().is_some() {
                    ch as u16
                } else {
                    first as u16
                }
            })
            .collect::<Vec<_>>()
    });
    Ok(paths)
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
///
/// A member's place comes from the bytes of its name as the archive stores
/// them, not from the decoded name: the file table hashes those bytes.
fn mount_archive(
    directories: &mut Directories,
    at: &[u8],
    archive: &Path,
    members: Vec<RawMember>,
) {
    let split = |path: &[u8]| path.iter().rposition(|&b| b == b'\\').map_or(0, |n| n + 1);
    for (member, raw_name) in members {
        let full = [at, &key(&raw_name)].concat();
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

/// The bytes of a name on disk as the original would see them: in the
/// Windows-1252 code page, as archive member names are, so a loose file
/// and a member of one name are one entry. A name that is not valid UTF-8
/// or has a character the code page lacks keeps the file system's bytes.
/// **Unverified**: the install holds no name outside ASCII.
fn disk_name(name: &std::ffi::OsStr) -> Vec<u8> {
    name.to_str()
        .and_then(far::encode_name)
        .unwrap_or_else(|| name.as_encoded_bytes().to_vec())
}

/// A directory's entries in NTFS name order: ASCII case folded to upper case
/// (NTFS's upcase table agrees with ASCII there), then raw bytes. This
/// stands in for the order Windows lists a directory in. That order decides
/// which of two names sharing a bucket of the original's file table comes
/// first, and which archive supplies a member two archives both hold.
/// **Unverified**: the capture the catalog order is checked against holds
/// no such pair.
pub(crate) fn sorted_entries(directory: &Path) -> Result<Vec<PathBuf>> {
    // The one listing in this crate; it sorts what it reads.
    #[allow(clippy::disallowed_methods)]
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

/// The most directories an archive member's name may hold. Each one is a
/// directory the index lists, so this bounds that work. The game's own
/// members are at most four deep.
const MAX_MEMBER_DEPTH: usize = 32;
/// The longest archive member name the index takes, in bytes. The game's
/// own are under 100 bytes.
const MAX_MEMBER_NAME: usize = 1024;

/// An archive's members, as [`far_members`] reads them, failing when any
/// member is left out. For directories that hold only the game's own
/// files, where a missing member would silently change what loads.
pub(crate) fn all_far_members(path: &Path) -> Result<Vec<Member>> {
    let (members, mut skipped) = far_members(path)?;
    match skipped.pop() {
        None => Ok(members),
        Some(reason) => Err(Error::Unsupported(reason)),
    }
}

/// An archive's members, and a diagnostic for each member left out.
///
/// FAR 1a stores members as they are. A member whose unpacked size differs
/// from its stored size is compressed; reading it as stored bytes would be
/// wrong, and decompression is not implemented, so it is left out, as the
/// prototype's reader refused it. Whether the original lists such a member
/// is unverified: the game's own archives hold none. A member whose name
/// is longer than 1024 bytes (`MAX_MEMBER_NAME`) or holds more than 32
/// directories (`MAX_MEMBER_DEPTH`) is left out too. The directory is at
/// most [`far::MAX_DIRECTORY`] bytes, checked before it is read.
pub fn far_members(path: &Path) -> Result<(Vec<Member>, Vec<String>)> {
    let (members, skipped) = archive_members(path)?;
    Ok((
        members.into_iter().map(|(member, _)| member).collect(),
        skipped,
    ))
}

/// An archive member with the bytes of its name as the archive stores them.
type RawMember = (Member, Vec<u8>);

/// [`far_members`], each with the bytes of its name.
fn archive_members(path: &Path) -> Result<(Vec<RawMember>, Vec<String>)> {
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
    // Bounded before reading: a sparse file may claim gigabytes.
    let directory_len = far::directory_len(offset, len).map_err(invalid)?;
    file.seek(SeekFrom::Start(u64::from(offset))).map_err(io)?;
    let mut directory = vec![0; directory_len];
    file.read_exact(&mut directory).map_err(io)?;
    let entries = far::directory(&directory, len).map_err(invalid)?;
    let mut members = Vec::with_capacity(entries.len());
    let mut skipped = Vec::new();
    for entry in entries {
        let depth = entry
            .raw_name
            .iter()
            .filter(|&&b| b == b'\\' || b == b'/')
            .count();
        if depth > MAX_MEMBER_DEPTH || entry.raw_name.len() > MAX_MEMBER_NAME {
            skipped.push(format!(
                "{}: member name of {} bytes in {depth} directories is too long",
                path.display(),
                entry.raw_name.len(),
            ));
            continue;
        }
        if entry.stored_size != entry.unpacked_size {
            skipped.push(format!(
                "{}: compressed member {} ({} bytes stored, {} unpacked) is not supported",
                path.display(),
                entry.name,
                entry.stored_size,
                entry.unpacked_size
            ));
            continue;
        }
        members.push((
            Member {
                name: entry.name,
                offset: u64::from(entry.offset),
                size: u64::from(entry.stored_size),
            },
            entry.raw_name,
        ));
    }
    Ok((members, skipped))
}
