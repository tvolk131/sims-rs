//! Shared 44123f synchronous removal: unregister references, remember the room, RemoveFromLot,
//! deletion Reset/destruction, then the existing room invalidation epilogue.
//! A diagnostic budget suspends this call without advancing the lot scheduler.
use crate::{Simulation, Step};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, ThreadId, Vm};

// Keep the field representation used by older portrait failed-search saves.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Removal {
    pub target: ObjectId,
    pub placement: super::detach::Detach,
    pub after_delete_room: Option<i16>,
    pub deleting: bool,
    #[serde(default, skip_serializing_if = "crate::lot_removal::immediate::zero")]
    depth: usize,
}
fn zero(n: &usize) -> bool {
    *n == 0
}
impl Removal {
    pub fn depth(&self) -> usize {
        self.depth
    }
    pub fn new(s: &mut Simulation, p: &Program, target: ObjectId) -> Result<Self, String> {
        Self::at_depth(s, p, target, 0)
    }
    pub(crate) fn at_depth(
        s: &mut Simulation,
        p: &Program,
        target: ObjectId,
        depth: usize,
    ) -> Result<Self, String> {
        if s.deletion.len() != depth || depth > 64 {
            return Err("synchronous removal destructor depth differs".into());
        }
        s.native_removal_tree(p, target)?;
        // 447d8b runs before lighting/room capture and before physical removal.
        // This prelude is synchronous; saves of an existing Removal have already
        // passed it and must not replay its first-match vector erases.
        s.unregister_immediate_removal(p, target)?;
        Ok(Self {
            target,
            after_delete_room: super::immediate_room(&s.vm.world, target)?,
            placement: super::detach::Detach::new(
                &s.vm.world,
                &s.native_lot
                    .as_ref()
                    .ok_or("synchronous removal lot absent")?
                    .placement
                    .links,
                target,
            )?,
            deleting: false,
            depth,
        })
    }
    ///445f84 uses virtual RemoveFromLot then440eeb directly. It does not
    ///execute44123f's unregister or extra room-invalidation epilogue.
    pub(crate) fn indicator(s: &Simulation, target: ObjectId) -> Result<Self, String> {
        if !s.deletion.is_empty() {
            return Err("indicator teardown during deletion".into());
        }
        Ok(Self {
            target,
            after_delete_room: None,
            placement: super::detach::Detach::new(
                &s.vm.world,
                &s.native_lot
                    .as_ref()
                    .ok_or("indicator lot absent")?
                    .placement
                    .links,
                target,
            )?,
            deleting: false,
            depth: 0,
        })
    }
    pub fn owns_deletion(&self, id: ObjectId) -> bool {
        self.depth == 0 && self.deleting && self.target == id
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        !self.deleting && self.placement.owns_callback(vm, p, thread, depth)
    }
    pub fn validate(&self, s: &Simulation, p: &Program, complete: bool) -> Result<(), String> {
        if self.target <= 0
            || self.depth > s.deletion.len()
            || !self.deleting && s.deletion.len() != self.depth
            || self.placement.target() != self.target
            || self.deleting && !self.placement.complete()
            || complete && !self.deleting
            || self.deleting
                && !complete
                && s.deletion.get(self.depth).is_none_or(|d| {
                    d.target != self.target || d.after_delete_room != self.after_delete_room
                })
            || complete
                && (s.vm.world.objects.contains_key(&self.target)
                    || s.retired_characters.contains_key(&self.target)
                    || s.retired_graphics.contains_key(&self.target)
                    || s.deletion.len() != self.depth)
        {
            return Err("invalid synchronous removal binding".into());
        }
        if !self.deleting {
            self.placement.validate(s, p, self.target)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Execution {
    tick: u64,
    pub(crate) removal: Removal,
    complete: bool,
}
impl Execution {
    pub fn target(&self) -> ObjectId {
        self.removal.target
    }
    pub fn complete(&self) -> bool {
        self.complete
    }
    pub(crate) fn owns_deletion(&self, id: ObjectId) -> bool {
        self.removal.owns_deletion(id)
    }
    pub(crate) fn owns_callback(
        &self,
        vm: &Vm,
        p: &Program,
        thread: ThreadId,
        depth: usize,
    ) -> bool {
        self.removal.owns_callback(vm, p, thread, depth)
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        if self.tick != s.vm.tick || s.native_lot.is_none() || s.portrait_person.is_some() {
            return Err("invalid synchronous removal owner".into());
        }
        self.removal.validate(s, p, self.complete)
    }
}

impl Simulation {
    /// Retain the child already created by44279a/445f84. In particular, do not
    /// reconstruct it through Removal::new and replay its unregister prelude.
    pub(crate) fn retain_frozen_removal(
        &mut self,
        p: &Program,
        removal: Removal,
    ) -> Result<(), String> {
        if !self.vm.world.registry_ui_disabled
            || self.synchronous_removal.is_some()
            || !self.deletion.is_empty()
            || self.vm.scheduled_object_visit.is_some()
            || removal.depth() != 0
        {
            return Err("frozen removal requires an available house-clear owner".into());
        }
        let e = Execution {
            tick: self.vm.tick,
            removal,
            complete: false,
        };
        e.validate(self, p)?;
        self.synchronous_removal = Some(e);
        Ok(())
    }
    pub fn begin_synchronous_removal(
        &mut self,
        p: &Program,
        target: ObjectId,
    ) -> Result<(), String> {
        if self.synchronous_removal.is_some()
            || (self.ground_placement.is_some()
                || self.object_pickup.is_some()
                || self.object_placement.is_some()
                || self.cursor_work_pending())
            || self.vm.player_menu.is_some()
            || self.native_selection.is_some()
            || self.portrait_person.is_some()
            || self.native_lot.is_none()
            || self.person_factories.values().any(|f| !f.finalized)
        {
            return Err("synchronous removal requires an available native owner".into());
        }
        self.synchronous_removal = Some(Execution {
            tick: self.vm.tick,
            removal: Removal::new(self, p, target)?,
            complete: false,
        });
        Ok(())
    }
    pub fn take_synchronous_removal(&mut self) -> Result<ObjectId, String> {
        let e = self
            .synchronous_removal
            .as_ref()
            .ok_or("synchronous removal absent")?;
        if !e.complete {
            return Err("synchronous removal is incomplete".into());
        }
        let target = e.target();
        self.synchronous_removal = None;
        Ok(target)
    }
    pub fn drive_synchronous_removal(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        self.drive_synchronous_removal_inner(p, remaining, step, true)
    }
    /// Frozen clear owns the outer pointer/nulling and spatial-publication
    /// order. Retain the same child for host services and failures, without
    /// adding the public live removal epilogue inside445f84's destructor.
    pub(crate) fn drive_frozen_removal(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        self.drive_synchronous_removal_inner(p, remaining, step, false)
    }
    fn drive_synchronous_removal_inner(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
        publish_spatial: bool,
    ) -> Result<bool, String> {
        self.synchronous_removal
            .as_ref()
            .ok_or("synchronous removal absent")?
            .validate(self, p)?;
        let mut e = self.synchronous_removal.take().unwrap();
        let result = if e.complete {
            Ok(true)
        } else {
            self.drive_immediate_removal(p, &mut e.removal, remaining, step)
        };
        if let Ok(complete) = result {
            e.complete = complete;
        }
        self.synchronous_removal = Some(e);
        if publish_spatial {
            self.sync_spatial_world();
            self.validate_native_lot()?;
        }
        result
    }
    pub(crate) fn drive_immediate_removal(
        &mut self,
        p: &Program,
        c: &mut Removal,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if *remaining == 0 {
            return Ok(false);
        }
        if !c.deleting {
            if !c.placement.advance(self, p, remaining, step)? {
                return Ok(false);
            }
            let mut d = crate::removal::Deletion::new(c.target);
            d.after_delete_room = c.after_delete_room;
            self.deletion.push(d);
            c.deleting = true;
        }
        // A Reset may suspend in Remove Object and destruction may suspend in
        // a contained child's RemoveFromLot. Finish that inner physical call
        // before resuming its caller, as the ordinary lot scheduler does.
        if !self.drive_native_removal(p, remaining, step)? {
            return Ok(false);
        }
        self.drive_deletion_stack_until(p, remaining, step, false, c.depth)
    }
}
