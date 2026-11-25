//! Helpers shared by the integration test modules.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Held for the whole of each `SIMS_INSTALL` sweep, so at most one runs at a time.
///
/// Each sweep keeps up to 64 install archives open. As separate test binaries the
/// sweeps ran one after another; in one process they would otherwise run in parallel
/// and could exhaust the file descriptor limit. Each module has at most one sweep, so
/// this keeps the old one-at-a-time order. A panicking sweep does not block the rest.
pub fn install_sweep_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}
