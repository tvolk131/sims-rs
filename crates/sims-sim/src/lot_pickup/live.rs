//! Actual VM callbacks and physical placement for the saved Buy removal call.
use super::{Call, Request, View};
use crate::{Simulation, Step, lot_dynamic, lot_placement};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, Status, ThreadId, Vm, World};

pub struct Borrowed<'a> {
    pub world: &'a World,
    pub program: &'a Program,
}
impl Borrowed<'_> {
    pub fn id(&self, handle: u32) -> Result<ObjectId, String> {
        self.world
            .scheduler_storage
            .as_ref()
            .and_then(|s| s.object_for_handle(handle))
            .ok_or("pickup object lifetime ended".into())
    }
    pub fn handle(&self, id: ObjectId) -> Result<u32, String> {
        self.world
            .scheduler_storage
            .as_ref()
            .and_then(|s| s.handle(id))
            .ok_or("pickup requires retained registry storage".into())
    }
    fn object(&self, handle: u32) -> Result<&sims_vm::Object, String> {
        self.world
            .objects
            .get(&self.id(handle)?)
            .ok_or("pickup object absent".into())
    }
}
impl View for Borrowed<'_> {
    fn multipart(&self, o: u32) -> Result<bool, String> {
        Ok(self.object(o)?.multitile.is_some())
    }
    fn dynamic(&self, o: u32) -> Result<bool, String> {
        sims_vm::multitile::dynamic(self.world, self.program, self.id(o)?)
    }
    fn grouped_definition(&self, o: u32) -> Result<bool, String> {
        let d = self
            .object(o)?
            .definition
            .as_ref()
            .ok_or("pickup definition absent")?;
        let word = self
            .program
            .modules
            .get(&d.source)
            .and_then(|m| m.object_definition_words.get(&d.object))
            .and_then(|w| w.get(10))
            .ok_or("pickup group word absent")?;
        Ok(*word != 0)
    }
    fn head(&self, o: u32) -> Result<Option<u32>, String> {
        self.object(o)?
            .multitile
            .as_ref()
            .ok_or("pickup multipart receiver absent")?
            .head
            .map(|id| self.handle(id))
            .transpose()
    }
    fn next(&self, o: u32) -> Result<Option<u32>, String> {
        self.object(o)?
            .multitile
            .as_ref()
            .ok_or("pickup multipart receiver absent")?
            .next
            .map(|id| self.handle(id))
            .transpose()
    }
    fn slots(&self, o: u32) -> Result<usize, String> {
        Ok(self.object(o)?.slots.len())
    }
    fn child(&self, o: u32, slot: usize) -> Result<Option<u32>, String> {
        //Original checks the CURRENT vector bound, then signed registry ID.
        Ok(self
            .object(o)?
            .slots
            .get(slot)
            .copied()
            .flatten()
            .and_then(|id| self.world.scheduler_storage.as_ref()?.handle(id)))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Callback {
    object: u32,
    id: ObjectId,
    thread: ThreadId,
    depth: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    tick: u64,
    call: Call,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    caller: Option<crate::edit_history::object::driver::ServiceCaller>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cursor_caller: Option<u32>,
    callback: Option<Callback>,
    split: Option<lot_dynamic::live::Execution>,
    pub(crate) placement: Option<lot_placement::live::Execution>,
}
impl Execution {
    pub fn complete(&self) -> bool {
        self.call.complete()
    }
    pub fn object(&self) -> u32 {
        self.call.object()
    }
    pub fn command_caller(&self) -> Option<crate::edit_history::object::driver::ServiceCaller> {
        self.caller
    }
    pub(crate) fn cursor_binding(&self) -> Option<(u32, u32)> {
        self.cursor_caller.map(|h| (h, self.object()))
    }
    pub fn pending(&self) -> Option<Request> {
        self.call.pending()
    }
    pub(crate) fn owns_callback(
        &self,
        vm: &Vm,
        p: &Program,
        thread: ThreadId,
        depth: usize,
    ) -> bool {
        self.split
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self
                .placement
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self.callback.as_ref().is_some_and(|c| {
                let Some(t) = vm.threads.get(&thread) else {
                    return false;
                };
                let Some(f) = t.stack.get(depth) else {
                    return false;
                };
                let Some(d) = vm
                    .world
                    .objects
                    .get(&c.id)
                    .and_then(|o| o.definition.as_ref())
                else {
                    return false;
                };
                let action = p
                    .modules
                    .get(&d.source)
                    .and_then(|m| m.object_functions.get(&d.object))
                    .and_then(|v| v.get(12))
                    .map(|f| f.action);
                c.thread == thread
                    && c.depth == depth
                    && vm
                        .world
                        .scheduler_storage
                        .as_ref()
                        .and_then(|s| s.handle(c.id))
                        == Some(c.object)
                    && t.callback
                        .iter()
                        .flat_map(|b| b.chain())
                        .any(|b| b.external && b.base_depth == depth)
                    && f.caller == c.id
                    && f.callee == c.id
                    && f.code_owner == d.source
                    && Some(f.location.bhav) == action
            })
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        self.call.validate()?;
        if self.cursor_binding() != s.cursor_factory.as_ref().and_then(|f| f.pickup_binding())
            || self.cursor_caller.is_some() && (self.caller.is_some() || self.call.split())
        {
            return Err("pickup cursor owner differs".into());
        }
        if let Some(caller) = self.caller {
            if command_request(caller.service)? != (self.object(), self.call.split()) {
                return Err("pickup lost object command binding".into());
            }
        }
        let view = Borrowed {
            world: &s.vm.world,
            program: p,
        };
        let id = view.id(self.object())?;
        if self.tick != s.vm.tick
            || s.native_lot.is_none()
            || s.ground_placement.is_some()
            || s.synchronous_removal.is_some()
            || s.native_selection.is_some()
            || s.portrait_person.is_some()
            || self.complete()
                && (self.callback.is_some() || self.split.is_some() || self.placement.is_some())
        {
            return Err("invalid live pickup owner".into());
        }
        if let Some(c) = &self.callback {
            if self.pending() != Some(Request::Function12(c.object))
                || !self.owns_callback(&s.vm, p, c.thread, c.depth)
            {
                return Err("pickup lost function12 callback".into());
            }
        }
        if let Some(e) = &self.split {
            if self.pending() != Some(Request::Split(self.object())) || e.binding() != id {
                return Err("pickup lost dynamic split".into());
            }
            e.validate(&s.vm, p)?;
        }
        if let Some(e) = &self.placement {
            if self.pending() != Some(Request::Physical(self.object())) {
                return Err("pickup lost physical removal".into());
            }
            e.validate(&s.vm, p, id)?;
        }
        Ok(())
    }
}
impl Simulation {
    pub fn begin_object_pickup(
        &mut self,
        p: &Program,
        handle: u32,
        split: bool,
    ) -> Result<(), String> {
        self.begin_pickup_owned(p, handle, split, None)
    }
    fn begin_pickup_owned(
        &mut self,
        p: &Program,
        handle: u32,
        split: bool,
        cursor_caller: Option<u32>,
    ) -> Result<(), String> {
        if cursor_caller.map(|h| (h, handle))
            != self
                .cursor_factory
                .as_ref()
                .and_then(|f| f.pickup_binding())
            || cursor_caller.is_none() && self.cursor_work_pending()
            || self.object_pickup.is_some()
            || self.object_placement.is_some()
            || self.ground_placement.is_some()
            || self.synchronous_removal.is_some()
            || self.native_selection.is_some()
            || self.portrait_person.is_some()
            || self.vm.player_menu.is_some()
            || !self.deletion.is_empty()
            || self.visit.is_some()
            || self.cursor_has_active_vm_callback()
            || self.person_factories.values().any(|f| !f.finalized)
        {
            return Err("pickup overlaps synchronous work".into());
        }
        let view = Borrowed {
            world: &self.vm.world,
            program: p,
        };
        let id = view.id(handle)?;
        if self.vm.world.objects[&id].person.is_some() {
            return Err("Buy pickup requires an ordinary or multipart object".into());
        }
        let e = Execution {
            tick: self.vm.tick,
            call: Call::new(handle, split),
            caller: None,
            cursor_caller,
            callback: None,
            split: None,
            placement: None,
        };
        e.validate(self, p)?;
        self.object_pickup = Some(e);
        Ok(())
    }
    pub(crate) fn begin_cursor_pickup(
        &mut self,
        p: &Program,
        owner: u32,
        target: u32,
    ) -> Result<(), String> {
        self.begin_pickup_owned(p, target, false, Some(owner))
    }
    pub(crate) fn finish_cursor_pickup(
        &mut self,
        p: &Program,
        owner: u32,
        target: u32,
    ) -> Result<(), String> {
        let e = self.object_pickup.as_ref().ok_or("cursor pickup absent")?;
        e.validate(self, p)?;
        if !e.complete() || e.cursor_binding() != Some((owner, target)) {
            return Err("cursor pickup not returned to its caller".into());
        }
        self.object_pickup = None;
        Ok(())
    }
    pub fn take_object_pickup(&mut self) -> Result<u32, String> {
        let e = self.object_pickup.as_ref().ok_or("pickup absent")?;
        if !e.complete() || e.caller.is_some() || e.cursor_caller.is_some() {
            return Err("pickup incomplete".into());
        }
        let object = e.object();
        self.object_pickup = None;
        Ok(object)
    }
    /// Bind the actual pending+6c service of the saved ObjectUndoable stack.
    pub fn begin_object_command_pickup(
        &mut self,
        p: &Program,
        caller: &crate::edit_history::object::driver::Execution,
    ) -> Result<(), String> {
        let identity = caller
            .service_caller()
            .ok_or("object command service absent")?;
        let (handle, split) = command_request(identity.service)?;
        self.begin_object_pickup(p, handle, split)?;
        self.object_pickup.as_mut().unwrap().caller = Some(identity);
        Ok(())
    }
    pub fn finish_object_command_pickup(
        &mut self,
        p: &Program,
        caller: &mut crate::edit_history::object::driver::Execution,
    ) -> Result<(), String> {
        use crate::edit_history::{
            object::driver,
            snapshot::{lifecycle, placement},
        };
        let e = self.object_pickup.as_ref().ok_or("object pickup absent")?;
        e.validate(self, p)?;
        let identity = e.caller.ok_or("pickup has no object command owner")?;
        if !e.complete() || caller.service_caller() != Some(identity) {
            return Err("object pickup unfinished or caller changed".into());
        }
        let reply = match identity.service {
            driver::Service::Placement(_) => driver::Reply::Placement(placement::Reply::Done),
            driver::Service::Restore(lifecycle::Request::Placement(_)) => {
                driver::Reply::Restore(lifecycle::Reply::Placement(placement::Reply::Done))
            }
            driver::Service::Restore(lifecycle::Request::Remove(_)) => {
                driver::Reply::Restore(lifecycle::Reply::Removed)
            }
            driver::Service::Remove(_) => driver::Reply::Done,
            _ => return Err("object command is not a pickup".into()),
        };
        caller.acknowledge(identity.service, reply)?;
        self.object_pickup = None;
        Ok(())
    }
    pub fn advance_object_pickup(&mut self, p: &Program, budget: usize) -> Result<Step, String> {
        self.object_pickup
            .as_ref()
            .ok_or("pickup absent")?
            .validate(self, p)?;
        let mut e = self.object_pickup.take().unwrap();
        let mut step = Step {
            stop: crate::StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        let mut remaining = budget;
        let result = self.drive_object_pickup(p, &mut e, &mut remaining, &mut step);
        self.object_pickup = Some(e);
        self.sync_spatial_world();
        self.validate_native_lot()?;
        result?;
        Ok(step)
    }
    fn drive_object_pickup(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<(), String> {
        while *remaining > 0 && !e.complete() {
            if let Some(c) = &e.callback {
                let t = self
                    .vm
                    .threads
                    .get(&c.thread)
                    .ok_or("pickup callback thread absent")?;
                if t.callback.is_some() {
                    if t.status != Status::Ready {
                        return Err(format!("pickup callback host required: {:?}", t.status));
                    }
                    let before = t.stack.clone();
                    let slice = self.vm.run_slice(p, c.thread, 1)?;
                    if slice.trace.is_empty()
                        && self.vm.threads[&c.thread].callback.is_some()
                        && self.vm.threads[&c.thread].stack == before
                        && self.vm.threads[&c.thread].status == Status::Ready
                    {
                        return Err("pickup callback made no progress".into());
                    }
                    *remaining -= slice.trace.len();
                    step.trace.extend(slice.trace);
                    if self.vm.threads[&c.thread].callback.is_some() {
                        continue;
                    }
                }
                self.vm.threads[&c.thread]
                    .callback_result
                    .ok_or("pickup callback result absent")?;
                e.call.acknowledge(Request::Function12(c.object))?;
                e.callback = None;
                self.publish_live_placement_caches()?;
                continue;
            }
            if let Some(split) = &mut e.split {
                if !self.drive_dynamic_update(p, split, remaining, step)? {
                    return Ok(());
                }
                e.call.acknowledge(Request::Split(e.object()))?;
                e.split = None;
                continue;
            }
            if let Some(placement) = &mut e.placement {
                if !self.drive_removal_placement(p, placement, remaining, step)? {
                    return Ok(());
                }
                e.call.acknowledge(Request::Physical(e.object()))?;
                e.placement = None;
                continue;
            }
            let view = Borrowed {
                world: &self.vm.world,
                program: p,
            };
            let Some(request) = e.call.advance(&view)? else {
                break;
            };
            //Count each callback boundary even if no tree is installed. A
            //corrupt live chain cannot spin forever without yielding a budget.
            *remaining -= 1;
            match request {
                Request::Function12(handle) => {
                    let id = view.id(handle)?;
                    let depth = self
                        .vm
                        .object_threads
                        .get(&id)
                        .and_then(|t| self.vm.threads.get(t))
                        .map_or(0, |t| t.stack.len());
                    if let Some(thread) = self.vm.begin_loaded_function(p, id, 12)? {
                        e.callback = Some(Callback {
                            object: handle,
                            id,
                            thread,
                            depth,
                        });
                    } else {
                        e.call.acknowledge(request)?;
                    }
                }
                Request::Split(handle) => {
                    let id = view.id(handle)?;
                    e.split = Some(lot_dynamic::live::Execution::begin_split(self, id)?);
                }
                Request::Physical(handle) => {
                    let id = view.id(handle)?;
                    e.placement = Some(lot_placement::live::Execution::new_virtual(
                        lot_placement::VirtualOperation::remove(
                            &self.vm.world,
                            &self.native_lot.as_ref().unwrap().placement.links,
                            id,
                            true,
                        )?,
                    ));
                }
            }
        }
        Ok(())
    }
}

fn command_request(
    service: crate::edit_history::object::driver::Service,
) -> Result<(u32, bool), String> {
    use crate::edit_history::{
        object::driver::Service,
        snapshot::{lifecycle, placement},
    };
    match service {
        Service::Placement(placement::Request::RemoveFromLot(o))
        | Service::Restore(lifecycle::Request::Placement(placement::Request::RemoveFromLot(o))) => {
            Ok((o, false))
        }
        Service::Remove(lifecycle::RemoveObject(o))
        | Service::Restore(lifecycle::Request::Remove(lifecycle::RemoveObject(o))) => Ok((o, true)),
        _ => Err("object command has no pending RemoveFromLot service".into()),
    }
}
