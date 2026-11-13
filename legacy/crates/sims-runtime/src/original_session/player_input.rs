//! Same-open-object observation of the real player application envelope.
//! An observation is diagnostic input identity, never routing admission.
use super::{Saved, Session};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sims_content::Index;
use std::{fs::{File, Metadata}, io::Read, path::Path};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Descriptor {
    pub bytes: u64,
    pub regular: bool,
    pub modified_unix_ns: Option<String>,
    pub device: Option<u64>,
    pub inode: Option<u64>,
    pub mode: Option<u32>,
    pub change_seconds: Option<i64>,
    pub change_nanoseconds: Option<i64>,
}
impl Descriptor {
    fn observe(metadata: Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            bytes: metadata.len(),
            regular: metadata.is_file(),
            modified_unix_ns: metadata.modified().ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos().to_string()),
            #[cfg(unix)] device: Some(metadata.dev()),
            #[cfg(not(unix))] device: None,
            #[cfg(unix)] inode: Some(metadata.ino()),
            #[cfg(not(unix))] inode: None,
            #[cfg(unix)] mode: Some(metadata.mode()),
            #[cfg(not(unix))] mode: None,
            #[cfg(unix)] change_seconds: Some(metadata.ctime()),
            #[cfg(not(unix))] change_seconds: None,
            #[cfg(unix)] change_nanoseconds: Some(metadata.ctime_nsec()),
            #[cfg(not(unix))] change_nanoseconds: None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Observation {
    pub boundary: &'static str,
    pub sha256: String,
    pub byte_count: u64,
    /// Requested spelling only. No canonical-path or pathname-lock claim.
    pub requested_path: String,
    pub path_identity_observed: bool,
    pub descriptor_before: Option<Descriptor>,
    pub descriptor_after: Option<Descriptor>,
    pub descriptor_status: &'static str,
}
impl Observation {
    pub fn eligible_input(&self) -> bool { self.descriptor_status == "consistent" }
}
fn descriptor_status(before: Option<&Descriptor>, after: Option<&Descriptor>, count: u64) -> &'static str {
    let (Some(before), Some(after)) = (before, after) else { return "unavailable"; };
    if before != after || !before.regular || before.bytes != count { return "inconsistent"; }
    if before.device.is_none() || before.inode.is_none() || before.mode.is_none()
        || before.modified_unix_ns.is_none() || before.change_seconds.is_none()
        || before.change_nanoseconds.is_none() { return "unavailable"; }
    "consistent"
}

/// Fields are deliberately private: neither Saved nor digest is substitutable.
/// There is no Clone, from-typed-Saved constructor or arbitrary origin setter.
pub struct Input {
    raw: Vec<u8>,
    envelope: Value,
    saved: Saved,
    observation: Observation,
}
impl Input {
    pub fn read(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        let before = file.metadata().ok().map(Descriptor::observe);
        let mut raw = Vec::new();
        file.read_to_end(&mut raw)?;
        let after = file.metadata().ok().map(Descriptor::observe);
        Self::decode(raw, path.to_string_lossy().into_owned(), before, after)
    }
    fn decode(raw: Vec<u8>, requested_path: String, before: Option<Descriptor>, after: Option<Descriptor>) -> Result<Self> {
        // Hash these exact bytes BEFORE JSON parsing. No reopening/reserialization.
        let byte_count = u64::try_from(raw.len()).context("player input byte count overflow")?;
        let sha256 = format!("{:x}", Sha256::digest(&raw));
        let descriptor_status = descriptor_status(before.as_ref(), after.as_ref(), byte_count);
        let envelope: Value = serde_json::from_slice(&raw)?;
        let object = envelope.as_object().context("player application envelope must be an object")?;
        let saved: Saved = serde_json::from_value(object.get("session")
            .context("player application envelope session absent")?.clone())?;
        // Reject the actual exploration schema/session/application boundary.
        // Other saved-field decoding (including historically absent selected)
        // remains with the existing player loader, preserving its defaults and
        // house/routing/restore checks before application-field validation.
        ensure!(!(object.contains_key("schema") && object.contains_key("application")
            && !["floor", "zoom", "resident"].iter().any(|key| object.contains_key(*key))),
            "exploration checkpoints are not player application envelopes");
        Ok(Self { raw, envelope, saved, observation: Observation {
            boundary: "player_application_envelope", sha256, byte_count, requested_path,
            path_identity_observed: false, descriptor_before: before, descriptor_after: after,
            descriptor_status,
        } })
    }
    pub fn saved(&self) -> &Saved { &self.saved }
    pub fn envelope(&self) -> &Value { &self.envelope }
    pub fn observation(&self) -> &Observation { &self.observation }
    pub fn raw_bytes(&self) -> &[u8] { &self.raw }
    /// Only this path attaches an observed origin, after the ordinary restore
    /// validates its OWN privately held Saved. Failure yields no Session.
    pub fn restore(self, index: &Index) -> Result<(Session, Value)> {
        let Self { raw: _, envelope, saved, observation } = self;
        let mut session = Session::restore(index, saved)?;
        session.loaded_input_origin = Some(observation);
        Ok((session, envelope))
    }
}
#[cfg(test)]
#[path = "player_input/tests.rs"]
mod tests;
