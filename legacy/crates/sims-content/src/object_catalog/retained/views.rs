//! Read retained normalized metadata without replaying OBJD resource loads.
use super::*;
use crate::Index;
use anyhow::Result;
use sha2::{Digest, Sha256};

impl Catalog<'_> {
    pub fn retained_neighbor_resource_flags(&self) -> Result<BTreeMap<u32, u32>, String> {
        self.ready()?;
        Ok(self
            .definitions
            .guids
            .iter()
            .map(|(&guid, &index)| (guid, self.definitions.entries[index].flags))
            .collect())
    }

    fn retained_definition(&self, ordinal: usize) -> Result<sims_vm::Definition, String> {
        self.ready()?;
        let entry = self
            .definitions
            .entries
            .get(ordinal)
            .ok_or("retained definition ordinal absent")?;
        if entry.data.len() != 216 {
            return Err("retained definition is not normalized".into());
        }
        let source = self.resources.retained_record_source(
            entry.key.resource,
            super::super::OBJD,
            entry.key.chunk,
        )?;
        Ok(sims_vm::Definition {
            source: source.into(),
            object: entry.key.chunk,
            guid: u32::from_le_bytes(entry.data[28..32].try_into().unwrap()),
            slots: u16::from_le_bytes(entry.data[40..42].try_into().unwrap()),
        })
    }

    pub fn retained_resource_storage(
        &self,
        index: &Index,
    ) -> Result<Vec<sims_vm::resource_storage::Definition>> {
        self.ready().map_err(anyhow::Error::msg)?;
        let mut records = vec![];
        let mut hashes = BTreeMap::new();
        for (ordinal, entry) in self.definitions.entries.iter().enumerate() {
            anyhow::ensure!(
                entry.data.len() == 216,
                "retained definition is not normalized"
            );
            let count = i16::from_le_bytes(entry.data[180..182].try_into().unwrap());
            if count == 0 {
                continue;
            }
            let definition = self
                .retained_definition(ordinal)
                .map_err(anyhow::Error::msg)?;
            if !hashes.contains_key(&definition.source) {
                hashes.insert(
                    definition.source.clone(),
                    format!(
                        "{:x}",
                        Sha256::digest(index.resources[index.find(&definition.source)?].read()?)
                    ),
                );
            }
            records.push(sims_vm::resource_storage::Definition {
                guid: definition.guid,
                count,
                reset_per_lot: entry.data[182] & 2 != 0,
                content_hash: hashes[&definition.source].clone(),
                source: definition.source,
                object: definition.object,
            });
        }
        Ok(records)
    }

    pub fn retained_global_controllers(
        &self,
        index: &Index,
    ) -> Result<Vec<crate::catalog::Controller>> {
        self.ready().map_err(anyhow::Error::msg)?;
        let mut controllers = vec![];
        let mut hashes = BTreeMap::new();
        for (ordinal, entry) in self.definitions.entries.iter().enumerate() {
            anyhow::ensure!(
                entry.data.len() == 216,
                "retained definition is not normalized"
            );
            let word = |at| u16::from_le_bytes(entry.data[at..at + 2].try_into().unwrap());
            let kind = word(18);
            if !matches!(kind, 7 | 12) || word(84) == 0 {
                continue;
            }
            let definition = self
                .retained_definition(ordinal)
                .map_err(anyhow::Error::msg)?;
            if !hashes.contains_key(&definition.source) {
                hashes.insert(
                    definition.source.clone(),
                    format!(
                        "{:x}",
                        Sha256::digest(index.resources[index.find(&definition.source)?].read()?)
                    ),
                );
            }
            controllers.push(crate::catalog::Controller {
                guid: definition.guid,
                label: sims_formats::strings::decode(&entry.name, 1),
                definition: crate::saves::DefinitionSource {
                    sha256: hashes[&definition.source].clone(),
                    source: definition.source,
                    chunk: definition.object,
                    kind,
                    multipart: word(20) != 0,
                },
            });
        }
        Ok(controllers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object_resources::{
        RetainedComponent, RetainedIdentities, RetainedResources, RetainedStream,
    };
    use sims_sim::object_catalog::{access::Access, composite::Composite, iff::*, resources};
    use std::sync::Arc;

    #[test]
    fn metadata_views_keep_order_and_leave_unloaded_objd_cursors_and_caches_untouched() {
        // Authored retained owner: disk payload bytes are deliberately unusable
        // as OBJD. These views must consume the captured normalized metadata.
        let bytes = vec![0; 256];
        let index = Index {
            root: "unused-metadata-test".into(),
            resources: vec![crate::Resource {
                locator: "fixture.iff".into(),
                name: "fixture.iff".into(),
                source: "must-not-open".into(),
                offset: 0,
                size: 256,
                unpacked_size: 256,
                archived: false,
                owned: Some(Arc::from(bytes.clone())),
            }],
            archives: 0,
            diagnostics: vec![],
        };
        let tag = u32::from_be_bytes(*b"OBJD");
        let mut inner = vec![vec![]; 193];
        let mut entries = vec![];
        for ordinal in 0..2usize {
            let chunk = 128 + ordinal as u16;
            inner[chunk as usize % 193].push((u32::from(chunk), ordinal));
            let mut data = vec![0; 216];
            data[18..20].copy_from_slice(&(7u16 + ordinal as u16 * 5).to_le_bytes());
            data[28..32].copy_from_slice(&55u32.to_le_bytes());
            data[84..86].copy_from_slice(&4096u16.to_le_bytes());
            data[180..182].copy_from_slice(&(ordinal as i16 + 1).to_le_bytes());
            data[182] = if ordinal == 0 { 0 } else { 2 };
            entries.push(definition::Entry {
                key: object_catalog::Key { resource: 7, chunk },
                semiglobal: None,
                sprite_resource: 3,
                global: None,
                name: b"retained".to_vec(),
                data,
                index: ordinal as u32,
                flags: u32::from(chunk) << 16,
            });
        }
        let mut outer = vec![vec![]; 193];
        outer[tag as usize % 193].push((
            tag,
            RetainedTable {
                buckets: inner,
                len: 2,
                short: true,
            },
        ));
        let iff = Iff::from_retained_parts(RetainedParts {
            valid: true,
            error: -7,
            map_offset: 176,
            unused: 0,
            write_cursor: 256,
            modified: false,
            table: RetainedTable {
                buckets: outer,
                len: 1,
                short: false,
            },
            storage: (0..2)
                .map(|i| Record {
                    tag,
                    id: 128 + i,
                    flags: 16,
                    disk_flags: 16,
                    ordinal: i + 1,
                    offset: 64 + u32::from(i) * 80,
                    name: vec![],
                    payload: None,
                })
                .collect(),
            handle_epoch: RetainedHandleEpoch::NewUnpublished { next: 1 },
        })
        .unwrap();
        let expected = iff.clone();
        let mut aggregate = Composite::empty();
        aggregate.configure(2, b".iff");
        aggregate.slots[0] = Some(11);
        let mut buckets = vec![vec![]; 193];
        let key = b"fixture.iff".to_vec();
        buckets[sims_sim::pose::clothing::name_map::hash(&key) as usize % 193].push(
            resources::Entry {
                key,
                references: 17,
                resource: 7,
            },
        );
        let installation = crate::avatar_installation::Installation::new(&index, vec![]).unwrap();
        let owner = Resources::from_retained(
            &index,
            crate::avatar_catalog::Directories::default(),
            installation.directory_search(),
            RetainedResources {
                cache_buckets: buckets,
                cache_len: 1,
                aggregates: BTreeMap::from([(7, aggregate)]),
                components: BTreeMap::from([(
                    11,
                    RetainedComponent {
                        iff,
                        stream: RetainedStream {
                            locator: "fixture.iff".into(),
                            source_sha256: Sha256::digest(&bytes).into(),
                            bytes,
                            filename: b"./fixture.iff".to_vec(),
                            access: Access {
                                opened: true,
                                writable: 0,
                            },
                            position: 123,
                            pending_backend_write_bytes: 0,
                        },
                    },
                )]),
                handles: BTreeMap::new(),
                buffers: BTreeMap::new(),
                identities: RetainedIdentities::NewUnpublished {
                    next_aggregate: 8,
                    next_component: 12,
                    next_handle: 1,
                },
            },
        )
        .unwrap();
        let mut catalog = Catalog::from_retained(
            owner,
            RetainedCatalog {
                definitions: definition::Catalog {
                    entries,
                    guids: BTreeMap::from([(55, 1)]),
                    attributes: vec![definition::Attributes {
                        guid: 55,
                        values: vec![9],
                    }],
                },
                sprite_entries: vec![sprite_pool::Entry {
                    wrapper: 3,
                    resource: 7,
                    references: 2,
                }],
                sprite_epoch: sprite_pool::IdentityEpoch::NewUnpublished { next: 4 },
                settings: RetainedSettings {
                    filter_flagged: false,
                    global_directory: b"./Global/".to_vec(),
                    global_people: 0,
                    global: Some(7),
                },
                observation_epoch: ObservationEpoch::NewUnpublished,
            },
        )
        .unwrap();
        let stores = catalog.retained_resource_storage(&index).unwrap();
        assert_eq!(
            stores
                .iter()
                .map(|r| (r.guid, r.count, r.object, r.reset_per_lot))
                .collect::<Vec<_>>(),
            [(55, 1, 128, false), (55, 2, 129, true)]
        );
        assert!(
            stores
                .iter()
                .all(|r| r.source == "fixture.iff" && r.content_hash.len() == 64)
        );
        let controllers = catalog.retained_global_controllers(&index).unwrap();
        assert_eq!(
            controllers
                .iter()
                .map(|c| c.definition.chunk)
                .collect::<Vec<_>>(),
            [128, 129]
        );
        assert_eq!(
            catalog.retained_neighbor_resource_flags().unwrap(),
            BTreeMap::from([(55, 129 << 16)])
        );
        let observed = catalog.resources.retained_global_receiver(7).unwrap();
        assert_eq!(observed.iff, &expected);
        assert_eq!(observed.position, 123);
        assert!(observed.handles.is_empty());
        assert_eq!(
            catalog.resources.cache.entries().next().unwrap().references,
            17
        );
        assert!(catalog.resources.component_opens().is_empty());
        catalog.definitions.entries[0].key.chunk = 130;
        assert!(catalog.retained_resource_storage(&index).is_err());
        assert!(catalog.retained_global_controllers(&index).is_err());
        catalog.definitions.entries[0].data.clear();
        assert!(catalog.retained_resource_storage(&index).is_err());
        assert!(catalog.retained_global_controllers(&index).is_err());
        let observed = catalog.resources.retained_global_receiver(7).unwrap();
        assert_eq!(observed.iff, &expected);
        assert_eq!(observed.position, 123);
    }
}
