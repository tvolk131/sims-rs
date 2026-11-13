//! Original4b1bda: refresh the registry's person vector, in its live order.
//! Field40 is the person word at+5dc, not an ordinary-object roof property.
use serde::{Deserialize, Serialize};
use sims_vm::ObjectId;

pub trait View {
    fn people(&self) -> &[ObjectId];
    fn level(&self) -> i32;
    fn size(&self) -> i32;
    fn tile(&self, id: ObjectId) -> Result<[i8; 3], String>;
    fn field40(&self, id: ObjectId) -> Result<i16, String>;
}
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    index: usize,
    pending: Option<ObjectId>,
}
impl Cursor {
    pub fn validate(&self, view: &impl View) -> Result<(), String> {
        if self.index > view.people().len()
            || self
                .pending
                .is_some_and(|id| id <= 0 || self.index == view.people().len())
        {
            return Err("invalid view person refresh continuation".into());
        }
        Ok(())
    }
    pub fn pending(&self) -> Option<ObjectId> {
        self.pending
    }
    pub fn advance(&mut self, view: &impl View) -> Result<Option<ObjectId>, String> {
        self.validate(view)?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        while self.index < view.people().len() {
            let id = view.people()[self.index];
            let [x, y, z] = view.tile(id)?.map(i32::from);
            let level = view.level();
            let interior = z <= level
                && x >= 1
                && x < view.size().wrapping_sub(1)
                && y >= 1
                && y < view.size().wrapping_sub(1);
            if interior || z == level.wrapping_add(1) && view.field40(id)? != 0 {
                self.pending = Some(id);
                return Ok(self.pending);
            }
            self.index += 1;
        }
        Ok(None)
    }
    pub fn acknowledge(&mut self, id: ObjectId) -> Result<(), String> {
        if self.pending != Some(id) {
            return Err("view person refresh reply differs".into());
        }
        self.index += 1;
        self.pending = None;
        Ok(())
    }
}

pub struct Live<'a> {
    pub world: &'a sims_vm::World,
    pub size: i32,
    pub level: i32,
}
impl View for Live<'_> {
    fn people(&self) -> &[ObjectId] {
        self.world.person.as_ref().map_or(&[], |p| &p.person_order)
    }
    fn level(&self) -> i32 {
        self.level
    }
    fn size(&self) -> i32 {
        self.size
    }
    fn tile(&self, id: ObjectId) -> Result<[i8; 3], String> {
        let p = self
            .world
            .positions
            .get(&id)
            .ok_or("view person position absent")?;
        Ok([(p.y >> 4) as i8, (p.x >> 4) as i8, p.level as i8])
    }
    fn field40(&self, id: ObjectId) -> Result<i16, String> {
        self.world
            .objects
            .get(&id)
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.get(&40))
            .copied()
            .ok_or("view person field40 absent".into())
    }
}
