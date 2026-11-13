//! Authored UNRUN. Positive cases use real owned files, never a dummy backend.

use super::*;
use super::host::Fault;
use sha2::{Digest, Sha256};
use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{symlink, DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const MAGIC: &[u8] = b"SIMS-PRIVATE-USER-ASCII\0\x01\0";
const LEAF: &str = "private-user.journal";
static SERIAL: AtomicU64 = AtomicU64::new(0);

struct Sandbox { path: PathBuf }

impl Sandbox {
    fn new() -> Self {
        // Resolve the test runner's temp parent once: production itself refuses
        // symlink ancestors (notably /var and /tmp aliases on macOS).
        let base = std::env::temp_dir().canonicalize().expect("real temp ancestor");
        for _ in 0..32 {
            let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
            let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            let path = base.join(format!("sims-registry-store-{time}-{serial}"));
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Self { path },
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create test sandbox: {e}"),
            }
        }
        panic!("test directory collision budget exhausted")
    }
    fn file(&self) -> PathBuf { self.path.join(LEAF) }
    fn create(&self) -> Hive { Hive::create_new(&self.path, OsStr::new(LEAF), limits()).unwrap() }
    fn open(&self) -> Result<Hive, Error> { Hive::open(&self.path, OsStr::new(LEAF), limits()) }
    fn write_raw(&self, bytes: &[u8]) {
        let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(self.file()).unwrap();
        f.write_all(bytes).unwrap(); f.sync_all().unwrap();
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) { fs::remove_dir_all(&self.path).expect("remove only this test sandbox"); }
}

fn limits() -> Limits { Limits { max_bytes: 1024 * 1024, max_keys: 4096, max_records: 4096 } }
fn kind<T: fmt::Debug>(result: Result<T, Error>, expected: ErrorKind) {
    assert_eq!(result.unwrap_err().kind(), expected);
}

// Independent wire-format fixture writer. It deliberately permits semantic
// errors while producing valid frame digests, so parser tests reach invariants.
fn wire(frames: &[(u32, &[&[u8]])]) -> Vec<u8> {
    let mut bytes = MAGIC.to_vec();
    let mut previous: [u8; 32] = Sha256::digest(MAGIC).into();
    for (sequence, paths) in frames {
        let len = 42 + paths.iter().map(|p| 2 + p.len()).sum::<usize>();
        let mut frame = Vec::new();
        frame.extend_from_slice(&(len as u32).to_le_bytes());
        frame.extend_from_slice(&sequence.to_le_bytes());
        frame.extend_from_slice(&(paths.len() as u16).to_le_bytes());
        for path in *paths {
            frame.extend_from_slice(&(path.len() as u16).to_le_bytes());
            frame.extend_from_slice(path);
        }
        let mut digest = Sha256::new(); digest.update(MAGIC); digest.update(previous); digest.update(&frame);
        previous = digest.finalize().into(); frame.extend_from_slice(&previous);
        bytes.extend_from_slice(&frame);
    }
    bytes
}

// Format rejection cannot establish exclusive descriptor ownership.
fn assert_independent_lock(path: &std::path::Path, busy: bool) {
    let file = OpenOptions::new().read(true).write(true).open(path).unwrap();
    // SAFETY: this separate descriptor owns the exact fixture file; flock is
    // pointer-free and explicitly nonblocking. File drop releases only its lock.
    let result = unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&file), libc::LOCK_EX | libc::LOCK_NB) };
    if busy {
        let error = std::io::Error::last_os_error();
        assert_eq!(result, -1, "retained uncertain owner released its file lock");
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    } else {
        assert_eq!(result, 0, "dropped owner retained its file lock");
    }
}

#[test]
fn exclusive_creation_real_lock_and_release() {
    let sandbox = Sandbox::new();
    let mut hive = sandbox.create();
    assert_eq!(fs::read(sandbox.file()).unwrap(), MAGIC);
    let m = fs::symlink_metadata(sandbox.file()).unwrap();
    assert!(m.is_file()); assert_eq!(m.mode() & 0o7777, 0o600); assert_eq!(m.nlink(), 1);
    assert_eq!(hive.snapshot(), Observation { keys: 1, records: 0, bytes: MAGIC.len(), poisoned: false, pending_write: false });
    assert!(hive.root().unwrap().canonical_ascii().is_empty());
    kind(sandbox.open(), ErrorKind::Unavailable); // distinct fd competes on actual flock
    kind(Hive::create_new(&sandbox.path, OsStr::new(LEAF), limits()), ErrorKind::Unavailable);
    assert_eq!(fs::read(sandbox.file()).unwrap(), MAGIC); // exclusive create never truncates
    drop(hive);
    sandbox.open().unwrap().validate().unwrap();
}

#[test]
fn nested_persistence_and_epoch_isolation() {
    let sandbox = Sandbox::new();
    let mut hive = sandbox.create();
    let root = hive.root().unwrap();
    let first = hive.create(&root, b"Software\\Maxis\\The Sims").unwrap();
    assert_eq!(first.canonical_ascii(), b"software\\maxis\\the sims");
    assert_eq!(hive.snapshot().keys, 4); assert_eq!(hive.snapshot().records, 1);
    let saved = hive.snapshot();
    let file_bytes = fs::read(sandbox.file()).unwrap();
    assert_eq!(file_bytes, wire(&[(1, &[b"software", b"software\\maxis", b"software\\maxis\\the sims"])]));
    drop(hive);
    let mut reopened = sandbox.open().unwrap();
    assert_eq!(reopened.snapshot(), saved);
    let new_root = reopened.root().unwrap();
    let second = reopened.create(&new_root, b"SOFTWARE\\MAXIS\\THE SIMS").unwrap();
    assert!(!first.same_node(&second));
    assert!(first.same_node(&first.clone())); // historical only
    kind(reopened.create(&first, b"child"), ErrorKind::Unsupported);
    assert_eq!(reopened.snapshot(), saved);
    assert_eq!(fs::read(sandbox.file()).unwrap(), file_bytes);
}

#[test]
fn interned_prefix_case_and_literal_name_bytes() {
    let sandbox = Sandbox::new(); let mut hive = sandbox.create();
    let root = hive.root().unwrap();
    let deep = hive.create(&root, b" Software \\.\\..\\a/b").unwrap();
    assert_eq!(deep.canonical_ascii(), b" software \\.\\..\\a/b");
    let before = hive.snapshot();
    let alias = hive.create(&root, b" SOFTWARE \\.\\..\\A/B").unwrap();
    assert!(deep.same_node(&alias)); assert_eq!(hive.snapshot(), before);
    let prefix = hive.create(&root, b" SOFTWARE ").unwrap();
    let via_prefix = hive.create(&prefix, b".\\..\\A/B").unwrap();
    assert!(via_prefix.same_node(&deep)); assert!(!prefix.same_node(&deep));
    assert_eq!(hive.snapshot(), before);
    assert_eq!(fs::read_dir(&sandbox.path).unwrap().count(), 1); // no filesystem name traversal
}

#[test]
fn foreign_nodes_never_adopt_equal_paths() {
    let one = Sandbox::new(); let two = Sandbox::new();
    let mut a = one.create(); let mut b = two.create();
    let ar = a.root().unwrap(); let br = b.root().unwrap();
    assert!(!ar.same_node(&br));
    let before = b.snapshot(); let bytes = fs::read(two.file()).unwrap();
    kind(b.create(&ar, b"key"), ErrorKind::Unsupported);
    assert_eq!(b.snapshot(), before); assert_eq!(fs::read(two.file()).unwrap(), bytes);
    b.create(&br, b"key").unwrap();
}

#[test]
fn path_refusal_is_prewrite_and_does_not_poison() {
    let sandbox = Sandbox::new(); let mut hive = sandbox.create(); let root = hive.root().unwrap();
    let before = hive.snapshot();
    for path in [&b""[..], b"\\a", b"a\\", b"a\\\\b", b"a\0b", b"a\x7fb", b"a\x1fb", b"a\x80b"] {
        kind(hive.create(&root, path), ErrorKind::Unsupported);
        assert_eq!(hive.snapshot(), before);
    }
    kind(hive.create(&root, &vec![b'x'; 238]), ErrorKind::Unsupported);
    assert_eq!(fs::read(sandbox.file()).unwrap(), MAGIC);
    let maximum = hive.create(&root, &vec![b'x'; 237]).unwrap();
    kind(hive.create(&maximum, b"x"), ErrorKind::Unsupported);
    hive.validate().unwrap();
}

#[test]
fn missing_level_limit_applies_to_missing_not_existing_levels() {
    let sandbox = Sandbox::new(); let mut hive = sandbox.create(); let root = hive.root().unwrap();
    let path32 = std::iter::repeat("k").take(32).collect::<Vec<_>>().join("\\");
    let path33 = format!("{path32}\\k");
    kind(hive.create(&root, path33.as_bytes()), ErrorKind::Unsupported);
    assert_eq!(hive.snapshot().records, 0);
    hive.create(&root, path32.as_bytes()).unwrap();
    let final_node = hive.create(&root, path33.as_bytes()).unwrap();
    assert_eq!(final_node.canonical_ascii(), path33.as_bytes());
    assert_eq!(hive.snapshot().keys, 34); assert_eq!(hive.snapshot().records, 2);
}

#[test]
fn limits_validate_before_any_path_access() {
    let sandbox = Sandbox::new();
    let nonexistent = sandbox.path.join("missing");
    for bad in [Limits { max_bytes: 0, ..limits() }, Limits { max_bytes: 8 * 1024 * 1024 + 1, ..limits() },
        Limits { max_keys: 0, ..limits() }, Limits { max_keys: 4097, ..limits() },
        Limits { max_records: 0, ..limits() }, Limits { max_records: 4097, ..limits() },
        Limits { max_bytes: MAGIC.len() - 1, ..limits() }]
    {
        kind(Hive::create_new(&nonexistent, OsStr::new(LEAF), bad), ErrorKind::Unsupported);
        kind(Hive::open(&nonexistent, OsStr::new(LEAF), bad), ErrorKind::Unsupported);
    }
    assert!(!nonexistent.exists()); assert!(!sandbox.file().exists());
}

#[test]
fn key_record_and_byte_budgets_are_prewrite_and_monotonic() {
    for bound in [Limits { max_keys: 2, ..limits() }, Limits { max_records: 1, ..limits() },
        Limits { max_bytes: MAGIC.len() + 45, ..limits() }]
    {
        let sandbox = Sandbox::new();
        let mut hive = Hive::create_new(&sandbox.path, OsStr::new(LEAF), bound).unwrap();
        let root = hive.root().unwrap(); hive.create(&root, b"a").unwrap();
        let before = hive.snapshot(); let bytes = fs::read(sandbox.file()).unwrap();
        kind(hive.create(&root, b"b"), ErrorKind::Unsupported);
        assert_eq!(hive.snapshot(), before); assert_eq!(fs::read(sandbox.file()).unwrap(), bytes);
        hive.create(&root, b"A").unwrap(); assert_eq!(hive.snapshot(), before);
        drop(hive);
        let mut reopened = Hive::open(&sandbox.path, OsStr::new(LEAF), bound).unwrap();
        let root = reopened.root().unwrap(); kind(reopened.create(&root, b"b"), ErrorKind::Unsupported);
        assert_eq!(reopened.snapshot(), before);
    }
}

#[test]
fn root_counts_against_key_limit_and_open_never_creates() {
    let sandbox = Sandbox::new(); kind(sandbox.open(), ErrorKind::Unavailable);
    assert!(!sandbox.file().exists());
    let bound = Limits { max_keys: 1, ..limits() };
    let mut hive = Hive::create_new(&sandbox.path, OsStr::new(LEAF), bound).unwrap();
    let root = hive.root().unwrap(); kind(hive.create(&root, b"a"), ErrorKind::Unsupported);
    assert_eq!(hive.snapshot().keys, 1); assert_eq!(fs::read(sandbox.file()).unwrap(), MAGIC);
}

#[test]
fn reopening_rejects_stricter_log_limits_without_prefix_adoption() {
    let sandbox = Sandbox::new(); let mut hive = sandbox.create(); let root = hive.root().unwrap();
    hive.create(&root, b"a").unwrap(); hive.create(&root, b"b").unwrap(); drop(hive);
    let bytes = fs::read(sandbox.file()).unwrap();
    for bound in [Limits { max_keys: 2, ..limits() }, Limits { max_records: 1, ..limits() },
        Limits { max_bytes: bytes.len() - 1, ..limits() }]
    {
        kind(Hive::open(&sandbox.path, OsStr::new(LEAF), bound), ErrorKind::Unavailable);
        assert_eq!(fs::read(sandbox.file()).unwrap(), bytes);
    }
    assert_eq!(sandbox.open().unwrap().snapshot().keys, 3);
}

#[test]
fn malformed_truncated_trailing_and_oversize_journals_refuse_whole() {
    let valid = wire(&[(1, &[b"a"])]);
    let mut cases = vec![vec![], b"regf".to_vec(), MAGIC[..MAGIC.len() - 1].to_vec()];
    for end in MAGIC.len() + 1..valid.len() { cases.push(valid[..end].to_vec()); }
    let mut flipped = valid.clone(); flipped[MAGIC.len() + 12] ^= 1; cases.push(flipped);
    let mut trailing = valid.clone(); trailing.push(0); cases.push(trailing);
    for length in [0u32, 41, 128 * 1024 + 1, u32::MAX] {
        let mut bad = valid.clone(); bad[MAGIC.len()..MAGIC.len() + 4].copy_from_slice(&length.to_le_bytes()); cases.push(bad);
    }
    for bytes in cases {
        let sandbox = Sandbox::new(); sandbox.write_raw(&bytes);
        kind(sandbox.open(), ErrorKind::Unavailable);
        assert_eq!(fs::read(sandbox.file()).unwrap(), bytes);
    }
}

#[test]
fn checksummed_semantic_corruption_refuses() {
    let fixtures = [
        wire(&[(2, &[b"a"])]), // chronology
        wire(&[(1, &[])]), // empty creation
        wire(&[(1, &[b"A"])]), // noncanonical ASCII
        wire(&[(1, &[b"a\\b"])]), // absent prefix
        wire(&[(1, &[b"a", b"b"])]), // not one consecutive suffix
        wire(&[(1, &[b"a"]), (2, &[b"a"])]), // duplicate
        wire(&[(1, &[b"a\\\\b"])]),
        wire(&[(1, &[b"a\0b"])]),
        wire(&[(1, &[b"a\x80b"])]),
    ];
    for bytes in fixtures {
        let sandbox = Sandbox::new(); sandbox.write_raw(&bytes);
        let error = sandbox.open().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unavailable); assert!(error.retained.is_some());
        assert_eq!(fs::read(sandbox.file()).unwrap(), bytes);
    }
}

#[test]
fn checksummed_excessive_name_and_missing_level_counts_refuse() {
    let long = vec![b'x'; 238];
    let path_storage = (1..=33).map(|n| std::iter::repeat("a").take(n).collect::<Vec<_>>().join("\\")).collect::<Vec<_>>();
    let paths = path_storage.iter().map(|p| p.as_bytes()).collect::<Vec<_>>();
    for bytes in [wire(&[(1, &[&long])]), wire(&[(1, &paths)])] {
        let sandbox = Sandbox::new(); sandbox.write_raw(&bytes);
        kind(sandbox.open(), ErrorKind::Unavailable);
        assert_eq!(fs::read(sandbox.file()).unwrap(), bytes);
    }
}

#[test]
fn symlink_leaf_and_ancestor_are_refused_without_following() {
    let sandbox = Sandbox::new(); let target = Sandbox::new(); let hive = target.create(); drop(hive);
    symlink(target.file(), sandbox.file()).unwrap();
    kind(sandbox.open(), ErrorKind::Unavailable);
    kind(Hive::create_new(&sandbox.path, OsStr::new(LEAF), limits()), ErrorKind::Unavailable);
    let alias = sandbox.path.join("alias"); symlink(&target.path, &alias).unwrap();
    kind(Hive::open(&alias, OsStr::new(LEAF), limits()), ErrorKind::Unavailable);
    assert_eq!(fs::read(target.file()).unwrap(), MAGIC);
    fs::remove_file(sandbox.file()).unwrap(); fs::remove_file(alias).unwrap();
}

#[test]
fn unsafe_leaf_names_and_relative_directories_refuse_before_create() {
    let sandbox = Sandbox::new();
    for leaf in [&b""[..], b".", b"..", b"a/b", b"a\0b"] {
        kind(Hive::create_new(&sandbox.path, OsStr::from_bytes(leaf), limits()), ErrorKind::Unsupported);
    }
    kind(Hive::create_new(Path::new("."), OsStr::new(LEAF), limits()), ErrorKind::Unsupported);
    assert_eq!(fs::read_dir(&sandbox.path).unwrap().count(), 0);
}

#[test]
fn wrong_modes_hardlinks_and_special_files_refuse() {
    let sandbox = Sandbox::new(); let hive = sandbox.create(); drop(hive);
    fs::set_permissions(&sandbox.path, Permissions::from_mode(0o755)).unwrap();
    kind(sandbox.open(), ErrorKind::Unavailable);
    fs::set_permissions(&sandbox.path, Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(sandbox.file(), Permissions::from_mode(0o640)).unwrap();
    kind(sandbox.open(), ErrorKind::Unavailable);
    fs::set_permissions(sandbox.file(), Permissions::from_mode(0o600)).unwrap();
    let alias = sandbox.path.join("second-link"); fs::hard_link(sandbox.file(), &alias).unwrap();
    kind(sandbox.open(), ErrorKind::Unavailable); fs::remove_file(alias).unwrap();
    fs::remove_file(sandbox.file()).unwrap();
    let cpath = std::ffi::CString::new(sandbox.file().as_os_str().as_bytes()).unwrap();
    // SAFETY: exact path inside this unique test-owned directory.
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    kind(sandbox.open(), ErrorKind::Unavailable); // must not block opening a FIFO
}

#[test]
fn live_path_replacement_poison_keeps_original_lock() {
    let sandbox = Sandbox::new(); let mut hive = sandbox.create(); let root = hive.root().unwrap();
    let old = sandbox.path.join("retained-original"); fs::rename(sandbox.file(), &old).unwrap();
    sandbox.write_raw(MAGIC);
    kind(hive.create(&root, b"a"), ErrorKind::Unavailable);
    assert!(hive.snapshot().poisoned); assert!(!hive.snapshot().pending_write);
    kind(Hive::open(&sandbox.path, OsStr::new("retained-original"), limits()), ErrorKind::Unavailable);
    kind(hive.root(), ErrorKind::Poisoned); assert_eq!(fs::read(&old).unwrap(), MAGIC);
    assert_eq!(fs::read(sandbox.file()).unwrap(), MAGIC);
}

#[test]
fn live_directory_replacement_refuses_even_with_same_leaf_name() {
    let outer = Sandbox::new(); let original = outer.path.join("private");
    DirBuilder::new().mode(0o700).create(&original).unwrap();
    let mut hive = Hive::create_new(&original, OsStr::new(LEAF), limits()).unwrap();
    let moved = outer.path.join("moved"); fs::rename(&original, &moved).unwrap();
    DirBuilder::new().mode(0o700).create(&original).unwrap();
    kind(hive.validate(), ErrorKind::Unavailable); assert!(hive.snapshot().poisoned);
    assert_eq!(fs::read(moved.join(LEAF)).unwrap(), MAGIC); assert!(!original.join(LEAF).exists());
}

#[test]
fn live_same_length_byte_tampering_is_detected() {
    let sandbox = Sandbox::new(); let mut hive = sandbox.create(); let root = hive.root().unwrap();
    hive.create(&root, b"a").unwrap(); let before = hive.snapshot();
    let mut bytes = fs::read(sandbox.file()).unwrap(); let i = MAGIC.len() + 12; bytes[i] = b'b';
    let mut f = OpenOptions::new().write(true).open(sandbox.file()).unwrap();
    f.write_all(&bytes).unwrap(); f.sync_all().unwrap(); drop(f);
    kind(hive.validate(), ErrorKind::Unavailable);
    assert_eq!(hive.snapshot(), Observation { poisoned: true, ..before });
    kind(hive.create(&root, b"c"), ErrorKind::Poisoned);
    assert_eq!(fs::read(sandbox.file()).unwrap(), bytes);
}

#[test]
fn live_size_mode_and_link_changes_refuse_without_appending() {
    for change in 0..3 {
        let sandbox = Sandbox::new(); let mut hive = sandbox.create(); let root = hive.root().unwrap();
        match change {
            0 => OpenOptions::new().write(true).open(sandbox.file()).unwrap().set_len(1).unwrap(),
            1 => fs::set_permissions(sandbox.file(), Permissions::from_mode(0o644)).unwrap(),
            _ => fs::hard_link(sandbox.file(), sandbox.path.join("alias")).unwrap(),
        }
        let changed_bytes = fs::read(sandbox.file()).unwrap();
        kind(hive.create(&root, b"a"), ErrorKind::Unavailable);
        assert_eq!(hive.snapshot().keys, 1); assert!(hive.snapshot().poisoned);
        assert!(!hive.snapshot().pending_write); assert_eq!(fs::read(sandbox.file()).unwrap(), changed_bytes);
    }
}

#[test]
fn partial_append_retains_unpublished_nodes_intent_file_and_lock() {
    let sandbox = Sandbox::new(); let mut hive = sandbox.create(); let root = hive.root().unwrap();
    let before = hive.snapshot(); hive.inject_once(Fault::Partial(7));
    kind(hive.create(&root, b"a\\b"), ErrorKind::Unavailable);
    let failed_bytes = fs::read(sandbox.file()).unwrap(); assert_eq!(failed_bytes.len(), MAGIC.len() + 7);
    assert_eq!(hive.snapshot(), Observation { poisoned: true, pending_write: true, ..before });
    assert_eq!(hive.retained_pending_keys(), 2);
    assert_independent_lock(&sandbox.file(), true);
    kind(hive.create(&root, b"a\\b"), ErrorKind::Poisoned);
    kind(hive.validate(), ErrorKind::Poisoned); kind(hive.root(), ErrorKind::Poisoned);
    kind(sandbox.open(), ErrorKind::Unavailable);
    assert_eq!(fs::read(sandbox.file()).unwrap(), failed_bytes);
    drop(hive);
    assert_independent_lock(&sandbox.file(), false);
    // Complete validation still refuses a partial file: no prefix replay/repair.
    kind(sandbox.open(), ErrorKind::Unavailable);
    assert_eq!(fs::read(sandbox.file()).unwrap(), failed_bytes);
}

#[test]
fn lost_sync_acknowledgement_never_publishes_or_retries() {
    let sandbox = Sandbox::new(); let mut hive = sandbox.create(); let root = hive.root().unwrap();
    let before = hive.snapshot(); hive.inject_once(Fault::SyncLoss);
    kind(hive.create(&root, b"a"), ErrorKind::Unavailable);
    let bytes = fs::read(sandbox.file()).unwrap(); assert_eq!(bytes, wire(&[(1, &[b"a"])]));
    assert_eq!(hive.snapshot(), Observation { poisoned: true, pending_write: true, ..before });
    assert_eq!(hive.retained_pending_keys(), 1);
    assert_independent_lock(&sandbox.file(), true);
    kind(hive.create(&root, b"a"), ErrorKind::Poisoned); kind(sandbox.open(), ErrorKind::Unavailable);
    assert_eq!(fs::read(sandbox.file()).unwrap(), bytes);
    drop(hive);
    assert_independent_lock(&sandbox.file(), false);
}

#[test]
fn constructor_uncertainty_retains_resources_and_never_deletes() {
    for fault in [Fault::Partial(5), Fault::SyncLoss] {
        let sandbox = Sandbox::new(); Hive::inject_constructor_once(fault);
        let error = Hive::create_new(&sandbox.path, OsStr::new(LEAF), limits()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unavailable); assert!(error.retained.is_some());
        let bytes = fs::read(sandbox.file()).unwrap(); assert!(!bytes.is_empty());
        assert_independent_lock(&sandbox.file(), true);
        kind(sandbox.open(), ErrorKind::Unavailable); // Separate format-refusal check
        kind(Hive::create_new(&sandbox.path, OsStr::new(LEAF), limits()), ErrorKind::Unavailable);
        assert_eq!(fs::read(sandbox.file()).unwrap(), bytes);
        drop(error); assert_eq!(fs::read(sandbox.file()).unwrap(), bytes);
        assert_independent_lock(&sandbox.file(), false);
    }
}

#[test]
#[ignore = "requires an explicitly isolated root test runner for real foreign-uid ownership"]
fn foreign_owned_real_leaf_is_refused() {
    // SAFETY: own credentials only. This test is opt-in, never silently emulated.
    assert_eq!(unsafe { libc::getuid() }, 0);
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let sandbox = Sandbox::new(); let hive = sandbox.create(); drop(hive);
    let path = std::ffi::CString::new(sandbox.file().as_os_str().as_bytes()).unwrap();
    // SAFETY: only this fixture's file; no host/installed paths or recursive chown.
    assert_eq!(unsafe { libc::chown(path.as_ptr(), 1, !0 as libc::gid_t) }, 0);
    assert_eq!(fs::metadata(sandbox.file()).unwrap().uid(), 1);
    kind(sandbox.open(), ErrorKind::Unavailable); assert_eq!(fs::read(sandbox.file()).unwrap(), MAGIC);
    // SAFETY: restore only this fixture's leaf, then change only its directory.
    assert_eq!(unsafe { libc::chown(path.as_ptr(), 0, !0 as libc::gid_t) }, 0);
    let directory = std::ffi::CString::new(sandbox.path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::chown(directory.as_ptr(), 1, !0 as libc::gid_t) }, 0);
    kind(sandbox.open(), ErrorKind::Unavailable);
}
