//! Resource-backed neighborhood identity for 4256f4 / 425afe and the CTSS
//! branch of 457d3b. Neighborhood IDs and retained live definitions have
//! separate resolution paths and are never treated as interchangeable IDs.
use crate::object_catalog::Catalog;
use sims_vm::{dialog::NeighborHost, person::WorldState};

pub struct Neighbors<'a, 'b> {
    pub world: &'a WorldState,
    pub catalog: &'a mut Catalog<'b>,
    pub language: u8,
}
impl NeighborHost for Neighbors<'_, '_> {
    fn live(&mut self, definition: &sims_vm::Definition) -> Result<Option<u32>, String> {
        self.catalog.live_dialog_resource(definition).map(Some)
    }
    fn neighbor(&mut self, id: i16) -> Result<Option<u32>, String> {
        if id <= 0 {
            return Ok(None);
        }
        let slots = self
            .world
            .neighbor_slots
            .as_ref()
            .ok_or("native neighbor vector unavailable")?;
        let Some(Some(record)) = slots.get(id as usize - 1) else {
            return Ok(None);
        };
        let neighbor = self
            .world
            .neighbors
            .get(record)
            .ok_or("neighbor vector record absent")?;
        self.catalog.portrait_resource(neighbor.guid)
    }
    fn name(&mut self, resource: u32) -> Result<String, String> {
        // Validate the resource handle through the catalog before reading its
        // normalized OBJD catalog-string field (+52).
        self.catalog.portrait_file(resource)?;
        let entry = &self.catalog.definitions.entries[resource as usize - 1];
        if entry.data.len() != 216 {
            return Err("dialog name definition is not normalized".into());
        }
        let strings = u16::from_le_bytes(entry.data[82..84].try_into().unwrap());
        if self
            .catalog
            .portrait_chunk(resource, *b"CATS", strings)?
            .is_some()
        {
            return Err("dialog legacy CATS name decoding unsupported".into());
        }
        let bytes = self
            .catalog
            .portrait_chunk(resource, *b"CTSS", strings)?
            .ok_or("dialog catalog name/fallback unavailable")?;
        let table = sims_formats::strings::StringTable::parse(&bytes).map_err(|e| e.to_string())?;
        table
            .get(0, self.language)
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| "dialog catalog name/fallback unavailable".into())
    }
}
