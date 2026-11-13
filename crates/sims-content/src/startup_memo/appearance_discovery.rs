use crate::Index;
use anyhow::{Context, Result, bail};
use std::{collections::BTreeMap, sync::Arc};

struct Provider {
    source: usize,
    conflict: Option<usize>,
}

pub(super) struct Discovery {
    providers: BTreeMap<String, Provider>,
}

impl Discovery {
    pub(super) fn prepare(index: &Index) -> Result<Self> {
        let mut providers: BTreeMap<String, (Provider, Arc<[u8]>)> = BTreeMap::new();
        let mut scan = crate::resource_scan::Scan::default();
        crate::avatar::appearance_catalogs_read(
            index,
            |resource| scan.read(resource),
            |source, bytes, catalog| {
                let bytes: Arc<[u8]> = bytes.into();
                for appearance in catalog.appearances {
                    let name = appearance.name.to_ascii_lowercase();
                    if let Some((provider, previous)) = providers.get_mut(&name) {
                        if provider.conflict.is_none() && previous.as_ref() != bytes.as_ref() {
                            provider.conflict = Some(source);
                        }
                    } else {
                        providers.insert(
                            name,
                            (
                                Provider {
                                    source,
                                    conflict: None,
                                },
                                bytes.clone(),
                            ),
                        );
                    }
                }
                Ok(())
            },
        )?;
        Ok(Self {
            providers: providers
                .into_iter()
                .map(|(name, (provider, _))| (name, provider))
                .collect(),
        })
    }

    pub(super) fn find(&self, index: &Index, name: &str) -> Result<usize> {
        let provider = self
            .providers
            .get(&name.to_ascii_lowercase())
            .with_context(|| format!("appearance {name} absent from catalogs"))?;
        if let Some(conflict) = provider.conflict {
            bail!(
                "conflicting appearance {name}: {} and {}",
                index.resources[provider.source].locator,
                index.resources[conflict].locator
            );
        }
        Ok(provider.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Resource;

    fn binary(name: &str, kind: u32) -> Vec<u8> {
        let mut bytes = 0u32.to_le_bytes().to_vec();
        bytes.extend(1u32.to_le_bytes());
        bytes.push(name.len() as u8);
        bytes.extend(name.as_bytes());
        for word in [kind, 0, 0, 0] {
            bytes.extend(word.to_le_bytes());
        }
        bytes
    }

    fn text(name: &str) -> Vec<u8> {
        format!("// synthetic\nversion 300\n0\n1\n{name}\n7\n0\n0\n0\n").into_bytes()
    }

    fn index(entries: Vec<(&str, Vec<u8>)>) -> Index {
        Index {
            root: Default::default(),
            resources: entries
                .into_iter()
                .map(|(locator, bytes)| Resource {
                    locator: locator.into(),
                    name: locator.rsplit('/').next().unwrap().into(),
                    source: Default::default(),
                    offset: 0,
                    size: bytes.len() as u64,
                    unpacked_size: bytes.len() as u64,
                    archived: false,
                    owned: Some(bytes.into()),
                })
                .collect(),
            archives: 0,
            diagnostics: vec![],
        }
    }

    fn assert_lookup(discovery: &Discovery, index: &Index, name: &str) {
        assert_eq!(
            discovery
                .find(index, name)
                .map_err(|error| format!("{error:#}")),
            crate::avatar::find_appearance_catalog(index, name)
                .map_err(|error| format!("{error:#}"))
        );
    }

    #[test]
    fn embedded_names_creator_exclusion_and_catalog_eligibility_match_source_scan() {
        let index = index(vec![
            ("first.cmx.bcf", binary("EmBeDdEd", 7)),
            ("equivalent.cmx.bcf", binary("EmBeDdEd", 7)),
            ("text.cmx", text("Text-Only")),
            ("CrEaToR_FiLeS/reference.cmx", b"malformed".to_vec()),
            ("Creator_Files/private.cmx.bcf", binary("creator-only", 7)),
            ("nested/Creator_Files/game.cmx", text("nested")),
            ("ignored.bcf", binary("ignored", 7)),
            ("mixed.cmx.bcf", vec![1; 16]),
            ("empty.cmx.bcf", vec![0; 12]),
            ("equivalent-text.cmx", text("Text-Only")),
        ]);
        let discovery = Discovery::prepare(&index).unwrap();
        for name in [
            "embedded",
            "EMBEDDED",
            "text-only",
            "TEXT-ONLY",
            "nested",
            "creator-only",
            "ignored",
            "missing",
            "first",
        ] {
            assert_lookup(&discovery, &index, name);
        }
        assert_eq!(discovery.find(&index, "embedded").unwrap(), 0);
        assert_eq!(discovery.find(&index, "TEXT-ONLY").unwrap(), 2);
        assert_eq!(discovery.find(&index, "nested").unwrap(), 5);
    }

    #[test]
    fn conflicts_require_identical_catalog_bytes_and_retain_first_distinct_provider() {
        for (locator, conflicting) in [
            ("different.cmx.bcf", binary("embedded", 8)),
            ("different.cmx.bcf", binary("EMBEDDED", 7)),
            ("different.cmx", text("embedded")),
        ] {
            let index = index(vec![
                ("first.cmx.bcf", binary("embedded", 7)),
                ("equivalent.cmx.bcf", binary("embedded", 7)),
                (locator, conflicting),
                ("last.cmx.bcf", binary("embedded", 9)),
                ("unique.cmx", text("unique")),
            ]);
            let discovery = Discovery::prepare(&index).unwrap();
            assert_lookup(&discovery, &index, "EmBeDdEd");
            assert_eq!(
                discovery.find(&index, "EmBeDdEd").unwrap_err().to_string(),
                format!("conflicting appearance EmBeDdEd: first.cmx.bcf and {locator}")
            );
            assert_lookup(&discovery, &index, "unique");
            assert_lookup(&discovery, &index, "missing");
        }
    }

    #[test]
    fn malformed_eligible_catalogs_are_not_acknowledged_even_after_a_match() {
        let mut mixed = binary("embedded", 7);
        let end = mixed.len();
        mixed[end - 4..].copy_from_slice(&1u32.to_le_bytes());
        for (name, malformed) in [
            ("bad.cmx", b"invalid".to_vec()),
            ("bad.cmx.bcf", vec![0, 0, 0, 0, 1, 0, 0, 0]),
            ("mixed.cmx.bcf", mixed),
            ("skeleton.cmx", b"// synthetic\nversion 300\n1\n".to_vec()),
        ] {
            let index = index(vec![
                ("first.cmx.bcf", binary("embedded", 7)),
                (name, malformed),
            ]);
            let expected = crate::avatar::find_appearance_catalog(&index, "embedded").unwrap_err();
            let actual = Discovery::prepare(&index).err().unwrap();
            assert_eq!(format!("{actual:#}"), format!("{expected:#}"));
        }
    }
}
