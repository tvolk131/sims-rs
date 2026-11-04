//! Rust-owned resource copies. This is a host storage policy, not an original
//! IFF writer. Native chunk replacement/flush drives it through MountedStream.
use crate::Index;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
pub(super) type Shared = std::sync::Arc<std::sync::Mutex<Overlay>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Copy {
    source_sha256: [u8; 32],
    bytes: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overlay {
    entries: BTreeMap<String, Copy>,
}

impl Overlay {
    /// Explicitly grant access to copies of these exact indexed resources.
    /// Originals are only opened for reading, including loose character IFFs.
    pub fn copy(index: &Index, sources: impl IntoIterator<Item = usize>) -> Result<Self, String> {
        let mut entries = BTreeMap::new();
        for source in sources {
            let resource = index.resources.get(source).ok_or("overlay source absent")?;
            if entries.contains_key(&resource.locator) {
                return Err("duplicate overlay source".into());
            }
            let bytes = resource.read().map_err(|e| e.to_string())?;
            entries.insert(
                resource.locator.clone(),
                Copy {
                    source_sha256: Sha256::digest(&bytes).into(),
                    bytes,
                },
            );
        }
        Ok(Self { entries })
    }

    /// Rebinding a saved overlay checks exact locators and source contents.
    /// A same-named resource from a different installation is not substituted.
    pub fn validate(&self, index: &Index) -> Result<(), String> {
        for (locator, copy) in &self.entries {
            let mut found = index.resources.iter().filter(|r| &r.locator == locator);
            let resource = found.next().ok_or("overlay source no longer indexed")?;
            if found.next().is_some() {
                return Err("overlay source identity is ambiguous".into());
            }
            let bytes = resource.read().map_err(|e| e.to_string())?;
            let hash: [u8; 32] = Sha256::digest(&bytes).into();
            if hash != copy.source_sha256 {
                return Err(format!("overlay source changed: {locator}"));
            }
            if copy.bytes.len() as u64 > crate::MAX_RESOURCE {
                return Err("overlay resource exceeds 128 MiB limit".into());
            }
        }
        Ok(())
    }

    pub(super) fn read(&self, locator: &str) -> Option<&[u8]> {
        self.entries.get(locator).map(|c| c.bytes.as_slice())
    }
    pub(super) fn commit(&mut self, locator: &str, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() as u64 > crate::MAX_RESOURCE {
            return Err("overlay resource exceeds 128 MiB".into());
        }
        self.entries
            .get_mut(locator)
            .ok_or("writable overlay resource absent")?
            .bytes = bytes.to_vec();
        Ok(())
    }
}
