//! Explicit shared Rust-port object authority. This is not Windows kernel
//! interoperability, default-DACL equivalence, or a completed synchronization
//! backend. The supported Mac service creates, opens and closes references;
//! waiting, release and abandonment remain unavailable to source consumers.

/// Owning anonymous events for the explicit Rust port. These are separate
/// from the named-object authority and grant no Windows handle interoperation.
pub mod event;

#[cfg(unix)]
mod auth;
#[cfg(unix)]
mod client;
#[cfg(unix)]
mod endpoint;
#[cfg(unix)]
mod protocol;
#[cfg(unix)]
pub mod server;
#[cfg(unix)]
pub use client::{Client, CreateOperation, CreateOutcome, Error, Failure, Lease};
#[cfg(unix)]
pub use server::{Server, ServerSummary};

#[cfg(not(unix))]
mod unavailable;
#[cfg(not(unix))]
pub use unavailable::*;

/// Actual current host UID, never a logical/native game identity.
pub fn current_user() -> Option<u32> {
    #[cfg(unix)]
    {
        Some(auth::current_uid())
    }
    #[cfg(not(unix))]
    {
        None
    }
}
