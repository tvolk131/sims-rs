//! Ordered cXPerson::Reset, 0x45e791, for own resets and object notifications, including
//! active use frames and ordered used-target notification/Reset requests.
//! Used by frozen loading and live deferred deletion. Animator, routing, placement and queue callbacks remain
//! synchronous host requests. The host must complete each before advancing.
use crate::{
    ObjectId, Program, Status, ThreadId, Vm,
    action::{Action, Callback},
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[cfg(test)]
#[path = "person_reset_tests.rs"]
mod tests;

/// Registry 0x444306. The host completes each returned person's notification
/// before advancing again. Cursor order follows the live person vector; a
/// callback may append, erase or reorder its remaining entries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Broadcast {
    target: ObjectId,
    cursor: usize,
    complete: bool,
}
impl Broadcast {
    pub fn new(target: ObjectId) -> Self {
        Self {
            target,
            cursor: 0,
            complete: false,
        }
    }
    pub fn target(&self) -> ObjectId {
        self.target
    }
    pub fn complete(&self) -> bool {
        self.complete
    }
    pub fn next(&mut self, world: &crate::World) -> Result<Option<ObjectId>, String> {
        if self.complete {
            return Ok(None);
        }
        if self.target <= 0 {
            return Err("person broadcast target absent".into());
        }
        let people = &world
            .person
            .as_ref()
            .ok_or("person broadcast registry absent")?
            .person_order;
        if self.cursor > people.len() {
            return Err("person broadcast cursor passed its live registry end".into());
        }
        while let Some(&person) = people.get(self.cursor) {
            self.cursor += 1;
            if person != self.target {
                if world
                    .objects
                    .get(&person)
                    .is_none_or(|o| o.person.is_none())
                {
                    return Err("person broadcast registry entry absent".into());
                }
                return Ok(Some(person));
            }
        }
        self.complete = true;
        Ok(None)
    }
}

/// Gameplay field writes of SAnimator::ReloadAppearance (0x5888d3). Null
/// StringSet entries preserve the old field; an empty string is still present.
/// Mesh/texture destruction and rebuilding remain presentation host work.
pub fn reload_profile_fields(
    person: &mut std::collections::BTreeMap<i16, i16>,
    profile: &[Option<&str>],
    skin_tones: &[&str],
) -> Result<(), String> {
    if [22, 23, 58, 60, 65].iter().any(|i| !person.contains_key(i)) {
        return Err("person appearance fields absent".into());
    }
    // Preserve the public helper's validation-before-mutation contract.
    let mut next = person.clone();
    reload_profile_identity_fields(&mut next, profile, skin_tones)?;
    next.insert(22, 0);
    next.insert(23, 0);
    *person = next;
    Ok(())
}

/// Identity writes after appearance selection in 5888d3. The hands were zeroed
/// before selection; do not erase callback changes by zeroing them again here.
pub fn reload_profile_identity_fields(
    person: &mut std::collections::BTreeMap<i16, i16>,
    profile: &[Option<&str>],
    skin_tones: &[&str],
) -> Result<(), String> {
    if [58, 60, 65].iter().any(|i| !person.contains_key(i)) {
        return Err("person identity fields absent".into());
    }
    let row = |i: usize| profile.get(i).copied().flatten();
    let age = row(13).map(profile_age).transpose()?;
    if let Some(gender) = row(12) {
        if let Some(index) = [
            "male",
            "female",
            "dogmale",
            "dogfemale",
            "catmale",
            "catfemale",
        ]
        .iter()
        .position(|name| name.eq_ignore_ascii_case(gender))
        {
            person.insert(65, index as i16);
        }
    }
    if let Some(age) = age {
        person.insert(58, age);
    }
    if let Some(tone) = row(14) {
        let value = skin_tones
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, name)| name.eq_ignore_ascii_case(tone))
            .map_or(0, |(index, _)| index as i16);
        person.insert(60, value);
    }
    Ok(())
}

fn profile_age(text: &str) -> Result<i16, String> {
    let text = text.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (negative, text) = if let Some(s) = text.strip_prefix('-') {
        (true, s)
    } else {
        (false, text.strip_prefix('+').unwrap_or(text))
    };
    let mut value = 0i64;
    for digit in text.bytes().take_while(u8::is_ascii_digit) {
        value = value
            .checked_mul(10)
            .and_then(|v| v.checked_add(i64::from(digit - b'0')))
            .ok_or("unverified overflowing CRT profile age")?;
    }
    let value = if negative { -value } else { value };
    Ok(i32::try_from(value).map_err(|_| "unverified overflowing CRT profile age")? as i16)
}

/// Use the character's OBJD body StringSet and its semiglobal skin-tone table.
/// Missing linked resources are errors, never demo appearance defaults.
pub fn reload_bound_profile(vm: &mut Vm, p: &Program, object: ObjectId) -> Result<(), String> {
    let o = vm
        .world
        .objects
        .get_mut(&object)
        .ok_or("profile object absent")?;
    let d = o.definition.as_ref().ok_or("profile definition absent")?;
    let m = p.modules.get(&d.source).ok_or("profile module absent")?;
    let profile = vm
        .world
        .body_profiles
        .load(p, d)?
        .ok_or("profile requires a person definition")?;
    let tones = m
        .semiglobal
        .as_ref()
        .and_then(|s| p.modules.get(s))
        .and_then(|m| m.strings.get(&8192))
        .ok_or("person skin-tone StringSet not loaded")?;
    reload_profile_fields(
        o.person.as_mut().ok_or("profile requires a person")?,
        &profile.iter().map(|s| Some(s.as_str())).collect::<Vec<_>>(),
        &tones.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Request {
    /// Withdraw the host's suspended Animate Sim boundary without taking its
    /// return branch. The original Reset will pop/restart the frames itself;
    /// the current pose is retained until ClearAnimator.
    InterruptAnimation {
        thread: ThreadId,
        request: crate::AnimationRequest,
    },
    /// Withdraw the yielded route call without completing its instruction.
    /// Native route contexts remain owned until the later ClearRouting service.
    InterruptRoute {
        thread: ThreadId,
        request: crate::RouteRequest,
    },
    /// 0x444306 broadcast after the latest use has been unwound. The host must
    /// expose this Reset's guard to nested notifications before calling them.
    NotifyUsed(ObjectId),
    /// Used object's virtual +0x38(0), after its broadcast has completed.
    ResetUsed(ObjectId),
    ClearAnimator(ObjectId),
    ReloadAppearance(ObjectId),
    ClearRouting(ObjectId),
    ClearActionIcons(ObjectId),
    /// Derive containment; detach/reposition only when contained or off-lot.
    /// The deletion bit suppresses only the off-lot relocation branch.
    Placement {
        object: ObjectId,
        marked_for_deletion: bool,
    },
    /// The original Reset function on the newly installed Main. Run only this
    /// synchronous callback; its Main must not execute inside the host operation.
    Script(ThreadId),
    Footprint(ObjectId),
    Cancel(Callback),
    /// Notification tail for a person target: null matching route-person
    /// references after action/queue cleanup, before releasing the Reset guard.
    ClearRouteReferences {
        object: ObjectId,
        target: ObjectId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reset {
    object: ObjectId,
    /// Native object +0x114, supplied by the host. Expose the guard while the
    /// operation is suspended so nested notifications observe it immediately.
    flags: u32,
    /// New native objects retain the real word across callbacks. Legacy
    /// snapshots and explicit component probes can still supply detached flags.
    #[serde(default)]
    storage_bound: bool,
    phase: u8,
    thread: Option<ThreadId>,
    /// Captured before unwinding the latest use; callbacks may change the
    /// remaining use stack, which is read afresh after this pair of services.
    #[serde(default)]
    used_target: Option<ObjectId>,
    /// Nonzero native Reset argument and its flags captured on entry. The
    /// target's 0x1000 controls used-object resets, not this person's flags.
    #[serde(default)]
    notification: Option<(ObjectId, u32)>,
    /// Native temporary queue while filtering. Unrelated records are restored
    /// only after all live pending records and cancellation callbacks finish.
    #[serde(default)]
    retained: VecDeque<Action>,
    #[serde(default)]
    animation: Option<crate::AnimationRequest>,
    #[serde(default)]
    route: Option<crate::RouteRequest>,
}
impl Reset {
    pub fn new(object: ObjectId, native_flags: u32) -> Self {
        Self {
            object,
            flags: native_flags,
            storage_bound: false,
            phase: if native_flags & 0x800 != 0 { 8 } else { 0 },
            thread: None,
            used_target: None,
            notification: None,
            retained: VecDeque::new(),
            animation: None,
            route: None,
        }
    }
    pub fn notified(
        object: ObjectId,
        native_flags: u32,
        target: ObjectId,
        target_flags: u32,
    ) -> Self {
        Self {
            notification: Some((target, target_flags)),
            ..Self::new(object, native_flags)
        }
    }
    /// Bind the actual native word, including the already-active guard case.
    /// Notification suppression captures the target flags at entry; placement
    /// and the final guard clear reread the recipient's live word later.
    pub fn bound(vm: &Vm, object: ObjectId, target: Option<ObjectId>) -> Result<Self, String> {
        let read = |id| {
            vm.world
                .objects
                .get(&id)
                .and_then(|o| o.native_flags)
                .ok_or_else(|| format!("native Reset flag storage absent for #{id}"))
        };
        let flags = read(object)?;
        let mut reset = if let Some(target) = target {
            // Native returns on the recipient's guard before dereferencing the
            // notification target. Nested Reset must retain that no-op case.
            let target_flags = if flags & 0x800 != 0 { 0 } else { read(target)? };
            Self::notified(object, flags, target, target_flags)
        } else {
            Self::new(object, flags)
        };
        reset.bind_storage(vm)?;
        Ok(reset)
    }
    /// Bind recipient storage before entering Reset. Explicit notification
    /// inputs remain captured by the caller (also used by legacy snapshots
    /// whose target flags still belong to their saved host continuation).
    pub fn bind_storage(&mut self, vm: &Vm) -> Result<(), String> {
        if self.storage_bound || self.thread.is_some() || !matches!(self.phase, 0 | 8) {
            return Err("native Reset storage must bind before entry".into());
        }
        self.flags = vm
            .world
            .objects
            .get(&self.object)
            .and_then(|o| o.native_flags)
            .ok_or("native Reset recipient flags absent")?;
        self.phase = if self.flags & 0x800 != 0 { 8 } else { 0 };
        self.storage_bound = true;
        Ok(())
    }
    fn live_flags(&self, vm: &Vm) -> Result<u32, String> {
        if self.storage_bound {
            vm.world
                .objects
                .get(&self.object)
                .and_then(|o| o.native_flags)
                .ok_or_else(|| "bound native Reset flags disappeared".into())
        } else {
            Ok(self.flags)
        }
    }
    fn guard(&mut self, vm: &mut Vm, enabled: bool) -> Result<(), String> {
        let flags = self.live_flags(vm)?;
        self.flags = if enabled {
            flags | 0x800
        } else {
            flags & !0x800
        };
        if self.storage_bound {
            vm.world.objects.get_mut(&self.object).unwrap().native_flags = Some(self.flags);
        }
        Ok(())
    }
    fn suppress_used_resets(&self) -> bool {
        self.notification.map_or(self.flags, |(_, flags)| flags) & 0x1000 != 0
    }
    pub fn native_flags(&self) -> u32 {
        self.flags
    }
    pub fn flags_are_bound(&self) -> bool {
        self.storage_bound
    }
    pub fn object(&self) -> ObjectId {
        self.object
    }
    pub fn notification_target(&self) -> Option<ObjectId> {
        self.notification.map(|(id, _)| id)
    }
    pub fn complete(&self) -> bool {
        self.phase == 8
    }
    /// A host may save between returning a service and completing it. Reject
    /// a service transplanted from another person or another reset phase.
    pub fn accepts_request(&self, request: &Request) -> bool {
        match request {
            Request::InterruptRoute { thread, request } => {
                self.phase == 18
                    && self.thread == Some(*thread)
                    && self.route.as_ref() == Some(request)
            }
            Request::InterruptAnimation { thread, request } => {
                self.phase == 17
                    && self.thread == Some(*thread)
                    && self.animation.as_ref() == Some(request)
            }
            Request::NotifyUsed(id) => self.phase == 9 && self.used_target == Some(*id),
            Request::ResetUsed(id) => self.phase == 10 && self.used_target == Some(*id),
            Request::ClearAnimator(id) => self.phase == 1 && *id == self.object,
            Request::ReloadAppearance(id) => self.phase == 2 && *id == self.object,
            Request::ClearRouting(id) => self.phase == 3 && *id == self.object,
            Request::ClearActionIcons(id) => self.phase == 4 && *id == self.object,
            Request::Placement {
                object,
                marked_for_deletion,
            } => {
                self.phase == 5
                    && *object == self.object
                    && *marked_for_deletion == (self.flags & 0x40 != 0)
            }
            Request::Script(thread) => self.phase == 6 && self.thread == Some(*thread),
            Request::Footprint(id) => matches!(self.phase, 7 | 12 | 15) && *id == self.object,
            Request::Cancel(c) => {
                matches!(self.phase, 7 | 13) && c.stack_object == self.object && c.function == 4
            }
            Request::ClearRouteReferences { object, target } => {
                self.phase == 16
                    && *object == self.object
                    && self.notification.is_some_and(|(id, _)| id == *target)
            }
        }
    }
    /// Host acknowledgment for the bookkeeping boundary above. No instruction
    /// completes, no result/event is generated, and no frame or variable moves.
    pub fn interrupt_animation(
        &mut self,
        vm: &mut Vm,
        thread: ThreadId,
        request: &crate::AnimationRequest,
    ) -> Result<(), String> {
        if !self.accepts_request(&Request::InterruptAnimation {
            thread,
            request: request.clone(),
        }) || vm
            .threads
            .get(&thread)
            .is_none_or(|t| t.status != Status::AwaitingAnimation(request.clone()))
        {
            return Err("stale person Reset animation interruption".into());
        }
        vm.threads.get_mut(&thread).unwrap().status = Status::Ready;
        self.animation = None;
        self.phase = 11;
        Ok(())
    }
    pub fn retained_route(&self, thread: ThreadId, request: u64) -> Option<&crate::RouteRequest> {
        self.route
            .as_ref()
            .filter(|r| self.thread == Some(thread) && r.id == request)
    }
    pub fn interrupt_route(
        &mut self,
        vm: &mut Vm,
        thread: ThreadId,
        request: &crate::RouteRequest,
    ) -> Result<(), String> {
        if !self.accepts_request(&Request::InterruptRoute {
            thread,
            request: request.clone(),
        }) || vm
            .threads
            .get(&thread)
            .is_none_or(|t| t.status != Status::AwaitingRoute(request.clone()))
        {
            return Err("stale person Reset route interruption".into());
        }
        vm.threads.get_mut(&thread).unwrap().status = Status::Ready;
        self.phase = 11;
        Ok(())
    }
    /// Bind a suspended external function-3 callback to this exact reset.
    /// The replacement Main is installed before the callback is pushed.
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        if self.phase != 6 || self.thread != Some(thread) || depth != 1 {
            return false;
        }
        let Some(t) = vm.threads.get(&thread) else {
            return false;
        };
        let Some(f) = t.stack.get(depth) else {
            return false;
        };
        let Some(d) = vm
            .world
            .objects
            .get(&self.object)
            .and_then(|o| o.definition.as_ref())
        else {
            return false;
        };
        let routine = p
            .modules
            .get(&d.source)
            .and_then(|m| m.object_functions.get(&d.object))
            .and_then(|v| v.get(3))
            .map(|f| f.action);
        vm.object_threads.get(&self.object) == Some(&thread)
            && t.callback
                .iter()
                .flat_map(|b| b.chain())
                .any(|b| b.external && b.base_depth == depth)
            && f.caller == self.object
            && f.callee == self.object
            && f.code_owner == d.source
            && routine == Some(f.location.bhav)
    }
    pub fn validate(&self, vm: &Vm, p: &Program) -> Result<(), String> {
        if self.storage_bound && (self.live_flags(vm)? & 0x800 != self.flags & 0x800) {
            return Err("native Reset guard storage mismatch".into());
        }
        if self.phase > 18
            || (self.flags & 0x800 != 0)
                != ((self.phase != 0 && self.phase != 8)
                    || self.phase == 8 && self.thread.is_none())
            || self.used_target.is_some() != matches!(self.phase, 9 | 10)
            || self
                .used_target
                .is_some_and(|id| id <= 0 || self.suppress_used_resets())
            || self.notification.is_some_and(|(id, _)| id <= 0)
            || (12..=16).contains(&self.phase) && self.notification.is_none()
            || self.animation.is_some() != (self.phase == 17)
            || self.route.is_some() && !matches!(self.phase, 18 | 11 | 9 | 10 | 1 | 2 | 3)
            || self.phase == 18 && self.route.is_none()
            || !self.retained.is_empty() && self.phase != 13
            || self.retained.len() > crate::action::CAPACITY
            || vm
                .world
                .objects
                .get(&self.object)
                .is_none_or(|o| o.person.is_none())
        {
            return Err("invalid person reset continuation".into());
        }
        if let Some(thread) = self.thread {
            if vm.object_threads.get(&self.object) != Some(&thread) {
                return Err("person reset thread binding mismatch".into());
            }
            let t = vm
                .threads
                .get(&thread)
                .ok_or("person reset thread absent")?;
            if let Some(request) = &self.animation {
                if request.caller != self.object
                    || t.status != Status::AwaitingAnimation(request.clone())
                    || t.stack
                        .last()
                        .is_none_or(|f| f.caller != self.object || f.location != request.origin)
                {
                    return Err("person Reset animation binding mismatch".into());
                }
            }
            if let Some(request) = &self.route {
                if request.caller != self.object
                    || request.id == 0
                    || self.phase == 18
                        && (t.status != Status::AwaitingRoute(request.clone())
                            || t.stack.last().is_none_or(|f| {
                                f.caller != self.object || f.location != request.origin
                            }))
                {
                    return Err("person Reset route binding mismatch".into());
                }
            }
            if self.phase == 6 && t.callback.is_some() && !self.owns_callback(vm, p, thread, 1) {
                return Err("person reset callback binding mismatch".into());
            }
        } else if self.phase != 0 && self.phase != 8 {
            return Err("person reset thread missing".into());
        }
        Ok(())
    }
    pub fn next(&mut self, vm: &mut Vm, p: &Program) -> Result<Option<Request>, String> {
        loop {
            let object = self.object;
            match self.phase {
                0 => {
                    let thread = *vm
                        .object_threads
                        .get(&object)
                        .ok_or("loaded person thread absent")?;
                    let t = &vm.threads[&thread];
                    let actor = vm
                        .world
                        .actions
                        .as_ref()
                        .and_then(|w| w.actors.get(&object))
                        .ok_or("loaded person action storage absent")?;
                    if let Some((target, _)) = self.notification {
                        if !vm.world.objects.contains_key(&target) {
                            return Err("person Reset notification target absent".into());
                        }
                        if !t.stack.iter().any(|f| f.action_target == Some(target))
                            && actor.icon_owner() != target
                        {
                            // An unrelated notification must not interrupt an
                            // animation, restart Main or erase unrelated work.
                            self.guard(vm, true)?;
                            self.thread = Some(thread);
                            self.phase = 12;
                            continue;
                        }
                    }
                    if t.callback.is_some()
                        || t.service.is_some()
                        || t.dispatch.is_some()
                        || t.autonomy.is_some()
                        || !matches!(
                            t.status,
                            Status::Ready
                                | Status::Sleeping { .. }
                                | Status::Returned(_)
                                | Status::AwaitingAnimation(_)
                                | Status::AwaitingRoute(_)
                        )
                        || !t.completed_actions.is_empty()
                    {
                        return Err("loaded person reset needs ordinary execution without pending VM services".into());
                    }
                    let animation = if let Status::AwaitingAnimation(request) = &t.status {
                        if request.caller != object
                            || t.stack
                                .last()
                                .is_none_or(|f| f.caller != object || f.location != request.origin)
                        {
                            return Err(
                                "person Reset animation does not bind its current frame".into()
                            );
                        }
                        Some(request.clone())
                    } else {
                        None
                    };
                    let route = if let Status::AwaitingRoute(request) = &t.status {
                        if request.caller != object
                            || t.stack
                                .last()
                                .is_none_or(|f| f.caller != object || f.location != request.origin)
                        {
                            return Err("person Reset route does not bind its current frame".into());
                        }
                        Some(request.clone())
                    } else {
                        None
                    };
                    let uses: Vec<_> = t
                        .stack
                        .iter()
                        .filter_map(|f| f.action_target.map(|target| (f.caller, target)))
                        .collect();
                    for &(caller, target) in &uses {
                        if caller != object
                            || vm
                                .world
                                .objects
                                .get(&target)
                                .is_none_or(|o| !o.data.contains_key(&62))
                        {
                            return Err(
                                "person Reset use-frame binding or target count absent".into()
                            );
                        }
                    }
                    let o = vm
                        .world
                        .objects
                        .get_mut(&object)
                        .ok_or("loaded person absent")?;
                    let person = o.person.as_mut().ok_or("loaded reset requires a person")?;
                    if [0, 30, 33, 40, 59, 71, 72, 73]
                        .iter()
                        .any(|i| !person.contains_key(i))
                        || [9, 34].iter().any(|i| !o.data.contains_key(i))
                    {
                        return Err("loaded person reset fields absent".into());
                    }
                    if !uses.is_empty() && person[&73] != 0 {
                        return Err("person Reset use-frame unwind needs footprint service".into());
                    }
                    self.guard(vm, true)?;
                    self.thread = Some(thread);
                    self.phase = 11;
                    if let Some(request) = animation {
                        self.phase = 17;
                        self.animation = Some(request.clone());
                        return Ok(Some(Request::InterruptAnimation { thread, request }));
                    }
                    if let Some(request) = route {
                        self.phase = 18;
                        self.route = Some(request.clone());
                        return Ok(Some(Request::InterruptRoute { thread, request }));
                    }
                }
                18 => {
                    return Ok(Some(Request::InterruptRoute {
                        thread: self.thread.unwrap(),
                        request: self.route.clone().unwrap(),
                    }));
                }
                17 => {
                    return Ok(Some(Request::InterruptAnimation {
                        thread: self.thread.unwrap(),
                        request: self.animation.clone().unwrap(),
                    }));
                }
                9 => {
                    self.phase = 10;
                    return Ok(Some(Request::ResetUsed(self.used_target.unwrap())));
                }
                10 => {
                    self.used_target = None;
                    self.phase = 11;
                }
                11 => {
                    let thread = self.thread.unwrap();
                    // Read the live latest use, not a list saved before host
                    // callbacks. Native Reset captures its target/depth, pops
                    // through that depth, then notifies and resets that target.
                    let latest = vm.threads[&thread]
                        .stack
                        .iter()
                        .enumerate()
                        .rev()
                        .find_map(|(i, f)| f.action_target.map(|id| (i + 1, id)));
                    if let Some((depth, target)) = latest {
                        let mut t = vm.threads.remove(&thread).unwrap();
                        let unwind: Result<(), String> = (|| {
                            while t.stack.len() >= depth {
                                vm.pop_person_reset_frame(&mut t)?;
                            }
                            Ok(())
                        })();
                        vm.threads.insert(thread, t);
                        unwind?;
                        if !self.suppress_used_resets() {
                            self.used_target = Some(target);
                            self.phase = 9;
                            return Ok(Some(Request::NotifyUsed(target)));
                        }
                        continue;
                    }
                    // 0x59297d(0) writes person field 30 before virtual +0x0c.
                    vm.world
                        .objects
                        .get_mut(&object)
                        .unwrap()
                        .person
                        .as_mut()
                        .unwrap()
                        .insert(30, 0);
                    self.phase = 1;
                    return Ok(Some(Request::ClearAnimator(object)));
                }
                1 => {
                    self.phase = 2;
                    return Ok(Some(Request::ReloadAppearance(object)));
                }
                2 => {
                    self.phase = 3;
                    return Ok(Some(Request::ClearRouting(object)));
                }
                3 => {
                    self.route = None;
                    self.phase = 4;
                    return Ok(Some(Request::ClearActionIcons(object)));
                }
                4 => {
                    self.flags = self.live_flags(vm)?;
                    self.phase = 5;
                    return Ok(Some(Request::Placement {
                        object,
                        marked_for_deletion: self.flags & 0x40 != 0,
                    }));
                }
                5 => {
                    self.thread = Some(vm.restart_loaded_main(p, object)?);
                    vm.begin_loaded_function(p, object, 3)?;
                    self.phase = 6;
                }
                6 => {
                    let thread = self.thread.unwrap();
                    if vm.threads[&thread].callback.is_some() {
                        return Ok(Some(Request::Script(thread)));
                    }
                    if vm.threads[&thread].status != Status::Ready {
                        return Err("person Reset callback did not return to frozen Main".into());
                    }
                    let o = vm.world.objects.get_mut(&object).unwrap();
                    let person = o.person.as_mut().unwrap();
                    for i in [0, 40, 59] {
                        person.insert(i, 0);
                    }
                    for i in [34, 9] {
                        o.data.insert(i, 0);
                    }
                    let footprint =
                        vm.clear_person_action(object, vm.threads[&thread].last_result)?;
                    self.phase = if self.notification.is_some() { 12 } else { 7 };
                    if footprint {
                        return Ok(Some(Request::Footprint(object)));
                    }
                }
                7 => {
                    // Pop one item before invoking its callback. That callback
                    // may mutate the remaining queue; do not precollect it.
                    let actor = vm
                        .world
                        .actions
                        .as_mut()
                        .unwrap()
                        .actors
                        .get_mut(&object)
                        .unwrap();
                    if let Some(action) = actor.queue.pending.pop_front() {
                        if action.target != 0 {
                            return Ok(Some(Request::Cancel(Callback {
                                object: action.target,
                                function: 4,
                                stack_object: object,
                            })));
                        }
                    } else {
                        self.guard(vm, false)?;
                        self.phase = 8;
                    }
                }
                12 => {
                    let target = self.notification.unwrap().0;
                    let actor = &vm.world.actions.as_ref().unwrap().actors[&object];
                    self.phase = if actor
                        .queue
                        .pending
                        .iter()
                        .any(|a| a.target == target || a.icon_owner == target)
                    {
                        13
                    } else {
                        14
                    };
                }
                13 => {
                    let target = self.notification.unwrap().0;
                    let actor = vm
                        .world
                        .actions
                        .as_mut()
                        .unwrap()
                        .actors
                        .get_mut(&object)
                        .unwrap();
                    if let Some(action) = actor.queue.pending.pop_front() {
                        if action.target == target || action.icon_owner == target {
                            if action.target != 0 {
                                return Ok(Some(Request::Cancel(Callback {
                                    object: action.target,
                                    function: 4,
                                    stack_object: object,
                                })));
                            }
                        } else {
                            if self.retained.len() == crate::action::CAPACITY {
                                return Err(
                                    "person Reset notification temporary queue overflow".into()
                                );
                            }
                            self.retained.push_back(action);
                        }
                    } else {
                        actor.queue.pending = std::mem::take(&mut self.retained);
                        self.phase = 14;
                    }
                }
                14 => {
                    let target = self.notification.unwrap().0;
                    let actor = vm
                        .world
                        .actions
                        .as_mut()
                        .unwrap()
                        .actors
                        .get_mut(&object)
                        .unwrap();
                    if actor.previous.as_ref().is_some_and(|a| a.target == target) {
                        actor.previous = None;
                    } else if let Some(a) =
                        actor.previous.as_mut().filter(|a| a.icon_owner == target)
                    {
                        a.icon_owner = 0;
                    }
                    let clear = actor.active.as_ref().is_some_and(|a| a.target == target);
                    if !clear && actor.icon_owner() == target {
                        actor.set_icon_owner(0);
                    }
                    self.phase = 15;
                    if clear
                        && vm.clear_person_action(
                            object,
                            vm.threads[&self.thread.unwrap()].last_result,
                        )?
                    {
                        return Ok(Some(Request::Footprint(object)));
                    }
                }
                15 => {
                    let target = self.notification.unwrap().0;
                    self.phase = 16;
                    if vm
                        .world
                        .objects
                        .get(&target)
                        .ok_or("person Reset notification target disappeared")?
                        .person
                        .is_some()
                    {
                        return Ok(Some(Request::ClearRouteReferences { object, target }));
                    }
                }
                16 => {
                    self.guard(vm, false)?;
                    self.phase = 8;
                }
                _ => return Ok(None),
            }
        }
    }
}
