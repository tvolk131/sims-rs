//! Source-backed object registration, retaining native resource-instance and
//! catalog order. Manager startup flags/global resource are explicit inputs.
use crate::{
    avatar_installation::{Installation, ObjectVisit, ObjectVisitor},
    object_resources::{NOOP_CALLBACK, Resources},
};
use sims_sim::{
    object_catalog::{self, Key, definition, sprite_pool},
    pose::clothing::catalog_providers,
};
mod household;
mod portraits;
pub mod products;
pub mod retained;
pub mod saved_types;
pub mod thumbnail;

const OBJD: u32 = u32::from_be_bytes(*b"OBJD");
const GLOB: u32 = u32::from_be_bytes(*b"GLOB");

/// Supplied startup state, not guessed from the set of files on disk.
pub struct Settings {
    pub filter_flagged: bool,
    pub global_directory: Vec<u8>,
    pub global_people: u8,
    pub global: Option<u32>,
}
pub struct Catalog<'a> {
    pub definitions: definition::Catalog,
    pub resources: Resources<'a>,
    pub sprites: sprite_pool::Pool,
    pub visits: Vec<ObjectVisit>,
    pub notifications: usize,
    settings: Settings,
    keys: Vec<Key<u32>>,
    fault: Option<String>,
}
impl<'a> Catalog<'a> {
    pub fn new(resources: Resources<'a>, settings: Settings) -> Self {
        Self {
            definitions: definition::Catalog::default(),
            resources,
            sprites: sprite_pool::Pool::default(),
            visits: vec![],
            notifications: 0,
            settings,
            keys: vec![],
            fault: None,
        }
    }
    fn ready(&self) -> Result<(), String> {
        self.fault.as_ref().map_or(Ok(()), |e| {
            Err(format!(
                "object catalog stopped at unresolved boundary: {e}"
            ))
        })
    }
    pub fn register_file(&mut self, visit: ObjectVisit) -> Result<(), String> {
        self.ready()?;
        let result = object_catalog::register_file(&visit.path, visit.category, visit.people, self);
        self.visits.push(visit);
        if let Err(e) = &result {
            self.fault = Some(e.clone());
        }
        result
    }
    /// 43bdc5 consumes the actual ordered definitions and resource filenames.
    pub fn providers(&mut self) -> Result<Vec<Vec<u8>>, String> {
        self.ready()?;
        let entries = self
            .definitions
            .entries
            .iter()
            .map(|e| catalog_providers::Entry {
                resource: e.key.resource,
                flags: e.flags,
            })
            .collect::<Vec<_>>();
        let mut paths = vec![];
        catalog_providers::collect(&entries, &mut paths, |id| self.resources.filename(id))?;
        Ok(paths)
    }
    /// Populate the move tool's original seven GUID ranges from these actual
    /// registered definitions. This leaves the resource/definition owners intact.
    pub fn populate_move_tool_groups(
        &self,
        groups: &mut object_catalog::move_tool::Groups,
    ) -> Result<(), String> {
        self.ready()?;
        groups.populate(&self.definitions)
    }
}
impl ObjectVisitor for Catalog<'_> {
    fn count(&self) -> usize {
        self.definitions.entries.len()
    }
    fn file(&mut self, _: &mut Installation<'_>, visit: ObjectVisit) -> anyhow::Result<()> {
        self.register_file(visit).map_err(anyhow::Error::msg)
    }
    fn changed(&mut self) -> anyhow::Result<()> {
        self.ready().map_err(anyhow::Error::msg)?;
        self.notifications += 1;
        Ok(())
    }
}
impl object_catalog::Host for Catalog<'_> {
    type Resource = u32;
    type Chunk = u32;
    fn acquire(&mut self, path: &[u8], people: u8) -> Result<Option<u32>, String> {
        self.resources.acquire(path, people)
    }
    fn valid(&mut self, resource: u32) -> Result<bool, String> {
        self.resources.valid(resource)
    }
    fn count(&mut self, resource: u32) -> Result<u32, String> {
        self.resources.count(resource, OBJD)
    }
    fn ordinal(&mut self, resource: u32, ordinal: u32) -> Result<u32, String> {
        self.resources
            .ordinal(resource, OBJD, ordinal, NOOP_CALLBACK)
    }
    fn id(&mut self, resource: u32, chunk: u32) -> Result<u32, String> {
        self.resources.id(resource, chunk)
    }
    fn release_chunk(&mut self, resource: u32, chunk: u32) -> Result<(), String> {
        self.resources.release_chunk(resource, chunk)
    }
    fn definitions(&self) -> &[Key<u32>] {
        &self.keys
    }
    fn register(&mut self, resource: u32, id: u32, category: u32) -> Result<(), String> {
        let settings = definition::Settings {
            filter_flagged: self.settings.filter_flagged,
            global_directory: &self.settings.global_directory,
            global_people: self.settings.global_people,
            global: self.settings.global,
        };
        let before = self.definitions.entries.len();
        self.definitions.register(
            resource,
            id,
            category,
            settings,
            &mut DefinitionHost {
                resources: &mut self.resources,
                sprites: &mut self.sprites,
            },
        )?;
        // Make each append visible to the next ordinal's duplicate check.
        self.keys
            .extend(self.definitions.entries[before..].iter().map(|e| e.key));
        Ok(())
    }
    fn release(&mut self, resource: u32) -> Result<(), String> {
        self.resources.release(resource)
    }
}
struct DefinitionHost<'a, 'b> {
    resources: &'a mut Resources<'b>,
    sprites: &'a mut sprite_pool::Pool,
}
impl definition::Host for DefinitionHost<'_, '_> {
    type Chunk = u32;
    fn definition(&mut self, resource: u32, id: u32) -> Result<u32, String> {
        self.resources.by_id(resource, OBJD, id, NOOP_CALLBACK)
    }
    fn data(&mut self, chunk: u32) -> Result<&mut Vec<u8>, String> {
        self.resources.data_mut(chunk)
    }
    fn name(&mut self, resource: u32, chunk: u32) -> Result<Vec<u8>, String> {
        self.resources.name(resource, chunk)
    }
    fn release_chunk(&mut self, resource: u32, chunk: u32) -> Result<(), String> {
        self.resources.release_chunk(resource, chunk)
    }
    fn release_resource(&mut self, resource: u32) -> Result<(), String> {
        self.resources.release(resource)
    }
    fn sprite_resource(&mut self, resource: u32) -> Result<u32, String> {
        self.sprites.acquire(resource)
    }
    fn glob(&mut self, resource: u32) -> Result<Option<Vec<u8>>, String> {
        let handle = self.resources.ordinal(resource, GLOB, 1, 0)?;
        // 43ba52 retains the GLOB handle; it releases only the OBJD above.
        if handle == 0 {
            Ok(None)
        } else {
            Ok(Some(self.resources.data(handle)?.to_vec()))
        }
    }
    fn acquire(&mut self, path: &[u8], people: u8) -> Result<Option<u32>, String> {
        self.resources.acquire(path, people)
    }
}
