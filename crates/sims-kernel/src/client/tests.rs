//! Authored for parent execution; no worker builds or runs these fixtures.
//! Successful behavior uses the real authority. CorruptPeer is restricted to
//! protocol/transport failure tests and is not a production backend.

use super::*;
use crate::server::{Server, ServerSummary};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::sync::Barrier;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

static DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap();
        for _ in 0..128 {
            let sequence = DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            // Keep the complete physical pathname below macOS sun_path's
            // bound even when temp_dir is the longer /private/var/folders path.
            let path = root.join(format!("skc-{time:x}-{sequence:x}"));
            match fs::create_dir(&path) {
                Ok(()) => {
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
                    return Self(path);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
                Err(error) => panic!("create private fixture directory: {error}"),
            }
        }
        panic!("private fixture directory collisions");
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Authority {
    directory: Directory,
    path: PathBuf,
    stop: Arc<AtomicBool>,
    done: Receiver<io::Result<ServerSummary>>,
    worker: Option<JoinHandle<()>>,
}

impl Authority {
    fn new() -> Self {
        let directory = Directory::new();
        let path = directory.0.join("a.sock");
        let server = Server::bind(&path, 0o600).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let (send, done) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _ = send.send(server.run(signal));
        });
        Self {
            directory,
            path,
            stop,
            done,
            worker: Some(worker),
        }
    }

    fn client(&self) -> Client {
        Client::connect(&self.path, auth::current_uid()).unwrap()
    }

    fn finish(mut self) {
        self.stop.store(true, Ordering::Release);
        let summary = self
            .done
            .recv_timeout(Duration::from_secs(10))
            .expect("authority still has a real connection/reference")
            .unwrap();
        self.worker.take().unwrap().join().unwrap();
        assert_eq!(summary.objects_remaining, 0);
        assert_eq!(summary.handles_remaining, 0);
        assert_eq!(summary.connections_remaining, 0);
    }

    fn finish_substituted(mut self) {
        self.stop.store(true, Ordering::Release);
        let result = self
            .done
            .recv_timeout(Duration::from_secs(10))
            .expect("substituted endpoint prevented connection drain");
        self.worker.take().unwrap().join().unwrap();
        // A server may report a cleanup error when its socket pathname has
        // been replaced. This client test permits that safe refusal to unlink.
        if let Ok(summary) = result {
            assert_eq!(summary.objects_remaining, 0);
            assert_eq!(summary.handles_remaining, 0);
            assert_eq!(summary.connections_remaining, 0);
        }
    }
}

impl Drop for Authority {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            let _ = self.worker.take().unwrap().join();
        }
        // On assertion unwind, don't block forever joining a fixture whose
        // caller may still own a lease. Normal paths always call finish().
    }
}

fn created(client: &Client, name: &[u8]) -> Lease {
    match client.begin_create(name).unwrap().resolve().unwrap() {
        CreateOutcome::Created(lease) => lease,
        CreateOutcome::Rejected(failure) => panic!("unexpected API rejection {}", failure.code()),
    }
}

/// Test-only corrupt authority with real Unix credentials, fresh socket and
/// bounded I/O. Its scripts intentionally violate the pinned protocol or lose
/// acknowledgements. No successful behavior test relies on this fixture.
struct CorruptPeer {
    _directory: Directory,
    path: PathBuf,
    worker: Option<JoinHandle<()>>,
}

impl CorruptPeer {
    fn raw(script: impl FnOnce(&mut UnixStream, &Path) + Send + 'static) -> Self {
        let directory = Directory::new();
        let path = directory.0.join("c.sock");
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let script_path = path.clone();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(IO_TIMEOUT)).unwrap();
            script(&mut stream, &script_path);
        });
        Self {
            _directory: directory,
            path,
            worker: Some(worker),
        }
    }

    fn after_hello(script: impl FnOnce(&mut UnixStream, &Path) + Send + 'static) -> Self {
        Self::raw(move |stream, path| {
            assert_eq!(
                protocol::read_request(stream).unwrap(),
                Some(Request::Hello)
            );
            protocol::write_reply(
                stream,
                &Reply::Hello {
                    epoch: [7; 16],
                    connection: 1,
                    uid: auth::current_uid(),
                    session: None,
                },
            )
            .unwrap();
            script(stream, path);
        })
    }

    fn client(&self) -> Client {
        Client::connect(&self.path, auth::current_uid()).unwrap()
    }
    fn finish(mut self) {
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn reservation_sends_no_create_and_preserves_exact_name_bytes() {
    let authority = Authority::new();
    let client = authority.client();
    let reserved = client.begin_create(b"Global\\reserve-exact").unwrap();
    let other = authority.client();
    let first = created(&other, b"Global\\reserve-exact");
    let first_object = first.object();
    first.close().unwrap();
    let second = created(&other, b"Global\\reserve-exact");
    assert_ne!(first_object, second.object());
    let issued = match reserved.resolve().unwrap() {
        CreateOutcome::Created(lease) => lease,
        outcome => panic!("unexpected {outcome:?}"),
    };
    assert_eq!(issued.object(), second.object());
    assert_eq!(issued.name(), b"Global\\reserve-exact");
    assert_eq!(issued.epoch(), client.epoch());
    assert_ne!(client.connection(), other.connection());
    issued.close().unwrap();
    second.close().unwrap();
    drop((client, other));
    authority.finish();
    // Closed operation history remains owned across authority EOF/shutdown.
    assert!(issued.is_closed());
    assert!(matches!(reserved.resolve(), Ok(CreateOutcome::Created(_))));
}

#[test]
fn resolve_and_clones_share_one_issued_reference_and_cleanup_obligation() {
    let authority = Authority::new();
    let client = authority.client();
    let operation = client.begin_create(b"Global\\resolve-once").unwrap();
    let operation_clone = operation.clone();
    let first = match operation.resolve().unwrap() {
        CreateOutcome::Created(lease) => lease,
        _ => panic!("expected create"),
    };
    let second = match operation_clone.resolve().unwrap() {
        CreateOutcome::Created(lease) => lease,
        _ => panic!("expected staged create"),
    };
    assert!(Arc::ptr_eq(&first.inner, &second.inner));
    let before = client.inner.lock().ledger.len();
    second.close().unwrap();
    first.close().unwrap();
    assert_eq!(client.inner.lock().ledger.len(), before + 1);
    assert!(first.is_closed());
    assert_eq!(first.check(), Err(Error::Closed));
    drop(client);
    authority.finish();
    assert_eq!(first.name(), b"Global\\resolve-once");
    assert!(matches!(operation.resolve(), Ok(CreateOutcome::Created(lease)) if lease.is_closed()));
}

#[test]
fn concurrent_resolve_clones_return_one_staged_lease() {
    let authority = Authority::new();
    let client = authority.client();
    let operation = client.begin_create(b"Global\\concurrent-resolve").unwrap();
    let first_operation = operation.clone();
    let second_operation = operation.clone();
    let barrier = Arc::new(Barrier::new(3));
    let first_barrier = barrier.clone();
    let second_barrier = barrier.clone();
    let first_worker = thread::spawn(move || {
        first_barrier.wait();
        first_operation.resolve()
    });
    let second_worker = thread::spawn(move || {
        second_barrier.wait();
        second_operation.resolve()
    });
    barrier.wait();
    let first = first_worker.join().unwrap().unwrap();
    let second = second_worker.join().unwrap().unwrap();
    let (CreateOutcome::Created(first), CreateOutcome::Created(second)) = (first, second) else {
        panic!("expected one successful operation")
    };
    assert!(Arc::ptr_eq(&first.inner, &second.inner));
    assert_eq!(client.inner.lock().ledger.len(), 1);
    first.close().unwrap();
    drop(client);
    authority.finish();
    assert!(second.is_closed());
}

#[test]
fn distinct_creates_open_one_object_with_distinct_live_handles() {
    let authority = Authority::new();
    let client = authority.client();
    let first = created(&client, b"Global\\two-references");
    let second = created(&client, b"Global\\two-references");
    assert_ne!(first.handle(), second.handle());
    assert_eq!(first.object(), second.object());
    first.close().unwrap();
    second.check().unwrap();
    let original = second.object();
    second.close().unwrap();
    let recreated = created(&client, b"Global\\two-references");
    assert_ne!(original, recreated.object());
    recreated.close().unwrap();
    drop(client);
    authority.finish();
}

#[test]
fn exact_ascii_case_is_not_folded() {
    let authority = Authority::new();
    let client = authority.client();
    let upper = created(&client, b"Global\\Case");
    let lower = created(&client, b"Global\\case");
    assert_ne!(upper.object(), lower.object());
    upper.close().unwrap();
    lower.close().unwrap();
    drop(client);
    authority.finish();
}

#[test]
fn dropping_one_clone_keeps_live_access_and_last_drop_closes() {
    let authority = Authority::new();
    let client = authority.client();
    let lease = created(&client, b"Global\\last-clone");
    let identity = lease.object();
    let clone = lease.clone();
    drop(lease);
    clone.check().unwrap();
    drop(clone);
    let recreated = created(&client, b"Global\\last-clone");
    assert_ne!(identity, recreated.object());
    recreated.close().unwrap();
    drop(client);
    authority.finish();
}

#[test]
fn live_lease_outlives_client_but_closed_history_does_not_pin_socket() {
    let authority = Authority::new();
    let client = authority.client();
    let lease = created(&client, b"Global\\history");
    let historical = (
        lease.epoch(),
        lease.handle(),
        lease.object(),
        lease.name().to_vec(),
    );
    drop(client);
    lease.check().unwrap();
    lease.close().unwrap();
    authority.finish(); // lease is deliberately retained
    assert_eq!(
        (
            lease.epoch(),
            lease.handle(),
            lease.object(),
            lease.name().to_vec()
        ),
        historical
    );
    assert_eq!(lease.check(), Err(Error::Closed));
    lease.close().unwrap();
}

#[test]
fn live_checks_each_send_a_new_real_request() {
    let authority = Authority::new();
    let client = authority.client();
    let lease = created(&client, b"Global\\fresh-check");
    let before = client.inner.lock().ledger.len();
    lease.check().unwrap();
    lease.check().unwrap();
    let state = client.inner.lock();
    assert_eq!(state.ledger.len(), before + 2);
    for entry in &state.ledger[before..] {
        assert!(entry.attempted);
        assert!(matches!(entry.reply, Some(Reply::Live { .. })));
    }
    drop(state);
    lease.close().unwrap();
    drop(client);
    authority.finish();
}

#[test]
fn oversized_name_and_identity_exhaustion_fail_before_send() {
    let authority = Authority::new();
    let client = authority.client();
    assert!(matches!(
        client.begin_create(&vec![b'A'; MAX_NAME + 1]),
        Err(Error::Unavailable { .. })
    ));
    assert!(client.inner.lock().ledger.is_empty());
    client.inner.lock().next_op = Some(u64::MAX);
    assert!(matches!(
        client.begin_create(b"Global\\overflow"),
        Err(Error::Unavailable { .. })
    ));
    assert!(client.inner.lock().ledger.is_empty());
    assert_eq!(client.inner.lock().next_op, Some(u64::MAX));
    drop(client);
    authority.finish();
}

#[test]
fn reserved_cleanup_capacity_survives_operation_ledger_exhaustion() {
    let authority = Authority::new();
    let client = authority.client();
    let lease = created(&client, b"Global\\cleanup-budget");
    {
        let mut state = client.inner.lock();
        // Exhaust local capacity without assuming the server's own bound or
        // making thousands of authority requests. Reserved IDs remain unused.
        while state.capacity(1).is_ok() {
            let op = state.allocate_op().unwrap();
            state.ledger.push(LedgerEntry {
                request: Request::Check {
                    op,
                    handle: lease.handle(),
                },
                attempted: false,
                reply: None,
            });
        }
    }
    assert!(matches!(lease.check(), Err(Error::Unavailable { .. })));
    lease.close().unwrap();
    assert!(lease.is_closed());
    drop(client);
    authority.finish();
}

#[test]
fn unused_reservations_are_never_sent_or_reused() {
    let authority = Authority::new();
    let client = authority.client();
    let first = client.begin_create(b"Global\\unused").unwrap();
    let first_op = first.inner.op;
    drop(first);
    assert_eq!(client.inner.lock().reserved_closes, 0);
    let next = client.begin_create(b"Global\\unused").unwrap();
    assert!(next.inner.op > first_op);
    assert!(
        client
            .inner
            .lock()
            .ledger
            .iter()
            .all(|entry| !entry.attempted)
    );
    drop((next, client));
    authority.finish();
}

#[test]
fn real_connection_loss_poisoning_never_reconnects_or_recreates() {
    let authority = Authority::new();
    let client = authority.client();
    let lease = created(&client, b"Global\\lost-transport");
    let unresolved = client.begin_create(b"Global\\never-recreate").unwrap();
    let history = (
        lease.epoch(),
        lease.handle(),
        lease.object(),
        lease.name().to_vec(),
    );
    client
        .inner
        .lock()
        .stream
        .as_ref()
        .unwrap()
        .shutdown(Shutdown::Both)
        .unwrap();
    assert!(lease.check().is_err());
    let first_error = unresolved.resolve().unwrap_err();
    assert_eq!(unresolved.resolve().unwrap_err(), first_error);
    assert!(client.begin_create(b"Global\\new-after-loss").is_err());
    assert!(lease.close().is_err());
    assert!(!lease.is_closed()); // no fabricated acknowledgement
    assert!(client.inner.lock().stream.is_none());
    authority.finish(); // all poisoned histories and client remain retained
    assert_eq!(
        (
            lease.epoch(),
            lease.handle(),
            lease.object(),
            lease.name().to_vec()
        ),
        history
    );
}

#[test]
fn wrong_expected_uid_and_symlink_endpoints_fail_closed() {
    let authority = Authority::new();
    assert!(matches!(
        Client::connect(&authority.path, auth::current_uid().wrapping_add(1)),
        Err(Error::Authentication(_))
    ));
    let alias = authority.directory.0.join("alias.sock");
    symlink(&authority.path, &alias).unwrap();
    assert!(matches!(
        Client::connect(&alias, auth::current_uid()),
        Err(Error::SocketIdentity(_))
    ));
    let alias_dir = authority.directory.0.join("alias-directory");
    symlink(&authority.directory.0, &alias_dir).unwrap();
    assert!(matches!(
        Client::connect(
            &alias_dir.join(authority.path.file_name().unwrap()),
            auth::current_uid()
        ),
        Err(Error::SocketIdentity(_))
    ));
    authority.finish();
}

#[test]
fn endpoint_substitution_retires_original_connection() {
    let authority = Authority::new();
    let client = authority.client();
    let lease = created(&client, b"Global\\substitution");
    let displaced = authority.directory.0.join("original.sock");
    fs::rename(&authority.path, &displaced).unwrap();
    let replacement = UnixListener::bind(&authority.path).unwrap();
    fs::set_permissions(&authority.path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(lease.check(), Err(Error::SocketIdentity(_))));
    assert!(client.inner.lock().stream.is_none());
    assert!(!lease.is_closed());
    drop(replacement);
    authority.finish_substituted();
}

#[test]
fn forged_handshake_client_identity_is_rejected() {
    let peer = CorruptPeer::raw(|stream, _| {
        assert_eq!(
            protocol::read_request(stream).unwrap(),
            Some(Request::Hello)
        );
        protocol::write_reply(
            stream,
            &Reply::Hello {
                epoch: [7; 16],
                connection: 1,
                uid: auth::current_uid().wrapping_add(1),
                session: None,
            },
        )
        .unwrap();
    });
    assert!(matches!(
        Client::connect(&peer.path, auth::current_uid()),
        Err(Error::Protocol(_))
    ));
    peer.finish();
}

#[test]
fn wrong_reply_operation_is_terminal_and_not_an_api_failure() {
    let peer = CorruptPeer::after_hello(|stream, _| {
        let request = protocol::read_request(stream).unwrap().unwrap();
        let Request::Create { op, .. } = request else {
            panic!("expected create")
        };
        protocol::write_reply(
            stream,
            &Reply::Rejected {
                op: op + 1,
                code: 5,
            },
        )
        .unwrap();
    });
    let client = peer.client();
    let operation = client.begin_create(b"Global\\wrong-operation").unwrap();
    let error = operation.resolve().unwrap_err();
    assert!(matches!(error, Error::Protocol(_)));
    assert_eq!(operation.resolve().unwrap_err(), error);
    assert!(client.inner.lock().stream.is_none());
    peer.finish();
}

#[test]
fn duplicate_issued_identity_is_staged_but_never_returned_as_success() {
    let peer = CorruptPeer::after_hello(|stream, _| {
        // Both frames pass the codec's nonzero-ID contract. The second
        // decoded reply violates the already-issued handle identity instead.
        for object in [9, 10] {
            let Some(Request::Create { op, .. }) = protocol::read_request(stream).unwrap() else {
                panic!("expected create")
            };
            protocol::write_reply(
                stream,
                &Reply::Created {
                    op,
                    handle: 7,
                    object,
                },
            )
            .unwrap();
        }
    });
    let client = peer.client();
    let first = created(&client, b"Global\\first-issued");
    let operation = client.begin_create(b"Global\\invalid-issued").unwrap();
    let error = operation.resolve().unwrap_err();
    assert!(matches!(error, Error::Protocol(_)));
    assert_eq!(operation.resolve().unwrap_err(), error);
    assert!(matches!(
        operation.inner.state.lock().unwrap().staged,
        Some(CreateOutcome::Created(_))
    ));
    assert!(client.inner.lock().stream.is_none());
    assert!(first.check().is_err());
    peer.finish();
}

#[test]
fn connect_rejects_writable_ancestor_above_private_endpoint_parent() {
    let directory = Directory::new();
    let writable = directory.0.join("writable");
    fs::create_dir(&writable).unwrap();
    fs::set_permissions(&writable, fs::Permissions::from_mode(0o777)).unwrap();
    let private = writable.join("private");
    fs::create_dir(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    let path = private.join("a.sock");
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    listener.set_nonblocking(true).unwrap();
    assert!(matches!(
        Client::connect(&path, auth::current_uid()),
        Err(Error::SocketIdentity(_))
    ));
    assert!(matches!(listener.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock));
}

#[test]
fn create_outcome_survives_post_reply_socket_validation_failure() {
    let peer = CorruptPeer::after_hello(|stream, path| {
        let Some(Request::Create { op, .. }) = protocol::read_request(stream).unwrap() else {
            panic!("expected create")
        };
        fs::rename(path, path.with_extension("displaced")).unwrap();
        let replacement = UnixListener::bind(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        protocol::write_reply(
            stream,
            &Reply::Created {
                op,
                handle: 1,
                object: 2,
            },
        )
        .unwrap();
        assert_eq!(protocol::read_request(stream).unwrap(), None);
        drop(replacement);
    });
    let client = peer.client();
    let operation = client
        .begin_create(b"Global\\staged-before-validation")
        .unwrap();
    let error = operation.resolve().unwrap_err();
    assert!(matches!(error, Error::SocketIdentity(_)));
    assert_eq!(operation.resolve().unwrap_err(), error);
    let outcome = operation
        .inner
        .state
        .lock()
        .unwrap()
        .staged
        .clone()
        .unwrap();
    let CreateOutcome::Created(lease) = outcome else {
        panic!("lost completed outcome")
    };
    assert_eq!(lease.object(), 2);
    assert!(lease.check().is_err());
    assert!(!lease.is_closed());
    assert!(client.inner.lock().stream.is_none());
    peer.finish();
}

#[test]
fn rejection_outcome_survives_post_reply_socket_validation_failure() {
    let peer = CorruptPeer::after_hello(|stream, path| {
        let Some(Request::Create { op, .. }) = protocol::read_request(stream).unwrap() else {
            panic!("expected create")
        };
        fs::rename(path, path.with_extension("displaced")).unwrap();
        let replacement = UnixListener::bind(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        protocol::write_reply(stream, &Reply::Rejected { op, code: 5 }).unwrap();
        assert_eq!(protocol::read_request(stream).unwrap(), None);
        drop(replacement);
    });
    let client = peer.client();
    let operation = client.begin_create(b"Global\\staged-rejection").unwrap();
    let error = operation.resolve().unwrap_err();
    assert!(matches!(error, Error::SocketIdentity(_)));
    assert_eq!(operation.resolve().unwrap_err(), error);
    let outcome = operation
        .inner
        .state
        .lock()
        .unwrap()
        .staged
        .clone()
        .unwrap();
    let CreateOutcome::Rejected(failure) = outcome else {
        panic!("lost actual rejection")
    };
    assert_eq!(failure.code(), 5);
    peer.finish();
}

#[test]
fn changed_live_object_identity_poisoning_preserves_history() {
    let peer = CorruptPeer::after_hello(|stream, _| {
        let Some(Request::Create { op, .. }) = protocol::read_request(stream).unwrap() else {
            panic!("expected create")
        };
        protocol::write_reply(
            stream,
            &Reply::Created {
                op,
                handle: 1,
                object: 2,
            },
        )
        .unwrap();
        let Some(Request::Check { op, .. }) = protocol::read_request(stream).unwrap() else {
            panic!("expected live check")
        };
        protocol::write_reply(stream, &Reply::Live { op, object: 3 }).unwrap();
    });
    let client = peer.client();
    let lease = created(&client, b"Global\\wrong-live-identity");
    assert!(matches!(lease.check(), Err(Error::Protocol(_))));
    assert_eq!(lease.object(), 2);
    assert_eq!(lease.name(), b"Global\\wrong-live-identity");
    assert!(client.inner.lock().stream.is_none());
    peer.finish();
}

#[test]
fn lost_close_acknowledgement_is_retained_and_never_claimed_closed() {
    let peer = CorruptPeer::after_hello(|stream, _| {
        let Some(Request::Create { op, .. }) = protocol::read_request(stream).unwrap() else {
            panic!("expected create")
        };
        protocol::write_reply(
            stream,
            &Reply::Created {
                op,
                handle: 1,
                object: 2,
            },
        )
        .unwrap();
        assert!(matches!(
            protocol::read_request(stream).unwrap(),
            Some(Request::Close { handle: 1, .. })
        ));
        // Deliberately lose the acknowledgement after receiving the close.
    });
    let client = peer.client();
    let lease = created(&client, b"Global\\uncertain-close");
    let error = lease.close().unwrap_err();
    assert!(!lease.is_closed());
    let before = client.inner.lock().ledger.len();
    assert_eq!(lease.close().unwrap_err(), error);
    let state = client.inner.lock();
    let entry = state.handles[lease.inner.slot].close_entry.unwrap();
    assert!(state.ledger[entry].attempted);
    assert!(matches!(
        state.ledger[entry].request,
        Request::Close { handle: 1, .. }
    ));
    assert!(state.ledger[entry].reply.is_none());
    assert_eq!(state.ledger.len(), before);
    assert!(state.stream.is_none());
    drop(state);
    peer.finish();
}

#[test]
fn unavailable_close_retries_exact_request_and_blocks_live_access() {
    let peer = CorruptPeer::after_hello(|stream, _| {
        let Some(Request::Create { op, .. }) = protocol::read_request(stream).unwrap() else {
            panic!("expected create")
        };
        protocol::write_reply(
            stream,
            &Reply::Created {
                op,
                handle: 1,
                object: 2,
            },
        )
        .unwrap();
        let first = protocol::read_request(stream).unwrap().unwrap();
        let Request::Close { op, .. } = &first else {
            panic!("expected close")
        };
        protocol::write_reply(stream, &Reply::Unavailable { op: *op, code: 8 }).unwrap();
        for _ in 0..2 {
            let retry = protocol::read_request(stream).unwrap().unwrap();
            assert_eq!(retry, first);
            protocol::write_reply(stream, &Reply::Unavailable { op: *op, code: 8 }).unwrap();
        }
        assert_eq!(protocol::read_request(stream).unwrap(), None);
    });
    let client = peer.client();
    let lease = created(&client, b"Global\\pending-close");
    assert!(matches!(lease.close(), Err(Error::Unavailable { .. })));
    assert!(!lease.is_closed());
    assert!(matches!(lease.check(), Err(Error::Unavailable { .. })));
    let before = client.inner.lock().ledger.len();
    assert!(matches!(lease.close(), Err(Error::Unavailable { .. })));
    assert!(!lease.is_closed());
    assert_eq!(client.inner.lock().ledger.len(), before);
    let slot = lease.inner.slot;
    drop(lease); // final-drop cleanup uses the same pending operation too
    assert!(client.inner.lock().handles[slot].close_entry.is_some());
    assert!(client.inner.lock().stream.is_some());
    drop(client); // retirement, not a fictional close, ends the obligation
    peer.finish();
}

#[test]
fn conflicting_completed_close_reply_is_terminal_and_first_reply_is_retained() {
    let peer = CorruptPeer::after_hello(|stream, _| {
        let Some(Request::Create { op, .. }) = protocol::read_request(stream).unwrap() else {
            panic!("expected create")
        };
        protocol::write_reply(
            stream,
            &Reply::Created {
                op,
                handle: 1,
                object: 2,
            },
        )
        .unwrap();
        let first = protocol::read_request(stream).unwrap().unwrap();
        let Request::Close { op, .. } = &first else {
            panic!("expected close")
        };
        protocol::write_reply(stream, &Reply::Unavailable { op: *op, code: 8 }).unwrap();
        assert_eq!(protocol::read_request(stream).unwrap(), Some(first.clone()));
        protocol::write_reply(stream, &Reply::Closed { op: *op }).unwrap();
    });
    let client = peer.client();
    let lease = created(&client, b"Global\\conflicting-close");
    assert!(matches!(lease.close(), Err(Error::Unavailable { .. })));
    assert!(matches!(lease.close(), Err(Error::Protocol(_))));
    assert!(!lease.is_closed());
    let state = client.inner.lock();
    let entry = state.handles[lease.inner.slot].close_entry.unwrap();
    assert!(matches!(
        state.ledger[entry].reply,
        Some(Reply::Unavailable { code: 8, .. })
    ));
    assert!(state.stream.is_none());
    drop(state);
    peer.finish();
}

#[test]
fn acknowledged_close_survives_post_reply_identity_failure() {
    let peer = CorruptPeer::after_hello(|stream, path| {
        let Some(Request::Create { op, .. }) = protocol::read_request(stream).unwrap() else {
            panic!("expected create")
        };
        protocol::write_reply(
            stream,
            &Reply::Created {
                op,
                handle: 1,
                object: 2,
            },
        )
        .unwrap();
        let Some(Request::Close { op, .. }) = protocol::read_request(stream).unwrap() else {
            panic!("expected close")
        };
        fs::rename(path, path.with_extension("displaced")).unwrap();
        let replacement = UnixListener::bind(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        protocol::write_reply(stream, &Reply::Closed { op }).unwrap();
        assert_eq!(protocol::read_request(stream).unwrap(), None);
        drop(replacement);
    });
    let client = peer.client();
    let lease = created(&client, b"Global\\acknowledged-before-validation");
    assert!(matches!(lease.close(), Err(Error::SocketIdentity(_))));
    assert!(lease.is_closed());
    assert_eq!(lease.check(), Err(Error::Closed));
    lease.close().unwrap();
    assert!(client.inner.lock().stream.is_none());
    peer.finish();
}

#[test]
fn partial_reply_retires_framing_and_keeps_uncertain_request() {
    use std::io::Write;
    let peer = CorruptPeer::after_hello(|stream, _| {
        assert!(matches!(
            protocol::read_request(stream).unwrap(),
            Some(Request::Create { .. })
        ));
        stream.write_all(&[1, 0]).unwrap(); // only half a length prefix, then EOF
    });
    let client = peer.client();
    let operation = client.begin_create(b"Global\\partial-reply").unwrap();
    let error = operation.resolve().unwrap_err();
    assert!(matches!(error, Error::Protocol(_)));
    assert_eq!(operation.resolve().unwrap_err(), error);
    let state = client.inner.lock();
    assert!(state.ledger[operation.inner.entry].attempted);
    assert!(state.ledger[operation.inner.entry].reply.is_none());
    assert!(state.stream.is_none());
    drop(state);
    peer.finish();
}

#[test]
fn empty_and_non_ascii_names_are_unavailable_not_fabricated_api_null() {
    let authority = Authority::new();
    let client = authority.client();
    for name in [&b""[..], &b"Global\\\xff"[..]] {
        let operation = client.begin_create(name).unwrap();
        assert!(matches!(
            operation.resolve(),
            Err(Error::Unavailable { .. })
        ));
        assert!(matches!(
            operation.resolve(),
            Err(Error::Unavailable { .. })
        ));
    }
    drop(client);
    authority.finish();
}
