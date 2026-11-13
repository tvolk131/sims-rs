//! Explicit private-user ASCII registry ownership for the Rust port.
//!
//! This owns the real sibling store, not Windows registry state. Parent must
//! declare both sibling modules, bind this registry to ProgramAccess and its
//! actual executable Module, and qualify any future ADVAPI32 ABI separately.
//! No import, guest output store, LSTATUS, Winerror or default setting is issued
//! here. Existing constructors and installed-import refusals remain unchanged.
//!
//! Each successful non-null call owns a fresh handle over an actual store Node.
//! The store retains keys; handle close/drop only revokes that handle. Registry
//! drop releases its Hive and issuer epoch even if a caller retains a lease.
//! Neither weak observations nor predefined root words can become ownership.

#![cfg(any(target_os = "macos", target_os = "linux"))]

use super::program_registry_store::{self, Error, ErrorKind, Hive, Node};
use crate::native_bootstrap::memory::{Identity, Memory};
use sims_sim::saved_matrix::Stop;
use std::{
    ffi::CStr,
    fmt,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

const MAX_HANDLES: usize = 4096;
const MAX_ABSOLUTE_BYTES: usize = 255;
const ROOT_LABEL_BYTES: usize = b"HKEY_CURRENT_USER".len();
const MAX_DEPTH: usize = 512;

/// No Default: the private logical user hive must be explicitly selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Disabled,
    PrivateUserAsciiV1,
}

/// Recognized PE32 root observations, not five independent backing hives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredefinedRoot {
    ClassesRoot,
    CurrentUser,
    LocalMachine,
    Users,
    CurrentConfig,
}

impl PredefinedRoot {
    /// An observed predefined constant; there is deliberately no inverse/adopt.
    pub fn word(self) -> u32 {
        match self {
            Self::ClassesRoot => 0x8000_0000,
            Self::CurrentUser => 0x8000_0001,
            Self::LocalMachine => 0x8000_0002,
            Self::Users => 0x8000_0003,
            Self::CurrentConfig => 0x8000_0005,
        }
    }
}

#[derive(Debug)]
pub enum Parent<'a> {
    Predefined(PredefinedRoot),
    Key(&'a KeyLease),
}

#[derive(Debug, Clone, Copy)]
pub enum SubKey<'a> {
    Null,
    Ansi(&'a CStr),
}

#[derive(Debug)]
pub enum Created {
    BorrowedRoot(PredefinedRoot),
    Owned(KeyLease),
}

// Only Registry owns a strong issuer reference. Weak references retain its
// allocation identity without keeping its epoch alive after Registry drop.
struct Issuer;

struct Handle {
    active: AtomicBool,
    // None exists only in a private, unpublished allocation before store commit.
    node: Option<Node>,
}

/// A non-Clone owning handle. No words, paths or public fields can construct it.
pub struct KeyLease {
    issuer: Weak<Issuer>,
    handle: Arc<Handle>,
}

impl Drop for KeyLease {
    fn drop(&mut self) {
        self.handle.active.store(false, Ordering::Release);
    }
}

impl fmt::Debug for KeyLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyLease")
            .field("issuer_retained", &(self.issuer.strong_count() != 0))
            .field("active", &self.handle.active.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

/// Clone copies only weak observations. There is no owning upgrade API.
#[derive(Clone)]
pub struct KeyObservation {
    issuer: Weak<Issuer>,
    handle: Weak<Handle>,
}

impl fmt::Debug for KeyObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyObservation")
            .field("issuer_retained", &(self.issuer.strong_count() != 0))
            .field("owner_retained", &(self.handle.strong_count() != 0))
            .finish_non_exhaustive()
    }
}

/// Owns the actual Hive, heap identity, unique issuer, and monotonic history.
/// Closed/dropped handle records remain in history until this registry drops.
/// History owns only weak references, so it cannot keep a dropped handle live.
pub struct Registry {
    hive: Hive,
    heap: Identity,
    issuer: Arc<Issuer>,
    history: Vec<Weak<Handle>>,
    max_handles: usize,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field("issued_handles", &self.history.len())
            .field("max_handles", &self.max_handles)
            .finish_non_exhaustive()
    }
}

/// A refused constructor returns the same owned Hive, including poison state
/// and retained resources. Debug requires no Debug implementation from A.
pub struct BeginError {
    pub error: Stop,
    pub hive: Hive,
}

impl fmt::Debug for BeginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BeginError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for BeginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for BeginError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

fn unavailable(message: &str) -> Stop {
    Stop::Unavailable(message.into())
}

fn store_error(error: Error) -> Stop {
    match error.kind() {
        ErrorKind::Unsupported => Stop::Unsupported(error.to_string()),
        ErrorKind::Unavailable | ErrorKind::Poisoned => Stop::Unavailable(error.to_string()),
    }
}

// Names remain exact CStr byte spans. This is a profile check, never a lookup
// or authority constructor. A additionally enforces actual missing-level and
// journal budgets using its interned tree, including existing long prefixes.
fn validate_path(parent: &[u8], relative: &[u8]) -> Result<(), Stop> {
    let prefix = parent.len().checked_add(usize::from(!parent.is_empty()))
        .ok_or_else(|| unsupported("registry path length overflow"))?;
    let relative_bytes = prefix.checked_add(relative.len())
        .ok_or_else(|| unsupported("registry path length overflow"))?;
    if relative_bytes > MAX_ABSOLUTE_BYTES - ROOT_LABEL_BYTES - 1 {
        return Err(unsupported("registry absolute path exceeds the private-user bound"));
    }
    if relative.is_empty()
        || relative.iter().any(|byte| !(0x20..=0x7e).contains(byte))
        || relative.split(|byte| *byte == b'\\').any(|part| part.is_empty())
    {
        return Err(unsupported("registry subkey is outside the nonempty printable ASCII profile"));
    }
    let parent_depth = if parent.is_empty() {
        0
    } else {
        1 + parent.iter().filter(|byte| **byte == b'\\').count()
    };
    let depth = parent_depth + 1 + relative.iter().filter(|byte| **byte == b'\\').count();
    if depth > MAX_DEPTH {
        return Err(unsupported("registry path exceeds the tree depth bound"));
    }
    Ok(())
}

impl Registry {
    pub fn new(
        memory: &Memory,
        mut hive: Hive,
        policy: Policy,
        max_handles: usize,
    ) -> Result<Self, BeginError> {
        // Refuse configuration before any Hive access, retaining this exact Hive.
        if policy != Policy::PrivateUserAsciiV1 || !(1..=MAX_HANDLES).contains(&max_handles) {
            return Err(BeginError {
                error: unsupported("registry policy or handle bound was not explicitly supported"),
                hive,
            });
        }
        let heap = memory.identity();
        if let Err(error) = hive.validate() {
            return Err(BeginError { error: store_error(error), hive });
        }
        Ok(Self {
            hive,
            heap,
            issuer: Arc::new(Issuer),
            history: Vec::new(),
            max_handles,
        })
    }

    fn check_memory(&self, memory: &Memory) -> Result<(), Stop> {
        if !memory.has_identity(&self.heap) {
            return Err(unsupported("registry belongs to a different heap lifetime"));
        }
        Ok(())
    }

    fn check_issued(&self, issuer: &Weak<Issuer>, handle: &Weak<Handle>) -> Result<(), Stop> {
        if !Weak::ptr_eq(issuer, &Arc::downgrade(&self.issuer))
            || !self.history.iter().any(|record| Weak::ptr_eq(record, handle))
        {
            return Err(unsupported("registry handle belongs to a different issuer lifetime"));
        }
        Ok(())
    }

    fn live_node<'a>(&self, key: &'a KeyLease) -> Result<&'a Node, Stop> {
        self.check_issued(&key.issuer, &Arc::downgrade(&key.handle))?;
        if !key.handle.active.load(Ordering::Acquire) {
            return Err(unsupported("registry handle is closed"));
        }
        key.handle.node.as_ref()
            .ok_or_else(|| unavailable("registry handle has no published store node"))
    }

    pub fn create(
        &mut self,
        memory: &Memory,
        parent: Parent<'_>,
        subkey: SubKey<'_>,
    ) -> Result<Created, Stop> {
        self.check_memory(memory)?;
        let relative = match subkey {
            SubKey::Null => {
                return match parent {
                    Parent::Predefined(root) => {
                        self.hive.validate().map_err(store_error)?;
                        Ok(Created::BorrowedRoot(root))
                    }
                    Parent::Key(_) => Err(unsupported("null subkey on an ordinary handle is unsupported")),
                };
            }
            SubKey::Ansi(name) => name.to_bytes(),
        };
        let parent_node = match parent {
            Parent::Predefined(PredefinedRoot::CurrentUser) => None,
            Parent::Predefined(_) => return Err(unsupported("only private current-user non-null keys are supported")),
            Parent::Key(key) => Some(self.live_node(key)?.clone()),
        };
        validate_path(
            parent_node.as_ref().map_or(&[][..], |node| node.canonical_ascii()),
            relative,
        )?;
        if self.history.len() >= self.max_handles {
            return Err(unavailable("registry lifetime handle history is exhausted"));
        }
        // Reserve every B allocation before A can commit a key. Arc metadata
        // allocation may abort on OOM, like existing services; it is not a
        // fabricated Windows failure. The unpublished Arc has no weak aliases.
        self.history.try_reserve(1)
            .map_err(|_| unavailable("registry handle history could not reserve memory"))?;
        let mut handle = Arc::new(Handle { active: AtomicBool::new(true), node: None });
        self.hive.validate().map_err(store_error)?;
        let parent_node = match parent_node {
            Some(node) => node,
            None => self.hive.root().map_err(store_error)?,
        };
        let node = self.hive.create(&parent_node, relative).map_err(store_error)?;
        // Unique by construction: no handle/observation/history was published.
        // No allocation or recoverable failure remains after store success.
        Arc::get_mut(&mut handle).expect("unpublished registry handle is unique").node = Some(node);
        self.history.push(Arc::downgrade(&handle));
        Ok(Created::Owned(KeyLease { issuer: Arc::downgrade(&self.issuer), handle }))
    }

    /// Revoke only this owned handle. Intentionally performs no Hive call, so
    /// an already-poisoned store cannot prevent close; the Hive stays retained.
    pub fn close(&mut self, memory: &Memory, key: &KeyLease) -> Result<(), Stop> {
        self.check_memory(memory)?;
        self.live_node(key)?;
        key.handle.active.store(false, Ordering::Release);
        Ok(())
    }

    pub fn observe(&self, memory: &Memory, key: &KeyLease) -> Result<KeyObservation, Stop> {
        self.check_memory(memory)?;
        self.live_node(key)?;
        Ok(KeyObservation { issuer: key.issuer.clone(), handle: Arc::downgrade(&key.handle) })
    }

    /// False means an authenticated handle was closed/dropped. Foreign issuer
    /// or heap lifetimes refuse; they cannot be adopted by an equal key path.
    pub fn observation_live(&self, memory: &Memory, key: &KeyObservation) -> Result<bool, Stop> {
        self.check_memory(memory)?;
        self.check_issued(&key.issuer, &key.handle)?;
        Ok(key.handle.upgrade().is_some_and(|handle| handle.active.load(Ordering::Acquire)))
    }

    /// Memory-only last-confirmed store counts, never handle/heap authority.
    pub fn store_snapshot(&self) -> program_registry_store::Observation {
        self.hive.snapshot()
    }
}

#[cfg(test)]
mod tests;
