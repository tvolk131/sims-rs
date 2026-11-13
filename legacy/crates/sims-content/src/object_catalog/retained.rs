//! Observed catalog metadata and aliases, with a new empty diagnostic history.
//! Definition-owned BodyStrings and renderer caches are separate consumers;
//! importing this owner does not manufacture either of those states.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) mod strict;
mod views;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObservationEpoch {
    NewUnpublished,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedSettings {
    pub filter_flagged: bool,
    pub global_directory: Vec<u8>,
    pub global_people: u8,
    #[serde(deserialize_with = "required_option")]
    pub global: Option<u32>,
}

fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedCatalog {
    #[serde(deserialize_with = "strict::catalog")]
    pub definitions: definition::Catalog,
    pub sprite_entries: Vec<sprite_pool::Entry>,
    pub sprite_epoch: sprite_pool::IdentityEpoch,
    pub settings: RetainedSettings,
    pub observation_epoch: ObservationEpoch,
}

impl<'a> Catalog<'a> {
    pub(crate) fn retained_global_resource(&self) -> Result<u32, String> {
        self.ready()?;
        self.settings
            .global
            .ok_or_else(|| "retained catalog Global owner absent".into())
    }

    /// Join complete already retained resources and metadata without registering
    /// files, loading chunks, rebuilding definitions, or replaying notifications.
    pub fn from_retained(
        resources: Resources<'a>,
        retained: RetainedCatalog,
    ) -> Result<Self, String> {
        let RetainedCatalog {
            definitions,
            sprite_entries,
            sprite_epoch,
            settings,
            observation_epoch,
        } = retained;
        let ObservationEpoch::NewUnpublished = observation_epoch;
        let sprites = sprite_pool::Pool::from_retained(sprite_entries, sprite_epoch)?;
        let mut guids = BTreeMap::new();
        for (index, entry) in definitions.entries.iter().enumerate() {
            if entry.data.len() != 216
                || entry.index as usize != index
                || entry.key.chunk != (entry.flags >> 16) as u16
                || !resources.retained_has_aggregate(entry.key.resource)
                || entry
                    .semiglobal
                    .is_some_and(|r| !resources.retained_has_aggregate(r))
                || entry
                    .global
                    .is_some_and(|r| !resources.retained_has_aggregate(r))
                || !sprites
                    .entries
                    .iter()
                    .any(|s| s.wrapper == entry.sprite_resource && s.resource == entry.key.resource)
            {
                return Err("invalid retained catalog definition/owner binding".into());
            }
            guids.insert(
                u32::from_le_bytes(entry.data[28..32].try_into().unwrap()),
                index,
            );
        }
        if guids != definitions.guids
            || settings
                .global
                .is_some_and(|r| !resources.retained_has_aggregate(r))
            || settings.global_directory.contains(&0)
        {
            return Err("invalid retained catalog GUID/settings owner".into());
        }
        let mut attribute_guids = BTreeSet::new();
        for attribute in &definitions.attributes {
            if !definitions.guids.contains_key(&attribute.guid)
                || !attribute_guids.insert(attribute.guid)
            {
                return Err("invalid retained shared attribute owner".into());
            }
        }
        let keys = definitions.entries.iter().map(|e| e.key).collect();
        Ok(Self {
            definitions,
            resources,
            sprites,
            keys,
            settings: Settings {
                filter_flagged: settings.filter_flagged,
                global_directory: settings.global_directory,
                global_people: settings.global_people,
                global: settings.global,
            },
            visits: vec![],
            notifications: 0,
            fault: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object_resources::{RetainedIdentities, RetainedResources};

    fn index() -> crate::Index {
        crate::Index {
            root: "unused-catalog-import".into(),
            resources: vec![],
            archives: 0,
            diagnostics: vec![],
        }
    }
    fn resources(index: &crate::Index) -> Resources<'_> {
        let installation = Installation::new(index, vec![]).unwrap();
        Resources::from_retained(
            index,
            crate::avatar_catalog::Directories::default(),
            installation.directory_search(),
            RetainedResources {
                cache_buckets: vec![vec![]; 193],
                cache_len: 0,
                aggregates: BTreeMap::from([(7, object_catalog::composite::Composite::empty())]),
                components: BTreeMap::new(),
                handles: BTreeMap::new(),
                buffers: BTreeMap::new(),
                identities: RetainedIdentities::NewUnpublished {
                    next_aggregate: 8,
                    next_component: 1,
                    next_handle: 1,
                },
            },
        )
        .unwrap()
    }
    fn input() -> RetainedCatalog {
        let mut data = vec![0; 216];
        data[28..32].copy_from_slice(&55u32.to_le_bytes());
        let mut entries = vec![];
        for index in 0..2 {
            data[180..182].copy_from_slice(&(index as i16 + 1).to_le_bytes());
            entries.push(definition::Entry {
                key: Key {
                    resource: 7,
                    chunk: 128 + index as u16,
                },
                semiglobal: None,
                sprite_resource: 3,
                global: None,
                name: b"retained".to_vec(),
                data: data.clone(),
                index,
                flags: (128 + index) << 16,
            });
        }
        RetainedCatalog {
            definitions: definition::Catalog {
                entries,
                guids: BTreeMap::from([(55, 1)]),
                attributes: vec![definition::Attributes {
                    guid: 55,
                    values: vec![19],
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
                global: None,
            },
            observation_epoch: ObservationEpoch::NewUnpublished,
        }
    }
    #[test]
    fn retained_catalog_keeps_duplicate_guid_order_and_independent_shared_attributes() {
        let index = index();
        let input = input();
        let expected = input.definitions.clone();
        let catalog = Catalog::from_retained(resources(&index), input).unwrap();
        assert_eq!(catalog.definitions, expected);
        assert_eq!(catalog.portrait_resource(55).unwrap(), Some(2));
        assert_eq!(
            catalog.keys,
            expected.entries.iter().map(|e| e.key).collect::<Vec<_>>()
        );
        assert!(catalog.visits.is_empty());
        assert_eq!(catalog.notifications, 0);
        assert_eq!(catalog.sprites.entries[0].references, 2);
    }
    #[test]
    fn retained_catalog_rejects_missing_owners_lookup_corruption_and_settings_omission() {
        let index = index();
        let mut bad = input();
        bad.definitions.entries[0].key.resource = 8;
        assert!(Catalog::from_retained(resources(&index), bad).is_err());
        let mut bad = input();
        bad.definitions.guids.insert(55, 0);
        assert!(Catalog::from_retained(resources(&index), bad).is_err());
        let mut bad = input();
        bad.sprite_entries[0].resource = 8;
        assert!(Catalog::from_retained(resources(&index), bad).is_err());
        let value = serde_json::to_value(input()).unwrap();
        for key in value["settings"].as_object().unwrap().keys() {
            let mut missing = value.clone();
            missing["settings"].as_object_mut().unwrap().remove(key);
            assert!(
                serde_json::from_value::<RetainedCatalog>(missing).is_err(),
                "{key}"
            );
        }
        let mut missing = value;
        missing.as_object_mut().unwrap().remove("observation_epoch");
        assert!(serde_json::from_value::<RetainedCatalog>(missing).is_err());
    }
    #[test]
    fn captured_definition_owners_require_presence_but_allow_explicit_null() {
        let value = serde_json::to_value(input()).unwrap();
        assert!(serde_json::from_value::<RetainedCatalog>(value.clone()).is_ok());
        for field in ["global", "semiglobal"] {
            let mut missing = value.clone();
            missing["definitions"]["entries"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(serde_json::from_value::<RetainedCatalog>(missing.clone()).is_err());
            let old: definition::Catalog =
                serde_json::from_value(missing["definitions"].clone()).unwrap();
            assert!(old.entries[0].global.is_none() && old.entries[0].semiglobal.is_none());
            let mut present = value.clone();
            present["definitions"]["entries"][0][field] = 7.into();
            assert!(serde_json::from_value::<RetainedCatalog>(present).is_ok());
        }
    }
}
