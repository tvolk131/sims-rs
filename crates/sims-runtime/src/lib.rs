//! Headless live-session orchestration shared by gameplay clients and research tools.
//! Owns loading, input dispatch, service continuations and save/resume coordination.
pub mod exploration;
pub mod conformance;
pub mod fixture_replay;
pub mod original_session;

pub use original_session::{Options, Progress, Saved, Session};
