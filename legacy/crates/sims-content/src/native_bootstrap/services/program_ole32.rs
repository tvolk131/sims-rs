//! Explicit balance of a genuine owned Rust-port STA initialization.
//!
//! Parent wiring must first authenticate ProgramAccess, the actual Module and
//! Memory, and the exact issuing-service apartment capability and containing
//! Application owner. The genuine Return is an additional Rust ownership
//! witness; it is not native ABI proof or a logical apartment integer.
//!
//! This backend has no Windows COM activation, guest DLL/RPC/conversation
//! resources or message pumping. Explicit release and fallback last-owner Rust
//! Drop do not provide those native CoUninitialize effects. Neither admits a
//! constructor return, source shutdown caller or Application/Game/World run.
//! This module performs no registry, export/import or IAT/source wiring;
//! ADVAPI32 dependency1 remains unsupported with zero IAT/source effects.

use sims_sim::{native_application::com, saved_matrix::Stop};

/// No Default: the parent must explicitly enable the owned Rust STA producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Disabled,
    OwnedRustSta,
}

/// Balance exactly the supplied successful Return's one shared obligation.
/// Success is void, including for a nested S_FALSE initialization; no HRESULT
/// or NULL is manufactured. All parent authentication must precede this call,
/// with no fallible post-release wiring that could cause a retry of the effect.
pub(super) fn uninitialize(policy: Policy, returned: &com::Return) -> Result<(), Stop> {
    match policy {
        Policy::Disabled => Err(Stop::Unsupported(
            "owned Rust STA balance was not explicitly enabled".into(),
        )),
        Policy::OwnedRustSta => returned.uninitialize_rust_port(),
    }
}

#[cfg(test)]
mod tests;
