//! Helpers the integration test modules share.

use sha2::{Digest, Sha256};

/// The SHA-256 of `bytes`, in lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
