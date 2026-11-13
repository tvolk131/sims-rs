//! Saved 4215e6 cursor construction, Reset and cursor-specific placement.
//! Focus and E2 publication are synchronous services of the application host.
use crate::{Simulation, Step, StepStop};
use serde::{Deserialize, Serialize};
use sims_vm::{Definition, ObjectId, Program, ThreadId, Vm, construction::ObjectDate};
pub mod acceptance;
pub mod cleanup;
pub mod permission;
pub mod placement;
pub mod release;
mod reset;
pub mod rotation;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Register,
    Pickup,
    Postconstruct,
    Reset,
    Select,
    Placement,
    Check,
    Place,
    Flags,
    Focus,
    Notification,
    Complete,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Factory {
    tick: u64,
    definition: Definition,
    date: ObjectDate,
    selected: ObjectId,
    furniture: Option<u32>,
    current: Option<u32>,
    grouped: bool,
    position: [i32; 2],
    level: i32,
    created: Vec<u32>,
    selected_cursor: Option<u32>,
    phase: Phase,
    reset: Option<reset::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    check: Option<crate::lot_acceptance::live::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) placement: Option<placement::Execution>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    placement_cheat: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    notification_started: bool,
}
///Native+54 must be called with these inputs before any+58/focus/E2 tail.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Placement {
    pub cursor: u32,
    pub position: [i32; 2],
    pub level: i32,
}
fn id(vm: &Vm, handle: u32) -> Result<ObjectId, String> {
    vm.world
        .scheduler_storage
        .as_ref()
        .and_then(|s| s.object_for_handle(handle))
        .filter(|id| vm.world.objects.contains_key(id))
        .ok_or("cursor factory lifetime ended".into())
}
fn handle(vm: &Vm, id: ObjectId) -> Result<u32, String> {
    vm.world
        .scheduler_storage
        .as_ref()
        .and_then(|s| s.handle(id))
        .ok_or("cursor factory registry storage absent".into())
}
impl Factory {
    ///Exact arguments retained by the outer421986 purchase wrapper.
    pub fn matches_creation(
        &self,
        definition: Option<&Definition>,
        selected: ObjectId,
        level: i32,
    ) -> bool {
        definition == Some(&self.definition)
            && self.selected == selected
            && self.level == if level == 0 { 1 } else { level }
    }
    pub fn returned(&self) -> Option<u32> {
        (self.phase == Phase::Complete)
            .then_some(self.selected_cursor)
            .flatten()
    }
    /// The source has already published view+1bc at these stages.
    pub fn published_focus(&self) -> Option<u32> {
        matches!(self.phase, Phase::Notification | Phase::Complete)
            .then_some(self.selected_cursor)
            .flatten()
    }
    pub fn pending_created(&self) -> Option<(u32, bool)> {
        (self.phase == Phase::Notification)
            .then_some(self.selected_cursor)
            .flatten()
            .map(|h| (h, self.notification_started))
    }
    /// 4a67de must finish before the factory's E2 notification and return.
    pub fn pending_focus(&self) -> Option<u32> {
        (self.phase == Phase::Focus)
            .then_some(self.selected_cursor)
            .flatten()
    }
    pub fn placement_request(&self) -> Option<placement::Request> {
        self.placement.as_ref().and_then(|e| e.pending())
    }
    pub fn placement(&self) -> Option<Placement> {
        (self.phase == Phase::Placement)
            .then_some(self.selected_cursor)
            .flatten()
            .map(|cursor| Placement {
                cursor,
                position: self.position,
                level: self.level,
            })
    }
    pub fn created(&self) -> &[u32] {
        &self.created
    }
    ///The outer application caller may admit only this factory's pickup child.
    pub fn owns_pickup(&self, child: &crate::lot_pickup::live::Execution) -> bool {
        self.pickup_binding().is_some() && self.pickup_binding() == child.cursor_binding()
    }
    pub(crate) fn pickup_binding(&self) -> Option<(u32, u32)> {
        if self.phase != Phase::Pickup {
            return None;
        }
        self.created.first().copied().zip(self.furniture)
    }
    pub(crate) fn owns_uninitialized(&self, vm: &Vm, object: ObjectId) -> bool {
        matches!(
            self.phase,
            Phase::Register | Phase::Pickup | Phase::Postconstruct | Phase::Reset
        ) && self.created.iter().any(|&h| id(vm, h) == Ok(object))
            && vm
                .world
                .objects
                .get(&object)
                .and_then(|o| o.cursor.as_ref())
                .is_some_and(|c| c.postconstructed)
    }
    pub(crate) fn owns_constructor(&self, vm: &Vm, object: ObjectId) -> bool {
        matches!(self.phase, Phase::Pickup | Phase::Postconstruct)
            && self
                .created
                .last()
                .is_some_and(|&h| id(vm, h) == Ok(object))
            && vm
                .world
                .objects
                .get(&object)
                .and_then(|o| o.cursor.as_ref())
                .is_some_and(|c| !c.postconstructed)
    }
    pub(crate) fn owns_visit(&self, vm: &Vm) -> bool {
        self.phase == Phase::Reset && self.reset.as_ref().is_some_and(|r| r.owns_visit(vm))
    }
    pub(crate) fn owns_callback(
        &self,
        vm: &Vm,
        p: &Program,
        thread: ThreadId,
        depth: usize,
    ) -> bool {
        self.check
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self
                .placement
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self.phase == Phase::Reset
                && self
                    .reset
                    .as_ref()
                    .is_some_and(|r| r.owns_callback(vm, p, thread, depth))
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        if self.tick != s.vm.tick
            || s.cursor_rotation.is_some()
            || s.cursor_release.is_some()
            || s.cursor_cleanup.is_some()
            || s.native_lot.is_none()
            || s.visit.is_some()
            || s.ground_placement.is_some()
            || s.native_selection.is_some()
            || s.synchronous_removal.is_some()
            || s.portrait_person.is_some()
            || s.vm.player_menu.is_some()
            || self.created.is_empty() && self.phase != Phase::Register
            || !self.grouped && self.created.len() > 1
            || self.created.len() > i16::MAX as usize
            || self.grouped && self.furniture.is_none()
            || self.phase == Phase::Register
                && (self.grouped && self.current.is_none()
                    || !self.grouped && !self.created.is_empty())
            || !self.grouped && self.current != self.furniture
            || self.phase == Phase::Pickup && self.pickup_binding().is_none()
            || self.reset.is_some() != (self.phase == Phase::Reset)
            || self.selected_cursor.is_some()
                != matches!(
                    self.phase,
                    Phase::Placement
                        | Phase::Check
                        | Phase::Place
                        | Phase::Flags
                        | Phase::Focus
                        | Phase::Notification
                        | Phase::Complete
                )
            || self.notification_started && self.phase != Phase::Notification
            || self.check.is_some() != (self.phase == Phase::Check)
            || self.placement.is_some() != (self.phase == Phase::Place)
            || s.native_scheduler
                .as_ref()
                .is_some_and(|n| n.request().is_some())
        {
            return Err("invalid cursor factory owner".into());
        }
        if let Some(h) = self.furniture {
            id(&s.vm, h)?;
        }
        if let Some(h) = self.current {
            id(&s.vm, h)?;
        }
        let mut unique = std::collections::BTreeSet::new();
        for &h in &self.created {
            let id = id(&s.vm, h)?;
            let o = &s.vm.world.objects[&id];
            let before_post = self.created.last() == Some(&h)
                && (self.phase == Phase::Postconstruct
                    || self.phase == Phase::Pickup && !self.grouped);
            if !unique.insert(h)
                || o.definition.as_ref() != Some(&self.definition)
                || o.cursor.is_none()
                || o.cursor
                    .as_ref()
                    .is_some_and(|c| c.postconstructed == before_post)
            {
                return Err("cursor factory lost its constructed chain".into());
            }
        }
        if self.selected_cursor.is_some_and(|h| !unique.contains(&h)) {
            return Err("cursor factory selected an unrelated cursor".into());
        }
        if self.grouped
            && self.selected_cursor.is_some_and(|h| {
                id(&s.vm, h)
                    .ok()
                    .and_then(|id| s.vm.world.objects[&id].cursor.as_ref())
                    .and_then(|c| c.underlying)
                    .and_then(|h| id(&s.vm, h).ok())
                    != Some(self.selected)
            })
        {
            return Err("cursor factory selected part binding differs".into());
        }
        if let Some(e) = &s.object_pickup {
            if e.cursor_binding() != self.pickup_binding() {
                return Err("cursor factory pickup owner differs".into());
            }
        }
        if let Some(r) = &self.reset {
            if self.created.first().copied() != Some(r.binding().0)
                || r.binding().1 != i16::from(self.grouped)
            {
                return Err("cursor Reset belongs to another factory".into());
            }
            r.validate(s, p)?;
        }
        if let Some(e) = &self.check {
            if e.binding()
                != (
                    id(&s.vm, self.selected_cursor.unwrap())?,
                    self.position,
                    self.level,
                )
                || e.container_binding().is_some()
            {
                return Err("cursor initial acceptance binding differs".into());
            }
            e.validate(&s.vm, p)?;
        }
        if let Some(e) = &self.placement {
            if e.binding()
                != (Placement {
                    cursor: self.selected_cursor.unwrap(),
                    position: self.position,
                    level: self.level,
                })
            {
                return Err("cursor placement belongs to another factory".into());
            }
            e.validate(s, p)?;
        }
        if s.vm.scheduled_object_visit.is_some() && !self.owns_visit(&s.vm) {
            return Err("cursor factory has unrelated native Update".into());
        }
        Ok(())
    }
}
impl Simulation {
    /// A Main's saved yielding service (for example a portal crossing) is not
    /// a global application callback. It may stay parked while Buy constructs
    /// an unrelated cursor. Concrete execution owners still prohibit reentry.
    pub(crate) fn cursor_has_active_vm_callback(&self) -> bool {
        if self.immediate.is_some() || self.reach_callback.is_some() || self.cancellation.is_some()
        {
            return true;
        }
        self.vm.threads.iter().any(|(&thread, t)| {
            if t.callback.is_some() {
                return true;
            }
            let Some(service) = &t.service else {
                return false;
            };
            let ordinary_main = !t.check
                && !matches!(
                    t.status,
                    sims_vm::Status::Returned(_) | sims_vm::Status::Faulted(_)
                )
                && self.vm.object_threads.iter().any(|(&object, &registered)| {
                    registered == thread
                        && self.vm.lifecycles.get(&object).is_some_and(|l| {
                            l.thread == thread
                                && l.phase == sims_vm::lifecycle::Phase::Main
                                && self
                                    .vm
                                    .world
                                    .objects
                                    .get(&object)
                                    .and_then(|o| o.definition.as_ref())
                                    .is_some_and(|d| {
                                        d.source == l.owner && d.object == l.definition
                                    })
                        })
                        && self.vm.world.scheduler_storage.as_ref().and_then(|r| {
                            r.handle(object)
                                .map(|h| r.object_for_handle(h) == Some(object))
                        }) == Some(true)
                        && t.stack.first().is_some_and(|f| f.caller == object)
                        && service.chain().all(|s| {
                            s.caller == object
                                && s.result.is_none()
                                && s.base_depth > 0
                                && s.base_depth < t.stack.len()
                        })
                });
            !ordinary_main
        })
    }
    ///Resource None and native permission denial return false without creating
    ///a cursor. Date is the actual engine calendar supplied by the live caller.
    pub fn begin_cursor_factory(
        &mut self,
        p: &Program,
        definition: Option<Definition>,
        selected: ObjectId,
        level: i32,
        cheat: bool,
        date: ObjectDate,
    ) -> Result<bool, String> {
        if self.cursor_work_pending()
            || self.object_pickup.is_some()
            || self.object_placement.is_some()
            || self.ground_placement.is_some()
            || self.synchronous_removal.is_some()
            || self.native_selection.is_some()
            || self.portrait_person.is_some()
            || self.vm.player_menu.is_some()
            || !self.deletion.is_empty()
            || self.visit.is_some()
            || self.vm.scheduled_object_visit.is_some()
            || self.cursor_has_active_vm_callback()
            || self.person_factories.values().any(|f| !f.finalized)
            || self
                .native_scheduler
                .as_ref()
                .is_some_and(|n| n.request().is_some())
        {
            return Err("cursor factory overlaps synchronous work".into());
        }
        let Some(definition) = definition else {
            return Ok(false);
        };
        self.validate_native_lot()?;
        let words = p
            .modules
            .get(&definition.source)
            .and_then(|m| m.object_definition_words.get(&definition.object))
            .ok_or("cursor resource absent")?;
        if words.len() <= 65 || words.get(102).is_some_and(|w| w & 1 != 0) {
            return Err("cursor factory requires supported sprite postconstruction".into());
        }
        let (position, grouped, target) = if let Some(o) = self
            .vm
            .world
            .objects
            .get(&selected)
            .filter(|_| selected > 0)
        {
            let q = self
                .vm
                .world
                .positions
                .get(&selected)
                .ok_or("cursor selected position absent")?;
            let d = o
                .definition
                .as_ref()
                .ok_or("cursor furniture definition absent")?;
            let grouped = *p
                .modules
                .get(&d.source)
                .and_then(|m| m.object_definition_words.get(&d.object))
                .and_then(|w| w.get(10))
                .ok_or("cursor furniture group word absent")?
                != 0;
            let head = if grouped {
                o.multitile
                    .as_ref()
                    .ok_or("cursor furniture multipart storage absent")?
                    .head
                    .unwrap_or(selected)
            } else {
                selected
            };
            ([q.x, q.y], grouped, Some(head))
        } else {
            ([128, 128], false, None)
        };
        if let Some(target) = target {
            if !permission::check(self, target, cheat)? {
                return Ok(false);
            }
            if self.vm.world.objects[&target].person.is_some()
                || self.vm.world.objects[&target].cursor.is_some()
            {
                return Err("cursor factory receiver subclass requires its own pickup host".into());
            }
        }
        let furniture = target.map(|id| handle(&self.vm, id)).transpose()?;
        let factory = Factory {
            tick: self.vm.tick,
            definition,
            date,
            selected,
            furniture,
            current: furniture,
            grouped,
            position,
            level: if level == 0 { 1 } else { level },
            created: vec![],
            selected_cursor: None,
            phase: Phase::Register,
            reset: None,
            check: None,
            placement: None,
            placement_cheat: false,
            notification_started: false,
        };
        factory.validate(self, p)?;
        self.cursor_factory = Some(factory);
        Ok(true)
    }
    /// Resume the saved pre-placement boundary with the current Buy cheat flag.
    /// The factory calls cursor +54 with a null container, hence 421e21 delegates
    /// to 422abb without the Buy-mode restrictions of the furniture's +60.
    pub fn begin_cursor_factory_placement(
        &mut self,
        p: &Program,
        cheat: bool,
    ) -> Result<(), String> {
        let f = self
            .cursor_factory
            .as_ref()
            .ok_or("cursor factory absent")?;
        f.validate(self, p)?;
        let q = f
            .placement()
            .ok_or("cursor factory is not awaiting placement")?;
        let check = crate::lot_acceptance::live::Execution::new(
            self,
            id(&self.vm, q.cursor)?,
            q.position,
            q.level,
        )?;
        let f = self.cursor_factory.as_mut().unwrap();
        f.check = Some(check);
        f.placement_cheat = cheat;
        f.phase = Phase::Check;
        Ok(())
    }
    pub fn pending_cursor_appearance(&self) -> Result<Option<ObjectId>, String> {
        self.cursor_factory
            .as_ref()
            .and_then(|f| f.placement.as_ref())
            .and_then(|e| e.appearance())
            .or_else(|| {
                self.cursor_rotation
                    .as_ref()
                    .and_then(|r| r.execution.appearance())
            })
            .or_else(|| self.cursor_cleanup.as_ref().and_then(|e| e.appearance()))
            .or_else(|| {
                self.deletion
                    .iter()
                    .rev()
                    .find_map(|d| d.cursor.as_ref().and_then(|e| e.appearance()))
            })
            .map(|h| id(&self.vm, h))
            .transpose()
    }
    /// The application has stored this lifetime in its actual view+1bc slot.
    pub fn complete_cursor_factory_focus(&mut self, cursor: u32) -> Result<(), String> {
        if self
            .cursor_factory
            .as_ref()
            .and_then(Factory::pending_focus)
            != Some(cursor)
        {
            return Err("cursor focus reply differs".into());
        }
        id(&self.vm, cursor)?;
        self.cursor_factory.as_mut().unwrap().phase = Phase::Notification;
        Ok(())
    }
    pub fn start_cursor_created(&mut self, cursor: u32) -> Result<(), String> {
        if self
            .cursor_factory
            .as_ref()
            .and_then(Factory::pending_created)
            != Some((cursor, false))
        {
            return Err("cursor creation notification caller differs".into());
        }
        id(&self.vm, cursor)?;
        self.cursor_factory.as_mut().unwrap().notification_started = true;
        Ok(())
    }
    pub fn complete_cursor_created(&mut self, cursor: u32) -> Result<(), String> {
        if self
            .cursor_factory
            .as_ref()
            .and_then(Factory::pending_created)
            != Some((cursor, true))
        {
            return Err("cursor creation notification reply differs".into());
        }
        id(&self.vm, cursor)?;
        let f = self.cursor_factory.as_mut().unwrap();
        f.notification_started = false;
        f.phase = Phase::Complete;
        Ok(())
    }
    pub fn finish_cursor_factory(&mut self, p: &Program) -> Result<ObjectId, String> {
        let f = self
            .cursor_factory
            .as_ref()
            .ok_or("cursor factory absent")?;
        f.validate(self, p)?;
        let h = f.returned().ok_or("cursor factory has not returned")?;
        let object = id(&self.vm, h)?;
        self.cursor_factory = None;
        Ok(object)
    }
    /// Called only after the graphic host has executed Animator +24(0).
    pub fn complete_cursor_appearance(&mut self, object: ObjectId) -> Result<(), String> {
        if self.pending_cursor_appearance()? != Some(object) {
            return Err("cursor appearance reply differs".into());
        }
        let h = handle(&self.vm, object)?;
        if let Some(e) = self
            .cursor_factory
            .as_mut()
            .and_then(|f| f.placement.as_mut())
        {
            e.acknowledge_appearance(h)
        } else if let Some(index) = self
            .deletion
            .iter()
            .rposition(|d| d.cursor.as_ref().is_some_and(|e| e.appearance() == Some(h)))
        {
            self.deletion[index]
                .cursor
                .as_mut()
                .unwrap()
                .acknowledge_appearance(h)
        } else if let Some(e) = self.cursor_cleanup.as_mut() {
            e.acknowledge_appearance(h)
        } else {
            self.cursor_rotation
                .as_mut()
                .ok_or("cursor appearance owner absent")?
                .execution
                .acknowledge_appearance(h)
        }
    }
    pub fn advance_cursor_factory(&mut self, p: &Program, budget: usize) -> Result<Step, String> {
        self.cursor_factory
            .as_ref()
            .ok_or("cursor factory absent")?
            .validate(self, p)?;
        let mut step = Step {
            stop: StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        let mut remaining = budget;
        let result = self.drive_cursor_factory(p, &mut remaining, &mut step);
        self.publish_live_placement_caches()?;
        self.validate_geometry()?;
        self.validate_native_lot()?;
        result?;
        self.cursor_factory.as_ref().unwrap().validate(self, p)?;
        Ok(step)
    }
    fn drive_cursor_factory(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<(), String> {
        while *remaining > 0 {
            let f = self.cursor_factory.as_ref().unwrap();
            match f.phase {
                Phase::Register => {
                    if f.current.is_some_and(|current| {
                        f.created.iter().any(|&h| {
                            id(&self.vm, h)
                                .ok()
                                .and_then(|id| self.vm.world.objects[&id].cursor.as_ref())
                                .is_some_and(|c| c.underlying == Some(current))
                        })
                    }) {
                        return Err("cyclic furniture chain during cursor construction".into());
                    }
                    let new = (1..=i16::MAX)
                        .find(|id| !self.vm.world.objects.contains_key(id))
                        .ok_or("cursor registry exhausted; allocation cleanup host required")?;
                    let current = f.current;
                    let head = f.created.first().map(|&h| id(&self.vm, h)).transpose()?;
                    let definition = f.definition.clone();
                    let captured = if let Some(h) = current {
                        let id = id(&self.vm, h)?;
                        let d = self
                            .native_lot
                            .as_ref()
                            .unwrap()
                            .placement
                            .links
                            .capture(&mut self.vm.world, id)?;
                        Some((
                            d.container.unwrap_or(0),
                            i16::try_from(d.slot).map_err(|_| "cursor captured slot overflow")?,
                        ))
                    } else {
                        None
                    };
                    self.vm
                        .register_cursor_storage(p, new, definition, current, head, captured)?;
                    let h = handle(&self.vm, new)?;
                    let n = self.native_lot.as_mut().unwrap();
                    n.placement.links.next.insert(new, 0);
                    n.placement.bounds.insert(new, [0; 4]);
                    let f = self.cursor_factory.as_mut().unwrap();
                    f.created.push(h);
                    f.phase = if !f.grouped && f.furniture.is_some() {
                        Phase::Pickup
                    } else {
                        Phase::Postconstruct
                    };
                    *remaining -= 1;
                }
                Phase::Pickup => {
                    let (owner, target) = f.pickup_binding().unwrap();
                    if self.object_pickup.is_none() {
                        self.begin_cursor_pickup(p, owner, target)?;
                    }
                    //Use the common live removal driver, retaining its ordinary
                    //graphic service for the existing content/renderer host.
                    let child = self.advance_object_pickup(p, *remaining)?;
                    step.trace.extend(child.trace);
                    step.events.extend(child.events);
                    *remaining = 0;
                    if self.object_pickup.as_ref().unwrap().complete() {
                        self.finish_cursor_pickup(p, owner, target)?;
                        let f = self.cursor_factory.as_mut().unwrap();
                        f.phase = if f.grouped {
                            Phase::Reset
                        } else {
                            Phase::Postconstruct
                        };
                        if f.grouped {
                            f.reset = Some(reset::Execution::new(f.created[0], 1)?);
                        }
                    }
                }
                Phase::Postconstruct => {
                    let object = id(&self.vm, *f.created.last().unwrap())?;
                    self.vm.finish_cursor_postconstruction(p, object)?;
                    self.native_lot
                        .as_mut()
                        .unwrap()
                        .placement
                        .postconstruct_bounds(&self.vm.world, p, object)?;
                    let f = self.cursor_factory.as_mut().unwrap();
                    if f.grouped {
                        let current =
                            id(&self.vm, f.current.ok_or("cursor group current absent")?)?;
                        let next = self.vm.world.objects[&current]
                            .multitile
                            .as_ref()
                            .ok_or("cursor furniture chain absent")?
                            .next;
                        f.current = next.map(|id| handle(&self.vm, id)).transpose()?;
                        f.phase = if f.current.is_some() {
                            Phase::Register
                        } else {
                            Phase::Pickup
                        };
                    } else {
                        f.phase = Phase::Reset;
                        f.reset = Some(reset::Execution::new(f.created[0], 0)?);
                    }
                    *remaining -= 1;
                }
                Phase::Reset => {
                    let date = f.date;
                    let mut reset = self.cursor_factory.as_mut().unwrap().reset.take().unwrap();
                    let result = reset.advance(self, p, date, remaining, step);
                    let f = self.cursor_factory.as_mut().unwrap();
                    if reset.complete() {
                        f.phase = Phase::Select;
                    } else {
                        f.reset = Some(reset);
                    }
                    result?;
                }
                Phase::Select => {
                    let selected = if f.grouped {
                        f.created
                            .iter()
                            .copied()
                            .find(|&h| {
                                id(&self.vm, h)
                                    .ok()
                                    .and_then(|id| self.vm.world.objects[&id].cursor.as_ref())
                                    .and_then(|c| c.underlying)
                                    .and_then(|h| id(&self.vm, h).ok())
                                    == Some(f.selected)
                            })
                            .ok_or("cursor factory selected furniture vanished")?
                    } else {
                        f.created[0]
                    };
                    if f.grouped {
                        let object = id(&self.vm, selected)?;
                        sims_vm::cursor::refresh_offsets(&mut self.vm.world, p, object)?;
                    }
                    let f = self.cursor_factory.as_mut().unwrap();
                    f.selected_cursor = Some(selected);
                    f.phase = Phase::Placement;
                    *remaining -= 1;
                }
                Phase::Check => {
                    let mut check = self.cursor_factory.as_mut().unwrap().check.take().unwrap();
                    let result = self.drive_native_acceptance(p, &mut check, remaining, step);
                    if matches!(result, Ok(true)) {
                        if check.outcome().unwrap().accepted {
                            let f = self.cursor_factory.as_ref().unwrap();
                            let q = Placement {
                                cursor: f.selected_cursor.unwrap(),
                                position: f.position,
                                level: f.level,
                            };
                            let e = placement::Execution::new(self, q, 0, true, f.placement_cheat)?;
                            let f = self.cursor_factory.as_mut().unwrap();
                            f.placement = Some(e);
                            f.phase = Phase::Place;
                        } else {
                            self.cursor_factory.as_mut().unwrap().phase = Phase::Flags;
                        }
                    } else {
                        self.cursor_factory.as_mut().unwrap().check = Some(check);
                    }
                    if !result? {
                        break;
                    }
                }
                Phase::Place => {
                    let mut e = self
                        .cursor_factory
                        .as_mut()
                        .unwrap()
                        .placement
                        .take()
                        .unwrap();
                    let result = e.advance(self, p, remaining, step);
                    let f = self.cursor_factory.as_mut().unwrap();
                    if matches!(result, Ok(true)) {
                        f.phase = Phase::Flags;
                    } else {
                        f.placement = Some(e);
                    }
                    if !result? {
                        break;
                    }
                }
                Phase::Flags => {
                    let selected = id(&self.vm, f.selected_cursor.unwrap())?;
                    // No callbacks occur during this native walk. Resolve the
                    // chain from the live selected cursor, not allocation order.
                    for object in sims_vm::multitile::chain(&self.vm.world, selected)? {
                        let flags = self
                            .vm
                            .world
                            .objects
                            .get_mut(&object)
                            .unwrap()
                            .native_flags
                            .as_mut()
                            .ok_or("cursor native flags absent")?;
                        *flags |= 0x80;
                    }
                    self.cursor_factory.as_mut().unwrap().phase = Phase::Focus;
                    *remaining -= 1;
                }
                Phase::Placement | Phase::Focus | Phase::Notification | Phase::Complete => break,
            }
        }
        Ok(())
    }
    ///Only this owner's newly registered cursor may temporarily lack a body:
    ///43189d's physical zeroes precede431aaf's off-lot initialization.
    pub(crate) fn cursor_constructor_pending(&self, object: ObjectId) -> bool {
        self.cursor_factory
            .as_ref()
            .is_some_and(|f| f.owns_constructor(&self.vm, object))
    }
    pub(crate) fn validate_cursor_constructor(&self, object: ObjectId) -> Result<(), String> {
        let o = self
            .vm
            .world
            .objects
            .get(&object)
            .ok_or("cursor constructor object absent")?;
        let q = self
            .vm
            .world
            .positions
            .get(&object)
            .ok_or("cursor constructor position absent")?;
        let n = self
            .native_lot
            .as_ref()
            .ok_or("cursor constructor lot absent")?;
        if !self.cursor_constructor_pending(object)
            || [q.x, q.y, q.level] != [0; 3]
            || o.container.is_some()
            || !o.slots.is_empty()
            || o.data.get(&14) != Some(&0)
            || o.data.get(&26) != Some(&0)
            || self.bodies.contains_key(&object)
            || self.unplaced.contains(&object)
            || n.placement.links.next.get(&object) != Some(&0)
            || n.placement.bounds.get(&object) != Some(&[0; 4])
            || n.placement
                .links
                .heads
                .iter()
                .flatten()
                .any(|&id| id == object)
        {
            return Err("cursor constructor transient storage differs".into());
        }
        Ok(())
    }
}
