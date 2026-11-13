//! AUTHORED, ALL UNRUN. Parent must merge real A and declare both modules.
//! These fixtures use actual private files, Hive methods and Memory identities;
//! there is no substitute backend, success callback or guessed Windows result.

use super::*;
use super::program_registry_store::Limits;
use std::{
    ffi::{CString, OsStr},
    fs::{self, DirBuilder, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::{fd::AsRawFd, unix::fs::DirBuilderExt},
    path::PathBuf,
    sync::atomic::AtomicU64,
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
const LEAF: &str = "private-user.v1";

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        // Canonicalize only the parent's chosen test temp base; A itself must
        // still perform its contracted no-symlink descriptor walk/validation.
        let base = std::env::temp_dir().canonicalize().unwrap();
        for _ in 0..64 {
            let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("sims-registry-b-{}-{serial}", std::process::id()));
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("exclusive test directory failed: {error}"),
            }
        }
        panic!("exclusive test directory names exhausted");
    }

    fn path(&self) -> PathBuf {
        self.0.join(LEAF)
    }

    fn create(&self, limits: Limits) -> Hive {
        Hive::create_new(&self.0, OsStr::new(LEAF), limits).unwrap()
    }

    fn open(&self, limits: Limits) -> Hive {
        Hive::open(&self.0, OsStr::new(LEAF), limits).unwrap()
    }

    fn bytes(&self) -> Vec<u8> {
        fs::read(self.path()).unwrap()
    }

    fn assert_retained_lock(&self, limits: Limits) {
        match Hive::open(&self.0, OsStr::new(LEAF), limits) {
            Err(error) => assert_eq!(error.kind(), ErrorKind::Unavailable),
            Ok(_) => panic!("a second Hive acquired an already-owned file"),
        }
    }

    fn assert_descriptor_lock_retained(&self) {
        // Independent oracle even when journal bytes are damaged: a second
        // Hive::open could otherwise refuse for format, hiding a released lock.
        let file = OpenOptions::new().read(true).write(true).open(self.path()).unwrap();
        // SAFETY: this is our live, separately opened owned fixture descriptor;
        // flock takes no pointers and the request is explicitly nonblocking.
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        let error = std::io::Error::last_os_error();
        assert_eq!(result, -1, "the original Hive released its exclusive lock");
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        // Ordinary File drop only; no manual unlock/retry of another owner.
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        // Cleanup is fixture-owned and only runs when the parent runs tests.
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn limits() -> Limits {
    Limits { max_bytes: 256 * 1024, max_keys: 256, max_records: 256 }
}

fn heap() -> Memory {
    Memory::new(8192).unwrap()
}

fn registry(directory: &Directory, memory: &Memory, max_handles: usize) -> Registry {
    Registry::new(memory, directory.create(limits()), Policy::PrivateUserAsciiV1, max_handles).unwrap()
}

fn create(registry: &mut Registry, memory: &Memory, parent: Parent<'_>, bytes: &[u8]) -> KeyLease {
    let name = CString::new(bytes).unwrap();
    match registry.create(memory, parent, SubKey::Ansi(&name)).unwrap() {
        Created::Owned(key) => key,
        Created::BorrowedRoot(_) => panic!("non-null create returned a borrowed root"),
    }
}

fn current(registry: &mut Registry, memory: &Memory, bytes: &[u8]) -> KeyLease {
    create(registry, memory, Parent::Predefined(PredefinedRoot::CurrentUser), bytes)
}

fn unsupported<T>(result: Result<T, Stop>) {
    match result {
        Err(Stop::Unsupported(_)) => (),
        Err(other) => panic!("expected Unsupported, got {other:?}"),
        Ok(_) => panic!("unsupported operation succeeded"),
    }
}

fn unavailable<T>(result: Result<T, Stop>) {
    match result {
        Err(Stop::Unavailable(_)) => (),
        Err(other) => panic!("expected Unavailable, got {other:?}"),
        Ok(_) => panic!("unavailable operation succeeded"),
    }
}

fn mapped_kind(kind: ErrorKind, error: Stop) {
    match kind {
        ErrorKind::Unsupported => unsupported::<()>(Err(error)),
        ErrorKind::Unavailable | ErrorKind::Poisoned => unavailable::<()>(Err(error)),
    }
}

#[test]
fn registry_b_all_null_predefined_results_stay_borrowed_even_at_handle_limit() {
    let directory = Directory::new();
    let memory = heap();
    let mut registry = registry(&directory, &memory, 1);
    let roots = [
        (PredefinedRoot::ClassesRoot, 0x8000_0000),
        (PredefinedRoot::CurrentUser, 0x8000_0001),
        (PredefinedRoot::LocalMachine, 0x8000_0002),
        (PredefinedRoot::Users, 0x8000_0003),
        (PredefinedRoot::CurrentConfig, 0x8000_0005),
    ];
    let before = registry.store_snapshot();
    let bytes = directory.bytes();
    for (root, word) in roots {
        assert_eq!(root.word(), word);
        match registry.create(&memory, Parent::Predefined(root), SubKey::Null).unwrap() {
            Created::BorrowedRoot(actual) => assert_eq!(actual, root),
            Created::Owned(_) => panic!("predefined null result became an owned handle"),
        }
    }
    assert_eq!(registry.store_snapshot(), before);
    assert_eq!(directory.bytes(), bytes);
    let key = current(&mut registry, &memory, b"Owned");
    let full = registry.store_snapshot();
    for (root, _) in roots {
        assert!(matches!(registry.create(&memory, Parent::Predefined(root), SubKey::Null),
            Ok(Created::BorrowedRoot(actual)) if actual == root));
    }
    assert_eq!(registry.store_snapshot(), full);
    registry.close(&memory, &key).unwrap();
}

#[test]
fn registry_b_begin_refusal_returns_same_hive_epoch_and_real_lock() {
    let directory = Directory::new();
    let memory = heap();
    let mut hive = directory.create(limits());
    let original_root = hive.root().unwrap();
    let original = hive.create(&original_root, b"Retained").unwrap();
    let before = hive.snapshot();
    for (policy, bound) in [
        (Policy::Disabled, 1),
        (Policy::PrivateUserAsciiV1, 0),
        (Policy::PrivateUserAsciiV1, 4097),
        (Policy::PrivateUserAsciiV1, usize::MAX),
    ] {
        let refusal = Registry::new(&memory, hive, policy, bound).unwrap_err();
        unsupported::<()>(Err(refusal.error));
        hive = refusal.hive;
        directory.assert_retained_lock(limits());
        assert_eq!(hive.snapshot(), before);
        let root = hive.root().unwrap();
        assert!(root.same_node(&original_root));
        assert!(hive.create(&root, b"rETAINED").unwrap().same_node(&original));
        assert_eq!(hive.snapshot(), before);
    }
    let mut registry = Registry::new(&memory, hive, Policy::PrivateUserAsciiV1, 1).unwrap();
    let key = current(&mut registry, &memory, b"RETAINED");
    assert_eq!(registry.store_snapshot(), before);
    registry.close(&memory, &key).unwrap();
}

#[test]
fn registry_b_disabled_and_invalid_bound_do_not_validate_unlinked_backing_path() {
    let directory = Directory::new();
    let memory = heap();
    let hive = directory.create(limits());
    let before = hive.snapshot();
    fs::rename(directory.path(), directory.0.join("moved.v1")).unwrap();
    let refusal = Registry::new(&memory, hive, Policy::Disabled, 1).unwrap_err();
    unsupported::<()>(Err(refusal.error));
    assert_eq!(refusal.hive.snapshot(), before);
    let refusal = Registry::new(&memory, refusal.hive, Policy::PrivateUserAsciiV1, 0).unwrap_err();
    unsupported::<()>(Err(refusal.error));
    assert_eq!(refusal.hive.snapshot(), before);
    // A valid enabled constructor must actually notice the missing path. Its
    // BeginError retains that unusable Hive; this test performs no restoration.
    let refusal = Registry::new(&memory, refusal.hive, Policy::PrivateUserAsciiV1, 1).unwrap_err();
    let retained = refusal.hive.snapshot();
    assert_eq!((retained.keys, retained.records, retained.bytes), (before.keys, before.records, before.bytes));
    assert!(directory.0.join("moved.v1").is_file());
}

#[test]
fn registry_b_duplicate_opens_close_independently_and_keys_persist_after_reopen() {
    let directory = Directory::new();
    let memory = heap();
    let mut registry = registry(&directory, &memory, 8);
    let first = current(&mut registry, &memory, b"Software\\Game");
    let stored = registry.store_snapshot();
    assert_eq!(stored.keys, 3); // Actual root and two stored components.
    let bytes = directory.bytes();
    let second = current(&mut registry, &memory, b"sOFTWARE\\gAME");
    assert_eq!(registry.store_snapshot(), stored);
    assert_eq!(directory.bytes(), bytes);
    let first_view = registry.observe(&memory, &first).unwrap();
    let second_view = registry.observe(&memory, &second).unwrap();
    registry.close(&memory, &first).unwrap();
    assert!(!registry.observation_live(&memory, &first_view).unwrap());
    assert!(registry.observation_live(&memory, &second_view).unwrap());
    unsupported(registry.close(&memory, &first));
    unsupported(registry.observe(&memory, &first));
    unsupported(registry.create(&memory, Parent::Key(&first), SubKey::Ansi(c"Unused")));
    let child = create(&mut registry, &memory, Parent::Key(&second), b"Options");
    assert_eq!(registry.store_snapshot().keys, 4);
    let child_view = registry.observe(&memory, &child).unwrap();
    let confirmed = registry.store_snapshot();
    let bytes = directory.bytes();
    drop(second);
    assert!(!registry.observation_live(&memory, &second_view).unwrap());
    assert!(registry.observation_live(&memory, &child_view).unwrap());
    registry.close(&memory, &child).unwrap();
    assert!(!registry.observation_live(&memory, &child_view).unwrap());
    assert_eq!(registry.store_snapshot(), confirmed);
    assert_eq!(directory.bytes(), bytes);
    drop(registry);
    let mut reopened = directory.open(limits());
    let root = reopened.root().unwrap();
    let persisted = reopened.create(&root, b"SOFTWARE\\GAME\\OPTIONS").unwrap();
    assert!(persisted.canonical_ascii().eq_ignore_ascii_case(b"Software\\Game\\Options"));
    assert_eq!(reopened.snapshot(), confirmed);
    assert_eq!(directory.bytes(), bytes);
}

#[test]
fn registry_b_heap_swap_refuses_every_operation_before_store_access_or_close() {
    let directory = Directory::new();
    let mut memory = heap();
    let mut replacement = heap();
    let mut registry = registry(&directory, &memory, 8);
    let key = current(&mut registry, &memory, b"Existing");
    let observation = registry.observe(&memory, &key).unwrap();
    let before = registry.store_snapshot();
    let bytes = directory.bytes();
    std::mem::swap(&mut memory, &mut replacement);
    fs::rename(directory.path(), directory.0.join("moved.v1")).unwrap();
    unsupported(registry.create(&memory, Parent::Predefined(PredefinedRoot::CurrentUser), SubKey::Null));
    unsupported(registry.create(&memory, Parent::Predefined(PredefinedRoot::CurrentUser), SubKey::Ansi(c"New")));
    unsupported(registry.create(&memory, Parent::Key(&key), SubKey::Ansi(c"Child")));
    unsupported(registry.close(&memory, &key));
    unsupported(registry.observe(&memory, &key));
    unsupported(registry.observation_live(&memory, &observation));
    assert_eq!(registry.store_snapshot(), before);
    assert_eq!(fs::read(directory.0.join("moved.v1")).unwrap(), bytes);
    // Identity travels with Memory, not with the address of the first variable.
    assert!(registry.observation_live(&replacement, &observation).unwrap());
    registry.close(&replacement, &key).unwrap();
    assert!(!registry.observation_live(&replacement, &observation).unwrap());
    assert_eq!(registry.store_snapshot(), before);
}

#[test]
fn registry_b_equal_names_and_same_heap_do_not_authorize_foreign_handles() {
    let left_directory = Directory::new();
    let right_directory = Directory::new();
    let memory = heap();
    let mut left = registry(&left_directory, &memory, 4);
    let mut right = registry(&right_directory, &memory, 4);
    let left_key = current(&mut left, &memory, b"Same");
    let right_key = current(&mut right, &memory, b"Same");
    let left_view = left.observe(&memory, &left_key).unwrap();
    let right_view = right.observe(&memory, &right_key).unwrap();
    let before = right.store_snapshot();
    let bytes = right_directory.bytes();
    unsupported(right.close(&memory, &left_key));
    unsupported(right.observe(&memory, &left_key));
    unsupported(right.observation_live(&memory, &left_view));
    unsupported(right.create(&memory, Parent::Key(&left_key), SubKey::Ansi(c"Child")));
    assert_eq!(right.store_snapshot(), before);
    assert_eq!(right_directory.bytes(), bytes);
    assert!(left.observation_live(&memory, &left_view).unwrap());
    assert!(right.observation_live(&memory, &right_view).unwrap());
}

#[test]
fn registry_b_registry_drop_releases_hive_and_reopen_cannot_adopt_retained_lease() {
    let directory = Directory::new();
    let memory = heap();
    let mut original = registry(&directory, &memory, 4);
    let old_key = current(&mut original, &memory, b"Same");
    let old_view = original.observe(&memory, &old_key).unwrap();
    let copied_view = old_view.clone();
    let before = original.store_snapshot();
    drop(original);
    // Retained leases/weak observations must not retain the Hive's real lock.
    let mut replacement = Registry::new(&memory, directory.open(limits()), Policy::PrivateUserAsciiV1, 4).unwrap();
    let fresh = current(&mut replacement, &memory, b"SAME");
    assert_eq!(replacement.store_snapshot(), before);
    unsupported(replacement.close(&memory, &old_key));
    unsupported(replacement.observe(&memory, &old_key));
    unsupported(replacement.observation_live(&memory, &old_view));
    unsupported(replacement.observation_live(&memory, &copied_view));
    unsupported(replacement.create(&memory, Parent::Key(&old_key), SubKey::Ansi(c"Child")));
    let fresh_view = replacement.observe(&memory, &fresh).unwrap();
    assert!(replacement.observation_live(&memory, &fresh_view).unwrap());
    assert_eq!(replacement.store_snapshot(), before);
}

#[test]
fn registry_b_closed_and_dropped_handles_exhaust_monotonic_history_before_io() {
    let directory = Directory::new();
    let memory = heap();
    let mut registry = registry(&directory, &memory, 2);
    let first = current(&mut registry, &memory, b"Existing");
    let first_view = registry.observe(&memory, &first).unwrap();
    registry.close(&memory, &first).unwrap();
    let second = current(&mut registry, &memory, b"EXISTING");
    let second_view = registry.observe(&memory, &second).unwrap();
    drop(second);
    assert!(!registry.observation_live(&memory, &first_view).unwrap());
    assert!(!registry.observation_live(&memory, &second_view).unwrap());
    let before = registry.store_snapshot();
    let bytes = directory.bytes();
    // A store validation would now fail. History exhaustion must precede it.
    fs::rename(directory.path(), directory.0.join("moved.v1")).unwrap();
    unavailable(registry.create(&memory, Parent::Predefined(PredefinedRoot::CurrentUser), SubKey::Ansi(c"Uncreated")));
    unavailable(registry.create(&memory, Parent::Predefined(PredefinedRoot::CurrentUser), SubKey::Ansi(c"Existing")));
    assert_eq!(registry.store_snapshot(), before);
    assert_eq!(fs::read(directory.0.join("moved.v1")).unwrap(), bytes);
}

#[test]
fn registry_b_unsupported_roots_null_handle_and_text_refuse_without_mutation() {
    let directory = Directory::new();
    let memory = heap();
    let mut registry = registry(&directory, &memory, 1);
    let before = registry.store_snapshot();
    let bytes = directory.bytes();
    for root in [PredefinedRoot::ClassesRoot, PredefinedRoot::LocalMachine, PredefinedRoot::Users, PredefinedRoot::CurrentConfig] {
        unsupported(registry.create(&memory, Parent::Predefined(root), SubKey::Ansi(c"Name")));
    }
    let too_long = vec![b'x'; 238];
    for invalid in [
        &b""[..], &b"\\Leading"[..], &b"Trailing\\"[..], &b"Two\\\\Parts"[..],
        &b"Control\x1f"[..], &b"Delete\x7f"[..], &b"NonAscii\x80"[..], &too_long[..],
    ] {
        let name = CString::new(invalid).unwrap();
        unsupported(registry.create(&memory, Parent::Predefined(PredefinedRoot::CurrentUser), SubKey::Ansi(&name)));
        assert_eq!(registry.store_snapshot(), before);
        assert_eq!(directory.bytes(), bytes);
    }
    // Refusals consume neither a handle record nor the single issuance budget.
    let key = current(&mut registry, &memory, b"Actual");
    let observation = registry.observe(&memory, &key).unwrap();
    let before = registry.store_snapshot();
    unsupported(registry.create(&memory, Parent::Key(&key), SubKey::Null));
    assert_eq!(registry.store_snapshot(), before);
    assert!(registry.observation_live(&memory, &observation).unwrap());
}

#[test]
fn registry_b_spaces_slashes_and_dots_are_registry_bytes_not_filesystem_paths() {
    let directory = Directory::new();
    let memory = heap();
    let mut registry = registry(&directory, &memory, 4);
    let key = current(&mut registry, &memory, b" A/..\\ . \\End ");
    let before = registry.store_snapshot();
    let bytes = directory.bytes();
    let duplicate = current(&mut registry, &memory, b" a/..\\ . \\END ");
    assert_eq!(registry.store_snapshot(), before);
    registry.close(&memory, &key).unwrap();
    registry.close(&memory, &duplicate).unwrap();
    drop(registry);
    let mut hive = directory.open(limits());
    let root = hive.root().unwrap();
    let node = hive.create(&root, b" A/..\\ . \\End ").unwrap();
    assert!(node.canonical_ascii().eq_ignore_ascii_case(b" A/..\\ . \\End "));
    assert_eq!(hive.snapshot(), before);
    assert_eq!(directory.bytes(), bytes);
}

#[test]
fn registry_b_absolute_bound_includes_actual_parent_and_root_label() {
    let directory = Directory::new();
    let memory = heap();
    let mut registry = registry(&directory, &memory, 4);
    let longest = vec![b'a'; 237]; // 17-byte root + separator + 237 = 255.
    let key = current(&mut registry, &memory, &longest);
    let before = registry.store_snapshot();
    let bytes = directory.bytes();
    unsupported(registry.create(&memory, Parent::Key(&key), SubKey::Ansi(c"x")));
    let excessive = CString::new(vec![b'b'; 238]).unwrap();
    unsupported(registry.create(&memory, Parent::Predefined(PredefinedRoot::CurrentUser), SubKey::Ansi(&excessive)));
    assert_eq!(registry.store_snapshot(), before);
    assert_eq!(directory.bytes(), bytes);
    // Boundary reopening still issues a fresh handle without another record.
    let second = current(&mut registry, &memory, &vec![b'A'; 237]);
    registry.close(&memory, &key).unwrap();
    let view = registry.observe(&memory, &second).unwrap();
    assert!(registry.observation_live(&memory, &view).unwrap());
    assert_eq!(registry.store_snapshot(), before);
}

#[test]
fn registry_b_32_missing_levels_uses_real_tree_not_relative_component_count() {
    let directory = Directory::new();
    let memory = heap();
    let mut registry = registry(&directory, &memory, 8);
    let prefix = vec!["x"; 32].join("\\");
    let key = current(&mut registry, &memory, prefix.as_bytes());
    assert_eq!(registry.store_snapshot().keys, 33);
    let before = registry.store_snapshot();
    let bytes = directory.bytes();
    let excessive = CString::new(vec!["y"; 33].join("\\")).unwrap();
    unsupported(registry.create(&memory, Parent::Predefined(PredefinedRoot::CurrentUser), SubKey::Ansi(&excessive)));
    assert_eq!(registry.store_snapshot(), before);
    assert_eq!(directory.bytes(), bytes);
    let full = format!("{prefix}\\z"); // 33 components, only ONE missing.
    let extended = current(&mut registry, &memory, full.as_bytes());
    assert_eq!(registry.store_snapshot().keys, 34);
    let before = registry.store_snapshot();
    let duplicate = current(&mut registry, &memory, full.as_bytes());
    assert_eq!(registry.store_snapshot(), before);
    registry.close(&memory, &key).unwrap();
    registry.close(&memory, &extended).unwrap();
    registry.close(&memory, &duplicate).unwrap();
}

#[test]
fn registry_b_real_key_and_record_budget_errors_map_without_issuing_a_handle() {
    for constrained in [
        Limits { max_keys: 2, ..limits() },
        Limits { max_records: 1, ..limits() },
    ] {
        let directory = Directory::new();
        let memory = heap();
        let mut hive = directory.create(constrained);
        let root = hive.root().unwrap();
        hive.create(&root, b"Existing").unwrap();
        let before = hive.snapshot();
        let actual_error = match hive.create(&root, b"Uncreated") {
            Err(error) => error,
            Ok(_) => panic!("actual store exceeded its key/record budget"),
        };
        let expected_kind = actual_error.kind();
        assert_eq!(hive.snapshot(), before);
        let bytes = directory.bytes();
        let mut registry = Registry::new(&memory, hive, Policy::PrivateUserAsciiV1, 1).unwrap();
        let error = registry.create(&memory, Parent::Predefined(PredefinedRoot::CurrentUser), SubKey::Ansi(c"Uncreated")).unwrap_err();
        mapped_kind(expected_kind, error);
        assert_eq!(registry.store_snapshot(), before);
        assert_eq!(directory.bytes(), bytes);
        // The failed real-store operation did not consume the only B record.
        let key = current(&mut registry, &memory, b"EXISTING");
        assert_eq!(registry.store_snapshot(), before);
        registry.close(&memory, &key).unwrap();
    }
}

#[test]
fn registry_b_real_store_failure_retains_resources_and_close_still_revokes() {
    let directory = Directory::new();
    let memory = heap();
    let mut registry = registry(&directory, &memory, 4);
    let key = current(&mut registry, &memory, b"Existing");
    let observation = registry.observe(&memory, &key).unwrap();
    let before = registry.store_snapshot();
    // Same-UID noncooperating tampering is explicitly outside A's protection;
    // its actual descriptor validation must nevertheless refuse these bytes.
    let mut file = OpenOptions::new().read(true).write(true).open(directory.path()).unwrap();
    let mut first = [0u8; 1];
    file.read_exact(&mut first).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&[first[0] ^ 0xff]).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let damaged = directory.bytes();
    assert!(registry.create(&memory, Parent::Predefined(PredefinedRoot::CurrentUser), SubKey::Ansi(c"MustNotSucceed")).is_err());
    assert!(registry.create(&memory, Parent::Predefined(PredefinedRoot::ClassesRoot), SubKey::Null).is_err());
    let failed = registry.store_snapshot();
    assert_eq!((failed.keys, failed.records, failed.bytes), (before.keys, before.records, before.bytes));
    registry.close(&memory, &key).unwrap();
    assert!(!registry.observation_live(&memory, &observation).unwrap());
    unsupported(registry.close(&memory, &key));
    // Close must not retry/validate/repair the retained store, including any
    // poison/pending flags A set for the original failure.
    assert_eq!(registry.store_snapshot(), failed);
    assert_eq!(directory.bytes(), damaged);
    directory.assert_descriptor_lock_retained();
    assert!(Hive::create_new(&directory.0, OsStr::new(LEAF), limits()).is_err());
    // Move the ORIGINAL real Hive out inside this child test module and prove
    // the failure did not substitute/drop it or release its descriptor lock.
    let Registry { hive, .. } = registry;
    directory.assert_descriptor_lock_retained();
    assert_eq!(hive.snapshot(), failed);
    assert_eq!(directory.bytes(), damaged);
    drop(hive);
    assert!(Hive::open(&directory.0, OsStr::new(LEAF), limits()).is_err());
}


#[test]
fn registry_b_pending_append_and_sync_loss_close_revokes_without_retry_or_unlock() {
    for lost_sync in [false, true] {
        let directory = Directory::new();
        let memory = heap();
        let mut registry = registry(&directory, &memory, 8);
        let key = current(&mut registry, &memory, b"Existing");
        let observation = registry.observe(&memory, &key).unwrap();
        let confirmed = registry.store_snapshot();
        registry.hive.inject_parent_test_write_failure_once(lost_sync);
        unavailable(registry.create(&memory, Parent::Predefined(PredefinedRoot::CurrentUser),
            SubKey::Ansi(c"Uncertain")));
        let pending = registry.store_snapshot();
        assert_eq!((pending.keys, pending.records, pending.bytes),
                   (confirmed.keys, confirmed.records, confirmed.bytes));
        assert!(pending.poisoned && pending.pending_write);
        let uncertain_bytes = directory.bytes();
        directory.assert_descriptor_lock_retained();
        registry.close(&memory, &key).unwrap();
        assert!(!registry.observation_live(&memory, &observation).unwrap());
        unsupported(registry.close(&memory, &key));
        unavailable(registry.create(&memory, Parent::Predefined(PredefinedRoot::CurrentUser),
            SubKey::Ansi(c"MustNotRetry")));
        assert_eq!(registry.store_snapshot(), pending);
        assert_eq!(directory.bytes(), uncertain_bytes);
        directory.assert_descriptor_lock_retained();
        drop(registry);
        // Format validity is independent of the lock oracle, including the
        // genuinely truncated append. The old lease/observation cannot retain
        // the file owner or acquire a new hive epoch after Registry drops.
        let file = OpenOptions::new().read(true).write(true).open(directory.path()).unwrap();
        assert_eq!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, 0);
        drop(file);
        assert_eq!(directory.bytes(), uncertain_bytes);
    }
}
