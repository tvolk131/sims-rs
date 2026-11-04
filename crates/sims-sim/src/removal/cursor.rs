//!440eeb's type9 branch, after Reset and before slot children. The definition
//!identity is captured before notification/Reset; it is not reread afterward.
use crate::{
    Simulation, Step,
    lot_cursor::{cleanup, release},
    lot_placement,
};
use serde::{Deserialize, Serialize};
use sims_vm::{Definition, ObjectId, Program, ThreadId, Vm};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Execution {
    definition: Definition,
    object: u32,
    depth: usize,
    release: Option<release::Execution>,
    cleanup: Option<cleanup::Execution>,
    complete: bool,
}
impl Execution {
    pub(crate) fn capture(
        s: &Simulation,
        p: &Program,
        object: ObjectId,
        depth: usize,
    ) -> Result<Option<Self>, String> {
        let o =
            s.vm.world
                .objects
                .get(&object)
                .ok_or("cursor destructor receiver absent")?;
        let definition = o
            .definition
            .clone()
            .ok_or("cursor destructor definition absent")?;
        if sims_vm::construction::definition_words(p, &definition)?[9] != 9 {
            return Ok(None);
        }
        if o.cursor.is_none() {
            return Err("type9 destruction requires cursor runtime class".into());
        }
        Ok(Some(Self {
            definition,
            object: s
                .vm
                .world
                .scheduler_storage
                .as_ref()
                .and_then(|r| r.handle(object))
                .ok_or("cursor destructor lifetime absent")?,
            depth,
            release: None,
            cleanup: None,
            complete: false,
        }))
    }
    pub(crate) fn complete(&self) -> bool {
        self.complete
    }
    pub(crate) fn may_be_onlot(&self) -> bool {
        self.cleanup.as_ref().is_some_and(|e| e.may_be_onlot())
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program, index: usize) -> Result<(), String> {
        let d = s
            .deletion
            .get(index)
            .ok_or("cursor destructor stack absent")?;
        if index != self.depth
            || d.entry_definition.as_ref() != Some(&self.definition)
            || d.group.is_some()
            || sims_vm::construction::definition_words(p, &self.definition)?[9] != 9
            || s.vm
                .world
                .scheduler_storage
                .as_ref()
                .and_then(|r| r.object_for_handle(self.object))
                != Some(d.target)
            || s.vm
                .world
                .objects
                .get(&d.target)
                .is_none_or(|o| o.cursor.is_none())
            || (self.release.is_some() || self.cleanup.is_some() || self.complete)
                && d.base_reset.as_ref().is_none_or(|e| !e.complete())
            || self.cleanup.is_some()
                && self
                    .release
                    .as_ref()
                    .is_none_or(|e| e.outcome() != Some(false))
            || self.complete
                && !self.cleanup.as_ref().map_or_else(
                    || {
                        self.release
                            .as_ref()
                            .is_some_and(|e| e.outcome() == Some(true))
                    },
                    |e| e.complete(),
                )
            || !self.complete && d.slot_limit.is_some()
        {
            return Err("invalid cursor destruction ordering or owner".into());
        }
        if let Some(e) = &self.release {
            if e.cursor() != self.object {
                return Err("cursor destruction release binding differs".into());
            }
            e.validate_call(s, p)?;
        }
        if let Some(e) = &self.cleanup {
            if e.cursor() != self.object || e.deletion_depth() != index + 1 {
                return Err("cursor destruction cleanup binding differs".into());
            }
            e.validate_call(s, p)?;
        }
        Ok(())
    }
    pub(crate) fn retired_references(
        &self,
        s: &Simulation,
        p: &Program,
        index: usize,
    ) -> Result<Vec<(ObjectId, u32)>, String> {
        self.validate(s, p, index)?;
        self.cleanup
            .as_ref()
            .map(|e| e.retired_references_call(s, p))
            .transpose()
            .map(Option::unwrap_or_default)
    }
    pub(crate) fn owns_callback(&self, v: &Vm, p: &Program, t: ThreadId, d: usize) -> bool {
        self.release
            .as_ref()
            .is_some_and(|e| e.owns_callback(v, p, t, d))
            || self
                .cleanup
                .as_ref()
                .is_some_and(|e| e.owns_callback(v, p, t, d))
    }
    pub(crate) fn graphic(&self) -> Option<&lot_placement::live::Execution> {
        self.cleanup
            .as_ref()
            .and_then(|e| e.graphic())
            .or_else(|| self.release.as_ref().and_then(|e| e.graphic()))
    }
    pub(crate) fn graphic_mut(&mut self) -> Option<&mut lot_placement::live::Execution> {
        if self.cleanup.as_ref().is_some_and(|e| e.graphic().is_some()) {
            return self.cleanup.as_mut().unwrap().graphic_mut();
        }
        self.release.as_mut().and_then(|e| e.graphic_mut())
    }
    pub(crate) fn appearance(&self) -> Option<u32> {
        self.cleanup.as_ref().and_then(|e| e.appearance())
    }
    pub(crate) fn acknowledge_appearance(&mut self, h: u32) -> Result<(), String> {
        self.cleanup
            .as_mut()
            .ok_or("cursor destructor appearance absent")?
            .acknowledge_appearance(h)
    }
    pub(crate) fn pending_focus_clear(&self) -> bool {
        self.cleanup
            .as_ref()
            .is_some_and(|e| e.pending_focus_clear())
    }
    fn focus_cleared(&self) -> bool {
        self.cleanup.as_ref().is_some_and(|e| e.complete())
    }
    pub(crate) fn acknowledge_focus(&mut self) -> Result<(), String> {
        self.cleanup
            .as_mut()
            .ok_or("cursor destructor focus absent")?
            .acknowledge_focus()
    }
    pub(crate) fn animator(&self) -> Option<u32> {
        self.release
            .as_ref()
            .and_then(|e| e.pending_animator_invalidation())
    }
    pub(crate) fn acknowledge_animator(&mut self, h: u32) -> Result<(), String> {
        self.release
            .as_mut()
            .ok_or("cursor destructor animator absent")?
            .acknowledge_animator(h)
    }
    fn advance(
        &mut self,
        s: &mut Simulation,
        p: &Program,
        left: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if self.complete {
            return Ok(true);
        }
        if *left == 0 {
            return Ok(false);
        }
        if self.release.is_none() {
            self.release = Some(release::Execution::call(s, self.object));
        }
        let r = self.release.as_mut().unwrap();
        if r.outcome().is_none() {
            r.advance(s, p, left, step)?;
        }
        match r.outcome() {
            None => return Ok(false),
            Some(true) => {
                self.complete = true;
                return Ok(true);
            }
            Some(false) => (),
        }
        if self.cleanup.is_none() {
            self.cleanup = Some(cleanup::Execution::call(
                s,
                self.object,
                cleanup::Kind::Discard,
                self.depth + 1,
            ));
        }
        let e = self.cleanup.as_mut().unwrap();
        e.advance(s, p, left, step)?;
        self.complete = e.complete();
        Ok(self.complete)
    }
}
impl Simulation {
    pub(crate) fn capture_deletion_cursor(&mut self, p: &Program) -> Result<(), String> {
        let index = self
            .deletion
            .len()
            .checked_sub(1)
            .ok_or("cursor capture stack absent")?;
        if self.deletion[index].entry_definition.is_none() {
            let target = self.deletion[index].target;
            let definition = self
                .vm
                .world
                .objects
                .get(&target)
                .and_then(|o| o.definition.clone())
                .ok_or("deletion entry definition absent")?;
            self.deletion[index].cursor = Execution::capture(self, p, target, index)?;
            self.deletion[index].entry_definition = Some(definition);
        }

        Ok(())
    }
    pub(crate) fn drive_deletion_cursor_at(
        &mut self,
        p: &Program,
        left: &mut usize,
        step: &mut Step,
        index: usize,
    ) -> Result<bool, String> {
        let Some(mut e) = self.deletion[index].cursor.take() else {
            return Ok(true);
        };
        let result = e.advance(self, p, left, step);
        //44123f may have appended an entire nested furniture destructor stack.
        self.deletion[index].cursor = Some(e);
        result
    }
    pub fn pending_cursor_cleanup_focus(&self) -> bool {
        self.cursor_cleanup
            .as_ref()
            .is_some_and(|e| e.pending_focus_clear())
            || self
                .deletion
                .iter()
                .rev()
                .any(|d| d.cursor.as_ref().is_some_and(|e| e.pending_focus_clear()))
    }
    pub fn cursor_cleanup_focus_cleared(&self) -> bool {
        self.cursor_cleanup.as_ref().is_some_and(|e| e.complete())
            || self
                .deletion
                .iter()
                .any(|d| d.cursor.as_ref().is_some_and(|e| e.focus_cleared()))
    }
    pub fn pending_cursor_release_animator(&self) -> Option<u32> {
        self.deletion
            .iter()
            .rev()
            .find_map(|d| d.cursor.as_ref().and_then(|e| e.animator()))
            .or_else(|| self.object_placement.as_ref().and_then(|e| e.animator()))
            .or_else(|| {
                self.cursor_release
                    .as_ref()
                    .and_then(|e| e.pending_animator_invalidation())
            })
    }
}
