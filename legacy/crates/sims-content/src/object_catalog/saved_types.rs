//! Join saved-type execution to the actual catalog, without duplicating its
//! definitions or substituting immutable OBJD bytes for live changed flags.
use super::Catalog;
use serde::{Deserialize, Serialize};
use sims_sim::{object_catalog::definition, saved_types};

/// Source cursors and lazy getter caches belong to this host. Getters receive
/// the catalog's actual resource owner, so they cannot require a cloned second
/// resource graph merely to satisfy Rust borrowing.
pub trait Host {
    fn retire_declaration(&mut self, declaration: &saved_types::Declaration) -> Result<(), String>;
    fn gtab(&mut self) -> Result<saved_types::Gtab, String>;
    fn begin_objt(&mut self) -> Result<Option<i32>, String>;
    fn next_objt(&mut self, version: i32) -> Result<Option<saved_types::Record>, String>;
    fn end_objt(&mut self) -> Result<(), String>;
    fn signature(
        &mut self,
        resources: &mut crate::object_resources::Resources<'_>,
        resource: sims_sim::object_catalog::Key<u32>,
        function: saved_types::Function,
    ) -> Result<u32, String>;
}

struct Services<'a, 'r, H> {
    resources: &'a mut crate::object_resources::Resources<'r>,
    host: &'a mut H,
}
impl<H: Host> saved_types::Host for Services<'_, '_, H> {
    fn retire_declaration(&mut self, d: &saved_types::Declaration) -> Result<(), String> {
        self.host.retire_declaration(d)
    }
    fn gtab(&mut self) -> Result<saved_types::Gtab, String> {
        self.host.gtab()
    }
    fn begin_objt(&mut self) -> Result<Option<i32>, String> {
        self.host.begin_objt()
    }
    fn next_objt(&mut self, version: i32) -> Result<Option<saved_types::Record>, String> {
        self.host.next_objt(version)
    }
    fn end_objt(&mut self) -> Result<(), String> {
        self.host.end_objt()
    }
    fn signature(
        &mut self,
        resource: sims_sim::object_catalog::Key<u32>,
        function: saved_types::Function,
    ) -> Result<u32, String> {
        self.host.signature(self.resources, resource, function)
    }
}

/// Source/getter services remain explicit owned state. A host using catalog
/// resource caches must retain those caches together with this read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Read<H> {
    pub state: saved_types::State,
    pub execution: saved_types::Execution,
    pub host: H,
}

/// Metadata-only cut, not a complete resource/loader checkpoint. Restore the
/// actual resource graph and catalog before accepting this matching read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata<H> {
    pub definitions: definition::Catalog,
    pub read: Read<H>,
}

impl Catalog<'_> {
    pub fn begin_saved_type_read<H>(
        &self,
        state: saved_types::State,
        lookup_capacity: usize,
        host: H,
    ) -> Result<Read<H>, String> {
        self.ready()?;
        state.validate(&self.definitions)?;
        Ok(Read {
            state,
            execution: saved_types::Execution::new(lookup_capacity),
            host,
        })
    }

    /// Successful earlier writes remain in this canonical catalog even when a
    /// later source/getter service is pending. No VM tick or RNG is processed.
    pub fn step_saved_type_read<H: Host>(&mut self, read: &mut Read<H>) -> Result<bool, String> {
        self.ready()?;
        read.execution.step(
            &mut read.state,
            &mut self.definitions,
            &mut Services {
                resources: &mut self.resources,
                host: &mut read.host,
            },
        )
    }

    pub fn saved_type_metadata<H: Clone>(&self, read: &Read<H>) -> Result<Metadata<H>, String> {
        self.ready()?;
        read.state.validate(&self.definitions)?;
        Ok(Metadata {
            definitions: self.definitions.clone(),
            read: read.clone(),
        })
    }

    /// Never copy flags into a different restored catalog or silently rebuild
    /// the source host. The caller must restore its actual resource owner too.
    pub fn resume_saved_type_read<H>(&self, cut: Metadata<H>) -> Result<Read<H>, String> {
        self.ready()?;
        if self.definitions != cut.definitions {
            return Err("saved-type metadata differs from restored canonical catalog".into());
        }
        cut.read.state.validate(&self.definitions)?;
        Ok(cut.read)
    }

    /// Before ObjM, publish the VM's derived GUID view from canonical native
    /// flags. Unlisted definitions retain their flags; GUID collisions use the
    /// catalog's existing last-registration-wins lookup. This is not TATT.
    pub fn publish_saved_type_changes<H>(
        &self,
        read: &Read<H>,
        world: &mut sims_vm::World,
    ) -> Result<(), String> {
        self.ready()?;
        read.state.validate(&self.definitions)?;
        if !read.execution.complete() {
            return Err("saved-type flag publication precedes metadata completion".into());
        }
        let flags = self.retained_neighbor_resource_flags()?;
        world.changed_resources = flags
            .into_iter()
            .filter_map(|(guid, flags)| (flags & 2 != 0).then_some(guid))
            .collect();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
