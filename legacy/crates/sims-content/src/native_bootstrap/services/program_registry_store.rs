//! Owning, synchronous private-user ASCII journal; never a Windows registry hive.
//!
//! Parent integration must explicitly select this profile. No import is installed
//! here. macOS/Linux only; other hosts refuse before filesystem access. The
//! directory must already exist, be absolute, and have mode 0700. All ancestors
//! are opened without following symlinks and retained, as is the locked leaf.
//! `flock` is advisory: hostile same-UID writers, rename-and-restore races, remote
//! filesystem semantics, and Windows caller/security/encoding fidelity are not
//! covered. File descriptors are never cloned or exposed.
//! The filesystem must support ordinary local advisory locks and file/directory
//! sync. No repair/reset/recovery entry point exists. Poison is retained for the
//! owning Hive lifetime; after ownership drops, a syntactically complete durable
//! journal cannot reveal whether an earlier caller lost a sync acknowledgement.
//! Checksums detect corruption, not an authorized same-UID writer's forgery.
//!
//! V1 format: HEADER, then frames consisting of LE u32 total size, LE u32
//! one-based sequence, LE u16 created-key count, count*(LE u16 path length,
//! lowercase full relative ASCII path), and SHA256(HEADER || previous digest ||
//! frame excluding digest). The initial previous digest is SHA256(HEADER).
//! Every frame is one creation transaction of 1..=32 consecutive missing levels.
//! The root is implicit, counts as one key, and consumes no journal record.
//! A malformed or incomplete file is refused whole, without repair or truncation.

use std::ffi::OsStr;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Weak};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_keys: usize,
    pub max_records: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Unsupported,
    Unavailable,
    Poisoned,
}

/// An acquisition failure retains all acquired resources until this error drops.
/// In particular a possibly written file is never removed or adopted here.
pub struct Error {
    kind: ErrorKind,
    context: &'static str,
    source: Option<std::io::Error>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    retained: Option<Box<host::Resources>>,
}

impl Error {
    pub fn kind(&self) -> ErrorKind { self.kind }

    fn new(kind: ErrorKind, context: &'static str) -> Self {
        Self {
            kind, context, source: None,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            retained: None,
        }
    }

    fn io(context: &'static str, source: std::io::Error) -> Self {
        let mut error = Self::new(ErrorKind::Unavailable, context);
        error.source = Some(source);
        error
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("Error");
        d.field("kind", &self.kind).field("context", &self.context)
            .field("source", &self.source);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        d.field("retains_resources", &self.retained.is_some());
        d.finish_non_exhaustive()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "private-user registry {:?}: {}", self.kind, self.context)?;
        if let Some(source) = &self.source { write!(f, ": {source}")?; }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e as &(dyn std::error::Error + 'static))
    }
}

#[derive(Debug)]
struct Epoch { _private: () }

#[derive(Debug)]
struct KeyRecord { canonical: Box<[u8]> }

/// An actual interned key plus a weak live-hive epoch, never a guest handle.
#[derive(Debug, Clone)]
pub struct Node {
    epoch: Weak<Epoch>,
    record: Arc<KeyRecord>,
}

impl Node {
    pub fn canonical_ascii(&self) -> &[u8] { &self.record.canonical }

    /// Historical comparison only; it does not establish that either hive lives.
    pub fn same_node(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.epoch, &other.epoch) && Arc::ptr_eq(&self.record, &other.record)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub keys: usize,
    pub records: usize,
    pub bytes: usize,
    pub poisoned: bool,
    pub pending_write: bool,
}

/// Owns the directory chain, file, exclusive lock, tree, epoch and write history.
pub struct Hive {
    _private: (),
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    state: host::State,
}

impl fmt::Debug for Hive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hive").field("observation", &self.snapshot()).finish_non_exhaustive()
    }
}

impl Hive {
    pub fn create_new(directory: &Path, leaf: &OsStr, limits: Limits) -> Result<Self, Error> {
        Self::begin(directory, leaf, limits, true)
    }

    pub fn open(directory: &Path, leaf: &OsStr, limits: Limits) -> Result<Self, Error> {
        Self::begin(directory, leaf, limits, false)
    }

    fn begin(directory: &Path, leaf: &OsStr, limits: Limits, create: bool) -> Result<Self, Error> {
        if !(1..=8 * 1024 * 1024).contains(&limits.max_bytes)
            || !(1..=4096).contains(&limits.max_keys)
            || !(1..=4096).contains(&limits.max_records)
        {
            return Err(Error::new(ErrorKind::Unsupported, "limits outside profile"));
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        { host::State::begin(directory, leaf, limits, create).map(|state| Self { _private: (), state }) }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = (directory, leaf, create);
            Err(Error::new(ErrorKind::Unsupported, "host outside macOS/Linux profile"))
        }
    }

    pub fn validate(&mut self) -> Result<(), Error> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        { self.state.validate() }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        { Err(Error::new(ErrorKind::Unsupported, "unsupported host")) }
    }

    pub fn root(&mut self) -> Result<Node, Error> {
        self.validate()?;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        { Ok(self.state.node(0)) }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        { Err(Error::new(ErrorKind::Unsupported, "unsupported host")) }
    }

    pub fn create(&mut self, parent: &Node, relative_ascii: &[u8]) -> Result<Node, Error> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        { self.state.create(parent, relative_ascii) }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = (parent, relative_ascii);
            Err(Error::new(ErrorKind::Unsupported, "unsupported host"))
        }
    }

    /// Memory only, including after uncertain effects; never probes the file.
    pub fn snapshot(&self) -> Observation {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        { self.state.snapshot() }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        { Observation { keys: 0, records: 0, bytes: 0, poisoned: false, pending_write: false } }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod host {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::ffi::{CStr, CString};
    use std::fs::{File, Metadata};
    use std::io::{self, Write};
    use std::mem::MaybeUninit;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{FileExt, MetadataExt};
    use std::path::Component;

    pub(super) const HEADER: &[u8] = b"SIMS-PRIVATE-USER-ASCII\0\x01\0";
    const FRAME_MAX: usize = 128 * 1024;
    const FRAME_FIXED: usize = 4 + 4 + 2 + 32;
    const MAX_RELATIVE: usize = 255 - 17 - 1;

    fn unsupported(why: &'static str) -> Error { Error::new(ErrorKind::Unsupported, why) }
    fn unavailable(why: &'static str) -> Error { Error::new(ErrorKind::Unavailable, why) }
    fn reserve<T>(v: &mut Vec<T>, count: usize) -> Result<(), Error> {
        v.try_reserve_exact(count).map_err(|_| unavailable("buffer reservation failed"))
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Identity { dev: u64, ino: u64, uid: u32, mode: u32, links: u64, bytes: u64 }

    impl Identity {
        fn metadata(m: &Metadata) -> Self {
            Self { dev: m.dev(), ino: m.ino(), uid: m.uid(), mode: m.mode(),
                links: m.nlink(), bytes: m.len() }
        }
        fn same_object(self, other: Self) -> bool {
            self.dev == other.dev && self.ino == other.ino
                && self.uid == other.uid && self.mode == other.mode
        }
        fn private_file(self, uid: u32, max: usize) -> bool {
            self.mode & libc::S_IFMT as u32 == libc::S_IFREG as u32 && self.mode & 0o7777 == 0o600
                && self.uid == uid && self.links == 1 && self.bytes <= max as u64
        }
        fn directory(self) -> bool {
            self.mode & libc::S_IFMT as u32 == libc::S_IFDIR as u32 && self.links > 0
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Stamp { identity: Identity, mtime: (i64, i64), ctime: (i64, i64) }
    fn stamp(file: &File) -> Result<Stamp, Error> {
        let m = file.metadata().map_err(|e| Error::io("descriptor metadata", e))?;
        Ok(Stamp { identity: Identity::metadata(&m), mtime: (m.mtime(), m.mtime_nsec()),
            ctime: (m.ctime(), m.ctime_nsec()) })
    }

    fn at(directory: i32, name: &CStr) -> Result<Identity, Error> {
        let mut s = MaybeUninit::<libc::stat>::uninit();
        // SAFETY: borrowed valid C string and writable stat; initialization checked.
        if unsafe { libc::fstatat(directory, name.as_ptr(), s.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
            return Err(Error::io("nofollow pathname metadata", io::Error::last_os_error()));
        }
        // SAFETY: successful fstatat initialized the object.
        let s = unsafe { s.assume_init() };
        if s.st_size < 0 { return Err(unavailable("negative object size")); }
        Ok(Identity { dev: s.st_dev as u64, ino: s.st_ino as u64, uid: s.st_uid,
            mode: s.st_mode as u32, links: s.st_nlink as u64, bytes: s.st_size as u64 })
    }

    fn open_at(directory: i32, name: &CStr, flags: i32) -> Result<File, Error> {
        // SAFETY: valid C string, mode is supplied for O_CREAT, descriptor uniquely
        // transferred into File on success. Never retry close or clone this fd.
        let fd = unsafe { libc::openat(directory, name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK, 0o600 as libc::c_uint) };
        if fd < 0 { return Err(Error::io("exclusive relative open", io::Error::last_os_error())); }
        // SAFETY: openat returned one newly owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    /// Some fields can be incomplete on acquisition failure. Error retains this
    /// exact object, not reconstructed names or a reopened descriptor.
    pub(super) struct Resources {
        directories: Vec<File>,
        names: Vec<CString>,
        directory_ids: Vec<Identity>,
        file: Option<File>,
        file_id: Option<Identity>,
        leaf: CString,
        uid: u32,
        max_bytes: usize,
        locked: bool,
    }

    impl Resources {
        fn acquire(directory: &Path, leaf: &OsStr, limits: Limits, create: bool) -> Result<Self, Error> {
            let raw = leaf.as_bytes();
            if raw.is_empty() || raw == b"." || raw == b".." || raw.contains(&b'/')
                || raw.len() > 255 || raw.contains(&0) || !directory.is_absolute()
                || directory.as_os_str().as_bytes().len() > 4096
            { return Err(unsupported("absolute directory and single ordinary leaf required")); }
            let mut names = Vec::new();
            for part in directory.components() {
                match part {
                    Component::RootDir => (),
                    Component::Normal(name) => {
                        if names.len() == 256 { return Err(unsupported("directory chain exceeds profile")); }
                        reserve(&mut names, 1)?;
                        names.push(CString::new(name.as_bytes()).map_err(|_| unsupported("NUL in directory"))?);
                    }
                    _ => return Err(unsupported("nonordinary directory component")),
                }
            }
            // SAFETY: these only observe the calling process's own credentials.
            let uid = unsafe { libc::getuid() };
            if uid != unsafe { libc::geteuid() } { return Err(unsupported("real/effective uid mismatch")); }
            let mut r = Self { directories: Vec::new(), names, directory_ids: Vec::new(),
                file: None, file_id: None, leaf: CString::new(raw).map_err(|_| unsupported("NUL leaf"))?,
                uid, max_bytes: limits.max_bytes, locked: false };
            reserve(&mut r.directories, r.names.len() + 1)?;
            reserve(&mut r.directory_ids, r.names.len() + 1)?;
            if let Err(error) = r.acquire_inner(create) { return Err(error.holding(r)); }
            Ok(r)
        }

        fn acquire_inner(&mut self, create: bool) -> Result<(), Error> {
            self.directories.push(open_at(libc::AT_FDCWD, c"/", libc::O_RDONLY | libc::O_DIRECTORY)?);
            self.directory_ids.push(stamp(&self.directories[0])?.identity);
            for name in &self.names {
                let parent = self.directories.last().expect("root retained").as_raw_fd();
                let before = at(parent, name)?;
                if !before.directory() { return Err(unavailable("symlink or special directory component")); }
                self.directories.push(open_at(parent, name, libc::O_RDONLY | libc::O_DIRECTORY)?);
                let id = stamp(self.directories.last().expect("just acquired"))?.identity;
                self.directory_ids.push(id);
                if !id.same_object(before) { return Err(unavailable("directory changed during open")); }
            }
            self.check_directories()?;
            let dir = self.directory().as_raw_fd();
            let before = if create { None } else {
                let id = at(dir, &self.leaf)?;
                if !id.private_file(self.uid, self.max_bytes) { return Err(unavailable("nonprivate or special leaf")); }
                Some(id)
            };
            let flags = libc::O_RDWR | libc::O_APPEND
                | if create { libc::O_CREAT | libc::O_EXCL } else { 0 };
            self.file = Some(open_at(dir, &self.leaf, flags)?);
            let id = stamp(self.file())?.identity;
            self.file_id = Some(id);
            if !id.private_file(self.uid, self.max_bytes)
                || before.is_some_and(|old| old != id) || (create && id.bytes != 0)
            { return Err(unavailable("leaf changed or is not owned regular 0600 single-link")); }
            // SAFETY: uniquely owned descriptor. Exactly one nonblocking lock
            // attempt, retained until ordinary File drop, without explicit unlock.
            if unsafe { libc::flock(self.file().as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(Error::io("exclusive nonblocking flock", io::Error::last_os_error()));
            }
            self.locked = true;
            self.check(None)?;
            Ok(())
        }

        fn directory(&self) -> &File { self.directories.last().expect("acquired directory") }
        fn file(&self) -> &File { self.file.as_ref().expect("acquired file") }

        fn check_directories(&self) -> Result<(), Error> {
            // SAFETY: own credentials only, not process enumeration/impersonation.
            if unsafe { libc::getuid() } != self.uid || unsafe { libc::geteuid() } != self.uid {
                return Err(unavailable("credentials changed"));
            }
            for (i, directory) in self.directories.iter().enumerate() {
                let id = stamp(directory)?.identity;
                let path = if i == 0 { at(libc::AT_FDCWD, c"/")? }
                    else { at(self.directories[i - 1].as_raw_fd(), &self.names[i - 1])? };
                if !id.directory() || !id.same_object(self.directory_ids[i]) || !id.same_object(path) {
                    return Err(unavailable("directory descriptor/path identity changed"));
                }
            }
            let final_id = stamp(self.directory())?.identity;
            if final_id.uid != self.uid || final_id.mode & 0o7777 != 0o700 {
                return Err(unavailable("final directory must be owned mode 0700"));
            }
            Ok(())
        }

        fn check(&self, expected_bytes: Option<usize>) -> Result<Stamp, Error> {
            self.check_directories()?;
            if !self.locked { return Err(unavailable("exclusive ownership not acquired")); }
            let s = stamp(self.file())?;
            let path = at(self.directory().as_raw_fd(), &self.leaf)?;
            if !s.identity.private_file(self.uid, self.max_bytes)
                || !s.identity.same_object(self.file_id.expect("acquired identity"))
                || s.identity != path
                || expected_bytes.is_some_and(|n| s.identity.bytes != n as u64)
            { return Err(unavailable("file descriptor/path/size/mode/link identity changed")); }
            Ok(s)
        }

        fn read_admitted(&self) -> Result<Vec<u8>, Error> {
            let before = self.check(None)?;
            let size = before.identity.bytes as usize;
            let mut bytes = Vec::new();
            reserve(&mut bytes, size)?;
            bytes.resize(size, 0);
            read_exact_at(self.file(), &mut bytes, 0)?;
            if self.check(Some(size))? != before { return Err(unavailable("file changed during admission read")); }
            Ok(bytes)
        }

        fn compare(&self, admitted: &[u8], pending: &[u8]) -> Result<(), Error> {
            let size = admitted.len().checked_add(pending.len()).ok_or_else(|| unavailable("size overflow"))?;
            let before = self.check(Some(size))?;
            let mut buffer = [0u8; 8192];
            let mut offset = 0;
            for expected in [admitted, pending] {
                for chunk in expected.chunks(buffer.len()) {
                    read_exact_at(self.file(), &mut buffer[..chunk.len()], offset)?;
                    if buffer[..chunk.len()] != *chunk { return Err(unavailable("actual journal bytes changed")); }
                    offset += chunk.len();
                }
            }
            if self.check(Some(size))? != before { return Err(unavailable("file changed around content read")); }
            Ok(())
        }
    }

    impl Error {
        fn holding(mut self, resources: Resources) -> Self {
            self.retained = Some(Box::new(resources));
            self
        }
    }

    fn read_exact_at(file: &File, mut output: &mut [u8], mut offset: usize) -> Result<(), Error> {
        while !output.is_empty() {
            let n = file.read_at(output, offset as u64).map_err(|e| Error::io("bounded descriptor read", e))?;
            if n == 0 { return Err(unavailable("truncated journal read")); }
            offset += n;
            output = &mut output[n..];
        }
        Ok(())
    }

    #[derive(Debug)]
    struct Intent {
        frame: Vec<u8>,
        nodes: Vec<Arc<KeyRecord>>,
        committed: bool,
    }

    pub(super) struct State {
        resources: Resources,
        limits: Limits,
        epoch: Arc<Epoch>,
        nodes: Vec<Arc<KeyRecord>>,
        journal: Vec<u8>,
        history: Vec<Intent>,
        digest: [u8; 32],
        records: usize,
        poisoned: bool,
        pending_write: bool,
        #[cfg(test)]
        fault: Option<Fault>,
    }

    impl State {
        pub(super) fn begin(directory: &Path, leaf: &OsStr, limits: Limits, create: bool) -> Result<Self, Error> {
            if limits.max_bytes < HEADER.len() { return Err(unsupported("journal budget cannot hold header")); }
            // Recoverable reservations precede resource acquisition and any write.
            let mut nodes = Vec::new(); reserve(&mut nodes, limits.max_keys)?;
            nodes.push(Arc::new(KeyRecord { canonical: Box::from(&b""[..]) }));
            let mut history = Vec::new(); reserve(&mut history, limits.max_records)?;
            let mut journal = Vec::new(); reserve(&mut journal, limits.max_bytes)?;
            let resources = Resources::acquire(directory, leaf, limits, create)?;
            let result: Result<(usize, [u8; 32]), Error> = (|| {
                if create {
                    append_once(resources.file(), HEADER, constructor_fault())?;
                    // The caller made the directory; acknowledge the newly created
                    // leaf's directory entry as well as its header, without chmod.
                    resources.directory().sync_all().map_err(|e| Error::io("new leaf directory sync", e))?;
                    resources.compare(HEADER, &[])?;
                    journal.extend_from_slice(HEADER);
                } else {
                    let admitted = resources.read_admitted()?;
                    journal.extend_from_slice(&admitted);
                }
                let (records, digest) = decode(&journal, limits, &mut nodes, &mut history)?;
                resources.compare(&journal, &[])?;
                Ok((records, digest))
            })();
            match result {
                Err(error) => Err(error.holding(resources)),
                Ok((records, digest)) => Ok(Self { resources, limits,
                    epoch: Arc::new(Epoch { _private: () }), nodes, journal, history,
                    digest, records, poisoned: false, pending_write: false,
                    #[cfg(test)] fault: None,
                }),
            }
        }

        pub(super) fn snapshot(&self) -> Observation {
            Observation { keys: self.nodes.len(), records: self.records, bytes: self.journal.len(),
                poisoned: self.poisoned, pending_write: self.pending_write }
        }

        pub(super) fn node(&self, i: usize) -> Node {
            Node { epoch: Arc::downgrade(&self.epoch), record: Arc::clone(&self.nodes[i]) }
        }

        pub(super) fn validate(&mut self) -> Result<(), Error> {
            if self.poisoned { return Err(Error::new(ErrorKind::Poisoned, "hive retained after failure")); }
            if self.pending_write || self.history.last().is_some_and(|intent| !intent.committed) {
                self.poisoned = true;
                return Err(Error::new(ErrorKind::Poisoned, "unconfirmed intent retained"));
            }
            if let Err(error) = self.resources.compare(&self.journal, &[]) {
                self.poisoned = true;
                return Err(error);
            }
            Ok(())
        }

        pub(super) fn create(&mut self, parent: &Node, relative: &[u8]) -> Result<Node, Error> {
            if self.poisoned { return Err(Error::new(ErrorKind::Poisoned, "hive retained after failure")); }
            let live = parent.epoch.upgrade().ok_or_else(|| unsupported("expired node epoch"))?;
            if !Arc::ptr_eq(&live, &self.epoch)
                || !self.nodes.iter().any(|n| Arc::ptr_eq(n, &parent.record))
            { return Err(unsupported("foreign node; equal paths do not authorize access")); }
            let canonical = canonical_child(parent.canonical_ascii(), relative)?;
            let mut missing = Vec::new(); reserve(&mut missing, 32)?;
            for end in component_ends(&canonical) {
                let path = &canonical[..end];
                if !self.nodes.iter().any(|n| n.canonical.as_ref() == path) {
                    if missing.len() == 32 { return Err(unsupported("more than 32 missing levels")); }
                    let mut name = Vec::new(); reserve(&mut name, path.len())?;
                    name.extend_from_slice(path);
                    missing.push(Arc::new(KeyRecord { canonical: name.into_boxed_slice() }));
                }
            }
            if self.nodes.len() + missing.len() > self.limits.max_keys
                || (!missing.is_empty() && self.records == self.limits.max_records)
            { return Err(unsupported("key or monotonic journal-record budget exhausted")); }
            if missing.is_empty() {
                self.validate()?;
                let i = self.nodes.iter().position(|n| n.canonical.as_ref() == canonical.as_slice()).expect("interned path");
                return Ok(self.node(i));
            }
            let frame = encode(self.records + 1, &missing, self.digest)?;
            if self.journal.len() + frame.len() > self.limits.max_bytes {
                return Err(unsupported("journal-byte budget exhausted"));
            }
            // capacities for tree/history/full journal were reserved in begin;
            // all new names, real node records and frame bytes now exist as well.
            self.validate()?;
            self.history.push(Intent { frame, nodes: missing, committed: false });
            self.pending_write = true; // one monotonic, retained write intent
            let intent = self.history.last().expect("reserved intent");
            #[cfg(test)]
            let fault = self.fault.take();
            #[cfg(not(test))]
            let fault = None;
            let effect = append_once(self.resources.file(), &intent.frame, fault)
                .and_then(|()| self.resources.compare(&self.journal, &intent.frame));
            if let Err(error) = effect {
                self.poisoned = true;
                // Keep descriptors, uncertain frame and unpublished real nodes.
                // No retry, repair, replay, truncation, reopen or guessed counts.
                return Err(error);
            }
            let intent = self.history.last_mut().expect("retained intent");
            self.journal.extend_from_slice(&intent.frame);
            self.nodes.extend(intent.nodes.iter().cloned());
            self.digest.copy_from_slice(&intent.frame[intent.frame.len() - 32..]);
            intent.committed = true;
            self.records += 1;
            self.pending_write = false;
            Ok(self.node(self.nodes.len() - 1))
        }
    }

    fn valid_path(path: &[u8]) -> bool {
        !path.is_empty() && path.len() <= MAX_RELATIVE
            && path.iter().all(|b| (0x20..=0x7e).contains(b))
            && path.split(|b| *b == b'\\').all(|part| !part.is_empty())
            && path.split(|b| *b == b'\\').count() <= 512
    }

    fn canonical_child(parent: &[u8], relative: &[u8]) -> Result<Vec<u8>, Error> {
        if !valid_path(relative) { return Err(unsupported("unsupported ASCII subkey path")); }
        let len = parent.len() + usize::from(!parent.is_empty()) + relative.len();
        if len > MAX_RELATIVE { return Err(unsupported("absolute registry name exceeds 255 bytes")); }
        let mut path = Vec::new(); reserve(&mut path, len)?;
        path.extend_from_slice(parent);
        if !parent.is_empty() { path.push(b'\\'); }
        path.extend(relative.iter().map(u8::to_ascii_lowercase));
        if !valid_path(&path) { return Err(unsupported("registry depth exceeds profile")); }
        Ok(path)
    }

    fn component_ends(path: &[u8]) -> impl Iterator<Item = usize> + '_ {
        path.iter().enumerate().filter_map(|(i, b)| (*b == b'\\').then_some(i)).chain(std::iter::once(path.len()))
    }

    fn digest(frame: &[u8], previous: [u8; 32]) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(HEADER); hash.update(previous); hash.update(frame);
        hash.finalize().into()
    }

    fn encode(sequence: usize, nodes: &[Arc<KeyRecord>], previous: [u8; 32]) -> Result<Vec<u8>, Error> {
        let len = FRAME_FIXED + nodes.iter().map(|n| 2 + n.canonical.len()).sum::<usize>();
        if nodes.is_empty() || nodes.len() > 32 || len > FRAME_MAX { return Err(unsupported("frame outside profile")); }
        let mut frame = Vec::new(); reserve(&mut frame, len)?;
        frame.extend_from_slice(&(len as u32).to_le_bytes());
        frame.extend_from_slice(&(sequence as u32).to_le_bytes());
        frame.extend_from_slice(&(nodes.len() as u16).to_le_bytes());
        for node in nodes {
            frame.extend_from_slice(&(node.canonical.len() as u16).to_le_bytes());
            frame.extend_from_slice(&node.canonical);
        }
        let hash = digest(&frame, previous);
        frame.extend_from_slice(&hash);
        Ok(frame)
    }

    fn decode(bytes: &[u8], limits: Limits, nodes: &mut Vec<Arc<KeyRecord>>, history: &mut Vec<Intent>)
        -> Result<(usize, [u8; 32]), Error>
    {
        if bytes.len() > limits.max_bytes || !bytes.starts_with(HEADER) {
            return Err(unavailable("invalid version/header or byte budget"));
        }
        let mut offset = HEADER.len();
        let mut records = 0;
        let mut previous: [u8; 32] = Sha256::digest(HEADER).into();
        while offset < bytes.len() {
            let tail = &bytes[offset..];
            if tail.len() < FRAME_FIXED { return Err(unavailable("incomplete trailing frame")); }
            let size = u32::from_le_bytes(tail[..4].try_into().expect("four bytes")) as usize;
            if !(FRAME_FIXED..=FRAME_MAX).contains(&size) || size > tail.len() {
                return Err(unavailable("invalid bounded frame length"));
            }
            let frame = &tail[..size];
            let sequence = u32::from_le_bytes(frame[4..8].try_into().expect("four bytes")) as usize;
            let count = u16::from_le_bytes(frame[8..10].try_into().expect("two bytes")) as usize;
            if sequence != records + 1 || records == limits.max_records || !(1..=32).contains(&count)
                || nodes.len() + count > limits.max_keys
            { return Err(unavailable("frame chronology/count/budget invalid")); }
            let body_end = size - 32;
            let hash = digest(&frame[..body_end], previous);
            if frame[body_end..] != hash { return Err(unavailable("journal frame checksum invalid")); }
            let mut pending: Vec<Arc<KeyRecord>> = Vec::new(); reserve(&mut pending, count)?;
            let mut cursor = 10;
            for _ in 0..count {
                if cursor + 2 > body_end { return Err(unavailable("missing path length")); }
                let len = u16::from_le_bytes(frame[cursor..cursor + 2].try_into().expect("two bytes")) as usize;
                cursor += 2;
                if len > body_end - cursor { return Err(unavailable("incomplete path payload")); }
                let path = &frame[cursor..cursor + len]; cursor += len;
                if !valid_path(path) || path.iter().any(u8::is_ascii_uppercase)
                    || nodes.iter().chain(pending.iter()).any(|n| n.canonical.as_ref() == path)
                { return Err(unavailable("noncanonical or duplicate journal key")); }
                let parent = path.iter().rposition(|b| *b == b'\\').map_or(&b""[..], |i| &path[..i]);
                let prefix_exists = if let Some(last) = pending.last() { last.canonical.as_ref() == parent }
                    else { nodes.iter().any(|n| n.canonical.as_ref() == parent) };
                if !prefix_exists { return Err(unavailable("missing or nonconsecutive parent key")); }
                let mut name = Vec::new(); reserve(&mut name, len)?; name.extend_from_slice(path);
                pending.push(Arc::new(KeyRecord { canonical: name.into_boxed_slice() }));
            }
            if cursor != body_end { return Err(unavailable("trailing frame payload")); }
            let mut retained = Vec::new(); reserve(&mut retained, frame.len())?;
            retained.extend_from_slice(frame);
            nodes.extend(pending.iter().cloned());
            history.push(Intent { frame: retained, nodes: pending, committed: true });
            previous = hash; records += 1; offset += size;
        }
        Ok((records, previous))
    }

    // Fault values and entry points are absent in production. They only inject
    // finite failures after real writes/syncs; no hook can fabricate success.
    #[cfg(test)]
    #[derive(Debug, Clone, Copy)]
    pub(super) enum Fault { Partial(usize), SyncLoss }
    #[cfg(not(test))]
    enum Fault {}

    #[cfg(test)]
    std::thread_local! { static CONSTRUCTOR_FAULT: std::cell::Cell<Option<Fault>> = const { std::cell::Cell::new(None) }; }
    fn constructor_fault() -> Option<Fault> {
        #[cfg(test)] { CONSTRUCTOR_FAULT.with(|slot| slot.take()) }
        #[cfg(not(test))] { None }
    }

    fn append_once(mut file: &File, bytes: &[u8], fault: Option<Fault>) -> Result<(), Error> {
        #[cfg(test)]
        if let Some(Fault::Partial(n)) = fault {
            let prefix = &bytes[..n.min(bytes.len().saturating_sub(1))];
            if !prefix.is_empty() {
                let _written = file.write(prefix).map_err(|e| Error::io("partial append fault write", e))?;
            }
            return Err(Error::io("injected partial append", io::Error::other("finite test fault")));
        }
        let n = file.write(bytes).map_err(|e| Error::io("single journal append", e))?;
        if n != bytes.len() { return Err(unavailable("short append; uncertain intent retained")); }
        file.sync_all().map_err(|e| Error::io("journal sync acknowledgement", e))?;
        #[cfg(test)]
        if matches!(fault, Some(Fault::SyncLoss)) {
            return Err(Error::io("injected sync acknowledgement loss", io::Error::other("finite test fault")));
        }
        #[cfg(not(test))] let _ = fault;
        Ok(())
    }

    #[cfg(test)]
    impl Hive {
        pub(super) fn inject_once(&mut self, fault: Fault) { self.state.fault = Some(fault); }
        pub(super) fn inject_constructor_once(fault: Fault) { CONSTRUCTOR_FAULT.with(|s| s.set(Some(fault))); }
        pub(super) fn retained_pending_keys(&self) -> usize {
            self.state.history.iter().filter(|i| !i.committed).map(|i| i.nodes.len()).sum()
        }
    }
}

// Parent integration controls share only these two fixed failure outcomes.
// This seam is absent from every production build and cannot return success.
#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
impl Hive {
    pub(crate) fn inject_parent_test_write_failure_once(&mut self, lost_sync: bool) {
        self.inject_once(if lost_sync { host::Fault::SyncLoss } else { host::Fault::Partial(7) });
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
#[path = "program_registry_store/tests.rs"]
mod tests;
