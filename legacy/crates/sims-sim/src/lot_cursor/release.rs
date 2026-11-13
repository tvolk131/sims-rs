//! Cursor release421a0e and the ordinary/group PlaceOnLot wrappers43562f/422e99.
//! Cursor destruction, purchase accounting and undo registration belong to
//! the caller. In particular,421e42's price is read but not deducted here.
use super::{handle, id, placement, rotation};
use crate::{Simulation, Step, lot_dynamic, lot_links::Descriptor, lot_placement};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, Status, ThreadId, Vm};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Gate,
    Detach,
    Head,
    Invalidate,
    Registry,
    Flags,
    Clear,
    Place,
    Complete(bool),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    tick: u64,
    cursor: u32,
    phase: Phase,
    furniture: Option<u32>,
    point: Option<([i32; 2], i32)>,
    current: Option<u32>,
    seen: BTreeSet<u32>,
    detach: Option<rotation::Execution>,
    placement: Option<Put>,
}
fn cursor(s: &Simulation, h: u32) -> Result<&sims_vm::cursor::State, String> {
    s.vm.world.objects[&id(&s.vm, h)?]
        .cursor
        .as_ref()
        .ok_or("release cursor absent".into())
}
impl Execution {
    pub(crate) fn call(s: &Simulation, h: u32) -> Self {
        Self {
            tick: s.vm.tick,
            cursor: h,
            phase: Phase::Gate,
            furniture: None,
            point: None,
            current: None,
            seen: BTreeSet::new(),
            detach: None,
            placement: None,
        }
    }
    pub(crate) fn acknowledge_animator(&mut self, h: u32) -> Result<(), String> {
        if self.pending_animator_invalidation() != Some(h) {
            return Err("release animator reply differs".into());
        }
        self.placement.as_mut().unwrap().animator_pending = false;
        Ok(())
    }

    pub fn outcome(&self) -> Option<bool> {
        if let Phase::Complete(value) = self.phase {
            Some(value)
        } else {
            None
        }
    }
    pub fn cursor(&self) -> u32 {
        self.cursor
    }
    pub(crate) fn owns_callback(&self, vm: &Vm, p: &Program, t: ThreadId, d: usize) -> bool {
        self.detach
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, t, d))
            || self
                .placement
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, t, d))
    }
    pub(crate) fn graphic(&self) -> Option<&lot_placement::live::Execution> {
        self.detach
            .as_ref()
            .and_then(|e| e.graphic())
            .or_else(|| self.placement.as_ref().and_then(|e| e.physical.as_ref()))
            .filter(|e| matches!(e.pending, Some(lot_placement::Request::RefreshAnimator(_))))
    }
    pub(crate) fn graphic_mut(&mut self) -> Option<&mut lot_placement::live::Execution> {
        if self.detach.as_ref().is_some_and(|e| e.graphic().is_some()) {
            return self.detach.as_mut().unwrap().graphic_mut();
        }
        self.placement
            .as_mut()
            .and_then(|e| e.physical.as_mut())
            .filter(|e| matches!(e.pending, Some(lot_placement::Request::RefreshAnimator(_))))
    }
    pub fn pending_animator_invalidation(&self) -> Option<u32> {
        self.placement
            .as_ref()
            .filter(|e| e.animator_pending)
            .map(|e| e.object)
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        if s.cursor_factory.is_some()
            || s.cursor_rotation.is_some()
            || s.cursor_cleanup.is_some()
            || s.object_pickup.is_some()
            || s.object_placement.is_some()
            || s.ground_placement.is_some()
            || s.synchronous_removal.is_some()
            || s.native_selection.is_some()
            || s.portrait_person.is_some()
            || s.vm.player_menu.is_some()
            || s.visit.is_some()
            || s.vm.scheduled_object_visit.is_some()
            || s.native_scheduler
                .as_ref()
                .is_some_and(|n| n.request().is_some())
        {
            return Err("cursor release overlaps synchronous work".into());
        }
        self.validate_call(s, p)
    }
    pub(crate) fn validate_call(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        if self.tick != s.vm.tick
            || !cursor(s, self.cursor)?.postconstructed
            || s.native_lot.is_none()
            || self.furniture.is_some() != self.point.is_some()
            || matches!(self.phase, Phase::Gate | Phase::Complete(false))
                != self.furniture.is_none()
            || self.detach.is_some() && self.phase != Phase::Detach
            || self.placement.is_some() && self.phase != Phase::Place
            || matches!(
                self.phase,
                Phase::Invalidate | Phase::Registry | Phase::Flags | Phase::Clear
            ) && self.current.is_none()
        {
            return Err("invalid cursor release owner".into());
        }
        if let Some(h) = self.furniture {
            id(&s.vm, h)?;
        }
        if let Some(h) = self.current {
            cursor(s, h)?;
        }
        if let Some(e) = &self.detach {
            if !e.is_removal(self.cursor) {
                return Err("release detach binding differs".into());
            }
            e.validate(s, p)?;
        }
        if let Some(e) = &self.placement {
            let (point, level) = self.point.ok_or("release destination absent")?;
            if Some(e.object) != self.furniture
                || e.destination.position != point
                || e.destination.level != level
            {
                return Err("release placement binding differs".into());
            }
            e.validate(s, p)?;
        }
        Ok(())
    }
    pub(crate) fn advance(
        &mut self,
        s: &mut Simulation,
        p: &Program,
        left: &mut usize,
        step: &mut Step,
    ) -> Result<(), String> {
        while *left > 0 && self.outcome().is_none() {
            if let Some(e) = &mut self.detach {
                if !e.advance(s, p, left, step)? {
                    return Ok(());
                }
                self.detach = None;
                self.phase = Phase::Head;
                continue;
            }
            if let Some(e) = &mut self.placement {
                if !e.advance(s, p, left, step)? {
                    return Ok(());
                }
                self.placement = None;
                self.phase = Phase::Complete(true);
                continue;
            }
            match self.phase {
                Phase::Gate => {
                    let c = cursor(s, self.cursor)?;
                    if c.underlying.is_none() || c.valid == 0 {
                        self.phase = Phase::Complete(false);
                    } else {
                        placement::cost(&s.vm, p, self.cursor)?;
                        let c = cursor(s, self.cursor)?;
                        self.furniture = c.underlying;
                        self.point = Some((c.position, c.level));
                        self.phase = Phase::Detach;
                    }
                }
                Phase::Detach => self.detach = Some(rotation::Execution::remove(s, self.cursor)?),
                Phase::Head => {
                    let object = id(&s.vm, self.cursor)?;
                    let head = s.vm.world.objects[&object]
                        .multitile
                        .as_ref()
                        .ok_or("release chain absent")?
                        .head
                        .unwrap_or(object);
                    self.current = Some(handle(&s.vm, head)?);
                    self.phase = Phase::Invalidate;
                }
                Phase::Invalidate => {
                    let h = self.current.unwrap();
                    if !self.seen.insert(h) {
                        return Err("cyclic cursor release".into());
                    }
                    let target = id(
                        &s.vm,
                        cursor(s, h)?
                            .underlying
                            .ok_or("released part lost furniture")?,
                    )?;
                    s.vm.refresh_object(target, target, 0, 0)?;
                    self.phase = Phase::Registry;
                }
                Phase::Registry => {
                    let target = id(
                        &s.vm,
                        cursor(s, self.current.unwrap())?
                            .underlying
                            .ok_or("release target absent")?,
                    )?;
                    s.vm.world
                        .objects
                        .get_mut(&target)
                        .unwrap()
                        .idle
                        .as_mut()
                        .ok_or("release registry word absent")?
                        .set_flags(1, true);
                    self.phase = Phase::Flags;
                }
                Phase::Flags => {
                    let target = id(
                        &s.vm,
                        cursor(s, self.current.unwrap())?
                            .underlying
                            .ok_or("release target absent")?,
                    )?;
                    let flags = s.vm.world.objects[&target]
                        .native_flags
                        .ok_or("release native flags absent")?
                        & 0x1e;
                    s.update_native_low_flags(p, target, flags)?;
                    self.phase = Phase::Clear;
                }
                Phase::Clear => {
                    let object = id(&s.vm, self.current.unwrap())?;
                    s.vm.world
                        .objects
                        .get_mut(&object)
                        .unwrap()
                        .cursor
                        .as_mut()
                        .unwrap()
                        .underlying = None;
                    let next = s.vm.world.objects[&object]
                        .multitile
                        .as_ref()
                        .ok_or("release next link absent")?
                        .next;
                    self.current = next.map(|o| handle(&s.vm, o)).transpose()?;
                    self.phase = if self.current.is_some() {
                        Phase::Invalidate
                    } else {
                        Phase::Place
                    };
                }
                Phase::Place => {
                    // These fields are deliberately read after all detach/flag
                    // work. Only XY, level and the furniture pointer were captured.
                    let c = cursor(s, self.cursor)?;
                    let container = (c.container > 0
                        && s.vm.world.objects.contains_key(&c.container))
                    .then_some(c.container);
                    let (position, level) = self.point.unwrap();
                    let destination = Descriptor {
                        on_lot: s
                            .native_lot
                            .as_ref()
                            .unwrap()
                            .placement
                            .links
                            .on_lot(position),
                        position,
                        level,
                        container,
                        slot: i32::from(c.slot),
                    };
                    self.placement = Some(Put::new(s, self.furniture.unwrap(), destination)?);
                }
                Phase::Complete(_) => unreachable!(),
            }
            *left -= 1;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum PutPhase {
    Physical,
    Animator,
    Base,
    Head,
    Part,
    SlotsStart,
    Slots,
    Next,
    Dynamic,
    Complete,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Callback {
    object: u32,
    thread: ThreadId,
    depth: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Put {
    object: u32,
    destination: Descriptor,
    multipart: bool,
    phase: PutPhase,
    group_pass: bool,
    current: Option<u32>,
    slot: usize,
    limit: usize,
    seen: BTreeSet<u32>,
    callback: Option<Callback>,
    physical: Option<lot_placement::live::Execution>,
    dynamic: Option<lot_dynamic::live::Execution>,
    animator_pending: bool,
}
impl Put {
    pub(crate) fn binding(&self) -> (u32, Descriptor) {
        (self.object, self.destination)
    }
    pub(crate) fn complete(&self) -> bool {
        self.phase == PutPhase::Complete
    }
    pub(crate) fn animator(&self) -> Option<u32> {
        self.animator_pending.then_some(self.object)
    }
    pub(crate) fn acknowledge_animator(&mut self, object: u32) -> Result<(), String> {
        if self.animator() != Some(object) {
            return Err("PlaceOnLot animator owner differs".into());
        }
        self.animator_pending = false;
        Ok(())
    }
    pub(crate) fn graphic(&self) -> Option<&lot_placement::live::Execution> {
        self.physical
            .as_ref()
            .filter(|e| matches!(e.pending, Some(lot_placement::Request::RefreshAnimator(_))))
    }
    pub(crate) fn graphic_mut(&mut self) -> Option<&mut lot_placement::live::Execution> {
        self.physical
            .as_mut()
            .filter(|e| matches!(e.pending, Some(lot_placement::Request::RefreshAnimator(_))))
    }

    pub(crate) fn new(
        s: &Simulation,
        object: u32,
        destination: Descriptor,
    ) -> Result<Self, String> {
        let o = &s.vm.world.objects[&id(&s.vm, object)?];
        if o.person.is_some() || o.cursor.is_some() {
            return Err("release furniture needs an ordinary/group PlaceOnLot".into());
        }
        Ok(Self {
            object,
            destination,
            multipart: o.multitile.is_some(),
            phase: PutPhase::Physical,
            group_pass: false,
            current: None,
            slot: 0,
            limit: 0,
            seen: BTreeSet::new(),
            callback: None,
            physical: None,
            dynamic: None,
            animator_pending: false,
        })
    }
    pub(crate) fn owns_callback(&self, vm: &Vm, p: &Program, t: ThreadId, d: usize) -> bool {
        self.physical
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, t, d))
            || self
                .dynamic
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, t, d))
            || self.callback.as_ref().is_some_and(|c| {
                let Ok(object) = id(vm, c.object) else {
                    return false;
                };
                let Some(thread) = vm.threads.get(&t) else {
                    return false;
                };
                let Some(frame) = thread.stack.get(d) else {
                    return false;
                };
                let Some(def) = vm.world.objects[&object].definition.as_ref() else {
                    return false;
                };
                c.thread == t
                    && c.depth == d
                    && thread
                        .callback
                        .iter()
                        .flat_map(|b| b.chain())
                        .any(|b| b.external && b.base_depth == d)
                    && frame.caller == object
                    && frame.callee == object
                    && frame.code_owner == def.source
                    && p.modules
                        .get(&def.source)
                        .and_then(|m| m.object_functions.get(&def.object))
                        .and_then(|f| f.get(11))
                        .map(|f| f.action)
                        == Some(frame.location.bhav)
            })
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        let object = id(&s.vm, self.object)?;
        if self.multipart != s.vm.world.objects[&object].multitile.is_some()
            || self.slot > self.limit
            || self.animator_pending && self.phase != PutPhase::Base
            || self.physical.is_some() && self.phase != PutPhase::Physical
            || self.dynamic.is_some() && self.phase != PutPhase::Dynamic
            || self.callback.is_some()
                && !matches!(self.phase, PutPhase::SlotsStart | PutPhase::Slots)
            || self.phase == PutPhase::Complete
                && (self.callback.is_some()
                    || self.physical.is_some()
                    || self.dynamic.is_some()
                    || self.animator_pending)
        {
            return Err("invalid PlaceOnLot continuation".into());
        }
        if let Some(c) = &self.callback {
            if !self.owns_callback(&s.vm, p, c.thread, c.depth) {
                return Err("PlaceOnLot callback binding differs".into());
            }
        }
        if let Some(e) = &self.physical {
            e.validate(&s.vm, p, object)?;
            if e.operation.binding() != (object, Some(self.destination)) {
                return Err("PlaceOnLot physical binding differs".into());
            }
        }
        if let Some(e) = &self.dynamic {
            e.validate(&s.vm, p)?;
            if e.binding() != object {
                return Err("PlaceOnLot dynamic binding differs".into());
            }
        }
        Ok(())
    }
    fn begin_function(&mut self, s: &mut Simulation, p: &Program, h: u32) -> Result<(), String> {
        let object = id(&s.vm, h)?;
        let depth =
            s.vm.object_threads
                .get(&object)
                .and_then(|t| s.vm.threads.get(t))
                .map_or(0, |t| t.stack.len());
        if let Some(thread) = s.vm.begin_loaded_function(p, object, 11)? {
            self.callback = Some(Callback {
                object: h,
                thread,
                depth,
            });
        }
        Ok(())
    }
    pub(crate) fn advance(
        &mut self,
        s: &mut Simulation,
        p: &Program,
        left: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        while *left > 0 && self.phase != PutPhase::Complete {
            if self.animator_pending {
                return Ok(false);
            }
            if let Some(c) = &self.callback {
                let t = &s.vm.threads[&c.thread];
                if t.callback.is_some() {
                    if t.status != Status::Ready {
                        return Err(format!(
                            "PlaceOnLot callback requires live host: {:?}",
                            t.status
                        ));
                    }
                    let slice = s.vm.run_slice(p, c.thread, 1)?;
                    *left -= slice.trace.len().max(1);
                    step.trace.extend(slice.trace);
                    if s.vm.threads[&c.thread].callback.is_some() {
                        continue;
                    }
                }
                s.vm.threads[&c.thread]
                    .callback_result
                    .ok_or("PlaceOnLot result absent")?;
                self.callback = None;
                s.publish_live_placement_caches()?;
                continue;
            }
            if let Some(e) = &mut self.physical {
                if !s.drive_removal_placement(p, e, left, step)? {
                    return Ok(false);
                }
                self.physical = None;
                self.phase = PutPhase::Animator;
                continue;
            }
            if let Some(e) = &mut self.dynamic {
                if !s.drive_dynamic_update(p, e, left, step)? {
                    return Ok(false);
                }
                self.dynamic = None;
                self.phase = PutPhase::Complete;
                continue;
            }
            let object = id(&s.vm, self.object)?;
            match self.phase {
                PutPhase::Physical => {
                    self.physical = Some(lot_placement::live::Execution::new_virtual(
                        lot_placement::VirtualOperation::set(
                            &s.vm.world,
                            &s.native_lot.as_ref().unwrap().placement.links,
                            object,
                            self.destination,
                            true,
                        )?,
                    ))
                }
                PutPhase::Animator => {
                    self.animator_pending = s.ordinary_graphic_required(p, object)?;
                    self.phase = PutPhase::Base;
                }
                PutPhase::Base => {
                    let def = s.vm.world.objects[&object]
                        .definition
                        .as_ref()
                        .ok_or("PlaceOnLot definition absent")?;
                    if sims_vm::construction::definition_words(p, def)?[10] == 0 {
                        self.current = Some(self.object);
                        self.phase = PutPhase::Part;
                    } else {
                        self.phase = if self.multipart {
                            PutPhase::Head
                        } else {
                            PutPhase::Complete
                        };
                    }
                }
                PutPhase::Head => {
                    let head = s.vm.world.objects[&object]
                        .multitile
                        .as_ref()
                        .ok_or("PlaceOnLot head absent")?
                        .head
                        .unwrap_or(object);
                    self.current = Some(handle(&s.vm, head)?);
                    self.group_pass = true;
                    self.seen.clear();
                    self.phase = PutPhase::Part;
                }
                PutPhase::Part => {
                    let h = self.current.ok_or("PlaceOnLot part absent")?;
                    if !self.seen.insert(h) {
                        return Err("cyclic PlaceOnLot traversal".into());
                    }
                    self.phase = PutPhase::SlotsStart;
                    self.begin_function(s, p, h)?;
                }
                PutPhase::SlotsStart => {
                    self.limit = s.vm.world.objects[&id(&s.vm, self.current.unwrap())?]
                        .slots
                        .len();
                    self.slot = 0;
                    self.phase = PutPhase::Slots;
                }
                PutPhase::Slots => {
                    if self.slot < self.limit {
                        let current = id(&s.vm, self.current.unwrap())?;
                        // Captured limit, live vector bound and signed registry lookup.
                        let child = s.vm.world.objects[&current]
                            .slots
                            .get(self.slot)
                            .copied()
                            .flatten()
                            .filter(|id| *id > 0 && s.vm.world.objects.contains_key(id));
                        self.slot += 1;
                        if let Some(child) = child {
                            self.begin_function(s, p, handle(&s.vm, child)?)?;
                        }
                    } else {
                        self.phase = PutPhase::Next;
                    }
                }
                PutPhase::Next => {
                    if self.group_pass {
                        let current = id(&s.vm, self.current.unwrap())?;
                        self.current = s.vm.world.objects[&current]
                            .multitile
                            .as_ref()
                            .ok_or("PlaceOnLot next absent")?
                            .next
                            .map(|id| handle(&s.vm, id))
                            .transpose()?;
                        self.phase = if self.current.is_some() {
                            PutPhase::Part
                        } else {
                            PutPhase::Dynamic
                        };
                    } else {
                        self.phase = if self.multipart {
                            PutPhase::Head
                        } else {
                            PutPhase::Complete
                        };
                    }
                }
                PutPhase::Dynamic => {
                    if sims_vm::multitile::dynamic(&s.vm.world, p, object)? {
                        self.dynamic = Some(lot_dynamic::live::Execution::begin(s, object)?);
                    } else {
                        self.phase = PutPhase::Complete;
                    }
                }
                PutPhase::Complete => unreachable!(),
            }
            *left -= 1;
        }
        Ok(self.phase == PutPhase::Complete)
    }
}
impl Simulation {
    pub fn begin_cursor_release(&mut self, p: &Program, object: ObjectId) -> Result<(), String> {
        if self.cursor_work_pending()
            || self.cursor_has_active_vm_callback()
            || !self.deletion.is_empty()
            || self.person_factories.values().any(|f| !f.finalized)
        {
            return Err("cursor release overlaps synchronous work".into());
        }
        self.validate_native_lot()?;
        let e = Execution::call(self, handle(&self.vm, object)?);
        e.validate(self, p)?;
        self.cursor_release = Some(e);
        Ok(())
    }
    pub fn advance_cursor_release(&mut self, p: &Program, budget: usize) -> Result<Step, String> {
        if budget == 0 {
            return Err("cursor release requires a positive budget".into());
        }
        self.cursor_release
            .as_ref()
            .ok_or("cursor release absent")?
            .validate(self, p)?;
        let mut e = self.cursor_release.take().unwrap();
        let mut step = Step {
            stop: crate::StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        let mut left = budget;
        let result = e.advance(self, p, &mut left, &mut step);
        self.cursor_release = Some(e);
        self.publish_live_placement_caches()?;
        self.validate_geometry()?;
        self.validate_native_lot()?;
        result?;
        self.cursor_release.as_ref().unwrap().validate(self, p)?;
        Ok(step)
    }
    pub fn finish_cursor_release(&mut self, p: &Program) -> Result<bool, String> {
        let e = self
            .cursor_release
            .as_ref()
            .ok_or("cursor release absent")?;
        e.validate(self, p)?;
        let result = e.outcome().ok_or("cursor release has not returned")?;
        self.cursor_release = None;
        Ok(result)
    }
    pub fn complete_cursor_release_animator(&mut self, object: ObjectId) -> Result<(), String> {
        let h = handle(&self.vm, object)?;
        if let Some(e) = self
            .deletion
            .iter_mut()
            .rev()
            .find_map(|d| d.cursor.as_mut().filter(|e| e.animator() == Some(h)))
        {
            return e.acknowledge_animator(h);
        }
        if let Some(e) = self
            .object_placement
            .as_mut()
            .filter(|e| e.animator() == Some(h))
        {
            return e.acknowledge_animator(h);
        }
        self.cursor_release
            .as_mut()
            .ok_or("release animator owner absent")?
            .acknowledge_animator(h)
    }
}
