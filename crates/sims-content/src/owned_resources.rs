//! Save-owned neighborhood resources mounted over immutable installation data.
//! Resource identity is its locator; physical paths are never serialized here.
use crate::{Index, MAX_RESOURCE, Resource};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub locator: String,
    pub name: String,
    pub sha256: String,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neighborhood {
    pub directory: String,
    pub entries: Vec<Entry>,
}
impl Neighborhood {
    pub fn capture(index: &Index, source: &str) -> Result<Self> {
        let (directory, file) = source
            .rsplit_once('/')
            .context("owned neighborhood path absent")?;
        ensure!(
            file.eq_ignore_ascii_case("Neighborhood.iff"),
            "owned neighborhood source differs"
        );
        validate_directory(directory)?;
        let prefix = format!("{directory}/").to_ascii_lowercase();
        let mut entries = vec![];
        for r in &index.resources {
            let path = r.locator.to_ascii_lowercase();
            if !path.starts_with(&prefix) || !(path.ends_with(".iff") || path.ends_with(".fam")) {
                continue;
            }
            ensure!(
                !r.archived,
                "archived mutable neighborhood resource unsupported"
            );
            let bytes = r.read()?;
            entries.push(Entry {
                locator: r.locator.clone(),
                name: r.name.clone(),
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                bytes,
            });
        }
        let result = Self {
            directory: directory.into(),
            entries,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<()> {
        validate_directory(&self.directory)?;
        let prefix = format!("{}/", self.directory).to_ascii_lowercase();
        let mut keys = BTreeSet::new();
        let mut size = 0u64;
        for e in &self.entries {
            let key = e.locator.to_ascii_lowercase();
            ensure!(
                key.starts_with(&prefix)
                    && !key.contains(['\\', ':', '\0'])
                    && key
                        .split('/')
                        .all(|p| !p.is_empty() && p != "." && p != ".."),
                "invalid owned resource locator"
            );
            ensure!(
                e.locator.rsplit('/').next() == Some(e.name.as_str()),
                "owned resource basename differs"
            );
            ensure!(keys.insert(key), "duplicate owned resource locator");
            size = size
                .checked_add(e.bytes.len() as u64)
                .context("owned size overflow")?;
            ensure!(size <= MAX_RESOURCE, "owned neighborhood exceeds128MiB");
            ensure!(
                format!("{:x}", Sha256::digest(&e.bytes)) == e.sha256,
                "owned resource hash differs: {}",
                e.locator
            );
        }
        ensure!(
            keys.contains(&format!("{prefix}neighborhood.iff")),
            "owned neighborhood absent"
        );
        Ok(())
    }
    /// Replace the whole selected save namespace, including files absent from
    /// this checkpoint. Other neighborhoods cannot satisfy a missing lookup.
    pub fn mount(&self, index: &Index) -> Result<Index> {
        self.validate()?;
        let prefix = format!("{}/", self.directory).to_ascii_lowercase();
        let mut result = index.clone();
        result
            .resources
            .retain(|r| !r.locator.to_ascii_lowercase().starts_with(&prefix));
        for e in &self.entries {
            result.resources.push(Resource {
                locator: e.locator.clone(),
                name: e.name.clone(),
                source: result.root.join(&e.locator),
                offset: 0,
                size: e.bytes.len() as u64,
                unpacked_size: e.bytes.len() as u64,
                archived: false,
                owned: Some(Arc::from(e.bytes.clone())),
            });
        }
        result.resources.sort_by(|a, b| a.locator.cmp(&b.locator));
        Ok(result)
    }
    pub fn hashes(&self) -> BTreeMap<String, String> {
        self.entries
            .iter()
            .map(|e| (e.locator.clone(), e.sha256.clone()))
            .collect()
    }
}
fn validate_directory(d: &str) -> Result<()> {
    let lower = d.to_ascii_lowercase();
    ensure!(
        lower
            .strip_prefix("userdata")
            .is_some_and(|tail| tail.bytes().all(|b| b.is_ascii_digit())),
        "unsupported owned neighborhood directory"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(locator: &str, bytes: &[u8]) -> Entry {
        Entry {
            locator: locator.into(),
            name: locator.rsplit('/').next().unwrap().into(),
            sha256: format!("{:x}", Sha256::digest(bytes)),
            bytes: bytes.into(),
        }
    }
    fn owned() -> Neighborhood {
        Neighborhood {
            directory: "UserData".into(),
            entries: vec![
                entry("UserData/Neighborhood.iff", b"owned-neighborhood"),
                entry("UserData/Characters/User00001.iff", b"owned-character"),
            ],
        }
    }
    #[test]
    fn namespace_mount_reads_owned_bytes_and_removes_stale_files() -> Result<()> {
        let root = std::path::PathBuf::from("/absent-managed-save-installation");
        let old = Resource {
            locator: "UserData/Characters/Stale.iff".into(),
            name: "Stale.iff".into(),
            source: root.join("UserData/Characters/Stale.iff"),
            offset: 0,
            size: 0,
            unpacked_size: 0,
            archived: false,
            owned: None,
        };
        let index = Index {
            root,
            resources: vec![old],
            archives: 0,
            diagnostics: vec![],
        };
        let mounted = owned().mount(&index)?;
        ensure!(mounted.find("Stale.iff").is_err());
        let character = &mounted.resources[mounted.find("UserData/Characters/User00001.iff")?];
        ensure!(character.read()? == b"owned-character");
        // The asset manager enumerates the owned Characters tree even though
        // there is deliberately no physical installation at this root.
        let installation = crate::avatar_installation::Installation::new(
            &mounted,
            vec![crate::avatar_installation::Binding {
                directory: b"./UserData/Characters/".to_vec(),
                physical: mounted.root.join("UserData/Characters"),
            }],
        )?;
        ensure!(installation.physical_exists(b"./UserData/Characters/User00001.iff")?);
        ensure!(!installation.physical_exists(b"./UserData/Characters/Stale.iff")?);
        Ok(())
    }
    #[test]
    fn corrupt_or_ambiguous_owned_namespace_is_rejected() {
        let mut invalid = owned();
        invalid.entries[0].bytes[0] ^= 1;
        assert!(invalid.validate().is_err());
        let mut invalid = owned();
        invalid
            .entries
            .push(entry("UserData/characters/user00001.iff", b"collision"));
        assert!(invalid.validate().is_err());
        for bad in [
            "UserData/../Other.iff",
            "UserData/Characters//Other.iff",
            "UserData2/Other.iff",
            "UserData/Characters/C:\\Other.iff",
        ] {
            let mut invalid = owned();
            invalid.entries.push(entry(bad, b"bad"));
            assert!(invalid.validate().is_err(), "accepted {bad}");
        }
    }
}
