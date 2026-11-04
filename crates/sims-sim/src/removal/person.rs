//! Own-person Reset(0) inside 0x440eeb, after reference notifications and before
//! child destruction. Ordinary scheduling stays frozen across host budgets.
use crate::{Simulation, Step};
use serde::{Deserialize, Serialize};
use sims_vm::{
    ObjectId, Program, ThreadId, Vm,
    person_reset::{Request, Reset},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Execution {
    pub reset: Reset,
    pub pending: Option<Request>,
    pub complete: bool,
    /// The asset host must retain its matching reload job while this is set.
    #[serde(default)]
    pub asset_service_started: bool,
}
impl Execution {
    pub fn new(object: ObjectId, suppress_reset: bool) -> Self {
        Self {
            reset: Reset::new(object, 0x40 | if suppress_reset { 0x1000 } else { 0 }),
            pending: None,
            complete: false,
            asset_service_started: false,
        }
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        matches!(self.pending, Some(Request::Script(id)) if id == thread)
            && self.reset.owns_callback(vm, p, thread, depth)
    }
    pub fn validate(&self, s: &Simulation, p: &Program, object: ObjectId) -> Result<(), String> {
        self.reset.validate(&s.vm, p)?;
        if self.reset.object() != object
            || self.reset.notification_target().is_some()
            // Legacy continuations own a fixed deletion input. A bound word
            // can change inside native callbacks, including clearing bit 6.
            || !self.reset.flags_are_bound() && self.reset.native_flags() & 0x40 == 0
            || self.complete != self.reset.complete()
            || self.complete && self.pending.is_some()
            || self.asset_service_started
                && (self.pending != Some(Request::ReloadAppearance(object))
                    || !s.characters.contains_key(&object))
            || self
                .pending
                .as_ref()
                .is_some_and(|r| !self.reset.accepts_request(r))
        {
            return Err("invalid deletion person reset binding".into());
        }
        if let Some(Request::Script(thread)) = self.pending {
            if !self.owns_callback(&s.vm, p, thread, 1) {
                return Err("unbound deletion Reset callback".into());
            }
        }
        Ok(())
    }
}

impl Simulation {
    pub fn pending_deletion_character_services(&self) -> Vec<ObjectId> {
        self.deletion
            .last()
            .and_then(|d| d.person_reset.as_ref())
            .and_then(|e| match e.pending {
                Some(Request::ReloadAppearance(id)) if self.characters.contains_key(&id) => {
                    Some(id)
                }
                _ => None,
            })
            .into_iter()
            .collect()
    }

    pub fn begin_deletion_character_service(&mut self, id: ObjectId) -> Result<(), String> {
        if !self.pending_deletion_character_services().contains(&id) {
            return Err("stale deletion character service".into());
        }
        let e = self
            .deletion
            .last_mut()
            .unwrap()
            .person_reset
            .as_mut()
            .unwrap();
        if e.asset_service_started {
            return Err("deletion character service already started".into());
        }
        e.asset_service_started = true;
        Ok(())
    }

    /// The appearance host has completed the actual record/geometry work.
    /// Reset remains in phase 2 until this matching saved request is completed.
    pub fn complete_deletion_character_service(
        &mut self,
        p: &Program,
        id: ObjectId,
    ) -> Result<(), String> {
        if !self.pending_deletion_character_services().contains(&id) {
            return Err("stale deletion character service".into());
        }
        let e = self.deletion.last().unwrap().person_reset.as_ref().unwrap();
        e.validate(self, p, id)?;
        if !e.asset_service_started {
            return Err("deletion appearance asset work not started".into());
        }
        let resident = &self.characters[&id];
        if resident.character.busy()
            || resident.ordinary_update.is_some()
            || resident.refresh_tick.is_some()
        {
            return Err("deletion appearance has a pending character update".into());
        }
        resident.character.validate(p)?;
        let e = self
            .deletion
            .last_mut()
            .unwrap()
            .person_reset
            .as_mut()
            .unwrap();
        e.pending = None;
        e.asset_service_started = false;
        Ok(())
    }

    pub(crate) fn drive_deletion_person_reset(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let id = self
            .deletion
            .last()
            .ok_or("person reset without deletion")?
            .target;
        if self.vm.world.objects[&id].person.is_none() {
            return Ok(true);
        }
        let suppress = self
            .removals
            .iter()
            .find(|r| r.target == id)
            .is_some_and(|r| r.suppress_reset);
        let retained = self.deletion.last_mut().unwrap().person_reset.take();
        let mut e = if let Some(retained) = retained {
            retained
        } else {
            let mut fresh = Execution::new(id, suppress);
            if self.vm.world.objects[&id].native_flags.is_some() {
                fresh.reset.bind_storage(&self.vm)?;
            }
            fresh
        };
        let result = self.drive_deletion_reset_inner(p, &mut e, remaining, step);
        self.deletion.last_mut().unwrap().person_reset = Some(e);
        result
    }
    fn drive_deletion_reset_inner(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        loop {
            if e.complete {
                return Ok(true);
            }
            if !self.drive_cancellation(p, remaining, step)? {
                return Ok(false);
            }
            if e.pending.is_none() {
                e.pending = e.reset.next(&mut self.vm, p)?;
            }
            let Some(request) = e.pending.clone() else {
                e.complete = true;
                return Ok(true);
            };
            match request {
                Request::InterruptAnimation { .. } | Request::InterruptRoute { .. } => {
                    return Err(
                        "deletion person Reset needs native suspended-call interruption".into(),
                    );
                }
                Request::NotifyUsed(_)
                | Request::ResetUsed(_)
                | Request::ClearRouteReferences { .. } => {
                    return Err(
                        "person Reset needs used-target notification and virtual Reset services"
                            .into(),
                    );
                }
                Request::ClearAnimator(id) => {
                    if self.characters.contains_key(&id) {
                        self.reset_character(id)?;
                    }
                    // Actual full reset (+0x0c / 0x589220), distinct from the
                    // placement refresh that preserved the foreground pose.
                    self.animations.remove(&id);
                    self.backgrounds.remove(&id);
                    self.carry.remove(&id);
                }
                Request::ReloadAppearance(id) => {
                    if self.characters.contains_key(&id) {
                        return Ok(false);
                    }
                    sims_vm::person_reset::reload_bound_profile(&mut self.vm, p, id)?;
                    self.vm
                        .world
                        .objects
                        .get_mut(&id)
                        .unwrap()
                        .accessories
                        .clear();
                }
                Request::ClearRouting(id) => {
                    let thread = self.vm.object_threads[&id];
                    // Suppressed used-target resets have already unwound their
                    // action frames. Pending VM route services remain gated.
                    self.release(thread);
                    self.route_control.retain(|_, c| c.thread != thread);
                    self.route_feedback.retain(|_, f| f.thread != thread);
                    self.turns.remove(&id);
                    self.portal_cache.remove(&id);
                }
                Request::ClearActionIcons(id) => {
                    if self.interactions.contains_key(&id) {
                        return Err("deletion reset of authored interaction icons required".into());
                    }
                }
                Request::Placement {
                    object,
                    marked_for_deletion,
                } => {
                    let location = self
                        .native_lot
                        .as_ref()
                        .ok_or("native person deletion host absent")?
                        .placement
                        .links
                        .capture(&mut self.vm.world, object)?;
                    if location.container.is_some() || !location.on_lot && !marked_for_deletion {
                        return Err("person reset detach/reposition service required".into());
                    }
                }
                Request::Script(thread) => {
                    if !self.drive_deletion_callback(p, thread, remaining, step)? {
                        return Ok(false);
                    }
                    self.vm.threads[&thread]
                        .callback_result
                        .ok_or("person deletion Reset result absent")?;
                }
                Request::Footprint(_) => {
                    return Err("person deletion Reset footprint service required".into());
                }
                Request::Cancel(callback) => {
                    self.vm
                        .world
                        .actions
                        .as_mut()
                        .ok_or("person reset action storage absent")?
                        .callbacks
                        .push_back(callback);
                }
            }
            e.pending = None;
        }
    }
}
