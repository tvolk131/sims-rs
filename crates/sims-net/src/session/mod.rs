//! Deterministic co-op session state and host-sequenced lockstep scheduler.
//!
//! - [`state`]: Pure deterministic [`SessionState`] implementing multi-reason
//!   pause, Mode Global 19 aggregation, optional host speed lock with player
//!   attribution, first-answer-wins blocking dialogs, per-player `Selected
//!   Person ID` / `camera_rotation` queries, and deterministic host failover
//!   selection.
//! - [`lockstep`]: [`LockstepNode`] driving both host and peer lockstep flows
//!   over any [`crate::transport::Transport`], including adaptive pacing to the
//!   slowest machine, periodic fingerprint comparison, drift resync, mid-game
//!   join snapshot transfer, and deterministic host failover.

pub mod lockstep;
pub mod state;

pub use lockstep::{HostStepResult, LockstepNode, NodeEvent};
pub use state::{
    ActivePauseReasons, AppliedBatch, AppliedCommand, BlockingDialog, CommandIgnoreReason,
    CommandOutcome, CommandRejectReason, PerPlayerState, SessionState, TimeControlState,
};
