//! Process-owned skeleton resources, separate from each animator's cloned rig.
//! The original catalog reader constructs every skeleton in a loaded file.
use crate::Index;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_formats::avatar::Skeleton;
use sims_sim::pose::skeleton::Resource;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Catalog {
    sha256: String,
    resources: Vec<Resource>,
}
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cache {
    catalogs: BTreeMap<String, Catalog>,
}
/// Already-loaded resource ownership. The native importer supplies actual noise
/// records; this path must never call Resource::read or reconstruct RNG history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedCatalog {
    pub locator: String,
    pub sha256: String,
    pub resources: Vec<Resource>,
}
impl Cache {
    pub fn from_retained(index: &Index, retained: Vec<RetainedCatalog>) -> Result<Self> {
        let mut catalogs = BTreeMap::new();
        for c in retained {
            ensure!(
                catalogs
                    .insert(
                        c.locator,
                        Catalog {
                            sha256: c.sha256,
                            resources: c.resources
                        }
                    )
                    .is_none(),
                "duplicate retained skeleton catalog"
            );
        }
        let cache = Self { catalogs };
        cache.validate(index)?;
        Ok(cache)
    }
    /// Source selection is explicitly bound to a captured catalog, rather than
    /// choosing a same-name resource from a newly enumerated installation.
    pub fn retained_source(&self, locator: &str, name: &str) -> Result<&Skeleton> {
        let catalog = self
            .catalogs
            .get(locator)
            .context("retained skeleton catalog absent")?;
        let mut matches = catalog.resources.iter().filter(|r| r.source.name == name);
        let source = &matches
            .next()
            .context("retained skeleton source absent")?
            .source;
        ensure!(
            matches.next().is_none(),
            "duplicate retained skeleton source name"
        );
        Ok(source)
    }
    pub fn load(&mut self, index: &Index, name: &str, crt: &mut u32) -> Result<Skeleton> {
        self.load_with_startup(index, name, crt, None)
    }

    pub(crate) fn load_with_startup(
        &mut self,
        index: &Index,
        name: &str,
        crt: &mut u32,
        startup: Option<&mut crate::startup_memo::Memo>,
    ) -> Result<Skeleton> {
        let (selected, id) = crate::avatar::load_skeleton_with_startup(index, name, startup)?;
        let source = &index.resources[id];
        if !self.catalogs.contains_key(&source.locator) {
            let bytes = source.read()?;
            let parsed = crate::avatar::read_catalog(&source.name, &bytes)?;
            let mut seed = *crt;
            let resources = parsed
                .skeletons
                .into_iter()
                .map(|s| Resource::read(s, &mut seed).map_err(anyhow::Error::msg))
                .collect::<Result<Vec<_>>>()?;
            ensure!(
                resources.iter().any(|r| r.source == selected),
                "resolved skeleton is absent from its source catalog"
            );
            self.catalogs.insert(
                source.locator.clone(),
                Catalog {
                    sha256: format!("{:x}", Sha256::digest(bytes)),
                    resources,
                },
            );
            *crt = seed;
        }
        let retained = self.catalogs[&source.locator]
            .resources
            .iter()
            .find(|r| r.source == selected)
            .context("cached catalog does not contain the requested skeleton")?;
        ensure!(
            retained.source == selected,
            "cached skeleton source changed"
        );
        Ok(retained.source.clone())
    }
    /// Restore checks provenance without reconstructing resources or taking RNG
    /// draws. Noise records, including zero-amplitude records, remain retained.
    pub fn validate(&self, index: &Index) -> Result<()> {
        for (locator, catalog) in &self.catalogs {
            let source = &index.resources[index.find(locator)?];
            let bytes = source.read()?;
            ensure!(
                format!("{:x}", Sha256::digest(&bytes)) == catalog.sha256,
                "skeleton catalog changed: {locator}"
            );
            let parsed = crate::avatar::read_catalog(&source.name, &bytes)?;
            ensure!(
                parsed.skeletons.len() == catalog.resources.len(),
                "skeleton cache size changed"
            );
            for (source, retained) in parsed.skeletons.iter().zip(&catalog.resources) {
                ensure!(
                    source == &retained.source && source.bones.len() == retained.wiggle.len(),
                    "skeleton cache source mismatch"
                );
                ensure!(
                    source
                        .bones
                        .iter()
                        .zip(&retained.wiggle)
                        .all(
                            |(bone, noise)| bone.wiggle.gradient_scale == noise.gradient_scale
                                && bone.wiggle.amplitude == noise.amplitude
                        ),
                    "cached skeleton noise parameters differ from source"
                );
            }
        }
        Ok(())
    }
}
