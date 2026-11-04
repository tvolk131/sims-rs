//! Live synchronous Reset call stack. Native broadcasts finish each recipient
//! before advancing the live person vector; guards belong to the active Reset,
//! not to scheduler flags. The enclosing host retains this stack across budgets.
use crate::{Simulation, Step, lot_reset};
use serde::{Deserialize, Serialize};
use sims_vm::{
    AnimationRequest, ObjectId, Program, Status, ThreadId, Vm,
    person_reset::{Broadcast, Request, Reset},
};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Person {
    reset: Reset,
    pending: Option<Request>,
    asset_started: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
enum Frame {
    Person(Person),
    Broadcast {
        controller: Broadcast,
        child: Option<ObjectId>,
    },
    Base(lot_reset::live::Execution),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Execution {
    object: ObjectId,
    #[serde(default)]
    broadcast_root: bool,
    stack: Vec<Frame>,
}
enum Child {
    Broadcast(ObjectId),
    Reset(ObjectId),
    Notify { person: ObjectId, target: ObjectId },
}
enum Outcome {
    Continue,
    Yield,
    Child(Child),
    Done,
}

impl Execution {
    pub fn new(s: &Simulation, object: ObjectId) -> Result<Self, String> {
        let mut e = Self {
            object,
            broadcast_root: false,
            stack: vec![],
        };
        let root = e.make_child(s, Child::Reset(object))?;
        e.stack.push(root);
        Ok(e)
    }
    pub(crate) fn broadcast(object: ObjectId) -> Self {
        Self {
            object,
            broadcast_root: true,
            stack: vec![Frame::Broadcast {
                controller: Broadcast::new(object),
                child: None,
            }],
        }
    }
    pub(crate) fn broadcasts(&self, object: ObjectId) -> bool {
        self.broadcast_root && self.object == object
    }
    pub fn object(&self) -> ObjectId {
        self.object
    }
    pub fn complete(&self) -> bool {
        self.stack.is_empty()
    }
    pub(crate) fn retained_route(
        &self,
        thread: ThreadId,
        request: u64,
    ) -> Option<&sims_vm::RouteRequest> {
        self.stack.iter().find_map(|f| match f {
            Frame::Person(e) => e.reset.retained_route(thread, request),
            _ => None,
        })
    }
    fn flags(&self, s: &Simulation, object: ObjectId) -> u32 {
        if let Some(flags) = s.vm.world.objects.get(&object).and_then(|o| o.native_flags) {
            return flags;
        }
        // Legacy snapshots did not store +114. Their existing continuation
        // owners still supply Reset/deletion bits; do not synthesize a word.
        let mut flags = 0;
        for pending in s.removals.iter().filter(|r| r.target == object) {
            flags |= 0x40 | if pending.suppress_reset { 0x1000 } else { 0 };
        }
        if s.deletion.iter().any(|d| d.target == object) {
            flags |= 0x40;
        }
        for frame in &self.stack {
            if let Frame::Person(e) = frame {
                if e.reset.object() == object {
                    flags |= e.reset.native_flags();
                }
            }
        }
        flags
    }
    fn make_child(&self, s: &Simulation, child: Child) -> Result<Frame, String> {
        let (object, notification) = match child {
            Child::Broadcast(target) => {
                return Ok(Frame::Broadcast {
                    controller: Broadcast::new(target),
                    child: None,
                });
            }
            Child::Reset(object) => (object, None),
            Child::Notify { person, target } => (person, Some(target)),
        };
        let o =
            s.vm.world
                .objects
                .get(&object)
                .ok_or("live Reset target absent")?;
        if o.person.is_none() {
            if notification.is_some() {
                return Err("person Reset notification addressed to a base object".into());
            }
            return Ok(Frame::Base(lot_reset::live::Execution::new(object)));
        }
        let flags = self.flags(s, object);
        let mut reset = if let Some(target) = notification {
            Reset::notified(object, flags, target, self.flags(s, target))
        } else {
            Reset::new(object, flags)
        };
        if o.native_flags.is_some() {
            reset.bind_storage(&s.vm)?;
        }
        Ok(Frame::Person(Person {
            reset,
            pending: None,
            asset_started: false,
        }))
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.stack.iter().any(|frame| match frame {
            Frame::Person(e) => {
                matches!(e.pending, Some(Request::Script(t)) if t == thread)
                    && e.reset.owns_callback(vm, p, thread, depth)
            }
            Frame::Base(e) => e.owns_callback(vm, p, thread, depth),
            _ => false,
        })
    }
    pub fn pending_appearance(&self) -> Option<(ObjectId, bool)> {
        match self.stack.last()? {
            Frame::Person(e) => match e.pending {
                Some(Request::ReloadAppearance(id)) => Some((id, e.asset_started)),
                _ => None,
            },
            _ => None,
        }
    }
    pub(crate) fn begin_appearance(&mut self, id: ObjectId) -> Result<(), String> {
        if self.pending_appearance() != Some((id, false)) {
            return Err("stale live Reset appearance start".into());
        }
        let Frame::Person(e) = self.stack.last_mut().unwrap() else {
            unreachable!()
        };
        e.asset_started = true;
        Ok(())
    }
    pub(crate) fn finish_appearance(&mut self, id: ObjectId) -> Result<(), String> {
        if self.pending_appearance() != Some((id, true)) {
            return Err("stale live Reset appearance completion".into());
        }
        let Frame::Person(e) = self.stack.last_mut().unwrap() else {
            unreachable!()
        };
        e.asset_started = false;
        e.pending = None;
        Ok(())
    }
    pub fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        if self.object <= 0
            || self.stack.len() > 256
            || !s.vm.world.objects.contains_key(&self.object)
        {
            return Err("invalid live Reset root or stack".into());
        }
        let own = |frame: &Frame, object| match frame {
            Frame::Person(e) => {
                e.reset.object() == object && e.reset.notification_target().is_none()
            }
            Frame::Base(e) => e.object() == object,
            _ => false,
        };
        if self.stack.first().is_some_and(|f| if self.broadcast_root {
            !matches!(f, Frame::Broadcast { controller, .. } if controller.target() == self.object)
        } else { !own(f, self.object) }) {
            return Err("live Reset root changed".into());
        }
        for (index, frame) in self.stack.iter().enumerate() {
            match frame {
                Frame::Person(e) => {
                    e.reset.validate(&s.vm, p)?;
                    if e.pending
                        .as_ref()
                        .is_some_and(|r| !e.reset.accepts_request(r))
                        || e.asset_started
                            && (!matches!(e.pending, Some(Request::ReloadAppearance(id)) if id == e.reset.object())
                                || index + 1 != self.stack.len())
                    {
                        return Err("invalid live person Reset service binding".into());
                    }
                }
                Frame::Base(e) => e.validate(s, p)?,
                Frame::Broadcast { controller, child } => {
                    if !s.vm.world.objects.contains_key(&controller.target())
                        || controller.complete() && child.is_some()
                    {
                        return Err("invalid live Reset broadcast".into());
                    }
                }
            }
            if let Some(child) = self.stack.get(index + 1) {
                let bound = match frame {
                    Frame::Person(e) => match e.pending {
                        Some(Request::NotifyUsed(target)) => {
                            matches!(child, Frame::Broadcast { controller, .. } if controller.target() == target)
                        }
                        Some(Request::ResetUsed(target)) => own(child, target),
                        _ => false,
                    },
                    Frame::Broadcast {
                        controller,
                        child: Some(person),
                    } => matches!(child,
                        Frame::Person(e) if e.reset.object() == *person && e.reset.notification_target() == Some(controller.target())),
                    _ => false,
                };
                if !bound {
                    return Err("orphan nested live Reset".into());
                }
            } else if matches!(frame, Frame::Broadcast { child: Some(_), .. }) {
                return Err("live Reset broadcast lost its recipient".into());
            }
        }
        Ok(())
    }
    fn finish_child(&mut self) -> Result<(), String> {
        match self.stack.last_mut() {
            Some(Frame::Person(e))
                if matches!(
                    e.pending,
                    Some(Request::NotifyUsed(_) | Request::ResetUsed(_))
                ) =>
            {
                e.pending = None
            }
            Some(Frame::Broadcast { child, .. }) if child.is_some() => *child = None,
            None => (),
            _ => return Err("completed live Reset has no waiting caller".into()),
        }
        Ok(())
    }
}

impl Simulation {
    pub(crate) fn reset_route_request(
        &self,
        thread: ThreadId,
        request: u64,
    ) -> Option<&sims_vm::RouteRequest> {
        self.reset_executions()
            .find_map(|e| e.retained_route(thread, request))
    }
    fn reset_executions(&self) -> impl Iterator<Item = &Execution> {
        self.native_burn
            .as_ref()
            .and_then(|b| b.reset.as_ref())
            .into_iter()
            .chain(
                self.deletion
                    .iter()
                    .filter_map(|d| d.entry.as_ref()?.references.as_ref()),
            )
    }
    fn reset_appearance_mut(&mut self, id: ObjectId) -> Result<&mut Execution, String> {
        self.native_burn
            .as_mut()
            .and_then(|b| b.reset.as_mut())
            .into_iter()
            .chain(
                self.deletion
                    .iter_mut()
                    .filter_map(|d| d.entry.as_mut()?.references.as_mut()),
            )
            .find(|e| {
                e.pending_appearance()
                    .is_some_and(|(object, _)| object == id)
            })
            .ok_or_else(|| "live Reset appearance owner absent".into())
    }

    /// 45e791 erases [begin,end) through 45fad7. Each 0x9c-byte context
    /// releases its candidate vector; it does not finish a route, cancel a
    /// shoo action, clear the retained path (+0xae0), or reset portal costs.
    fn clear_reset_route_contexts(&mut self, id: ObjectId) -> Result<(), String> {
        let thread = *self
            .vm
            .object_threads
            .get(&id)
            .ok_or("Reset route owner absent")?;
        let native = self.native_lot.as_ref().ok_or("Reset route host absent")?;
        if native.route.is_some()
            || native.shoo.is_some()
            || native.movement.is_some()
            || native.completion.is_some()
            || self.routes.contains_key(&thread)
            || self.turns.contains_key(&id)
            || self
                .characters
                .get(&id)
                .is_some_and(|r| r.native_turn.is_some())
            || self.route_feedback.values().any(|f| f.thread == thread)
        {
            return Err("Reset route clear has an interrupted physical/route host service".into());
        }
        self.route_control.retain(|_, c| c.caller != id);
        Ok(())
    }
    /// 45e791's notification tail leaves each route context in place. A
    /// matching blocker (+80) is cleared, success (+84) becomes true, and its
    /// action serial (+90) becomes zero. It does not cancel the shoo action or
    /// finish/restart the route, and ignores the action's original serial.
    fn clear_reset_route_references(
        &mut self,
        object: ObjectId,
        target: ObjectId,
    ) -> Result<(), String> {
        for id in [object, target] {
            if self
                .vm
                .world
                .objects
                .get(&id)
                .is_none_or(|o| o.person.is_none())
            {
                return Err("person route-reference cleanup target absent".into());
            }
        }
        for state in self
            .route_control
            .values_mut()
            .filter(|s| s.caller == object)
        {
            if state.link.as_ref().is_some_and(|l| l.blocker == target) {
                state.link = None;
                state.blocker_success = true;
            }
        }
        Ok(())
    }
    pub fn pending_reset_character_services(&self) -> Vec<(ObjectId, bool)> {
        self.reset_executions()
            .filter_map(Execution::pending_appearance)
            .collect()
    }
    pub fn begin_reset_character_service(&mut self, id: ObjectId) -> Result<(), String> {
        self.reset_appearance_mut(id)?.begin_appearance(id)
    }
    pub fn complete_reset_character_service(
        &mut self,
        p: &Program,
        id: ObjectId,
    ) -> Result<(), String> {
        let e = self
            .reset_executions()
            .find(|e| {
                e.pending_appearance()
                    .is_some_and(|(object, _)| object == id)
            })
            .ok_or("live Reset appearance owner absent")?;
        e.validate(self, p)?;
        if e.pending_appearance() != Some((id, true)) {
            return Err("live Reset appearance is not in flight".into());
        }
        let r = self
            .characters
            .get(&id)
            .ok_or("live Reset character absent")?;
        if r.character.busy() || r.ordinary_update.is_some() || r.refresh_tick.is_some() {
            return Err("live Reset appearance has unfinished character work".into());
        }
        r.character.validate(p)?;
        self.reset_appearance_mut(id)?.finish_appearance(id)
    }

    pub(crate) fn interrupt_reset_animation(
        &mut self,
        reset: &mut Reset,
        thread: ThreadId,
        request: &AnimationRequest,
    ) -> Result<(), String> {
        let r = self
            .characters
            .get(&request.caller)
            .ok_or("Reset interruption requires a loaded native character")?;
        if r.animation_yield
            .is_none_or(|(t, id, tick)| t != thread || id != request.id || tick > self.vm.tick)
            || r.character.busy()
            || r.ordinary_update.is_some()
            || r.person_update.is_some()
            || r.refresh_tick.is_some()
            || r.skill_profile.is_some()
            || r.physical_movement.is_some()
            || r.native_walk.is_some()
            || r.native_turn.is_some()
        {
            return Err("Reset interruption has unfinished native character work".into());
        }
        reset.interrupt_animation(&mut self.vm, thread, request)?;
        self.characters
            .get_mut(&request.caller)
            .unwrap()
            .animation_yield = None;
        Ok(())
    }
    pub(crate) fn interrupt_reset_route(
        &mut self,
        reset: &mut Reset,
        thread: ThreadId,
        request: &sims_vm::RouteRequest,
    ) -> Result<(), String> {
        let id = request.caller;
        let native = self.native_lot.as_ref().ok_or("Reset route host absent")?;
        let resident = self
            .characters
            .get(&id)
            .ok_or("Reset route requires a loaded native character")?;
        if native.route.is_some()
            || native.shoo.is_some()
            || native.movement.is_some()
            || native.completion.is_some()
            || self.routes.contains_key(&thread)
            || self.turns.contains_key(&id)
            || self.route_feedback.values().any(|f| f.thread == thread)
            || resident.character.busy()
            || resident.ordinary_update.is_some()
            || resident.person_update.is_some()
            || resident.refresh_tick.is_some()
            || resident.skill_profile.is_some()
            || resident.physical_movement.is_some()
            || resident.native_turn.is_some()
            || resident.native_walk.as_ref().is_some_and(|w| {
                w.thread != thread
                    || w.request != request.id
                    || w.target != request.target
                    || w.poll.is_some()
                    || w.physical.is_some()
                    || w.stopping.is_some()
            })
            || self.route_control.get(&request.id).is_some_and(|c| {
                c.thread != thread
                    || c.caller != id
                    || c.original.is_none()
                    || c.callback.is_some()
                    || c.deciding
                    || c.original
                        .as_ref()
                        .is_some_and(|o| o.crossing || o.seated.is_some() || o.standing.is_some())
            })
        {
            return Err("Reset route interruption has unfinished synchronous host work".into());
        }
        reset.interrupt_route(&mut self.vm, thread, request)?;
        // This wrapper owns the yielded Rust call, not SAnimator's path/pose.
        // Those remain unchanged until the native ClearAnimator service. The
        // route context and shoo link survive until native ClearRouting.
        self.characters.get_mut(&id).unwrap().native_walk = None;
        Ok(())
    }
    pub(crate) fn drive_person_reset(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        loop {
            let Some(mut frame) = e.stack.pop() else {
                return Ok(true);
            };
            let result = self.drive_reset_frame(p, &mut frame, remaining, step);
            match result {
                Ok(Outcome::Done) => e.finish_child()?,
                other => {
                    e.stack.push(frame);
                    match other? {
                        Outcome::Yield => return Ok(false),
                        Outcome::Child(child) => {
                            if e.stack.len() >= 256 {
                                return Err("live Reset call stack limit".into());
                            }
                            let child = e.make_child(self, child)?;
                            e.stack.push(child);
                        }
                        Outcome::Continue => (),
                        Outcome::Done => unreachable!(),
                    }
                }
            }
        }
    }
    fn drive_reset_frame(
        &mut self,
        p: &Program,
        frame: &mut Frame,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Outcome, String> {
        match frame {
            Frame::Broadcast { controller, child } => {
                if child.is_some() {
                    return Err("live Reset broadcast recipient still pending".into());
                }
                if let Some(person) = controller.next(&self.vm.world)? {
                    *child = Some(person);
                    Ok(Outcome::Child(Child::Notify {
                        person,
                        target: controller.target(),
                    }))
                } else {
                    Ok(Outcome::Done)
                }
            }
            Frame::Base(e) => {
                let c = &self
                    .household
                    .as_ref()
                    .ok_or("live base Reset needs lot calendar")?
                    .clock;
                let now = sims_vm::construction::ObjectDate {
                    year: c.year,
                    month: c.month,
                    day: c.day,
                };
                if e.advance(self, p, now, false, remaining, step)? {
                    Ok(Outcome::Done)
                } else {
                    Ok(Outcome::Yield)
                }
            }
            Frame::Person(e) => {
                if !self.drive_cancellation(p, remaining, step)? {
                    return Ok(Outcome::Yield);
                }
                if e.pending.is_none() {
                    e.pending = e.reset.next(&mut self.vm, p)?;
                }
                let Some(request) = e.pending.clone() else {
                    return Ok(Outcome::Done);
                };
                match request {
                    Request::InterruptRoute { thread, request } => {
                        self.interrupt_reset_route(&mut e.reset, thread, &request)?
                    }
                    Request::InterruptAnimation { thread, request } => {
                        self.interrupt_reset_animation(&mut e.reset, thread, &request)?
                    }
                    Request::NotifyUsed(id) => return Ok(Outcome::Child(Child::Broadcast(id))),
                    Request::ResetUsed(id) => return Ok(Outcome::Child(Child::Reset(id))),
                    Request::ClearAnimator(id) => self.reset_character(id)?,
                    Request::ReloadAppearance(_) => return Ok(Outcome::Yield),
                    Request::ClearRouting(id) => self.clear_reset_route_contexts(id)?,
                    Request::ClearActionIcons(id) => {
                        if self.interactions.contains_key(&id) {
                            return Err(
                                "ordinary person Reset requires authored interaction icon cleanup"
                                    .into(),
                            );
                        }
                    }
                    Request::Placement {
                        object,
                        marked_for_deletion,
                    } => {
                        let d = self
                            .native_lot
                            .as_ref()
                            .ok_or("live person Reset native lot absent")?
                            .placement
                            .links
                            .capture(&mut self.vm.world, object)?;
                        if d.container.is_some() || !d.on_lot && !marked_for_deletion {
                            return Err("ordinary person Reset detach/reposition required".into());
                        }
                    }
                    Request::Script(thread) => {
                        if self.vm.threads[&thread].callback.is_some() {
                            if *remaining == 0 {
                                return Ok(Outcome::Yield);
                            }
                            if self.vm.threads[&thread].status != Status::Ready {
                                return Err(format!(
                                    "live person Reset callback host required: {:?}",
                                    self.vm.threads[&thread].status
                                ));
                            }
                            let slice = self.vm.run_slice(p, thread, 1)?;
                            *remaining -= slice.trace.len();
                            step.trace.extend(slice.trace);
                            if self.vm.threads[&thread].callback.is_some() {
                                return Ok(Outcome::Continue);
                            }
                        }
                        self.vm.threads[&thread]
                            .callback_result
                            .ok_or("live person Reset callback result absent")?;
                    }
                    Request::Cancel(callback) => self
                        .vm
                        .world
                        .actions
                        .as_mut()
                        .ok_or("live Reset actions absent")?
                        .callbacks
                        .push_back(callback),
                    Request::Footprint(_) => {
                        return Err("ordinary person Reset footprint refresh required".into());
                    }
                    Request::ClearRouteReferences { object, target } => {
                        self.clear_reset_route_references(object, target)?
                    }
                }
                e.pending = None;
                Ok(Outcome::Continue)
            }
        }
    }
}
