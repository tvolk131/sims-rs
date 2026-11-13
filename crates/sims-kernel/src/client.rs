//! Authenticated, single-authority Unix-stream client.
//!
//! A reserved create has no external effect until `resolve`. Completed outcomes
//! are immutable history; only `check` establishes current authority access.
//! There is deliberately no reconnect, endpoint adoption, or local fallback.

use crate::auth;
use crate::protocol::{self, Reply, Request};
use std::fmt;
use std::fs;
use std::io;
use std::net::Shutdown;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

const MAX_NAME: usize = 4096;
const MAX_OPERATIONS: usize = 4096;
const MAX_CREATES: usize = 1024;
const MAX_RETAINED_NAME_BYTES: usize = 1024 * 1024;
const MAX_ENDPOINT_BYTES: usize = 4096;
const MAX_ENDPOINT_COMPONENTS: usize = 256;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Host/transport errors are never a normal CreateMutex-style null result.
/// Authoritative create rejection instead arrives as `CreateOutcome::Rejected`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Transport {
        kind: io::ErrorKind,
        message: String,
    },
    Authentication(&'static str),
    SocketIdentity(&'static str),
    Protocol(&'static str),
    AuthorityLost,
    Unavailable {
        code: Option<u32>,
        reason: &'static str,
    },
    ApiRejected {
        code: u32,
    },
    Closed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport { kind, message } => {
                write!(f, "authority transport {kind:?}: {message}")
            }
            Self::Authentication(reason) => write!(f, "authority authentication: {reason}"),
            Self::SocketIdentity(reason) => write!(f, "authority socket identity: {reason}"),
            Self::Protocol(reason) => write!(f, "authority protocol: {reason}"),
            Self::AuthorityLost => {
                f.write_str("authority connection lost; operation will not be recreated")
            }
            Self::Unavailable { code, reason } => {
                write!(f, "authority unavailable ({code:?}): {reason}")
            }
            Self::ApiRejected { code } => {
                write!(f, "authority rejected access with API code {code}")
            }
            Self::Closed => f.write_str("lease is acknowledged closed"),
        }
    }
}

impl std::error::Error for Error {}

fn transport(error: io::Error) -> Error {
    Error::Transport {
        kind: error.kind(),
        message: error.to_string(),
    }
}

fn unavailable(reason: &'static str) -> Error {
    Error::Unavailable { code: None, reason }
}

fn api_code(code: u32) -> bool {
    matches!(code, 5 | 6 | 123)
}

#[derive(Clone, Debug)]
pub struct Client {
    inner: Arc<Connection>,
}

#[derive(Debug)]
struct Connection {
    epoch: [u8; 16],
    connection: u64,
    session: Option<u32>,
    state: Mutex<ConnectionState>,
}

#[derive(Debug)]
struct ConnectionState {
    stream: Option<UnixStream>,
    terminal: Option<Error>,
    socket: SocketIdentity,
    next_op: Option<u64>,
    ledger: Vec<LedgerEntry>,
    handles: Vec<HandleRecord>,
    reserved_closes: usize,
    retained_name_bytes: usize,
}

#[derive(Debug)]
struct LedgerEntry {
    request: Request,
    // Set before write: even a failed write may have delivered a partial frame.
    attempted: bool,
    // Own the decoded reply before any validation or socket revalidation.
    reply: Option<Reply>,
}

#[derive(Debug)]
struct HandleRecord {
    issued: Option<(u64, u64)>,
    close_entry: Option<usize>,
    closed: bool,
}

/// All clones resolve the same reservation and own the same staged outcome.
#[derive(Clone, Debug)]
pub struct CreateOperation {
    inner: Arc<CreateInner>,
}

#[derive(Debug)]
struct CreateInner {
    op: u64,
    entry: usize,
    slot: usize,
    name: Arc<[u8]>,
    state: Mutex<CreateState>,
}

#[derive(Debug)]
struct CreateState {
    // Released after resolution. Rejected/closed histories cannot pin a socket.
    connection: Option<Arc<Connection>>,
    // Kept even if validation after the reply fails. Never overwritten.
    staged: Option<CreateOutcome>,
    result: Option<Result<CreateOutcome, Error>>,
}

#[derive(Clone, Debug)]
pub enum CreateOutcome {
    Created(Lease),
    Rejected(Failure),
}

/// Only an authenticated, actual rejection reply can construct this value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    code: u32,
}

impl Failure {
    pub fn code(&self) -> u32 {
        self.code
    }
}

/// Clones share one handle and one cleanup obligation, not new references.
#[derive(Clone, Debug)]
pub struct Lease {
    inner: Arc<LeaseInner>,
}

#[derive(Debug)]
struct LeaseInner {
    epoch: [u8; 16],
    handle: u64,
    object: u64,
    name: Arc<[u8]>,
    slot: usize,
    // Taken only after an acknowledged close. Keeping immutable history then
    // cannot keep a connection alive. Retirement closes the fd immediately
    // even if an uncertain lease still retains this Arc.
    connection: Mutex<Option<Arc<Connection>>>,
}

impl Client {
    pub fn connect(path: &Path, expected_server_uid: u32) -> Result<Self, Error> {
        let socket = SocketIdentity::capture(path, expected_server_uid)?;
        // Preallocate bounded ledgers before sending anything to the authority.
        let mut ledger = Vec::new();
        ledger
            .try_reserve_exact(MAX_OPERATIONS)
            .map_err(|_| unavailable("operation ledger allocation failed"))?;
        let mut handles = Vec::new();
        handles
            .try_reserve_exact(MAX_CREATES)
            .map_err(|_| unavailable("handle ledger allocation failed"))?;

        let mut stream = UnixStream::connect(&socket.path).map_err(transport)?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(transport)?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(transport)?;
        socket.validate()?;
        let peer = auth::peer_identity(&stream)
            .map_err(|_| Error::Authentication("kernel peer credentials unavailable"))?;
        if peer.uid != expected_server_uid {
            return Err(Error::Authentication("unexpected kernel peer UID"));
        }
        // The peer's session is the server's session. Hello's session instead
        // describes this client as authenticated by the configured authority.
        protocol::write_request(&mut stream, &Request::Hello).map_err(transport)?;
        let reply = protocol::read_reply(&mut stream)
            .map_err(decode_error)?
            .ok_or(Error::AuthorityLost)?;
        let (epoch, connection, session) = match reply {
            Reply::Hello {
                epoch,
                connection,
                uid,
                session,
            } if epoch != [0; 16] && connection != 0 && uid == auth::current_uid() => {
                (epoch, connection, session)
            }
            Reply::Hello { .. } => return Err(Error::Protocol("invalid handshake identity")),
            _ => return Err(Error::Protocol("expected the initial Hello reply")),
        };
        socket.validate()?;
        Ok(Self {
            inner: Arc::new(Connection {
                epoch,
                connection,
                session,
                state: Mutex::new(ConnectionState {
                    stream: Some(stream),
                    terminal: None,
                    socket,
                    next_op: Some(1),
                    ledger,
                    handles,
                    reserved_closes: 0,
                    retained_name_bytes: 0,
                }),
            }),
        })
    }

    /// Reserve the request, its identity, and capacity for its eventual close.
    /// No socket write, including a create, occurs here.
    pub fn begin_create(&self, name: &[u8]) -> Result<CreateOperation, Error> {
        if name.len() > MAX_NAME {
            return Err(unavailable("name exceeds the wire bound"));
        }
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(name.len())
            .map_err(|_| unavailable("name allocation failed"))?;
        owned.extend_from_slice(name);
        let mut wire_name = Vec::new();
        wire_name
            .try_reserve_exact(name.len())
            .map_err(|_| unavailable("request allocation failed"))?;
        wire_name.extend_from_slice(name);
        let name: Arc<[u8]> = owned.into();
        let mut state = self.inner.lock();
        state.available()?;
        state.capacity(2)?; // create plus its reserved cleanup operation
        if state.handles.len() == MAX_CREATES {
            return Err(unavailable("create ledger exhausted"));
        }
        if name.len() > MAX_RETAINED_NAME_BYTES - state.retained_name_bytes {
            return Err(unavailable("retained name ledger exhausted"));
        }
        let op = state.allocate_op()?;
        let entry = state.ledger.len();
        let slot = state.handles.len();
        state.ledger.push(LedgerEntry {
            request: Request::Create {
                op,
                name: wire_name,
            },
            attempted: false,
            reply: None,
        });
        state.handles.push(HandleRecord {
            issued: None,
            close_entry: None,
            closed: false,
        });
        state.reserved_closes += 1;
        state.retained_name_bytes += name.len();
        Ok(CreateOperation {
            inner: Arc::new(CreateInner {
                op,
                entry,
                slot,
                name,
                state: Mutex::new(CreateState {
                    connection: Some(self.inner.clone()),
                    staged: None,
                    result: None,
                }),
            }),
        })
    }

    pub fn epoch(&self) -> [u8; 16] {
        self.inner.epoch
    }
    pub fn connection(&self) -> u64 {
        self.inner.connection
    }
    pub fn peer_session(&self) -> Option<u32> {
        self.inner.session
    }
}

impl CreateOperation {
    pub fn resolve(&self) -> Result<CreateOutcome, Error> {
        let mut operation = match self.inner.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                let error = Error::Protocol("create resolution panicked");
                if let Some(connection) = &state.connection {
                    connection.lock().retire(error.clone());
                }
                if state.result.is_none() {
                    state.result = Some(Err(error));
                }
                state.connection = None;
                state
            }
        };
        if let Some(result) = &operation.result {
            return result.clone();
        }
        let connection = operation
            .connection
            .as_ref()
            .ok_or(Error::Protocol("missing reserved connection"))?
            .clone();
        let result = self.resolve_once(&connection, &mut operation);
        operation.result = Some(result.clone());
        operation.connection = None;
        result
    }

    fn resolve_once(
        &self,
        connection: &Arc<Connection>,
        operation: &mut CreateState,
    ) -> Result<CreateOutcome, Error> {
        let mut state = connection.lock();
        let reply = state.exchange(self.inner.entry)?;
        match reply {
            Reply::Created { op, handle, object } if op == self.inner.op => {
                let lease = Lease {
                    inner: Arc::new(LeaseInner {
                        epoch: connection.epoch,
                        handle,
                        object,
                        name: self.inner.name.clone(),
                        slot: self.inner.slot,
                        connection: Mutex::new(Some(connection.clone())),
                    }),
                };
                // Stage ownership before identity/duplicate/post-path checks.
                operation.staged = Some(CreateOutcome::Created(lease.clone()));
                if handle == 0
                    || object == 0
                    || state
                        .handles
                        .iter()
                        .any(|record| record.issued.is_some_and(|issued| issued.0 == handle))
                {
                    Err(state.retire(Error::Protocol("invalid or reused issued handle identity")))
                } else {
                    state.handles[self.inner.slot].issued = Some((handle, object));
                    state
                        .validate_socket()
                        .map(|()| CreateOutcome::Created(lease))
                }
            }
            Reply::Rejected { op, code } if op == self.inner.op => {
                let failure = Failure { code };
                operation.staged = Some(CreateOutcome::Rejected(failure.clone()));
                if !api_code(code) {
                    Err(state.retire(Error::Protocol("unsupported authoritative rejection code")))
                } else {
                    state.reserved_closes -= 1;
                    state.handles[self.inner.slot].closed = true;
                    state
                        .validate_socket()
                        .map(|()| CreateOutcome::Rejected(failure))
                }
            }
            Reply::Unavailable { op, code } if op == self.inner.op => {
                state.reserved_closes -= 1;
                state.handles[self.inner.slot].closed = true;
                state.validate_socket()?;
                Err(Error::Unavailable {
                    code: Some(code),
                    reason: "create unavailable at authority",
                })
            }
            _ => Err(state.retire(Error::Protocol("unexpected or mismatched create reply"))),
        }
    }
}

impl Drop for CreateInner {
    fn drop(&mut self) {
        let operation = match self.state.get_mut() {
            Ok(operation) => operation,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(connection) = operation.connection.take() {
            let mut state = connection.lock();
            // Abandoning an unsent reservation has no authority effect and no
            // future cleanup obligation. Keep its identity/name ledger bounded
            // and never reuse its operation ID.
            if !state.ledger[self.entry].attempted && !state.handles[self.slot].closed {
                state.handles[self.slot].closed = true;
                state.reserved_closes -= 1;
            }
        }
    }
}

impl Lease {
    /// A fresh serialized request to the original authority; never a cache hit.
    pub fn check(&self) -> Result<(), Error> {
        let connection = self.inner.link().as_ref().ok_or(Error::Closed)?.clone();
        connection.check(self.inner.slot, self.inner.handle, self.inner.object)
    }

    /// Shared and idempotent only after an actual matching Closed acknowledgement.
    pub fn close(&self) -> Result<(), Error> {
        let mut link = self.inner.link();
        let Some(connection) = link.as_ref().cloned() else {
            return Ok(());
        };
        let result = connection.close(self.inner.slot);
        if result.is_ok() || connection.lock().handles[self.inner.slot].closed {
            *link = None;
        }
        result
    }

    /// Records acknowledgement, not a guess based on descriptor/authority loss.
    pub fn is_closed(&self) -> bool {
        self.inner.link().is_none()
    }
    pub fn handle(&self) -> u64 {
        self.inner.handle
    }
    pub fn object(&self) -> u64 {
        self.inner.object
    }
    pub fn epoch(&self) -> [u8; 16] {
        self.inner.epoch
    }
    pub fn name(&self) -> &[u8] {
        &self.inner.name
    }
}

impl LeaseInner {
    fn link(&self) -> MutexGuard<'_, Option<Arc<Connection>>> {
        match self.connection.lock() {
            Ok(link) => link,
            Err(poisoned) => {
                let link = poisoned.into_inner();
                if let Some(connection) = link.as_ref() {
                    connection
                        .lock()
                        .retire(Error::Protocol("lease access panicked"));
                }
                link
            }
        }
    }
}

impl Drop for LeaseInner {
    fn drop(&mut self) {
        let link = match self.connection.get_mut() {
            Ok(link) => link,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(connection) = link.take() {
            // Failure retains the exact pending close in ConnectionState. The
            // last connection owner dropping still closes the real descriptor.
            let _ = connection.close(self.slot);
        }
    }
}

fn decode_error(error: io::Error) -> Error {
    match error.kind() {
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => {
            Error::Protocol("invalid or truncated authority frame")
        }
        _ => transport(error),
    }
}

/// Retire on unwinding as well as ordinary I/O errors: a retained historical
/// operation must never keep a partially framed connection alive after panic.
struct StateGuard<'a>(MutexGuard<'a, ConnectionState>);

impl std::ops::Deref for StateGuard<'_> {
    type Target = ConnectionState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for StateGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for StateGuard<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0
                .retire(Error::Protocol("connection operation panicked"));
        }
    }
}

impl Connection {
    fn lock(&self) -> StateGuard<'_> {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.retire(Error::Protocol("connection lock poisoned"));
                state
            }
        };
        StateGuard(state)
    }

    fn check(&self, slot: usize, handle: u64, object: u64) -> Result<(), Error> {
        let mut state = self.lock();
        state.available()?;
        if state.handles[slot].closed {
            return Err(Error::Closed);
        }
        if state.handles[slot].close_entry.is_some() {
            return Err(unavailable("close acknowledgement remains pending"));
        }
        state.capacity(1)?;
        let op = state.allocate_op()?;
        let entry = state.ledger.len();
        state.ledger.push(LedgerEntry {
            request: Request::Check { op, handle },
            attempted: false,
            reply: None,
        });
        match state.exchange(entry)? {
            Reply::Live {
                op: response_op,
                object: response_object,
            } if response_op == op && response_object == object => state.validate_socket(),
            Reply::Rejected {
                op: response_op,
                code,
            } if response_op == op && api_code(code) => {
                state.validate_socket()?;
                Err(Error::ApiRejected { code })
            }
            Reply::Unavailable {
                op: response_op,
                code,
            } if response_op == op => {
                state.validate_socket()?;
                Err(Error::Unavailable {
                    code: Some(code),
                    reason: "live check unavailable at authority",
                })
            }
            _ => Err(state.retire(Error::Protocol("unexpected or mismatched live reply"))),
        }
    }

    fn close(&self, slot: usize) -> Result<(), Error> {
        let mut state = self.lock();
        if state.handles[slot].closed {
            return Ok(());
        }
        state.available()?;
        let (handle, _) = state.handles[slot]
            .issued
            .ok_or_else(|| unavailable("no validated issued handle to close"))?;
        let entry = if let Some(entry) = state.handles[slot].close_entry {
            entry
        } else {
            // Capacity and an operation-ID budget were reserved before create.
            let op = state.allocate_op()?;
            let entry = state.ledger.len();
            state.ledger.push(LedgerEntry {
                request: Request::Close { op, handle },
                attempted: false,
                reply: None,
            });
            state.reserved_closes -= 1;
            state.handles[slot].close_entry = Some(entry);
            entry
        };
        let op = match state.ledger[entry].request {
            Request::Close { op, .. } => op,
            _ => return Err(state.retire(Error::Protocol("invalid retained close request"))),
        };
        match state.exchange(entry)? {
            Reply::Closed { op: response_op } if response_op == op => {
                // Record the real acknowledgement before post-reply validation.
                state.handles[slot].closed = true;
                state.validate_socket()
            }
            Reply::Rejected {
                op: response_op,
                code,
            } if response_op == op && api_code(code) => {
                state.validate_socket()?;
                Err(Error::ApiRejected { code })
            }
            Reply::Unavailable {
                op: response_op,
                code,
            } if response_op == op => {
                state.validate_socket()?;
                Err(Error::Unavailable {
                    code: Some(code),
                    reason: "close acknowledgement remains pending",
                })
            }
            _ => Err(state.retire(Error::Protocol("unexpected or mismatched close reply"))),
        }
    }
}

impl ConnectionState {
    fn available(&self) -> Result<(), Error> {
        if let Some(error) = &self.terminal {
            return Err(error.clone());
        }
        if self.stream.is_none() {
            return Err(Error::AuthorityLost);
        }
        Ok(())
    }

    fn capacity(&self, additional: usize) -> Result<(), Error> {
        if self.ledger.len() + self.reserved_closes + additional > MAX_OPERATIONS {
            return Err(unavailable("operation ledger exhausted"));
        }
        let needed = self.reserved_closes + additional;
        if needed != 0
            && self
                .next_op
                .and_then(|next| next.checked_add((needed - 1) as u64))
                .is_none()
        {
            return Err(unavailable("operation identities exhausted"));
        }
        Ok(())
    }

    fn allocate_op(&mut self) -> Result<u64, Error> {
        let op = self
            .next_op
            .filter(|op| *op != 0)
            .ok_or_else(|| unavailable("operation identities exhausted"))?;
        self.next_op = op.checked_add(1);
        Ok(op)
    }

    fn validate_socket(&mut self) -> Result<(), Error> {
        match self.socket.validate() {
            Ok(()) => Ok(()),
            Err(error) => Err(self.retire(error)),
        }
    }

    fn exchange(&mut self, entry: usize) -> Result<Reply, Error> {
        self.available()?;
        self.validate_socket()?;
        self.ledger[entry].attempted = true;
        let result = {
            let stream = self.stream.as_mut().expect("available stream");
            protocol::write_request(stream, &self.ledger[entry].request)
                .map_err(transport)
                .and_then(|()| protocol::read_reply(stream).map_err(decode_error))
        };
        match result {
            Ok(Some(reply)) => {
                if self.ledger[entry]
                    .reply
                    .as_ref()
                    .is_some_and(|prior| *prior != reply)
                {
                    return Err(self.retire(Error::Protocol("completed operation reply changed")));
                }
                if self.ledger[entry].reply.is_none() {
                    self.ledger[entry].reply = Some(reply.clone());
                }
                Ok(reply)
            }
            Ok(None) => Err(self.retire(Error::AuthorityLost)),
            Err(error) => Err(self.retire(error)),
        }
    }

    fn retire(&mut self, error: Error) -> Error {
        if self.terminal.is_none() {
            self.terminal = Some(error);
        }
        if let Some(stream) = self.stream.take() {
            let _ = stream.shutdown(Shutdown::Both);
            drop(stream); // real EOF releases server references despite history
        }
        self.terminal.as_ref().expect("retired error").clone()
    }
}

impl Drop for ConnectionState {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    // Directories may gain unrelated entries while their own identity is stable.
    // Socket change time and link count additionally detect unlink/replacement.
    socket_change: Option<(i64, i64, u64)>,
}

impl FileIdentity {
    fn read(metadata: &fs::Metadata, socket: bool) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            socket_change: socket
                .then(|| (metadata.ctime(), metadata.ctime_nsec(), metadata.nlink())),
        }
    }
}

#[derive(Debug)]
struct SocketIdentity {
    path: PathBuf,
    chain: Vec<(PathBuf, FileIdentity)>,
}

impl SocketIdentity {
    fn capture(path: &Path, expected_uid: u32) -> Result<Self, Error> {
        if path.as_os_str().is_empty() {
            return Err(Error::SocketIdentity("empty endpoint path"));
        }
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_err(transport)?.join(path)
        };
        if absolute.as_os_str().as_encoded_bytes().len() > MAX_ENDPOINT_BYTES {
            return Err(Error::SocketIdentity(
                "endpoint path exceeds the retained bound",
            ));
        }
        let mut path = PathBuf::new();
        for component in absolute.components() {
            match component {
                Component::RootDir | Component::Normal(_) => path.push(component.as_os_str()),
                Component::CurDir => (),
                _ => {
                    return Err(Error::SocketIdentity(
                        "endpoint contains a parent traversal",
                    ));
                }
            }
        }
        let mut chain = Vec::new();
        let mut current = PathBuf::new();
        for component in path.components() {
            if chain.len() == MAX_ENDPOINT_COMPONENTS {
                return Err(Error::SocketIdentity("endpoint has too many ancestors"));
            }
            current.push(component.as_os_str());
            let metadata = fs::symlink_metadata(&current).map_err(transport)?;
            if metadata.file_type().is_symlink() {
                return Err(Error::SocketIdentity("symlink in endpoint path"));
            }
            let socket = current == path;
            if socket {
                if !metadata.file_type().is_socket() || metadata.nlink() != 1 {
                    return Err(Error::SocketIdentity(
                        "endpoint is not a single-link Unix socket",
                    ));
                }
                if metadata.uid() != expected_uid {
                    return Err(Error::Authentication(
                        "socket owner differs from expected server UID",
                    ));
                }
                if !matches!(metadata.mode() & 0o7777, 0o600 | 0o666) {
                    return Err(Error::SocketIdentity(
                        "endpoint mode must be explicitly 0600 or 0666",
                    ));
                }
            } else if metadata.uid() != 0 && metadata.uid() != expected_uid {
                return Err(Error::Authentication("untrusted endpoint ancestor UID"));
            } else if !crate::endpoint::trusted_directory(&metadata, expected_uid) {
                return Err(Error::SocketIdentity("untrusted endpoint ancestor"));
            }
            chain.push((current.clone(), FileIdentity::read(&metadata, socket)));
        }
        let identity = Self { path, chain };
        identity.validate()?;
        Ok(identity)
    }

    fn validate(&self) -> Result<(), Error> {
        for (path, expected) in &self.chain {
            let metadata = fs::symlink_metadata(path).map_err(|_| {
                Error::SocketIdentity("endpoint path disappeared or became unreadable")
            })?;
            if metadata.file_type().is_symlink()
                || FileIdentity::read(&metadata, path == &self.path) != *expected
            {
                return Err(Error::SocketIdentity("endpoint path identity changed"));
            }
        }
        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
