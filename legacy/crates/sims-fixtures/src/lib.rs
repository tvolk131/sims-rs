//! Versioned research fixtures and evidence, independent of simulation/rendering.
//! Storage integrity never implies original-engine semantic parity. Captures,
//! normalized observations and resumable checkpoints have distinct schemas.
pub mod archive;
pub mod legacy;
pub mod model;
pub mod readset;
pub mod store;
pub mod trace;

pub use model::*;
pub use store::{BlobRef, Compression, GcPreview, Pin, RunReport, Store, VerificationReport};

/// SHA-256 of exact uncompressed payload bytes. Compression is not identity.
pub fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
