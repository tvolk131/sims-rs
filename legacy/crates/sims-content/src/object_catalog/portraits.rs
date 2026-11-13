//! Portrait views over the actual registered definition vector. Handles are
//! one-based definition indexes, distinct from aggregate IFF-resource handles.
use super::Catalog;
use sims_vm::image_cache::{
    Event, State,
    catalog::{Host, Resource},
};

impl Catalog<'_> {
    pub fn portrait_file(&self, handle: u32) -> Result<u32, String> {
        self.ready()?;
        Ok(self
            .definitions
            .entries
            .get(handle.wrapping_sub(1) as usize)
            .ok_or("portrait definition resource absent")?
            .key
            .resource)
    }
    pub fn populate_portraits(
        &self,
        cache: &mut State,
        reset: bool,
        initialized: bool,
        observer: &mut impl FnMut(Event, &State),
    ) -> Result<(), String> {
        self.ready()?;
        struct View<'a, F> {
            entries: &'a [sims_sim::object_catalog::definition::Entry],
            observer: F,
        }
        impl<F: FnMut(Event, &State)> Host for View<'_, F> {
            fn resource(&mut self, index: u32, _: &State) -> Result<Option<Resource>, String> {
                self.entries
                    .get(index as usize)
                    .map(|entry| {
                        let data = &entry.data;
                        if data.len() != 216 {
                            return Err("portrait definition is not normalized".into());
                        }
                        Ok(Resource {
                            kind: u16::from_le_bytes(data[18..20].try_into().unwrap()),
                            guid: u32::from_le_bytes(data[28..32].try_into().unwrap()),
                        })
                    })
                    .transpose()
            }
            fn image_event(&mut self, event: Event, cache: &State) {
                (self.observer)(event, cache);
            }
        }
        cache.populate(
            reset,
            initialized,
            &mut View {
                entries: &self.definitions.entries,
                observer,
            },
        )
    }

    /// Native manager GUID lookup keeps the latest registered definition.
    pub fn portrait_resource(&self, guid: u32) -> Result<Option<u32>, String> {
        self.ready()?;
        self.definitions
            .guids
            .get(&guid)
            .map(|&index| {
                u32::try_from(index)
                    .ok()
                    .and_then(|i| i.checked_add(1))
                    .ok_or_else(|| "portrait resource identity exhausted".into())
            })
            .transpose()
    }

    /// The live object's retained definition is authoritative for dialog mode
    /// zero. Unlike neighborhood GUID lookup, this may refer to an older entry
    /// superseded by another registration of the same GUID.
    pub fn live_dialog_resource(
        &mut self,
        definition: &sims_vm::Definition,
    ) -> Result<u32, String> {
        self.ready()?;
        let candidates = self
            .definitions
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                (entry.data.len() == 216
                    && entry.key.chunk == definition.object
                    && u32::from_le_bytes(entry.data[28..32].try_into().unwrap())
                        == definition.guid)
                    .then_some(index as u32 + 1)
            })
            .collect::<Vec<_>>();
        let mut matched = None;
        for handle in candidates {
            if self.portrait_definition(handle)? == *definition {
                if matched.replace(handle).is_some() {
                    return Err("ambiguous retained live dialog definition".into());
                }
            }
        }
        matched.ok_or_else(|| {
            format!(
                "retained live dialog definition absent: {} #{}",
                definition.source, definition.object
            )
        })
    }

    /// Read an exact chunk without mutating the original resource. Missing
    /// chunks are distinct from unavailable catalog/resource services.
    pub fn portrait_chunk(
        &mut self,
        handle: u32,
        tag: [u8; 4],
        id: u16,
    ) -> Result<Option<Vec<u8>>, String> {
        self.ready()?;
        let entry = self
            .definitions
            .entries
            .get(handle.wrapping_sub(1) as usize)
            .ok_or("portrait definition resource absent")?;
        let resource = entry.key.resource;
        let chunk = self
            .resources
            .by_id(resource, u32::from_be_bytes(tag), u32::from(id), 0)?;
        if chunk == 0 {
            return Ok(None);
        }
        let bytes = self.resources.data(chunk)?.to_vec();
        self.resources.release_chunk(resource, chunk)?;
        Ok(Some(bytes))
    }

    pub fn portrait_filename(&mut self, handle: u32) -> Result<Vec<u8>, String> {
        self.ready()?;
        let entry = self
            .definitions
            .entries
            .get(handle.wrapping_sub(1) as usize)
            .ok_or("portrait definition resource absent")?;
        self.resources.filename(entry.key.resource)
    }

    /// Native resource virtual +20, used by 4e8c5d/4e9000. A successfully
    /// opened read-only source is valid but cannot pass this capability check.
    pub fn portrait_writable(&mut self, handle: u32) -> Result<bool, String> {
        self.ready()?;
        let entry = self
            .definitions
            .entries
            .get(handle.wrapping_sub(1) as usize)
            .ok_or("portrait definition resource absent")?;
        self.resources.writable(entry.key.resource)
    }

    /// Bind the selected catalog definition to its exact mounted OBJD source,
    /// rather than selecting an arbitrary loaded module with the same GUID.
    /// The caller must load that module before passing this to the simulation.
    pub fn portrait_definition(&mut self, handle: u32) -> Result<sims_vm::Definition, String> {
        self.ready()?;
        let entry = self
            .definitions
            .entries
            .get(handle.wrapping_sub(1) as usize)
            .ok_or("portrait definition resource absent")?;
        if entry.data.len() != 216 {
            return Err("portrait definition is not normalized".into());
        }
        let resource = entry.key.resource;
        let object = entry.key.chunk;
        let guid = u32::from_le_bytes(entry.data[28..32].try_into().unwrap());
        let slots = u16::from_le_bytes(entry.data[40..42].try_into().unwrap());
        let chunk = self
            .resources
            .by_id(resource, super::OBJD, u32::from(object), 0)?;
        if chunk == 0 {
            return Err("registered portrait definition chunk absent".into());
        }
        let source = self.resources.chunk_source(chunk).map(str::to_owned);
        self.resources.release_chunk(resource, chunk)?;
        Ok(sims_vm::Definition {
            source: source?,
            object,
            guid,
            slots,
        })
    }
}
