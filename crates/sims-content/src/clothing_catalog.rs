//! Ordered clothing inputs from the loaded appearance registry and mounted
//! texture paths. This layer enumerates names; it does not decode textures or
//! replace the native resource selected when a binding is later opened.
use crate::{avatar_catalog::Directories, avatar_installation::Loaded};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_formats::avatar::index::Kind;
use sims_sim::pose::clothing::{catalog_files, generator::fixed, texture_names};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Names {
    pub bodies: Vec<Vec<u8>>,
    pub textures: texture_names::Cache,
}

/// Process-owned lazy caches. A loaded cache is saved rather than regenerated
/// during selection. Restore verifies the ordered inputs against this closed,
/// read-only Complete Collection installation before accepting the cache.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    names: Option<Names>,
    /// Current native cache ownership, distinct from an installed-input profile.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    retained: bool,
}
impl Cache {
    /// Retain observed lazy cache contents without enumerating an installation.
    /// Source provenance belongs to the importing boundary.
    pub fn from_retained(names: Names) -> Result<Self> {
        let cache = Self {
            names: Some(names),
            retained: true,
        };
        cache.validate_retained()?;
        Ok(cache)
    }
    fn validate_retained(&self) -> Result<()> {
        let names = self
            .names
            .as_ref()
            .context("retained clothing names unavailable")?;
        for entries in [&names.bodies, &names.textures.names] {
            ensure!(
                entries.len() <= 10000 && entries.iter().all(|n| n.len() <= 63),
                "retained clothing cache extent invalid"
            );
        }
        Ok(())
    }
    pub fn loaded(&self) -> bool {
        self.names.is_some()
    }
    fn names_using(&mut self, load: impl FnOnce() -> Result<Names>) -> Result<&Names> {
        if self
            .names
            .as_ref()
            .is_none_or(|n| n.bodies.is_empty() || n.textures.names.is_empty())
        {
            ensure!(
                !self.retained,
                "native clothing cache population requires its observed catalog/path provider"
            );
            let names = load()?;
            let retained = self.names.get_or_insert_with(Names::default);
            if retained.bodies.is_empty() {
                retained.bodies = names.bodies;
            }
            if retained.textures.names.is_empty() {
                retained.textures = names.textures;
            }
        }
        Ok(self.names.as_ref().unwrap())
    }
    pub fn validate(&self, index: &crate::Index) -> Result<()> {
        if self.retained {
            return self.validate_retained();
        }
        if let Some(saved) = &self.names {
            ensure!(
                *saved == installed_names(index)?,
                "saved clothing catalog differs from installed sources"
            );
        }
        Ok(())
    }
}

pub trait Provider {
    fn names(&mut self) -> Result<&Names>;
}
impl Provider for &Names {
    fn names(&mut self) -> Result<&Names> {
        Ok(self)
    }
}
/// Used by callers which have not supplied process asset ownership.
pub struct Unavailable;
impl Provider for Unavailable {
    fn names(&mut self) -> Result<&Names> {
        anyhow::bail!("native appearance outfit 5 needs its pajama selector (45f034)")
    }
}
pub struct Installed<'a> {
    pub index: &'a crate::Index,
    pub cache: &'a mut Cache,
}
impl Provider for Installed<'_> {
    fn names(&mut self) -> Result<&Names> {
        self.cache.names_using(|| installed_names(self.index))
    }
}

/// Keep the saved clothing cache lazy while reusing a live session's prepared
/// installation at the first actual outfit request. Research callers without
/// a memo retain the ordinary source-backed discovery path.
pub(crate) struct WithStartup<'a> {
    pub index: &'a crate::Index,
    pub cache: &'a mut Cache,
    pub startup: Option<&'a mut crate::startup_memo::Memo>,
}
impl Provider for WithStartup<'_> {
    fn names(&mut self) -> Result<&Names> {
        self.cache.names_using(|| {
            let Some(startup) = self.startup.as_deref_mut() else {
                return installed_names(self.index);
            };
            let (loaded, mut directories) = startup.installed_catalog(self.index)?;
            let mut names = Names::default();
            names.load(&loaded, &mut directories)?;
            Ok(names)
        })
    }
}

/// The verified asset-startup subset needed by clothing lookup. Other engine
/// startup services (global animation caches, Tuning.txt, People lifecycle) are
/// not fabricated here. Directory order is the explicit virtual-installation
/// host policy; every provider and name then follows native ordering rules.
pub fn installed_names(index: &crate::Index) -> Result<Names> {
    let (loaded, mut directories) = installed_catalog(index)?;
    let mut names = Names::default();
    names.load(&loaded, &mut directories)?;
    Ok(names)
}

/// The same source-backed startup registry used by clothing enumeration. Keep
/// its complete provider order and source-error checks when proving a missing
/// appearance; a failed global filename scan alone cannot establish absence.
pub(crate) fn installed_catalog(index: &crate::Index) -> Result<(Loaded, Directories)> {
    let installation = crate::avatar_installation::Installation::new(
        index,
        crate::avatar_installation::complete_collection_asset_bindings(&index.root),
    )?;
    installed_catalog_from(index, installation, None)
}

pub(crate) fn installed_catalog_from(
    index: &crate::Index,
    mut installation: crate::avatar_installation::Installation<'_>,
    inputs: Option<&crate::startup_inputs::Session>,
) -> Result<(Loaded, Directories)> {
    use crate::{
        object_catalog::{Catalog, Settings},
        object_resources::Resources,
    };
    if let Some(inputs) = inputs {
        installation.observe(inputs);
    }
    let initial = installation.initial_mounts(b"./", 255, false)?;
    let paths = sims_sim::object_scan::Paths {
        objects: initial[1].clone(),
        user_objects: initial[2].clone(),
        downloads: initial.last().unwrap().clone(),
        expansions: std::array::from_fn(|n| initial[3 + n].clone()),
        shared: initial[11].clone(),
        npcs: initial[12].clone(),
        ..Default::default()
    };
    let mut resources = Resources::new(
        index,
        installation.directories()?,
        installation.directory_search(),
    );
    if let Some(inputs) = inputs {
        resources.observe(inputs)?;
    }
    let global = resources
        .acquire(&[initial[0].as_slice(), b"Global"].concat(), 0)
        .map_err(anyhow::Error::msg)?
        .context("clothing startup global resource absent")?;
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
    let providers = catalog.providers().map_err(anyhow::Error::msg)?;
    let loaded = installation.load_catalogs(b"./", b"./GameData/", 255, &providers)?;
    ensure!(
        installation.open_failures().is_empty()
            && loaded.catalog.index_read_failures.is_empty()
            && loaded.catalog.read_failures.is_empty()
            && loaded.catalog.startup_open_failures.is_empty(),
        "clothing startup has unresolved source loads"
    );
    Ok((loaded, installation.directories()?))
}
impl Names {
    /// Empty caches retry; nonempty caches retain their original order even
    /// if the supplied registry or paths have subsequently changed.
    pub fn load(&mut self, loaded: &Loaded, directories: &mut Directories) -> Result<()> {
        if self.bodies.is_empty() {
            self.bodies.extend(
                loaded
                    .catalog
                    .iter(Kind::Appearance)
                    .map(|record| record.location.name.as_slice())
                    .filter(|name| !name.starts_with(b"suit-"))
                    .map(|name| fixed(name, 63).to_vec()),
            );
        }
        self.textures
            .load(&mut TextureSources {
                paths: &loaded.textures,
                directories,
                resources: vec![],
            })
            .map_err(anyhow::Error::msg)
    }
}
struct TextureSources<'a> {
    paths: &'a [Vec<u8>],
    directories: &'a mut Directories,
    resources: Vec<catalog_files::Resource<Option<usize>>>,
}
impl texture_names::Host for TextureSources<'_> {
    fn enumerate(&mut self, kind: u32, names: &mut Vec<Vec<u8>>) -> Result<i32, String> {
        catalog_files::enumerate_into(
            self.paths,
            kind,
            // 59e10b..59e17a / 5c80eb register SKIN, ANIM, INDX,
            // IMAG, ZBUF and Mesh aliases. Neither raw BMP nor TGA is a
            // registry key; 40895a queries the raw types independently.
            None,
            &mut self.resources,
            self.directories,
        )?;
        *names = self
            .resources
            .iter()
            .map(|r| r.name.bytes.clone())
            .collect();
        Ok(0)
    }
}
