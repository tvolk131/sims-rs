//! Recoverable rejection of explicit player Load before load-owner preparation.
//!
//! Parent integration contract:
//! - Check the existing application's/menu's/VM's health before explicit Load.
//! - Call `preflight(path, |input| Some(&input.saved().source) ==
//!   self.args.original.as_ref())` before resets, restore or native preparation.
//!   The predicate is the existing exact String comparison, with no path policy.
//! - Move the returned Input into `Viewer::load_prepared_with_index(input,
//!   index) -> Result<()>`, the parent's factored, work-scoped loader. Do not
//!   reopen the path, recreate a Saved or replace its private origin observation.
//! - Only the explicit Message::Load preflight error branch calls `recover`.
//!   Rejection returns before the Load common tail AND preparation_result;
//!   ordinary read-only feedback publication must not certify an older input.
//! - Startup propagates these errors. Restore/native/generation/publication and
//!   common-tail errors propagate without a preflight context. Success feedback
//!   is retained only after the entire normal Load tail succeeds.
//!
//! Viewer fields, consuming-loader factoring and retained feedback/UI belong to
//! the parent. This module has no Viewer, engine, registration or UI mutations.
use anyhow::Result;
use sims_runtime::original_session::player_input::Input;
use std::{
    fmt,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy)]
enum Operation {
    Read,
    House,
}

/// Private provenance, attached only to the two preflight failure boundaries.
/// An arbitrary I/O/decode/native error must never opt itself into recovery.
#[derive(Debug)]
struct PreflightFailure {
    destination: PathBuf,
    operation: Operation,
}

impl fmt::Display for PreflightFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.operation {
            Operation::Read => "could not read or decode the player save",
            Operation::House => "snapshot belongs to a different original house",
        };
        write!(f, "Load failed for {}: {reason}", self.destination.display())
    }
}

impl std::error::Error for PreflightFailure {}

/// Read the actual envelope once and return its same, non-Clone owned Input.
/// `same_house` must be the caller's pure, existing house predicate. It runs
/// once, only after a successful read/decode, and cannot replace that Input.
pub(super) fn preflight(path: &Path, same_house: impl FnOnce(&Input) -> bool) -> Result<Input> {
    let failure = |operation| PreflightFailure {
        destination: path.to_path_buf(),
        operation,
    };
    let input = Input::read(path).map_err(|error| error.context(failure(Operation::Read)))?;
    if !same_house(&input) {
        return Err(anyhow::Error::new(failure(Operation::House)));
    }
    Ok(input)
}

/// Recognize typed preflight provenance, never error text or a generic I/O kind.
/// The caller has already refused an unhealthy owner and must return from the
/// rejected Load branch without restore, normal Load preparation or accounting.
pub(super) fn recover(error: anyhow::Error) -> Result<String> {
    if error.downcast_ref::<PreflightFailure>().is_none() {
        return Err(error);
    }
    Ok(format!(
        "{error:#}. Load was rejected before replacing the current household. \
         Check the save path, readable player-save contents and original house, then retry Load."
    ))
}

#[cfg(test)]
#[path = "load_feedback/tests.rs"]
mod tests;
