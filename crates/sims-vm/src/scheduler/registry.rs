//! Borrow the live VM's registry for 46d553. The native pointer handle is
//! projected to a saved, non-reused lifetime handle. If a callback destroys the
//! retained object, fail at that boundary instead of reading a reused ID.
use super::Registry;
use crate::{ObjectId, Status, Vm};

pub struct Borrowed<'a> {
    vm: &'a mut Vm,
}
impl<'a> Borrowed<'a> {
    pub fn new(vm: &'a mut Vm) -> Result<Self, String> {
        if vm.world.scheduler_storage.is_none() {
            return Err("native registry extent and vacant words are unknown".into());
        }
        vm.world.validate_scheduler_storage()?;
        Ok(Self { vm })
    }
    fn slot(&self, slot: i32) -> Result<ObjectId, String> {
        let id = ObjectId::try_from(slot).map_err(|_| "native registry slot exceeds signed IDs")?;
        if id <= 0 || id as usize > self.len() {
            return Err("native registry slot outside retained extent".into());
        }
        Ok(id)
    }
    pub fn object_id(&self, handle: u32) -> Result<ObjectId, String> {
        self.vm
            .world
            .scheduler_storage
            .as_ref()
            .unwrap()
            .object_for_handle(handle)
            .ok_or_else(|| "retained scheduler object lifetime has ended".into())
    }
}
impl Registry for Borrowed<'_> {
    fn len(&self) -> usize {
        self.vm.world.scheduler_storage.as_ref().unwrap().len()
    }
    fn object(&self, slot: i32) -> Result<Option<u32>, String> {
        let id = self.slot(slot)?;
        Ok(self.vm.world.scheduler_storage.as_ref().unwrap().handle(id))
    }
    fn word(&self, slot: i32) -> Result<u32, String> {
        self.vm
            .world
            .scheduler_storage
            .as_ref()
            .unwrap()
            .word(&self.vm.world, self.slot(slot)?)
    }
    fn set_word(&mut self, slot: i32, word: u32) -> Result<(), String> {
        let id = self.slot(slot)?;
        // The scheduler writes only occupied slots, after resolving the object.
        // Preserve last_run: it is host visit bookkeeping, not part of this word.
        if !self
            .vm
            .world
            .scheduler_storage
            .as_ref()
            .unwrap()
            .contains(id)
        {
            return Err("scheduler word write requires an occupied slot".into());
        }
        let idle = self
            .vm
            .world
            .objects
            .get_mut(&id)
            .and_then(|o| o.idle.as_mut())
            .ok_or("registered scheduler word absent")?;
        idle.countdown = word as i16;
        idle.flags = (word >> 16) as u16;
        Ok(())
    }
    fn error(&self, object: u32) -> Result<i16, String> {
        let id = self.object_id(object)?;
        if let Some(thread) = self.vm.object_threads.get(&id) {
            let thread = self
                .vm
                .threads
                .get(thread)
                .ok_or("native error owner thread absent")?;
            if matches!(thread.status, Status::Faulted(_)) {
                return Err("replacement VM fault has no native TreeSim error mapping".into());
            }
        }
        self.vm
            .world
            .objects
            .get(&id)
            .ok_or("retained scheduler object absent")?
            .native_tree_error
            .ok_or_else(|| "native TreeSim error word is unknown".into())
    }
}
