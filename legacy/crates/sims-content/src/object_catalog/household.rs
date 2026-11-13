//! Explicit Complete Collection mounts for the selected household's neighborhood.
//! This uses the verified scan/registration order; it does not emulate installation.
use super::{Catalog, Settings};
use crate::{
    Index,
    avatar_installation::{Binding, Installation, complete_collection_asset_bindings},
    object_resources::{Overlay, Resources},
};
use anyhow::{Context, Result, ensure};
use std::sync::Arc;

impl Catalog<'static> {
    pub fn household(
        index: Arc<Index>,
        neighborhood_source: &str,
        overlay: Overlay,
    ) -> Result<Self> {
        let (directory, file) = neighborhood_source
            .rsplit_once('/')
            .context("neighborhood path absent")?;
        ensure!(
            file.eq_ignore_ascii_case("Neighborhood.iff")
                && !directory.contains(['/', '\\'])
                && directory.to_ascii_lowercase().starts_with("userdata"),
            "unsupported neighborhood directory"
        );
        let people = format!("./{directory}/Characters/").into_bytes();
        let mut bindings = complete_collection_asset_bindings(&index.root);
        bindings.push(Binding {
            directory: people.clone(),
            physical: index.root.join(directory).join("Characters"),
        });
        let mut installation = Installation::new(&index, bindings)?;
        let initial = installation.initial_mounts(b"./", 255, false)?;
        installation.register(&people, false, true)?;
        let mut paths = sims_sim::object_scan::Paths {
            objects: initial[1].clone(),
            user_objects: initial[2].clone(),
            // Fresh startup scans installed objects before the neighborhood
            // supplies its Characters path. Retain that registration history;
            // moving people earlier changes every later catalog index.
            people: vec![],
            downloads: initial.last().unwrap().clone(),
            expansions: std::array::from_fn(|n| initial[3 + n].clone()),
            shared: initial[11].clone(),
            npcs: initial[12].clone(),
        };
        let mut resources = Resources::owned(
            index.clone(),
            installation.directories()?,
            installation.directory_search(),
            overlay,
        )
        .map_err(anyhow::Error::msg)?;
        let global = resources
            .acquire(&[initial[0].as_slice(), b"Global"].concat(), 0)
            .map_err(anyhow::Error::msg)?
            .context("global resource absent")?;
        let mut catalog = Catalog::new(
            resources,
            Settings {
                filter_flagged: true,
                global_directory: initial[0].clone(),
                global_people: 0,
                global: Some(global),
            },
        );
        installation.scan_objects(&paths, 255, 0, false, &mut catalog)?;
        paths = sims_sim::object_scan::Paths {
            people,
            ..Default::default()
        };
        installation.scan_objects(&paths, 255, 0, true, &mut catalog)?;
        Ok(catalog)
    }
}
impl Catalog<'_> {
    ///43ba52 creates shared attribute records in registration order, including
    ///duplicate GUIDs. Use the same normalized, accepted definitions as objects.
    pub fn resource_storage(
        &mut self,
        index: &Index,
    ) -> Result<Vec<sims_vm::resource_storage::Definition>> {
        use sha2::{Digest, Sha256};
        self.ready().map_err(anyhow::Error::msg)?;
        let mut records = vec![];
        let mut hashes = std::collections::BTreeMap::new();
        for row in 0..self.definitions.entries.len() {
            let data = &self.definitions.entries[row].data;
            let count = i16::from_le_bytes(data[180..182].try_into().unwrap());
            if count == 0 {
                continue;
            }
            let reset_per_lot = data[182] & 2 != 0;
            let definition = self
                .portrait_definition(row as u32 + 1)
                .map_err(anyhow::Error::msg)?;
            if !hashes.contains_key(&definition.source) {
                let bytes = index.resources[index.find(&definition.source)?].read()?;
                hashes.insert(
                    definition.source.clone(),
                    format!("{:x}", Sha256::digest(bytes)),
                );
            }
            records.push(sims_vm::resource_storage::Definition {
                guid: definition.guid,
                count,
                reset_per_lot,
                content_hash: hashes[&definition.source].clone(),
                source: definition.source,
                object: definition.object,
            });
        }
        Ok(records)
    }

    /// 442b9f consumes the registered resource vector, including distinct
    /// resources with the same GUID. Never replace it with source-path order.
    pub fn global_controllers(&mut self, index: &Index) -> Result<Vec<crate::catalog::Controller>> {
        use sha2::{Digest, Sha256};
        let mut controllers = vec![];
        let mut hashes = std::collections::BTreeMap::new();
        for row in 0..self.definitions.entries.len() {
            let entry = &self.definitions.entries[row];
            let word =
                |offset| u16::from_le_bytes(entry.data[offset..offset + 2].try_into().unwrap());
            let kind = word(18);
            if !matches!(kind, 7 | 12) || word(84) == 0 {
                continue;
            }
            let multipart = word(20) != 0;
            let label = sims_formats::strings::decode(&entry.name, 1);
            let definition = self
                .portrait_definition(row as u32 + 1)
                .map_err(anyhow::Error::msg)?;
            if !hashes.contains_key(&definition.source) {
                let bytes = index.resources[index.find(&definition.source)?].read()?;
                hashes.insert(
                    definition.source.clone(),
                    format!("{:x}", Sha256::digest(bytes)),
                );
            }
            controllers.push(crate::catalog::Controller {
                guid: definition.guid,
                label,
                definition: crate::saves::DefinitionSource {
                    sha256: hashes[&definition.source].clone(),
                    source: definition.source,
                    chunk: definition.object,
                    kind,
                    multipart,
                },
            });
        }
        Ok(controllers)
    }
    pub fn identity(&self) -> Result<String, String> {
        use sha2::{Digest, Sha256};
        self.ready()?;
        // Normalized definition contents plus ordered resource identities. This
        // fingerprint is internal snapshot version 1, not an original file hash.
        Ok(format!(
            "{:x}",
            Sha256::digest(format!("{:?}/{:?}", self.definitions, self.visits).as_bytes())
        ))
    }
}
