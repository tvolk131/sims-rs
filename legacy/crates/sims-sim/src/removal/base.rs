//! cXObject::Reset(0) during destruction (0x431fbe). The deletion mark is
//! already set: an absent function 3 still detaches virtually, but skips
//! storage reconstruction and repositioning. A configured function 3 runs
//! synchronously and restarts Main before the object is released.
use crate::{Simulation, Step, lot_removal::detach::Detach};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, ThreadId, Vm, loading::BaseCallbackReset};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Execution {
    Detach(Detach),
    Callback(BaseCallbackReset),
    Complete,
}
impl Execution {
    pub fn complete(&self) -> bool {
        matches!(self, Self::Complete)
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        match self {
            Self::Detach(e) => e.owns_callback(vm, p, thread, depth),
            Self::Callback(e) => e.owns_callback(vm, p, thread, depth),
            Self::Complete => false,
        }
    }
    pub fn validate(&self, s: &Simulation, p: &Program, object: ObjectId) -> Result<(), String> {
        if s.vm
            .world
            .objects
            .get(&object)
            .is_none_or(|o| o.person.is_some())
        {
            return Err("base deletion Reset object mismatch".into());
        }
        let d = s.vm.world.objects[&object]
            .definition
            .as_ref()
            .ok_or("base Reset definition absent")?;
        let has_tree = p.modules[&d.source].object_functions[&d.object]
            .get(3)
            .is_some_and(|f| f.action != 0);
        match self {
            Self::Detach(e) => {
                if has_tree {
                    return Err("base deletion Reset virtual dispatch mismatch".into());
                }
                e.validate(s, p, object)
            }
            Self::Callback(e) => {
                if !has_tree || e.object() != object {
                    return Err("base deletion Reset callback target mismatch".into());
                }
                e.validate(&s.vm, p)
            }
            Self::Complete => Ok(()),
        }
    }
}

impl Simulation {
    pub(crate) fn drive_deletion_base_reset(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let id = self
            .deletion
            .last()
            .ok_or("base Reset without deletion")?
            .target;
        let object = &self.vm.world.objects[&id];
        if object.person.is_some() {
            return Ok(true);
        }
        let mut e = if let Some(e) = self.deletion.last_mut().unwrap().base_reset.take() {
            e
        } else {
            let d = object
                .definition
                .as_ref()
                .ok_or("base Reset definition absent")?;
            if p.modules[&d.source].object_functions[&d.object]
                .get(3)
                .is_some_and(|f| f.action != 0)
            {
                Execution::Callback(self.vm.begin_loaded_base_callback_reset(p, id)?)
            } else {
                let links = &self.native_lot.as_ref().unwrap().placement.links;
                // Capture normalizes logical predecessor/sentinel storage even
                // though marked deletion will not reuse the saved location.
                links.capture(&mut self.vm.world, id)?;
                Execution::Detach(Detach::new(&self.vm.world, links, id)?)
            }
        };
        let result = self.drive_base_reset_inner(p, id, &mut e, remaining, step);
        self.deletion.last_mut().unwrap().base_reset = Some(e);
        result
    }

    fn drive_base_reset_inner(
        &mut self,
        p: &Program,
        id: ObjectId,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        match e {
            Execution::Complete => return Ok(true),
            Execution::Detach(placement) => {
                if !placement.advance(self, p, remaining, step)? {
                    return Ok(false);
                }
                self.vm.finish_loaded_reset(id)?;
            }
            Execution::Callback(reset) => {
                let thread = reset.thread();
                if !self.drive_deletion_callback(p, thread, remaining, step)? {
                    return Ok(false);
                }
                reset.clone().finish(&mut self.vm, p)?;
            }
        }
        *e = Execution::Complete;
        Ok(true)
    }
}
