//! Borrowed original-lot queries for snapshots. Lifetime handles come from the
//! existing registry owner; script-ID reuse cannot redirect a saved reference.
//! Definition handles must come from the caller's retained resource catalogue.
use super::{CaptureView, Position, Registry, placement};
use crate::lot_links::Links;
use sims_vm::{Definition, ObjectId, Program, World};

pub struct View<'a> {
    pub world: &'a mut World,
    pub program: &'a Program,
    pub links: &'a Links,
    pub definition: &'a dyn Fn(&Definition) -> Result<u32, String>,
}
impl View<'_> {
    fn storage(&self) -> Result<&sims_vm::scheduler::storage::Storage, String> {
        self.world
            .scheduler_storage
            .as_ref()
            .ok_or("snapshot registry lifetime storage absent".into())
    }
    fn id(&self, object: u32) -> Result<ObjectId, String> {
        self.storage()?
            .object_for_handle(object)
            .ok_or("snapshot object lifetime has ended".into())
    }
    fn handle(&self, id: ObjectId) -> Result<u32, String> {
        self.storage()?
            .handle(id)
            .ok_or("snapshot object not registered".into())
    }
}
impl Registry for View<'_> {
    fn lookup(&self, script_id: i16) -> Result<Option<u32>, String> {
        Ok(self.storage()?.handle(script_id))
    }
    fn on_lot(&mut self, object: u32) -> Result<bool, String> {
        //432595 uses4325b5, including zero-slot predecessor normalization.
        let id = self.id(object)?;
        Ok(self.links.capture(self.world, id)?.on_lot)
    }
}
impl CaptureView for View<'_> {
    fn normalized_master(&self, object: u32) -> Result<Option<u32>, String> {
        let id = self.id(object)?;
        let o = self
            .world
            .objects
            .get(&id)
            .ok_or("snapshot object absent")?;
        let d = o.definition.as_ref().ok_or("snapshot definition absent")?;
        let words = self
            .program
            .modules
            .get(&d.source)
            .and_then(|m| m.object_definition_words.get(&d.object))
            .ok_or("snapshot OBJD absent")?;
        if *words.get(10).ok_or("snapshot multipart field absent")? == 0 {
            return Ok(None);
        }
        if sims_vm::multitile::dynamic(self.world, self.program, id)? {
            return Ok(None);
        }
        let p = o
            .multitile
            .as_ref()
            .ok_or("multipart snapshot has no native multipart receiver")?;
        Ok(Some(self.handle(p.head.unwrap_or(id))?))
    }
    fn script_id(&self, object: u32) -> Result<i16, String> {
        self.id(object)
    }
    fn capture(&mut self, object: u32) -> Result<Position, String> {
        let id = self.id(object)?;
        let d = self.links.capture(self.world, id)?;
        Ok(Position {
            position: d.position,
            level: d.level,
            container: d.container.map(|id| self.handle(id)).transpose()?,
            slot: d.slot as i16,
        })
    }
    fn definition(&self, object: u32) -> Result<u32, String> {
        let d = self
            .world
            .objects
            .get(&self.id(object)?)
            .and_then(|o| o.definition.as_ref())
            .ok_or("snapshot retained definition absent")?;
        (self.definition)(d)
    }
    fn facing(&self, object: u32) -> Result<i16, String> {
        self.world
            .objects
            .get(&self.id(object)?)
            .and_then(|o| o.data.get(&1))
            .copied()
            .ok_or("snapshot orientation word absent".into())
    }
}
impl placement::View for View<'_> {
    fn facing(&self, object: u32) -> Result<i16, String> {
        CaptureView::facing(self, object)
    }
}
