//! One explicitly bound Unix authority, with a serialized in-memory namespace.
//!
//! Rust-port access policy: the authenticated creator UID owns a named mutex;
//! only that UID may open it. A denied create is an actual broker API-null
//! (`Rejected`, code 5). This is not Windows default-DACL equivalence. Transport,
//! unsupported profiles and host quotas never become that normal null outcome.
//! Create uses initial_owner=false: references do not acquire/release a mutex.
//!
//! Supported names are nonempty, NUL-free ASCII shorter than MAX_PATH (260
//! bytes including any prefix). Global\\ and Local\\ are exact byte prefixes;
//! the suffix is preserved, including case. Empty, non-ASCII, embedded-NUL and
//! longer ANSI profiles remain unavailable. A remaining backslash is invalid
//! name syntax after a recognized scope prefix. Unknown/private namespace
//! prefixes remain unavailable. Primary naming evidence:
//! https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexa
//! https://learn.microsoft.com/en-us/windows/win32/termserv/kernel-object-namespaces
//! https://learn.microsoft.com/en-us/windows/win32/sync/object-namespaces
//!
//! Quotas bound connections, threads, objects, handles, ledger entries and all
//! variable-length retained bytes. Accounting includes conservative node charges;
//! fixed allocator/thread overhead is additionally bounded by those counts.
//! Ledger exhaustion sends a terminal Unavailable and retires the connection so
//! an unrecordable operation can never be retried on a still-live connection.

#![cfg(unix)]

use crate::auth::{Peer, current_uid, peer_identity};
use crate::protocol::{Reply, Request, read_request, write_reply};
use std::collections::BTreeMap;
use std::fs::{self, File, Permissions};
use std::io::{self, Read};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const ACCESS_DENIED: u32 = 5;
const INVALID_HANDLE: u32 = 6;
const INVALID_NAME: u32 = 123;

// Codes belong to Reply::Unavailable, not the API rejection/error-code domain.
pub(crate) const UNAVAILABLE_ENCODING: u32 = 1;
pub(crate) const UNAVAILABLE_SCOPE: u32 = 2;
pub(crate) const UNAVAILABLE_CAPACITY: u32 = 3;
pub(crate) const UNAVAILABLE_IDENTITIES: u32 = 4;
pub(crate) const UNAVAILABLE_LEDGER: u32 = 5;
pub(crate) const UNAVAILABLE_PROFILE: u32 = 6;
const UNAVAILABLE_STATE: u32 = 7;
const CONNECTION_BYTES: usize = 1024;
const OBJECT_BYTES: usize = 256;
const HANDLE_BYTES: usize = 128;
const LEDGER_BYTES: usize = 384;

#[derive(Clone, Copy)]
struct Limits {
    connections: usize,
    objects: usize,
    handles: usize,
    connection_handles: usize,
    operations: usize,
    retained_bytes: usize,
    connection_ledger_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            connections: 64,
            objects: 4096,
            handles: 16384,
            connection_handles: 1024,
            operations: 4096,
            retained_bytes: 32 * 1024 * 1024,
            connection_ledger_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    Global,
    Session(u32),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Name {
    scope: Scope,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObjectKind {
    Mutex,
}

struct Object {
    identity: u64,
    owner: u32,
    kind: ObjectKind,
    references: usize,
}

enum NameError {
    Rejected(u32),
    Unavailable(u32),
}

fn name_key(bytes: &[u8], peer: Peer) -> Result<Name, NameError> {
    if !bytes.is_ascii() {
        return Err(NameError::Unavailable(UNAVAILABLE_ENCODING));
    }
    if bytes.is_empty() || bytes.contains(&0) || bytes.len() >= 260 {
        return Err(NameError::Unavailable(UNAVAILABLE_PROFILE));
    }
    let (global, remainder) = if let Some(name) = bytes.strip_prefix(b"Global\\") {
        (true, name)
    } else if let Some(name) = bytes.strip_prefix(b"Local\\") {
        (false, name)
    } else {
        // A case-changed prefix is not Global/Local. Windows also has private
        // namespace aliases of the form prefix\\name; this authority has no
        // private-namespace profile and cannot invent its API-null semantics.
        if bytes.contains(&b'\\') {
            return Err(NameError::Unavailable(UNAVAILABLE_SCOPE));
        }
        (false, bytes)
    };
    if remainder.is_empty() {
        return Err(NameError::Unavailable(UNAVAILABLE_PROFILE));
    }
    if remainder.contains(&b'\\') {
        return Err(NameError::Rejected(INVALID_NAME));
    }
    let scope = if global {
        Scope::Global
    } else {
        Scope::Session(
            peer.session
                .ok_or(NameError::Unavailable(UNAVAILABLE_SCOPE))?,
        )
    };
    Ok(Name {
        scope,
        bytes: remainder.to_vec(),
    })
}

struct Sequence {
    next: Option<u64>,
}
impl Sequence {
    fn new() -> Self {
        Self { next: Some(1) }
    }
    fn take(&mut self) -> Option<u64> {
        let value = self.next?;
        self.next = value.checked_add(1);
        Some(value)
    }
}

struct Completed {
    request: Request,
    reply: Reply,
}
struct Connection {
    peer: Peer,
    greeted: bool,
    handles: BTreeMap<u64, Arc<Name>>,
    ledger: BTreeMap<u64, Completed>,
    ledger_bytes: usize,
}

struct Registry {
    epoch: [u8; 16],
    limits: Limits,
    next_connection: Sequence,
    next_object: Sequence,
    next_handle: Sequence,
    connections: BTreeMap<u64, Connection>,
    objects: BTreeMap<Arc<Name>, Object>,
    handles: usize,
    retained_bytes: usize,
}

struct Dispatch {
    reply: Reply,
    retire: bool,
}

fn protocol_error(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

impl Registry {
    fn new(epoch: [u8; 16], limits: Limits) -> Self {
        Self {
            epoch,
            limits,
            next_connection: Sequence::new(),
            next_object: Sequence::new(),
            next_handle: Sequence::new(),
            connections: BTreeMap::new(),
            objects: BTreeMap::new(),
            handles: 0,
            retained_bytes: 0,
        }
    }

    fn room(&self, bytes: usize) -> bool {
        self.retained_bytes
            .checked_add(bytes)
            .is_some_and(|n| n <= self.limits.retained_bytes)
    }

    fn connect(&mut self, peer: Peer) -> io::Result<u64> {
        if self.connections.len() >= self.limits.connections || !self.room(CONNECTION_BYTES) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "authority connection quota",
            ));
        }
        let id = self
            .next_connection
            .take()
            .ok_or_else(|| io::Error::other("connection identities exhausted"))?;
        self.connections.insert(
            id,
            Connection {
                peer,
                greeted: false,
                handles: BTreeMap::new(),
                ledger: BTreeMap::new(),
                ledger_bytes: 0,
            },
        );
        self.retained_bytes += CONNECTION_BYTES;
        Ok(id)
    }

    fn dispatch(&mut self, id: u64, request: Request) -> io::Result<Dispatch> {
        let connection = self
            .connections
            .get(&id)
            .ok_or_else(|| protocol_error("retired connection"))?;
        if matches!(request, Request::Hello) {
            if connection.greeted {
                return Err(protocol_error("Hello repeated"));
            }
            let peer = connection.peer;
            self.connections
                .get_mut(&id)
                .ok_or_else(|| protocol_error("retired connection"))?
                .greeted = true;
            return Ok(Dispatch {
                reply: Reply::Hello {
                    epoch: self.epoch,
                    connection: id,
                    uid: peer.uid,
                    session: peer.session,
                },
                retire: false,
            });
        }
        if !connection.greeted {
            return Err(protocol_error("Hello must be first"));
        }
        let op = match &request {
            Request::Create { op, .. } | Request::Close { op, .. } | Request::Check { op, .. } => {
                *op
            }
            Request::Hello => return Err(protocol_error("unexpected Hello")),
        };
        if op == 0 {
            return Err(protocol_error("zero operation"));
        }
        if let Some(completed) = connection.ledger.get(&op) {
            if completed.request != request {
                return Err(protocol_error("operation payload conflict"));
            }
            return Ok(Dispatch {
                reply: completed.reply.clone(),
                retire: false,
            });
        }
        // IDs are reserved monotonically by the client, but staged operations
        // can resolve in another order. Keep every completed ID (no eviction)
        // rather than confusing arrival order with allocation order.
        let name_bytes = match &request {
            Request::Create { name, .. } => name.len(),
            _ => 0,
        };
        // The codec enforces this as well; keep the state machine independently bounded.
        if name_bytes > crate::protocol::MAX_NAME_BYTES {
            return Err(protocol_error("name wire bound"));
        }
        let charge = LEDGER_BYTES + name_bytes;
        if connection.ledger.len() >= self.limits.operations
            || !connection
                .ledger_bytes
                .checked_add(charge)
                .is_some_and(|n| n <= self.limits.connection_ledger_bytes)
            || !self.room(charge)
        {
            // No effects, no forgotten replayable reply. The handler closes this
            // connection immediately after writing this terminal response.
            return Ok(Dispatch {
                reply: Reply::Unavailable {
                    op,
                    code: UNAVAILABLE_LEDGER,
                },
                retire: true,
            });
        }
        self.retained_bytes += charge;
        let connection = self
            .connections
            .get_mut(&id)
            .ok_or_else(|| protocol_error("retired connection"))?;
        connection.ledger_bytes += charge;
        let reply = match &request {
            Request::Create { name, .. } => self.create(id, op, name),
            Request::Close { handle, .. } => self.close(id, op, *handle),
            Request::Check { handle, .. } => self.check(id, op, *handle),
            Request::Hello => return Err(protocol_error("unexpected Hello")),
        };
        let connection = self
            .connections
            .get_mut(&id)
            .ok_or_else(|| protocol_error("retired connection"))?;
        connection.ledger.insert(
            op,
            Completed {
                request,
                reply: reply.clone(),
            },
        );
        Ok(Dispatch {
            reply,
            retire: false,
        })
    }

    fn create(&mut self, id: u64, op: u64, bytes: &[u8]) -> Reply {
        let Some(connection) = self.connections.get(&id) else {
            return Reply::Unavailable {
                op,
                code: UNAVAILABLE_STATE,
            };
        };
        let peer = connection.peer;
        let key = match name_key(bytes, peer) {
            Ok(key) => Arc::new(key),
            Err(NameError::Rejected(code)) => return Reply::Rejected { op, code },
            Err(NameError::Unavailable(code)) => return Reply::Unavailable { op, code },
        };
        let existing = self.objects.get_key_value(&key);
        if let Some((_, object)) = existing {
            // Access is tested in the one shared namespace, before issuing a
            // handle. Another UID never gets a second same-name object.
            if object.owner != peer.uid {
                return Reply::Rejected {
                    op,
                    code: ACCESS_DENIED,
                };
            }
            if object.kind != ObjectKind::Mutex {
                return Reply::Rejected {
                    op,
                    code: INVALID_HANDLE,
                };
            }
        }
        let is_new = existing.is_none();
        let charge = HANDLE_BYTES
            + if is_new {
                OBJECT_BYTES + key.bytes.len()
            } else {
                0
            };
        if self.handles >= self.limits.handles
            || connection.handles.len() >= self.limits.connection_handles
            || (is_new && self.objects.len() >= self.limits.objects)
            || !self.room(charge)
        {
            return Reply::Unavailable {
                op,
                code: UNAVAILABLE_CAPACITY,
            };
        }
        if self.next_handle.next.is_none() || (is_new && self.next_object.next.is_none()) {
            return Reply::Unavailable {
                op,
                code: UNAVAILABLE_IDENTITIES,
            };
        }
        let key = existing.map(|(key, _)| Arc::clone(key)).unwrap_or(key);
        // All capacity and ID checks precede either insertion. Sequence values
        // can finish at u64::MAX, but never wrap or become reusable.
        let Some(handle) = self.next_handle.take() else {
            return Reply::Unavailable {
                op,
                code: UNAVAILABLE_IDENTITIES,
            };
        };
        let object = if is_new {
            let Some(identity) = self.next_object.take() else {
                return Reply::Unavailable {
                    op,
                    code: UNAVAILABLE_IDENTITIES,
                };
            };
            self.objects.insert(
                Arc::clone(&key),
                Object {
                    identity,
                    owner: peer.uid,
                    kind: ObjectKind::Mutex,
                    references: 1,
                },
            );
            identity
        } else {
            let Some(object) = self.objects.get_mut(&key) else {
                return Reply::Unavailable {
                    op,
                    code: UNAVAILABLE_STATE,
                };
            };
            object.references += 1; // bounded by the checked total handle quota
            object.identity
        };
        let Some(connection) = self.connections.get_mut(&id) else {
            return Reply::Unavailable {
                op,
                code: UNAVAILABLE_STATE,
            };
        };
        connection.handles.insert(handle, key);
        self.handles += 1;
        self.retained_bytes += charge;
        Reply::Created { op, handle, object }
    }

    fn check(&self, id: u64, op: u64, handle: u64) -> Reply {
        let key = self
            .connections
            .get(&id)
            .and_then(|c| c.handles.get(&handle));
        match key {
            None => Reply::Rejected {
                op,
                code: INVALID_HANDLE,
            },
            Some(key) => match self.objects.get(key) {
                Some(object) => Reply::Live {
                    op,
                    object: object.identity,
                },
                None => Reply::Unavailable {
                    op,
                    code: UNAVAILABLE_STATE,
                },
            },
        }
    }

    fn close(&mut self, id: u64, op: u64, handle: u64) -> Reply {
        let key = self
            .connections
            .get_mut(&id)
            .and_then(|c| c.handles.remove(&handle));
        let Some(key) = key else {
            return Reply::Rejected {
                op,
                code: INVALID_HANDLE,
            };
        };
        let existed = self.objects.contains_key(&key);
        self.release(key);
        if existed {
            Reply::Closed { op }
        } else {
            Reply::Unavailable {
                op,
                code: UNAVAILABLE_STATE,
            }
        }
    }

    fn release(&mut self, key: Arc<Name>) {
        self.handles -= 1;
        self.retained_bytes -= HANDLE_BYTES;
        if let Some(object) = self.objects.get_mut(&key) {
            if object.references > 1 {
                object.references -= 1;
            } else {
                self.objects.remove(&key);
                self.retained_bytes -= OBJECT_BYTES + key.bytes.len();
            }
        }
    }

    fn disconnect(&mut self, id: u64) {
        if let Some(connection) = self.connections.remove(&id) {
            for (_, key) in connection.handles {
                self.release(key);
            }
            self.retained_bytes -= CONNECTION_BYTES + connection.ledger_bytes;
            // Completed outcomes are history, and are never live references.
        }
    }

    fn summary(&self) -> ServerSummary {
        ServerSummary {
            objects_remaining: self.objects.len(),
            handles_remaining: self.handles,
            connections_remaining: self.connections.len(),
        }
    }
}

fn lock_registry(registry: &Mutex<Registry>) -> MutexGuard<'_, Registry> {
    // Handler unwinding must still release its records. Operations contain no
    // user callbacks and maintain quotas before effects; poison is not an excuse
    // to skip connection EOF/unwind cleanup.
    registry.lock().unwrap_or_else(|poison| poison.into_inner())
}

struct ConnectionGuard {
    registry: Arc<Mutex<Registry>>,
    id: u64,
}
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        lock_registry(&self.registry).disconnect(self.id);
    }
}

fn serve(mut stream: UnixStream, guard: ConnectionGuard) {
    let _guard = guard;
    // Accepted nonblocking inheritance varies by OS. Reads are deliberately
    // blocking without timeouts: a partial frame is never retried as a new one.
    // A failed/partial write closes the entire descriptor instead of resuming IO.
    if stream.set_nonblocking(false).is_err()
        || stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .is_err()
    {
        return;
    }
    loop {
        let request = match read_request(&mut stream) {
            Ok(Some(request)) => request,
            _ => return,
        };
        let outcome = lock_registry(&_guard.registry).dispatch(_guard.id, request);
        let Ok(outcome) = outcome else { return };
        if write_reply(&mut stream, &outcome.reply).is_err() || outcome.retire {
            return;
        }
    }
}

fn random_epoch() -> io::Result<[u8; 16]> {
    let mut epoch = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut epoch)?;
    Ok(epoch)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}
impl FileIdentity {
    fn from_metadata(meta: &fs::Metadata) -> Self {
        Self {
            device: meta.dev(),
            inode: meta.ino(),
        }
    }
}

struct OwnedSocket {
    path: PathBuf,
    ancestors: crate::endpoint::Ancestors,
    parent: FileIdentity,
    socket: FileIdentity,
    armed: bool,
}
impl OwnedSocket {
    fn matches(&self) -> io::Result<bool> {
        if !self.ancestors.matches()? {
            return Ok(false);
        }
        let Some(parent) = self.path.parent() else {
            return Ok(false);
        };
        let parent_meta = fs::symlink_metadata(parent)?;
        if !parent_meta.is_dir()
            || parent_meta.uid() != current_uid()
            || parent_meta.mode() & 0o022 != 0
            || FileIdentity::from_metadata(&parent_meta) != self.parent
        {
            return Ok(false);
        }
        match fs::symlink_metadata(&self.path) {
            Ok(meta) => Ok(meta.file_type().is_socket()
                && meta.nlink() == 1
                && FileIdentity::from_metadata(&meta) == self.socket),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn cleanup(&mut self) -> io::Result<()> {
        if !self.armed {
            return Ok(());
        }
        self.armed = false;
        if self.matches()? {
            fs::remove_file(&self.path)
        } else {
            match fs::symlink_metadata(&self.path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
                Ok(_) => Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "owned socket path was replaced; replacement retained",
                )),
            }
        }
    }
}
impl Drop for OwnedSocket {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

// Joining in Drop also keeps owned socket cleanup behind real handler teardown
// if the run loop unwinds. Never detach a handler that can still hold a lease.
struct Handlers(Vec<JoinHandle<()>>);
impl Handlers {
    fn reap(&mut self) -> bool {
        let mut panicked = false;
        let mut index = 0;
        while index < self.0.len() {
            if self.0[index].is_finished() {
                panicked |= self.0.swap_remove(index).join().is_err();
            } else {
                index += 1;
            }
        }
        panicked
    }
    fn drain(&mut self) -> bool {
        let mut panicked = false;
        while let Some(handle) = self.0.pop() {
            panicked |= handle.join().is_err();
        }
        panicked
    }
}
impl Drop for Handlers {
    fn drop(&mut self) {
        self.drain();
    }
}

/// A single explicit authority endpoint; binding never adopts/unlinks an old path.
pub struct Server {
    listener: UnixListener,
    owned: OwnedSocket,
    epoch: [u8; 16],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerSummary {
    pub objects_remaining: usize,
    pub handles_remaining: usize,
    pub connections_remaining: usize,
}

impl Server {
    pub fn bind(path: &Path, socket_mode: u32) -> io::Result<Self> {
        if !matches!(socket_mode, 0o600 | 0o666) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "socket mode must be explicitly 0600 or 0666",
            ));
        }
        let filename = path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "socket filename required")
        })?;
        if !matches!(
            Path::new(filename).components().next(),
            Some(Component::Normal(_))
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "socket filename required",
            ));
        }
        let ancestors = crate::endpoint::Ancestors::capture(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
            current_uid(),
        )?;
        let parent = ancestors.path().to_path_buf();
        let parent_meta = fs::symlink_metadata(&parent)?;
        // Other UIDs may traverse/read this directory for an explicit 0666
        // authority, but may not replace its endpoint. Same-UID filesystem
        // adversaries are outside this Rust-port deployment security policy.
        if !parent_meta.is_dir()
            || parent_meta.uid() != current_uid()
            || parent_meta.mode() & 0o022 != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "socket parent must be owned by broker UID and not group/other writable",
            ));
        }
        let path = parent.join(filename);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "socket path already exists",
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        let epoch = random_epoch()?;
        let listener = UnixListener::bind(&path)?;
        let meta = fs::symlink_metadata(&path)?;
        if !meta.file_type().is_socket() || meta.nlink() != 1 || meta.uid() != current_uid() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "new socket identity unavailable",
            ));
        }
        let owned = OwnedSocket {
            path,
            ancestors,
            parent: FileIdentity::from_metadata(&parent_meta),
            socket: FileIdentity::from_metadata(&meta),
            armed: true,
        };
        if !owned.matches()? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "socket parent/path replaced during bind",
            ));
        }
        fs::set_permissions(&owned.path, Permissions::from_mode(socket_mode))?;
        if !owned.matches()? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "socket path replaced during chmod",
            ));
        }
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            owned,
            epoch,
        })
    }

    pub fn epoch(&self) -> [u8; 16] {
        self.epoch
    }

    /// Stop accepting on `stop`, then retain the host/socket until every real
    /// connection ends. This intentionally can wait indefinitely for a client
    /// holding a descriptor; no quit-time synthetic close/reclamation occurs.
    pub fn run(self, stop: Arc<AtomicBool>) -> io::Result<ServerSummary> {
        let Self {
            listener,
            mut owned,
            epoch,
        } = self;
        let limits = Limits::default();
        let registry = Arc::new(Mutex::new(Registry::new(epoch, limits)));
        let mut handlers = Handlers(Vec::with_capacity(limits.connections));
        let mut failure = None;
        while !stop.load(Ordering::Acquire) {
            if handlers.reap() {
                failure = Some(io::Error::other("authority handler panicked"));
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    if stop.load(Ordering::Acquire) {
                        drop(stream);
                        break;
                    }
                    // At admission exhaustion there is no Hello/op yet; closing
                    // transport is unavailable, never a fabricated API-null.
                    if handlers.0.len() >= limits.connections {
                        drop(stream);
                        continue;
                    }
                    let peer = match peer_identity(&stream) {
                        Ok(peer) => peer,
                        Err(_) => continue,
                    };
                    let id = match lock_registry(&registry).connect(peer) {
                        Ok(id) => id,
                        Err(_) => continue,
                    };
                    let guard = ConnectionGuard {
                        registry: Arc::clone(&registry),
                        id,
                    };
                    match thread::Builder::new()
                        .name(format!("sims-kernel-{id}"))
                        .stack_size(256 * 1024)
                        .spawn(move || serve(stream, guard))
                    {
                        Ok(handler) => handlers.0.push(handler),
                        Err(e) => {
                            // The failed spawn drops its captured guard; this
                            // idempotent removal also makes admission cleanup explicit.
                            lock_registry(&registry).disconnect(id);
                            failure = Some(e);
                            break;
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => (),
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        drop(listener);
        if handlers.drain() && failure.is_none() {
            failure = Some(io::Error::other("authority handler panicked"));
        }
        let summary = lock_registry(&registry).summary();
        let cleanup = owned.cleanup();
        if let Some(error) = failure {
            return Err(error);
        }
        cleanup?;
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{read_reply, write_request};
    use std::io::Write;
    use std::net::Shutdown;

    fn registry() -> Registry {
        Registry::new([7; 16], Limits::default())
    }
    fn peer(uid: u32, session: u32) -> Peer {
        Peer {
            uid,
            session: Some(session),
        }
    }
    fn connect(registry: &mut Registry, peer: Peer) -> u64 {
        let id = registry.connect(peer).unwrap();
        assert_eq!(
            registry.dispatch(id, Request::Hello).unwrap().reply,
            Reply::Hello {
                epoch: [7; 16],
                connection: id,
                uid: peer.uid,
                session: peer.session
            }
        );
        id
    }
    fn request(registry: &mut Registry, id: u64, value: Request) -> Reply {
        let dispatch = registry.dispatch(id, value).unwrap();
        assert!(!dispatch.retire);
        dispatch.reply
    }
    fn create(registry: &mut Registry, id: u64, op: u64, name: &[u8]) -> (u64, u64) {
        match request(
            registry,
            id,
            Request::Create {
                op,
                name: name.to_vec(),
            },
        ) {
            Reply::Created { handle, object, .. } => (handle, object),
            reply => panic!("unexpected create: {reply:?}"),
        }
    }
    fn empty(registry: &Registry) {
        assert_eq!(
            registry.summary(),
            ServerSummary {
                objects_remaining: 0,
                handles_remaining: 0,
                connections_remaining: 0
            }
        );
        assert_eq!(registry.retained_bytes, 0);
    }

    #[test]
    fn exact_names_local_alias_session_isolation_and_cross_session_global() {
        let mut registry = registry();
        let a = connect(&mut registry, peer(7, 10));
        let b = connect(&mut registry, peer(7, 11));
        let c = connect(&mut registry, peer(7, 10));
        let (_, local) = create(&mut registry, a, 1, b"MiXeD");
        assert_eq!(create(&mut registry, c, 1, b"Local\\MiXeD").1, local);
        assert_ne!(create(&mut registry, b, 1, b"MiXeD").1, local);
        assert_ne!(create(&mut registry, a, 2, b"mixed").1, local);
        let (_, global) = create(&mut registry, a, 3, b"Global\\MiXeD");
        assert_ne!(global, local);
        assert_eq!(create(&mut registry, b, 2, b"Global\\MiXeD").1, global);
        assert_eq!(create(&mut registry, c, 2, b"Global\\MiXeD").1, global);
        assert_ne!(create(&mut registry, c, 3, b"Global\\mixed").1, global);
        assert_eq!(
            request(
                &mut registry,
                a,
                Request::Create {
                    op: 4,
                    name: b"global\\MiXeD".to_vec()
                }
            ),
            Reply::Unavailable {
                op: 4,
                code: UNAVAILABLE_SCOPE
            }
        );
        // These are exact object names, never filesystem paths or normalized strings.
        let x = create(&mut registry, a, 5, b"a/../b").1;
        assert_ne!(create(&mut registry, a, 6, b"b").1, x);
        assert_ne!(create(&mut registry, a, 7, b"A/../b").1, x);
        for id in [a, b, c] {
            registry.disconnect(id);
        }
        empty(&registry);
    }

    #[test]
    fn missing_session_keeps_global_available_and_other_profiles_unavailable() {
        let mut registry = registry();
        let a = connect(
            &mut registry,
            Peer {
                uid: 7,
                session: None,
            },
        );
        let b = connect(&mut registry, peer(7, 20));
        let (_, global) = create(&mut registry, a, 1, b"Global\\exact");
        assert_eq!(create(&mut registry, b, 1, b"Global\\exact").1, global);
        for (op, name, code) in [
            (2, b"exact".as_slice(), UNAVAILABLE_SCOPE),
            (3, b"Local\\exact".as_slice(), UNAVAILABLE_SCOPE),
            (4, b"".as_slice(), UNAVAILABLE_PROFILE),
            (5, b"Global\\".as_slice(), UNAVAILABLE_PROFILE),
            (6, b"Global\\a\0b".as_slice(), UNAVAILABLE_PROFILE),
            (7, b"Global\\\xff".as_slice(), UNAVAILABLE_ENCODING),
        ] {
            assert_eq!(
                request(
                    &mut registry,
                    a,
                    Request::Create {
                        op,
                        name: name.to_vec()
                    }
                ),
                Reply::Unavailable { op, code }
            );
        }
        assert_eq!(
            request(
                &mut registry,
                b,
                Request::Create {
                    op: 2,
                    name: b"Global\\a\\b".to_vec()
                }
            ),
            Reply::Rejected {
                op: 2,
                code: INVALID_NAME
            }
        );
        assert_eq!(
            request(
                &mut registry,
                b,
                Request::Create {
                    op: 3,
                    name: vec![b'a'; 260]
                }
            ),
            Reply::Unavailable {
                op: 3,
                code: UNAVAILABLE_PROFILE
            }
        );
        assert_eq!(registry.objects.len(), 1);
        registry.disconnect(a);
        registry.disconnect(b);
        empty(&registry);
    }

    #[test]
    fn owner_denial_is_actual_api_null_in_the_shared_global_namespace() {
        let mut registry = registry();
        let owner = connect(&mut registry, peer(7, 10));
        let other = connect(&mut registry, peer(8, 999));
        let (handle, original) = create(&mut registry, owner, 1, b"Global\\owned");
        let denied = Request::Create {
            op: 1,
            name: b"Global\\owned".to_vec(),
        };
        assert_eq!(
            request(&mut registry, other, denied.clone()),
            Reply::Rejected { op: 1, code: 5 }
        );
        assert_eq!(registry.objects.len(), 1);
        assert_eq!(registry.handles, 1);
        assert!(registry.connections[&other].handles.is_empty());
        assert_eq!(
            request(&mut registry, owner, Request::Close { op: 2, handle }),
            Reply::Closed { op: 2 }
        );
        // Replaying denial must not turn into a successful create after deletion.
        assert_eq!(
            request(&mut registry, other, denied),
            Reply::Rejected { op: 1, code: 5 }
        );
        let (_, recreated) = create(&mut registry, other, 2, b"Global\\owned");
        assert!(recreated > original);
        assert_eq!(
            request(
                &mut registry,
                owner,
                Request::Create {
                    op: 3,
                    name: b"Global\\owned".to_vec()
                }
            ),
            Reply::Rejected { op: 3, code: 5 }
        );
        registry.disconnect(owner);
        registry.disconnect(other);
        empty(&registry);
    }

    #[test]
    fn distinct_references_replay_close_final_close_and_new_generation() {
        let mut registry = registry();
        let a = connect(&mut registry, peer(7, 10));
        let first_request = Request::Create {
            op: 1,
            name: b"one".to_vec(),
        };
        let first = request(&mut registry, a, first_request.clone());
        let (h1, object) = match &first {
            Reply::Created { handle, object, .. } => (*handle, *object),
            _ => unreachable!(),
        };
        assert_eq!(request(&mut registry, a, first_request.clone()), first);
        assert_eq!(registry.handles, 1);
        let (h2, second_object) = create(&mut registry, a, 2, b"one");
        assert_ne!(h1, h2);
        assert_eq!(object, second_object);
        let close = Request::Close { op: 3, handle: h1 };
        assert_eq!(
            request(&mut registry, a, close.clone()),
            Reply::Closed { op: 3 }
        );
        assert_eq!(request(&mut registry, a, close), Reply::Closed { op: 3 });
        assert_eq!(registry.handles, 1);
        assert_eq!(
            request(&mut registry, a, Request::Check { op: 4, handle: h2 }),
            Reply::Live { op: 4, object }
        );
        assert_eq!(
            request(&mut registry, a, Request::Check { op: 5, handle: h1 }),
            Reply::Rejected { op: 5, code: 6 }
        );
        assert_eq!(
            request(&mut registry, a, Request::Close { op: 6, handle: h2 }),
            Reply::Closed { op: 6 }
        );
        assert!(registry.objects.is_empty());
        // Cached creation is immutable history and cannot reacquire a reference.
        assert_eq!(request(&mut registry, a, first_request), first);
        assert!(registry.objects.is_empty());
        assert_eq!(registry.handles, 0);
        assert_eq!(
            request(&mut registry, a, Request::Close { op: 7, handle: h1 }),
            Reply::Rejected { op: 7, code: 6 }
        );
        let (h3, newer) = create(&mut registry, a, 8, b"one");
        assert!(h3 > h2);
        assert!(newer > object);
        // A cached live result is also history; a fresh operation observes closure.
        assert_eq!(
            request(&mut registry, a, Request::Check { op: 4, handle: h2 }),
            Reply::Live { op: 4, object }
        );
        assert_eq!(
            request(&mut registry, a, Request::Check { op: 9, handle: h2 }),
            Reply::Rejected { op: 9, code: 6 }
        );
        registry.disconnect(a);
        empty(&registry);
    }

    #[test]
    fn handle_records_belong_to_their_actual_connections() {
        let mut registry = registry();
        let a = connect(&mut registry, peer(7, 10));
        let b = connect(&mut registry, peer(7, 10));
        let (h1, object) = create(&mut registry, a, 1, b"one");
        assert_eq!(
            request(&mut registry, b, Request::Check { op: 1, handle: h1 }),
            Reply::Rejected { op: 1, code: 6 }
        );
        assert_eq!(
            request(&mut registry, b, Request::Close { op: 2, handle: h1 }),
            Reply::Rejected { op: 2, code: 6 }
        );
        let (h2, same) = create(&mut registry, b, 3, b"one");
        assert_ne!(h1, h2);
        assert_eq!(object, same);
        registry.disconnect(a);
        assert_eq!(registry.handles, 1);
        assert_eq!(
            request(&mut registry, b, Request::Check { op: 4, handle: h2 }),
            Reply::Live { op: 4, object }
        );
        registry.disconnect(b);
        empty(&registry);
    }

    #[test]
    fn hello_is_strict_and_payload_conflicts_have_no_effects() {
        let mut registry = registry();
        let id = registry.connect(peer(7, 10)).unwrap();
        assert!(
            registry
                .dispatch(
                    id,
                    Request::Create {
                        op: 1,
                        name: b"one".to_vec()
                    }
                )
                .is_err()
        );
        registry.dispatch(id, Request::Hello).unwrap();
        assert!(registry.dispatch(id, Request::Hello).is_err());
        create(&mut registry, id, 2, b"one");
        assert!(
            registry
                .dispatch(
                    id,
                    Request::Create {
                        op: 2,
                        name: b"One".to_vec()
                    }
                )
                .is_err()
        );
        assert!(
            registry
                .dispatch(id, Request::Close { op: 2, handle: 1 })
                .is_err()
        );
        assert!(
            registry
                .dispatch(id, Request::Check { op: 0, handle: 1 })
                .is_err()
        );
        assert_eq!(registry.objects.len(), 1);
        assert_eq!(registry.handles, 1);
        registry.disconnect(id);
        empty(&registry);
    }

    #[test]
    fn staged_operation_ids_can_arrive_out_of_reservation_order() {
        let mut registry = registry();
        let id = connect(&mut registry, peer(7, 10));
        let (later, object) = create(&mut registry, id, 2, b"one");
        let (earlier, same) = create(&mut registry, id, 1, b"one");
        assert_ne!(earlier, later);
        assert_eq!(object, same);
        assert_eq!(
            request(
                &mut registry,
                id,
                Request::Create {
                    op: 2,
                    name: b"one".to_vec()
                }
            ),
            Reply::Created {
                op: 2,
                handle: later,
                object
            }
        );
        assert_eq!(registry.handles, 2);
        registry.disconnect(id);
        empty(&registry);
    }

    #[test]
    fn object_quota_and_cached_unavailable_never_fabricate_api_rejection() {
        let mut registry = Registry::new(
            [7; 16],
            Limits {
                objects: 1,
                ..Limits::default()
            },
        );
        let id = connect(&mut registry, peer(7, 10));
        let (handle, _) = create(&mut registry, id, 1, b"one");
        let blocked = Request::Create {
            op: 2,
            name: b"two".to_vec(),
        };
        assert_eq!(
            request(&mut registry, id, blocked.clone()),
            Reply::Unavailable {
                op: 2,
                code: UNAVAILABLE_CAPACITY
            }
        );
        assert_eq!(registry.handles, 1);
        request(&mut registry, id, Request::Close { op: 3, handle });
        assert_eq!(
            request(&mut registry, id, blocked),
            Reply::Unavailable {
                op: 2,
                code: UNAVAILABLE_CAPACITY
            }
        );
        create(&mut registry, id, 4, b"two");
        registry.disconnect(id);
        empty(&registry);
    }

    #[test]
    fn handle_connection_and_byte_capacity_are_bounded_before_effects() {
        for limits in [
            Limits {
                handles: 1,
                ..Limits::default()
            },
            Limits {
                connection_handles: 1,
                ..Limits::default()
            },
        ] {
            let mut registry = Registry::new([7; 16], limits);
            let id = connect(&mut registry, peer(7, 10));
            create(&mut registry, id, 1, b"one");
            assert_eq!(
                request(
                    &mut registry,
                    id,
                    Request::Create {
                        op: 2,
                        name: b"one".to_vec()
                    }
                ),
                Reply::Unavailable {
                    op: 2,
                    code: UNAVAILABLE_CAPACITY
                }
            );
            assert_eq!(registry.handles, 1);
            registry.disconnect(id);
            empty(&registry);
        }
        let mut registry = Registry::new(
            [7; 16],
            Limits {
                connections: 1,
                ..Limits::default()
            },
        );
        let id = connect(&mut registry, peer(7, 10));
        assert!(registry.connect(peer(7, 10)).is_err());
        registry.disconnect(id);
        empty(&registry);
        let mut registry = Registry::new(
            [7; 16],
            Limits {
                retained_bytes: CONNECTION_BYTES + LEDGER_BYTES + 1,
                ..Limits::default()
            },
        );
        let id = connect(&mut registry, peer(7, 10));
        assert_eq!(
            request(
                &mut registry,
                id,
                Request::Create {
                    op: 1,
                    name: b"a".to_vec()
                }
            ),
            Reply::Unavailable {
                op: 1,
                code: UNAVAILABLE_CAPACITY
            }
        );
        assert!(registry.objects.is_empty());
        assert_eq!(registry.handles, 0);
        registry.disconnect(id);
        empty(&registry);
    }

    #[test]
    fn operation_and_ledger_byte_exhaustion_require_transport_retirement() {
        for limits in [
            Limits {
                operations: 1,
                ..Limits::default()
            },
            Limits {
                connection_ledger_bytes: LEDGER_BYTES + 3,
                ..Limits::default()
            },
            Limits {
                retained_bytes: CONNECTION_BYTES
                    + LEDGER_BYTES
                    + 3
                    + HANDLE_BYTES
                    + OBJECT_BYTES
                    + 3,
                ..Limits::default()
            },
        ] {
            let mut registry = Registry::new([7; 16], limits);
            let id = connect(&mut registry, peer(7, 10));
            let first = Request::Create {
                op: 1,
                name: b"one".to_vec(),
            };
            let original = request(&mut registry, id, first.clone());
            assert_eq!(request(&mut registry, id, first), original);
            let dispatch = registry
                .dispatch(id, Request::Close { op: 2, handle: 1 })
                .unwrap();
            assert!(dispatch.retire);
            assert_eq!(
                dispatch.reply,
                Reply::Unavailable {
                    op: 2,
                    code: UNAVAILABLE_LEDGER
                }
            );
            assert_eq!(registry.handles, 1); // unrecordable close did not happen
            registry.disconnect(id);
            empty(&registry);
        }
    }

    #[test]
    fn object_handle_and_connection_ids_finish_without_wrapping_or_reuse() {
        let mut registry = registry();
        let id = connect(&mut registry, peer(7, 10));
        registry.next_object.next = Some(u64::MAX);
        registry.next_handle.next = Some(u64::MAX);
        assert_eq!(create(&mut registry, id, 1, b"one"), (u64::MAX, u64::MAX));
        assert_eq!(
            request(
                &mut registry,
                id,
                Request::Create {
                    op: 2,
                    name: b"one".to_vec()
                }
            ),
            Reply::Unavailable {
                op: 2,
                code: UNAVAILABLE_IDENTITIES
            }
        );
        request(
            &mut registry,
            id,
            Request::Close {
                op: 3,
                handle: u64::MAX,
            },
        );
        assert_eq!(
            request(
                &mut registry,
                id,
                Request::Create {
                    op: 4,
                    name: b"one".to_vec()
                }
            ),
            Reply::Unavailable {
                op: 4,
                code: UNAVAILABLE_IDENTITIES
            }
        );
        registry.disconnect(id);
        empty(&registry);
        registry.next_connection.next = Some(u64::MAX);
        let last = connect(&mut registry, peer(7, 10));
        assert_eq!(last, u64::MAX);
        registry.disconnect(last);
        assert!(registry.connect(peer(7, 10)).is_err());
        empty(&registry);
    }

    fn exchange(stream: &mut UnixStream, request: Request) -> Reply {
        write_request(stream, &request).unwrap();
        read_reply(stream).unwrap().expect("authority reply")
    }
    fn handler_pair() -> (UnixStream, Arc<Mutex<Registry>>, JoinHandle<()>) {
        // Synthetic identity is only a registry fixture; authentication has its
        // own kernel tests, and Server::run always uses peer_identity.
        let registry = Arc::new(Mutex::new(registry()));
        let id = lock_registry(&registry).connect(peer(7, 10)).unwrap();
        let guard = ConnectionGuard {
            registry: Arc::clone(&registry),
            id,
        };
        let (mut client, server) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let handler = thread::spawn(move || serve(server, guard));
        assert!(matches!(
            exchange(&mut client, Request::Hello),
            Reply::Hello { .. }
        ));
        (client, registry, handler)
    }

    #[test]
    fn actual_stream_eof_closes_all_records_and_drops_completed_history() {
        let (mut client, registry, handler) = handler_pair();
        let one = exchange(
            &mut client,
            Request::Create {
                op: 1,
                name: b"one".to_vec(),
            },
        );
        let two = exchange(
            &mut client,
            Request::Create {
                op: 2,
                name: b"one".to_vec(),
            },
        );
        assert!(
            matches!((one, two), (Reply::Created { object: a, handle: x, .. },
            Reply::Created { object: b, handle: y, .. }) if a == b && x != y)
        );
        assert_eq!(lock_registry(&registry).handles, 2);
        drop(client);
        handler.join().unwrap();
        empty(&lock_registry(&registry));
    }

    #[test]
    fn terminal_ledger_unavailable_is_sent_before_real_transport_retirement() {
        let registry = Arc::new(Mutex::new(Registry::new(
            [7; 16],
            Limits {
                operations: 1,
                ..Limits::default()
            },
        )));
        let id = lock_registry(&registry).connect(peer(7, 10)).unwrap();
        let guard = ConnectionGuard {
            registry: Arc::clone(&registry),
            id,
        };
        let (mut client, server) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let handler = thread::spawn(move || serve(server, guard));
        exchange(&mut client, Request::Hello);
        let create = Request::Create {
            op: 1,
            name: b"one".to_vec(),
        };
        let original = exchange(&mut client, create.clone());
        assert_eq!(exchange(&mut client, create), original);
        assert_eq!(
            exchange(&mut client, Request::Check { op: 2, handle: 1 }),
            Reply::Unavailable {
                op: 2,
                code: UNAVAILABLE_LEDGER
            }
        );
        assert_eq!(read_reply(&mut client).unwrap(), None);
        handler.join().unwrap();
        empty(&lock_registry(&registry));
    }

    #[test]
    fn malformed_partial_eof_and_conflicting_frames_release_real_connection() {
        for mode in 0..3 {
            let (mut client, registry, handler) = handler_pair();
            exchange(
                &mut client,
                Request::Create {
                    op: 1,
                    name: b"one".to_vec(),
                },
            );
            match mode {
                0 => {
                    client.write_all(&[20, 0, 0, 0, 1, 2]).unwrap();
                    client.shutdown(Shutdown::Write).unwrap();
                }
                1 => {
                    write_request(&mut client, &Request::Hello).unwrap();
                }
                _ => {
                    write_request(
                        &mut client,
                        &Request::Create {
                            op: 1,
                            name: b"changed".to_vec(),
                        },
                    )
                    .unwrap();
                }
            }
            assert_eq!(read_reply(&mut client).unwrap(), None);
            handler.join().unwrap();
            empty(&lock_registry(&registry));
        }
    }

    #[test]
    fn panic_and_poison_still_release_the_connections_live_records() {
        let registry = Arc::new(Mutex::new(registry()));
        let id = lock_registry(&registry).connect(peer(7, 10)).unwrap();
        let guard = ConnectionGuard {
            registry: Arc::clone(&registry),
            id,
        };
        let thread = thread::spawn(move || {
            let _guard = guard;
            let mut registry = lock_registry(&_guard.registry);
            registry.dispatch(id, Request::Hello).unwrap();
            registry
                .dispatch(
                    id,
                    Request::Create {
                        op: 1,
                        name: b"one".to_vec(),
                    },
                )
                .unwrap();
            panic!("intentional handler unwind while registry lock is held");
        });
        assert!(thread.join().is_err());
        assert!(registry.is_poisoned());
        empty(&lock_registry(&registry));
    }

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let suffix: String = random_epoch()
                .unwrap()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            // Endpoint admission deliberately rejects symlink ancestors. Use
            // the physical test root, including on macOS where /tmp is a link.
            let path = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("sims-kernel-{suffix}"));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
        fn socket(&self) -> PathBuf {
            self.0.join("authority.sock")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn fresh_bind_explicit_permissions_and_owned_endpoint_cleanup() {
        let directory = Directory::new();
        let path = directory.socket();
        assert!(Server::bind(&path, 0o660).is_err());
        assert!(!path.exists());
        for mode in [0o600, 0o666] {
            let server = Server::bind(&path, mode).unwrap();
            assert_eq!(fs::symlink_metadata(&path).unwrap().mode() & 0o777, mode);
            assert!(Server::bind(&path, mode).is_err());
            drop(server);
            assert!(!path.exists());
        }
        fs::write(&path, b"existing").unwrap();
        assert!(Server::bind(&path, 0o600).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"existing");
    }

    #[test]
    fn socket_replacement_and_symlink_are_never_adopted_or_unlinked() {
        use std::os::unix::fs::symlink;
        let directory = Directory::new();
        let path = directory.socket();
        let server = Server::bind(&path, 0o600).unwrap();
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"replacement").unwrap();
        drop(server);
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
        fs::remove_file(&path).unwrap();
        let target = directory.0.join("target");
        fs::write(&target, b"target").unwrap();
        symlink(&target, &path).unwrap();
        assert!(Server::bind(&path, 0o600).is_err());
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&target).unwrap(), b"target");
    }

    #[test]
    fn untrusted_writable_parent_is_unavailable_before_binding() {
        let directory = Directory::new();
        fs::set_permissions(&directory.0, Permissions::from_mode(0o777)).unwrap();
        assert!(
            matches!(Server::bind(&directory.socket(), 0o600), Err(e) if e.kind() == io::ErrorKind::PermissionDenied)
        );
        assert!(!directory.socket().exists());
    }

    #[test]
    fn bind_rejects_writable_ancestor_above_private_endpoint_parent() {
        let directory = Directory::new();
        let writable = directory.0.join("writable");
        fs::create_dir(&writable).unwrap();
        fs::set_permissions(&writable, Permissions::from_mode(0o777)).unwrap();
        let private = writable.join("private");
        fs::create_dir(&private).unwrap();
        fs::set_permissions(&private, Permissions::from_mode(0o700)).unwrap();
        let path = private.join("a.sock");
        assert!(
            matches!(Server::bind(&path, 0o666), Err(error) if error.kind() == io::ErrorKind::PermissionDenied)
        );
        assert!(!path.exists());
    }

    #[cfg(target_os = "macos")]
    struct Running {
        stop: Arc<AtomicBool>,
        done: std::sync::mpsc::Receiver<io::Result<ServerSummary>>,
        thread: Option<JoinHandle<()>>,
    }
    #[cfg(target_os = "macos")]
    impl Running {
        fn new(server: Server) -> Self {
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let (sender, done) = std::sync::mpsc::channel();
            let thread = Some(thread::spawn(move || {
                let _ = sender.send(server.run(worker_stop));
            }));
            Self { stop, done, thread }
        }
    }
    #[cfg(target_os = "macos")]
    impl Drop for Running {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn authenticated_socket_retains_host_until_real_eof_after_semantic_stop() {
        use std::sync::mpsc::RecvTimeoutError;
        let directory = Directory::new();
        let path = directory.socket();
        let server = Server::bind(&path, 0o600).unwrap();
        let epoch = server.epoch();
        let running = Running::new(server);
        let mut client = UnixStream::connect(&path).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        match exchange(&mut client, Request::Hello) {
            Reply::Hello {
                epoch: actual,
                uid,
                connection,
                ..
            } => {
                assert_eq!(actual, epoch);
                assert_eq!(uid, current_uid());
                assert_ne!(connection, 0);
            }
            reply => panic!("unexpected Hello: {reply:?}"),
        }
        let (handle, object) = match exchange(
            &mut client,
            Request::Create {
                op: 1,
                name: b"Global\\retained".to_vec(),
            },
        ) {
            Reply::Created { handle, object, .. } => (handle, object),
            reply => panic!("unexpected create: {reply:?}"),
        };
        running.stop.store(true, Ordering::Release);
        assert!(matches!(
            running.done.recv_timeout(Duration::from_millis(50)),
            Err(RecvTimeoutError::Timeout)
        ));
        assert!(path.exists());
        assert_eq!(
            exchange(&mut client, Request::Check { op: 2, handle }),
            Reply::Live { op: 2, object }
        );
        assert_eq!(
            exchange(&mut client, Request::Close { op: 3, handle }),
            Reply::Closed { op: 3 }
        );
        // Even a zero-handle live connection retains the draining host.
        assert!(matches!(
            running.done.recv_timeout(Duration::from_millis(50)),
            Err(RecvTimeoutError::Timeout)
        ));
        assert!(path.exists());
        drop(client);
        assert_eq!(
            running
                .done
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap(),
            ServerSummary {
                objects_remaining: 0,
                handles_remaining: 0,
                connections_remaining: 0
            }
        );
        assert!(!path.exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn independently_authenticated_connections_share_one_global_object() {
        let directory = Directory::new();
        let path = directory.socket();
        let server = Server::bind(&path, 0o600).unwrap();
        let _running = Running::new(server);
        let mut a = UnixStream::connect(&path).unwrap();
        let mut b = UnixStream::connect(&path).unwrap();
        for stream in [&a, &b] {
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
        }
        let hello_a = exchange(&mut a, Request::Hello);
        let hello_b = exchange(&mut b, Request::Hello);
        assert!(
            matches!((hello_a, hello_b), (Reply::Hello { connection: x, epoch: e1, .. },
            Reply::Hello { connection: y, epoch: e2, .. }) if x != y && e1 == e2)
        );
        let one = exchange(
            &mut a,
            Request::Create {
                op: 1,
                name: b"Global\\shared".to_vec(),
            },
        );
        let two = exchange(
            &mut b,
            Request::Create {
                op: 1,
                name: b"Global\\shared".to_vec(),
            },
        );
        let (handle, object) = match (one, two) {
            (
                Reply::Created {
                    object: x,
                    handle: h1,
                    ..
                },
                Reply::Created {
                    object: y,
                    handle: h2,
                    ..
                },
            ) => {
                assert_eq!(x, y);
                assert_ne!(h1, h2);
                (h2, y)
            }
            replies => panic!("unexpected creates: {replies:?}"),
        };
        drop(a);
        assert_eq!(
            exchange(&mut b, Request::Check { op: 2, handle }),
            Reply::Live { op: 2, object }
        );
        drop(b);
    }
}
