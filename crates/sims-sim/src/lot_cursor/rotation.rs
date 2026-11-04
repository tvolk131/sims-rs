//! Live421be4,422937,433f75 and cursor detach421963. These are actual
//! notch calls, independent of Create Object's four-attempt direction wrapper.
use super::{Placement, handle, id, placement};
use crate::{Simulation, Step, lot_acceptance, lot_links::Descriptor, lot_placement};
use serde::{Deserialize, Serialize};
use sims_vm::cursor::orientation;
use sims_vm::{ObjectId, Program, ThreadId, Vm};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Root {
    tick: u64,
    pub(crate) execution: Execution,
}
impl Root {
    pub fn point(&self) -> Option<Point> {
        match self.execution.call {
            Call::Point(p) => Some(p),
            _ => None,
        }
    }
    pub fn removal(&self) -> bool {
        matches!(self.execution.call, Call::Remove(true))
    }
    pub fn complete(&self) -> bool {
        self.execution.complete()
    }
    pub fn binding(&self) -> (u32, i32) {
        self.execution.binding()
    }
    pub fn orientation(&self) -> Option<i32> {
        self.execution
            .orientation
            .as_ref()
            .map(orientation::Call::direction)
    }
    pub(crate) fn owns_callback(&self, vm: &Vm, p: &Program, t: ThreadId, d: usize) -> bool {
        self.execution.owns_callback(vm, p, t, d)
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        if self.tick != s.vm.tick
            || s.cursor_factory.is_some()
            || s.cursor_release.is_some()
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
            || s.native_lot.is_none()
            || s.native_scheduler
                .as_ref()
                .is_some_and(|n| n.request().is_some())
        {
            return Err("cursor rotation overlaps synchronous work".into());
        }
        if s.vm.world.objects[&id(&s.vm, self.execution.object)?]
            .cursor
            .as_ref()
            .is_none_or(|c| !c.postconstructed)
            || !matches!(
                self.execution.call,
                Call::Rotate(_) | Call::Orient(_) | Call::Remove(true) | Call::Point(_)
            )
        {
            return Err("cursor rotation root lost receiver".into());
        }
        self.execution.validate(s, p)
    }
}
impl Simulation {
    ///Actual421963 virtual+50(1), retained as a synchronous application child.
    pub fn begin_cursor_detach(&mut self, p: &Program, object: ObjectId) -> Result<(), String> {
        if self.cursor_work_pending()
            || !self.deletion.is_empty()
            || self.cursor_has_active_vm_callback()
            || self.person_factories.values().any(|f| !f.finalized)
        {
            return Err("cannot detach cursor during synchronous work".into());
        }
        self.validate_native_lot()?;
        let root = Root {
            tick: self.vm.tick,
            execution: Execution::remove(self, handle(&self.vm, object)?)?,
        };
        root.validate(self, p)?;
        self.cursor_rotation = Some(root);
        Ok(())
    }
    pub fn advance_cursor_detach(&mut self, p: &Program, budget: usize) -> Result<Step, String> {
        if self.cursor_rotation.as_ref().is_none_or(|r| !r.removal()) {
            return Err("cursor detach has no removal caller".into());
        }
        self.advance_cursor_call(p, budget)
    }
    pub fn finish_cursor_detach(&mut self, p: &Program) -> Result<(), String> {
        if self.cursor_rotation.as_ref().is_none_or(|r| !r.removal()) {
            return Err("cursor detach has no removal caller".into());
        }
        self.finish_cursor_call(p)
    }
    pub fn cursor_work_pending(&self) -> bool {
        self.cursor_factory.is_some()
            || self.cursor_rotation.is_some()
            || self.cursor_release.is_some()
            || self.cursor_cleanup.is_some()
    }
    pub fn begin_cursor_rotation(
        &mut self,
        p: &Program,
        object: ObjectId,
        notches: i32,
        cheat: bool,
    ) -> Result<(), String> {
        self.begin_cursor_call(p, object, Call::Rotate(notches), cheat)
    }
    ///4226bc owns detach, repeated virtual rotations, acceptance and placement.
    pub fn begin_cursor_orientation(
        &mut self,
        p: &Program,
        object: ObjectId,
        direction: i32,
        cheat: bool,
    ) -> Result<(), String> {
        self.begin_cursor_call(p, object, Call::Orient(direction), cheat)
    }
    ///A direct cursor virtual+54 or+58 call, with the same synchronous child
    ///ownership used by rotation. No ordinary simulation tick can interleave.
    pub fn begin_cursor_point(
        &mut self,
        p: &Program,
        object: ObjectId,
        point: Point,
        cheat: bool,
    ) -> Result<(), String> {
        self.begin_cursor_call(p, object, Call::Point(point), cheat)
    }
    pub fn advance_cursor_point(&mut self, p: &Program, budget: usize) -> Result<Step, String> {
        if self
            .cursor_rotation
            .as_ref()
            .and_then(Root::point)
            .is_none()
        {
            return Err("cursor point caller absent".into());
        }
        self.advance_cursor_call(p, budget)
    }
    pub fn finish_cursor_point(&mut self, p: &Program) -> Result<PointResult, String> {
        let root = self
            .cursor_rotation
            .as_ref()
            .ok_or("cursor point caller absent")?;
        if root.point().is_none() {
            return Err("cursor point caller absent".into());
        }
        let result = root
            .execution
            .point_result
            .ok_or("cursor point has not returned")?;
        self.finish_cursor_call(p)?;
        Ok(result)
    }
    fn begin_cursor_call(
        &mut self,
        p: &Program,
        object: ObjectId,
        call: Call,
        cheat: bool,
    ) -> Result<(), String> {
        if self.cursor_work_pending()
            || self.cursor_has_active_vm_callback()
            || !self.deletion.is_empty()
            || self.person_factories.values().any(|f| !f.finalized)
        {
            return Err("cannot start cursor rotation during synchronous work".into());
        }
        self.validate_native_lot()?;
        let root = Root {
            tick: self.vm.tick,
            execution: Execution::begin(self, handle(&self.vm, object)?, call, cheat)?,
        };
        root.validate(self, p)?;
        self.cursor_rotation = Some(root);
        Ok(())
    }
    pub fn advance_cursor_rotation(&mut self, p: &Program, budget: usize) -> Result<Step, String> {
        if self
            .cursor_rotation
            .as_ref()
            .is_some_and(|r| r.removal() || r.orientation().is_some() || r.point().is_some())
        {
            return Err("cursor rotation cannot advance a removal caller".into());
        }
        self.advance_cursor_call(p, budget)
    }
    pub fn advance_cursor_orientation(
        &mut self,
        p: &Program,
        budget: usize,
    ) -> Result<Step, String> {
        if self
            .cursor_rotation
            .as_ref()
            .and_then(Root::orientation)
            .is_none()
        {
            return Err("cursor orientation caller absent".into());
        }
        self.advance_cursor_call(p, budget)
    }
    pub fn finish_cursor_orientation(&mut self, p: &Program) -> Result<(), String> {
        if self
            .cursor_rotation
            .as_ref()
            .and_then(Root::orientation)
            .is_none()
        {
            return Err("cursor orientation caller absent".into());
        }
        self.finish_cursor_call(p)
    }
    fn advance_cursor_call(&mut self, p: &Program, budget: usize) -> Result<Step, String> {
        if budget == 0 {
            return Err("cursor rotation requires a positive budget".into());
        }
        self.cursor_rotation
            .as_ref()
            .ok_or("cursor rotation absent")?
            .validate(self, p)?;
        let mut root = self.cursor_rotation.take().unwrap();
        let mut step = Step {
            stop: crate::StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        let mut remaining = budget;
        let result = root.execution.advance(self, p, &mut remaining, &mut step);
        self.cursor_rotation = Some(root);
        self.publish_live_placement_caches()?;
        self.validate_geometry()?;
        self.validate_native_lot()?;
        result?;
        self.cursor_rotation.as_ref().unwrap().validate(self, p)?;
        Ok(step)
    }
    pub fn finish_cursor_rotation(&mut self, p: &Program) -> Result<(), String> {
        if self
            .cursor_rotation
            .as_ref()
            .is_some_and(|r| r.removal() || r.orientation().is_some() || r.point().is_some())
        {
            return Err("cursor rotation cannot finish a removal caller".into());
        }
        self.finish_cursor_call(p)
    }
    fn finish_cursor_call(&mut self, p: &Program) -> Result<(), String> {
        let root = self
            .cursor_rotation
            .as_ref()
            .ok_or("cursor rotation absent")?;
        root.validate(self, p)?;
        if !root.complete() {
            return Err("cursor rotation has not returned".into());
        }
        self.cursor_rotation = None;
        Ok(())
    }
}

///Null-container cursor calls used by Move's pointer handler. A slot is only
///meaningful to +58's furniture placement/search; +54 is floor acceptance.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Point {
    Check {
        position: [i32; 2],
        level: i32,
    },
    Place {
        position: [i32; 2],
        level: i32,
        slot: i32,
        notify: bool,
    },
}
impl Point {
    fn location(self) -> ([i32; 2], i32) {
        match self {
            Self::Check { position, level }
            | Self::Place {
                position, level, ..
            } => (position, level),
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PointResult {
    Checked {
        outcome: lot_acceptance::Outcome,
        position: [i32; 2],
    },
    Placed,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Call {
    Point(Point),
    Remove(bool),
    Rotate(i32),
    Orient(i32),
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Mode {
    Point,
    Remove,
    Base,
    Group,
    Cursor,
    Orientation,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Point,
    Orientation,
    RemoveStart,
    RemoveFurniture,
    Base,
    GroupDetach,
    GroupDelta,
    GroupMembers,
    GroupShadow,
    GroupNext,
    GroupFirstCheck,
    GroupSecondCheck,
    GroupPlace,
    CursorDetach,
    CursorTarget,
    CursorChildren,
    CursorOffsets,
    CursorCheck,
    CursorPlace,
    Complete,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    object: u32,
    call: Call,
    mode: Mode,
    phase: Phase,
    cheat: bool,
    old: Option<Placement>,
    delta: i16,
    current: Option<u32>,
    slot: i32,
    grouped_children: bool,
    seen: BTreeSet<u32>,
    child: Option<Box<Execution>>,
    child_slot: Option<(u32, usize, u32)>,
    physical: Option<lot_placement::live::Execution>,
    check: Option<lot_acceptance::live::Execution>,
    placement: Option<Box<placement::Execution>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    orientation: Option<orientation::Call>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    point_result: Option<PointResult>,
}
struct OrientationView<'a> {
    s: &'a Simulation,
    object: u32,
}
impl orientation::View for OrientationView<'_> {
    fn has_furniture(&self) -> Result<bool, String> {
        Ok(cursor(self.s, self.object)?.underlying.is_some())
    }
    fn facing(&self, furniture: bool) -> Result<i16, String> {
        let h = if furniture {
            cursor(self.s, self.object)?
                .underlying
                .ok_or("orientation furniture disappeared")?
        } else {
            self.object
        };
        self.s.vm.world.objects[&id(&self.s.vm, h)?]
            .data
            .get(&1)
            .copied()
            .ok_or("orientation facing absent".into())
    }
    fn position(&self) -> Result<[i32; 2], String> {
        Ok(cursor(self.s, self.object)?.position)
    }
    fn level(&self) -> Result<i32, String> {
        Ok(cursor(self.s, self.object)?.level)
    }
}
fn cursor(s: &Simulation, h: u32) -> Result<&sims_vm::cursor::State, String> {
    s.vm.world.objects[&id(&s.vm, h)?]
        .cursor
        .as_ref()
        .ok_or("rotation cursor absent".into())
}
fn words<'a>(s: &Simulation, p: &'a Program, h: u32) -> Result<&'a [i16], String> {
    let d = s.vm.world.objects[&id(&s.vm, h)?]
        .definition
        .as_ref()
        .ok_or("rotation definition absent")?;
    sims_vm::construction::definition_words(p, d)
}
impl Execution {
    pub(crate) fn remove(s: &Simulation, object: u32) -> Result<Self, String> {
        Self::begin(s, object, Call::Remove(true), false)
    }
    pub(crate) fn is_removal(&self, object: u32) -> bool {
        self.object == object && self.call == Call::Remove(true)
    }
    pub fn new(s: &Simulation, object: u32, notches: i32, cheat: bool) -> Result<Self, String> {
        Self::begin(s, object, Call::Rotate(notches), cheat)
    }
    fn begin(s: &Simulation, object: u32, call: Call, cheat: bool) -> Result<Self, String> {
        let o = &s.vm.world.objects[&id(&s.vm, object)?];
        Self::begin_object(o, object, call, cheat)
    }
    pub(crate) fn remove_object(world: &sims_vm::World, object: ObjectId) -> Result<Self, String> {
        let h = world
            .scheduler_storage
            .as_ref()
            .and_then(|r| r.handle(object))
            .ok_or("cursor removal lifetime absent")?;
        let o = world
            .objects
            .get(&object)
            .ok_or("cursor removal object absent")?;
        Self::begin_object(o, h, Call::Remove(true), false)
    }
    fn begin_object(
        o: &sims_vm::Object,
        object: u32,
        call: Call,
        cheat: bool,
    ) -> Result<Self, String> {
        if o.person.is_some() {
            return Err("person virtual rotation/removal adapter required".into());
        }
        let (mode, phase) = match call {
            Call::Point(_) => (Mode::Point, Phase::Point),
            Call::Orient(_) => (Mode::Orientation, Phase::Orientation),
            Call::Remove(_) => (Mode::Remove, Phase::RemoveStart),
            Call::Rotate(_) if o.cursor.as_ref().is_some_and(|c| c.underlying.is_some()) => {
                (Mode::Cursor, Phase::CursorDetach)
            }
            Call::Rotate(_) if o.multitile.is_some() => (Mode::Group, Phase::GroupDetach),
            Call::Rotate(_) => (Mode::Base, Phase::Base),
        };
        Ok(Self {
            object,
            call,
            mode,
            phase,
            cheat,
            old: None,
            delta: 0,
            current: None,
            slot: -1,
            grouped_children: false,
            seen: BTreeSet::new(),
            child: None,
            child_slot: None,
            physical: None,
            check: None,
            placement: None,
            point_result: None,
            orientation: if let Call::Orient(direction) = call {
                Some(orientation::Call::new(direction))
            } else {
                None
            },
        })
    }
    pub fn binding(&self) -> (u32, i32) {
        (
            self.object,
            match self.call {
                Call::Rotate(n) => n,
                Call::Orient(n) => n,
                Call::Remove(_) | Call::Point(_) => 0,
            },
        )
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub(crate) fn cheat(&self) -> bool {
        self.cheat
    }
    fn notches(&self) -> Result<i32, String> {
        if let Call::Rotate(n) = self.call {
            Ok(n)
        } else {
            Err("removal is not a notch call".into())
        }
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, t: ThreadId, d: usize) -> bool {
        self.child
            .as_ref()
            .is_some_and(|c| c.owns_callback(vm, p, t, d))
            || self
                .physical
                .as_ref()
                .is_some_and(|c| c.owns_callback(vm, p, t, d))
            || self
                .check
                .as_ref()
                .is_some_and(|c| c.owns_callback(vm, p, t, d))
            || self
                .placement
                .as_ref()
                .is_some_and(|c| c.owns_callback(vm, p, t, d))
    }
    pub(crate) fn graphic(&self) -> Option<&lot_placement::live::Execution> {
        self.child
            .as_ref()
            .and_then(|c| c.graphic())
            .or_else(|| self.placement.as_ref().and_then(|c| c.graphic()))
            .or(self.physical.as_ref())
            .filter(|e| matches!(e.pending, Some(lot_placement::Request::RefreshAnimator(_))))
    }
    pub(crate) fn graphic_mut(&mut self) -> Option<&mut lot_placement::live::Execution> {
        if self.child.as_ref().is_some_and(|c| c.graphic().is_some()) {
            return self.child.as_mut().unwrap().graphic_mut();
        }
        if self
            .placement
            .as_ref()
            .is_some_and(|c| c.graphic().is_some())
        {
            return self.placement.as_mut().unwrap().graphic_mut();
        }
        self.physical
            .as_mut()
            .filter(|e| matches!(e.pending, Some(lot_placement::Request::RefreshAnimator(_))))
    }
    pub(crate) fn appearance(&self) -> Option<u32> {
        self.child
            .as_ref()
            .and_then(|c| c.appearance())
            .or_else(|| self.placement.as_ref().and_then(|c| c.appearance()))
    }
    pub(crate) fn acknowledge_appearance(&mut self, h: u32) -> Result<(), String> {
        if self
            .child
            .as_ref()
            .is_some_and(|c| c.appearance() == Some(h))
        {
            return self.child.as_mut().unwrap().acknowledge_appearance(h);
        }
        self.placement
            .as_mut()
            .ok_or("rotation appearance owner absent")?
            .acknowledge_appearance(h)
    }
    pub fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        self.validate_depth(s, p, 0)
    }
    pub(crate) fn validate_depth(
        &self,
        s: &Simulation,
        p: &Program,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 64 {
            return Err("native cursor rotation nesting limit".into());
        }
        let object = id(&s.vm, self.object)?;
        let receiver = &s.vm.world.objects[&object];
        if !(0..8).contains(&self.delta)
            || self.slot < -1
            || self.complete()
                && (self.child.is_some()
                    || self.physical.is_some()
                    || self.check.is_some()
                    || self.placement.is_some())
            || (self.mode == Mode::Point) != matches!(self.call, Call::Point(_))
            || self.point_result.is_some() != (self.mode == Mode::Point && self.complete())
            || self.mode == Mode::Point && receiver.cursor.is_none()
            || (self.mode == Mode::Remove) != matches!(self.call, Call::Remove(_))
            || (self.mode == Mode::Orientation) != matches!(self.call, Call::Orient(_))
            || self.orientation.is_some() != (self.mode == Mode::Orientation)
            || [
                self.child.is_some(),
                self.physical.is_some(),
                self.check.is_some(),
                self.placement.is_some(),
            ]
            .into_iter()
            .filter(|b| *b)
            .count()
                > 1
            || self.child_slot.is_some()
                != (self.phase == Phase::CursorChildren && self.child.is_some())
            || receiver.person.is_some()
            || self.mode == Mode::Base
                && (receiver.cursor.is_some() || receiver.multitile.is_some())
            || self.mode == Mode::Group && receiver.multitile.is_none()
            || self.mode == Mode::Cursor && receiver.cursor.is_none()
            || self.mode == Mode::Orientation && receiver.cursor.is_none()
            || self.mode == Mode::Group
                && (self.phase != Phase::GroupDetach || self.child.is_some())
                && self.old.is_none()
            || self.old.is_some_and(|q| q.cursor != self.object)
            || matches!(self.phase, Phase::GroupShadow | Phase::GroupNext) && self.current.is_none()
        {
            return Err("invalid native cursor rotation owner".into());
        }
        let allowed = match self.mode {
            Mode::Point => matches!(self.phase, Phase::Point | Phase::Complete),
            Mode::Orientation => matches!(self.phase, Phase::Orientation | Phase::Complete),
            Mode::Remove => matches!(
                self.phase,
                Phase::RemoveStart | Phase::RemoveFurniture | Phase::Complete
            ),
            Mode::Base => matches!(self.phase, Phase::Base | Phase::Complete),
            Mode::Group => matches!(
                self.phase,
                Phase::GroupDetach
                    | Phase::GroupDelta
                    | Phase::GroupMembers
                    | Phase::GroupShadow
                    | Phase::GroupNext
                    | Phase::GroupFirstCheck
                    | Phase::GroupSecondCheck
                    | Phase::GroupPlace
                    | Phase::Complete
            ),
            Mode::Cursor => matches!(
                self.phase,
                Phase::CursorDetach
                    | Phase::CursorTarget
                    | Phase::CursorChildren
                    | Phase::CursorOffsets
                    | Phase::CursorCheck
                    | Phase::CursorPlace
                    | Phase::Complete
            ),
        };
        if !allowed {
            return Err("rotation phase differs from virtual call".into());
        }
        if let Some(c) = &self.orientation {
            c.validate()?;
            if self.call != Call::Orient(c.direction()) || self.complete() != c.complete() {
                return Err("orientation wrapper lost its call".into());
            }
        }
        if let Some(result) = self.point_result {
            let valid = match (self.call, result) {
                (
                    Call::Point(Point::Check { position, .. }),
                    PointResult::Checked {
                        position: checked, ..
                    },
                ) => position == checked,
                (Call::Point(Point::Place { .. }), PointResult::Placed) => true,
                _ => false,
            };
            if !valid {
                return Err("cursor point result differs from caller".into());
            }
        }
        if let Some(h) = self.current {
            id(&s.vm, h)?;
        }
        if let Some(c) = &self.child {
            let expected = match self.phase {
                Phase::Orientation => (
                    self.object,
                    match self
                        .orientation
                        .as_ref()
                        .and_then(orientation::Call::request)
                    {
                        Some(orientation::Request::Detach) => Call::Remove(true),
                        Some(orientation::Request::Rotate) => Call::Rotate(1),
                        _ => return Err("orientation rotation child lost caller".into()),
                    },
                ),
                Phase::GroupDetach | Phase::CursorDetach => (self.object, Call::Remove(true)),
                Phase::RemoveFurniture => (
                    cursor(s, self.object)?
                        .underlying
                        .ok_or("removed cursor target absent")?,
                    self.call,
                ),
                Phase::CursorTarget => (
                    cursor(s, self.object)?
                        .underlying
                        .ok_or("rotated cursor target absent")?,
                    self.call,
                ),
                Phase::CursorChildren => (
                    self.child_slot.ok_or("rotation slot receiver absent")?.2,
                    self.call,
                ),
                _ => return Err("rotation child has no caller".into()),
            };
            if (c.object, c.call) != expected {
                return Err("rotation child binding differs".into());
            }
            c.validate_depth(s, p, depth + 1)?;
        }
        if let Some(e) = &self.physical {
            if self.phase == Phase::Base {
                e.validate_native_rotation(&s.vm, p, object, self.notches()?)?;
            } else {
                if !matches!(self.phase, Phase::RemoveStart | Phase::GroupPlace) {
                    return Err("rotation physical child has no caller".into());
                }
                e.validate(&s.vm, p, object)?;
                let expected = if self.phase == Phase::RemoveStart {
                    None
                } else {
                    let q = self.old.ok_or("group rotation lost captured position")?;
                    Some(Descriptor {
                        on_lot: true,
                        position: q.position,
                        level: q.level,
                        container: None,
                        slot: 0,
                    })
                };
                if e.operation.binding() != (object, expected) {
                    return Err("rotation physical descriptor differs".into());
                }
            }
        }
        if let Some(e) = &self.check {
            e.validate(&s.vm, p)?;
            if !matches!(
                self.phase,
                Phase::Point
                    | Phase::GroupFirstCheck
                    | Phase::GroupSecondCheck
                    | Phase::CursorCheck
                    | Phase::Orientation
            ) || e.binding().0 != object
                || e.container_binding().is_some()
            {
                return Err("rotation acceptance owner differs".into());
            }
            if self.mode == Mode::Point {
                let Call::Point(Point::Check { position, level }) = self.call else {
                    return Err("cursor point check lost caller".into());
                };
                if e.binding() != (object, position, level) {
                    return Err("cursor point check arguments differ".into());
                }
            }
            if self.mode == Mode::Orientation {
                let Some(orientation::Request::Check { position, level }) = self
                    .orientation
                    .as_ref()
                    .and_then(orientation::Call::request)
                else {
                    return Err("orientation acceptance lost caller".into());
                };
                if e.binding() != (object, position, level) {
                    return Err("orientation acceptance arguments differ".into());
                }
            }
            if self.mode == Mode::Group {
                let q = self
                    .old
                    .ok_or("group rotation lost its captured position")?;
                if e.binding() != (object, q.position, q.level) {
                    return Err("group rotation check moved its saved point".into());
                }
            }
        }
        if let Some(e) = &self.placement {
            if !matches!(
                self.phase,
                Phase::Point | Phase::GroupPlace | Phase::CursorPlace | Phase::Orientation
            ) || e.binding().cursor != self.object
            {
                return Err("rotation cursor placement owner differs".into());
            }
            if self.mode == Mode::Point {
                let Call::Point(Point::Place {
                    position,
                    level,
                    slot,
                    notify,
                }) = self.call
                else {
                    return Err("cursor point placement lost caller".into());
                };
                if e.binding()
                    != (Placement {
                        cursor: self.object,
                        position,
                        level,
                    })
                    || !e.has_options(slot, notify, self.cheat)
                {
                    return Err("cursor point placement arguments differ".into());
                }
            }
            if self.mode == Mode::Orientation {
                let Some(orientation::Request::Place { position, level }) = self
                    .orientation
                    .as_ref()
                    .and_then(orientation::Call::request)
                else {
                    return Err("orientation placement lost caller".into());
                };
                if e.binding()
                    != (Placement {
                        cursor: self.object,
                        position,
                        level,
                    })
                {
                    return Err("orientation placement arguments differ".into());
                }
            }
            e.validate_depth(s, p, depth + 1)?;
        }
        Ok(())
    }
    fn child(&mut self, s: &Simulation, h: u32, call: Call) -> Result<(), String> {
        self.child = Some(Box::new(Self::begin(s, h, call, self.cheat)?));
        Ok(())
    }
    fn head(&self, s: &Simulation) -> Result<u32, String> {
        handle(
            &s.vm,
            sims_vm::multitile::chain(&s.vm.world, id(&s.vm, self.object)?)?[0],
        )
    }
    fn offsets(s: &mut Simulation, p: &Program, object: ObjectId) -> Result<(), String> {
        if s.vm.world.objects[&object].cursor.is_some() {
            sims_vm::cursor::refresh_offsets(&mut s.vm.world, p, object)
        } else {
            sims_vm::multitile::update_offsets(&mut s.vm.world, object)
        }
    }
    fn next_part(s: &Simulation, h: u32) -> Result<Option<u32>, String> {
        s.vm.world.objects[&id(&s.vm, h)?]
            .multitile
            .as_ref()
            .ok_or("rotation part chain absent")?
            .next
            .map(|o| handle(&s.vm, o))
            .transpose()
    }
    fn begin_check(&mut self, s: &Simulation) -> Result<(), String> {
        let q = if self.mode == Mode::Group {
            self.old.ok_or("group rotation point absent")?
        } else {
            let c = cursor(s, self.object)?;
            Placement {
                cursor: self.object,
                position: c.position,
                level: c.level,
            }
        };
        self.check = Some(lot_acceptance::live::Execution::new(
            s,
            id(&s.vm, self.object)?,
            q.position,
            q.level,
        )?);
        Ok(())
    }
    fn begin_place(&mut self, s: &Simulation) -> Result<(), String> {
        let q = if self.mode == Mode::Group {
            self.old.ok_or("group rotation point absent")?
        } else {
            let c = cursor(s, self.object)?;
            Placement {
                cursor: self.object,
                position: c.position,
                level: c.level,
            }
        };
        if s.vm.world.objects[&id(&s.vm, self.object)?]
            .cursor
            .is_some()
        {
            let slot = if self.mode == Mode::Cursor {
                i32::from(cursor(s, self.object)?.slot)
            } else {
                0
            };
            self.placement = Some(Box::new(placement::Execution::new(
                s, q, slot, true, self.cheat,
            )?));
        } else {
            let links = &s
                .native_lot
                .as_ref()
                .ok_or("rotation physical lot absent")?
                .placement
                .links;
            self.physical = Some(lot_placement::live::Execution::new_virtual(
                lot_placement::VirtualOperation::set(
                    &s.vm.world,
                    links,
                    id(&s.vm, self.object)?,
                    Descriptor {
                        on_lot: true,
                        position: q.position,
                        level: q.level,
                        container: None,
                        slot: 0,
                    },
                    true,
                )?,
            ));
        }
        Ok(())
    }
    fn begin_slots(&mut self, s: &Simulation, p: &Program) -> Result<(), String> {
        let target = cursor(s, self.object)?
            .underlying
            .ok_or("cursor target disappeared after rotation")?;
        self.grouped_children = words(s, p, target)?[10] != 0;
        self.current = Some(if self.grouped_children {
            handle(
                &s.vm,
                sims_vm::multitile::chain(&s.vm.world, id(&s.vm, target)?)?[0],
            )?
        } else {
            target
        });
        self.seen.clear();
        self.capture_slots(s)
    }
    fn capture_slots(&mut self, s: &Simulation) -> Result<(), String> {
        if let Some(h) = self.current {
            if !self.seen.insert(h) {
                return Err("cyclic contained rotation traversal".into());
            }
            self.slot = i32::try_from(s.vm.world.objects[&id(&s.vm, h)?].slots.len())
                .map_err(|_| "native rotation slot count overflow")?
                - 1;
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
        self.advance_depth(s, p, left, step, 0)
    }
    pub(crate) fn advance_depth(
        &mut self,
        s: &mut Simulation,
        p: &Program,
        left: &mut usize,
        step: &mut Step,
        depth: usize,
    ) -> Result<bool, String> {
        if depth > 64 {
            return Err("native cursor rotation nesting limit".into());
        }
        while *left > 0 && !self.complete() {
            if let Some(c) = &mut self.child {
                if !c.advance_depth(s, p, left, step, depth + 1)? {
                    return Ok(false);
                }
                self.child = None;
                self.child_slot = None;
                self.phase = match self.phase {
                    Phase::Orientation => {
                        let call = self.orientation.as_mut().unwrap();
                        call.acknowledge(
                            call.request().ok_or("orientation child absent")?,
                            orientation::Reply::Done,
                        )?;
                        Phase::Orientation
                    }
                    Phase::RemoveFurniture => Phase::Complete,
                    Phase::GroupDetach => Phase::GroupDelta,
                    Phase::CursorDetach => Phase::CursorTarget,
                    Phase::CursorTarget => {
                        self.begin_slots(s, p)?;
                        Phase::CursorChildren
                    }
                    Phase::CursorChildren => Phase::CursorChildren,
                    _ => return Err("rotation child return has no caller".into()),
                };
                continue;
            }
            if let Some(e) = &mut self.physical {
                if !s.drive_removal_placement(p, e, left, step)? {
                    return Ok(false);
                }
                self.physical = None;
                self.phase = if self.phase == Phase::RemoveStart {
                    Phase::RemoveFurniture
                } else {
                    Phase::Complete
                };
                continue;
            }
            if let Some(e) = &mut self.placement {
                if !e.advance_depth(s, p, left, step, depth + 1)? {
                    return Ok(false);
                }
                self.placement = None;
                if self.mode == Mode::Point {
                    self.point_result = Some(PointResult::Placed);
                }
                if let Some(call) = &mut self.orientation {
                    call.acknowledge(
                        call.request().ok_or("orientation placement absent")?,
                        orientation::Reply::Done,
                    )?;
                }
                self.phase = Phase::Complete;
                continue;
            }
            if let Some(e) = &mut self.check {
                if !s.drive_native_acceptance(p, e, left, step)? {
                    return Ok(false);
                }
                let outcome = e.outcome().unwrap();
                let accepted = outcome.accepted;
                let checked_position = e.binding().1;
                self.check = None;
                match self.phase {
                    Phase::Point => {
                        self.point_result = Some(PointResult::Checked {
                            outcome,
                            position: checked_position,
                        });
                        self.phase = Phase::Complete;
                    }
                    Phase::Orientation => {
                        let call = self.orientation.as_mut().unwrap();
                        call.acknowledge(
                            call.request().ok_or("orientation acceptance absent")?,
                            orientation::Reply::Checked {
                                accepted,
                                position: checked_position,
                            },
                        )?;
                        if call.complete() {
                            self.phase = Phase::Complete;
                        }
                    }
                    Phase::GroupFirstCheck => {
                        let q = self.old.unwrap();
                        if !accepted
                            && s.native_lot
                                .as_ref()
                                .unwrap()
                                .placement
                                .links
                                .on_lot(q.position)
                        {
                            //422937 rollback has no Invalidate/Shadow calls.
                            for o in
                                sims_vm::multitile::chain(&s.vm.world, id(&s.vm, self.object)?)?
                            {
                                let object = s.vm.world.objects.get_mut(&o).unwrap();
                                object
                                    .data
                                    .insert(1, object.data[&1].wrapping_sub(self.delta) & 7);
                            }
                            let selected = id(&s.vm, self.object)?;
                            Self::offsets(s, p, selected)?;
                        }
                        self.phase = Phase::GroupSecondCheck;
                    }
                    Phase::GroupSecondCheck => {
                        self.phase = if accepted {
                            Phase::GroupPlace
                        } else {
                            Phase::Complete
                        }
                    }
                    Phase::CursorCheck => {
                        if !accepted {
                            for o in
                                sims_vm::multitile::chain(&s.vm.world, id(&s.vm, self.object)?)?
                            {
                                let Some(c) =
                                    s.vm.world.objects.get_mut(&o).unwrap().cursor.as_mut()
                                else {
                                    break;
                                };
                                c.valid = 0;
                            }
                        }
                        self.phase = if accepted {
                            Phase::CursorPlace
                        } else {
                            Phase::Complete
                        };
                    }
                    _ => return Err("rotation acceptance return has no caller".into()),
                }
                continue;
            }
            let object = id(&s.vm, self.object)?;
            match self.phase {
                Phase::Point => {
                    let Call::Point(point) = self.call else {
                        return Err("cursor point caller absent".into());
                    };
                    let (position, level) = point.location();
                    match point {
                        Point::Check { .. } => {
                            self.check = Some(lot_acceptance::live::Execution::new(
                                s, object, position, level,
                            )?)
                        }
                        Point::Place { slot, notify, .. } => {
                            self.placement = Some(Box::new(placement::Execution::new(
                                s,
                                Placement {
                                    cursor: self.object,
                                    position,
                                    level,
                                },
                                slot,
                                notify,
                                self.cheat,
                            )?))
                        }
                    }
                }
                Phase::Orientation => {
                    let request = self
                        .orientation
                        .as_mut()
                        .unwrap()
                        .advance(&OrientationView {
                            s,
                            object: self.object,
                        })?;
                    match request {
                        Some(orientation::Request::Detach) => {
                            self.child(s, self.object, Call::Remove(true))?
                        }
                        Some(orientation::Request::Rotate) => {
                            self.child(s, self.object, Call::Rotate(1))?
                        }
                        Some(orientation::Request::Check { position, level }) => {
                            self.check = Some(lot_acceptance::live::Execution::new(
                                s, object, position, level,
                            )?);
                        }
                        Some(orientation::Request::Place { position, level }) => {
                            self.placement = Some(Box::new(placement::Execution::new(
                                s,
                                Placement {
                                    cursor: self.object,
                                    position,
                                    level,
                                },
                                0,
                                true,
                                self.cheat,
                            )?));
                        }
                        None => self.phase = Phase::Complete,
                    }
                }
                Phase::RemoveStart => {
                    let Call::Remove(notify) = self.call else {
                        return Err("rotation remove argument absent".into());
                    };
                    //Explicit422d86 for a cursor, followed by its represented
                    //object's actual virtual+50. Never stop at the cursor chain.
                    self.physical = Some(lot_placement::live::Execution::new_virtual(
                        lot_placement::VirtualOperation::remove(
                            &s.vm.world,
                            &s.native_lot
                                .as_ref()
                                .ok_or("rotation lot absent")?
                                .placement
                                .links,
                            object,
                            notify,
                        )?,
                    ));
                }
                Phase::RemoveFurniture => {
                    if let Some(target) = s.vm.world.objects[&object]
                        .cursor
                        .as_ref()
                        .and_then(|c| c.underlying)
                    {
                        self.child(s, target, self.call)?;
                    } else {
                        self.phase = Phase::Complete;
                    }
                }
                Phase::Base => {
                    self.physical = Some(lot_placement::live::Execution::new(
                        lot_placement::Operation::rotate(object, self.notches()?),
                    ))
                }
                Phase::GroupDetach => {
                    let q =
                        s.vm.world
                            .positions
                            .get(&object)
                            .ok_or("group rotation position absent")?;
                    self.old = Some(Placement {
                        cursor: self.object,
                        position: [q.x, q.y],
                        level: q.level,
                    });
                    self.child(s, self.object, Call::Remove(true))?;
                }
                Phase::GroupDelta => {
                    self.delta = (i32::from(s.vm.world.objects[&object].data[&23])
                        .wrapping_mul(self.notches()?)
                        & 7) as i16;
                    if self.delta == 0 {
                        self.phase = Phase::Complete;
                    } else {
                        self.current = Some(self.head(s)?);
                        self.seen.clear();
                        self.phase = Phase::GroupMembers;
                    }
                }
                Phase::GroupMembers => {
                    if let Some(h) = self.current {
                        if !self.seen.insert(h) {
                            return Err("cyclic multipart notch rotation".into());
                        }
                        let o = id(&s.vm, h)?;
                        let data = &mut s.vm.world.objects.get_mut(&o).unwrap().data;
                        data.insert(1, data[&1].wrapping_add(self.delta) & 7);
                        s.native_lot.as_mut().unwrap().observers.push(
                            lot_placement::Request::Invalidate {
                                object: o,
                                children_only: false,
                            },
                        );
                        self.phase = Phase::GroupShadow;
                    } else {
                        Self::offsets(s, p, object)?;
                        self.phase = Phase::GroupFirstCheck;
                    }
                }
                Phase::GroupShadow => {
                    s.native_lot
                        .as_mut()
                        .unwrap()
                        .observers
                        .push(lot_placement::Request::Shadow(id(
                            &s.vm,
                            self.current.unwrap(),
                        )?));
                    self.phase = Phase::GroupNext;
                }
                Phase::GroupNext => {
                    self.current = Self::next_part(s, self.current.unwrap())?;
                    self.phase = Phase::GroupMembers;
                }
                Phase::GroupFirstCheck | Phase::GroupSecondCheck | Phase::CursorCheck => {
                    self.begin_check(s)?
                }
                Phase::GroupPlace | Phase::CursorPlace => self.begin_place(s)?,
                Phase::CursorDetach => self.child(s, self.object, Call::Remove(true))?,
                Phase::CursorTarget => self.child(
                    s,
                    cursor(s, self.object)?
                        .underlying
                        .ok_or("cursor underlying rotation absent")?,
                    self.call,
                )?,
                Phase::CursorChildren => {
                    let Some(h) = self.current else {
                        self.phase = Phase::CursorOffsets;
                        continue;
                    };
                    if self.slot < 0 {
                        self.current = if self.grouped_children {
                            Self::next_part(s, h)?
                        } else {
                            None
                        };
                        self.capture_slots(s)?;
                    } else {
                        //Ordinary source rereads cursor.underlying; groups keep
                        //the current group pointer. Bounds and IDs remain live.
                        let parent = if self.grouped_children {
                            h
                        } else {
                            cursor(s, self.object)?
                                .underlying
                                .ok_or("cursor slot parent vanished")?
                        };
                        let child = s.vm.world.objects[&id(&s.vm, parent)?]
                            .slots
                            .get(self.slot as usize)
                            .copied()
                            .flatten();
                        let slot = self.slot as usize;
                        self.slot -= 1;
                        if let Some(o) = child.filter(|o| s.vm.world.objects.contains_key(o)) {
                            let h = handle(&s.vm, o)?;
                            if words(s, p, h)?[10] == 0 && s.vm.world.objects[&o].data[&23] == 2 {
                                self.child_slot = Some((parent, slot, h));
                                self.child(s, h, self.call)?;
                            }
                        }
                    }
                }
                Phase::CursorOffsets => {
                    Self::offsets(s, p, object)?;
                    self.phase = Phase::CursorCheck;
                }
                Phase::Complete => break,
            }
            *left -= 1;
        }
        Ok(self.complete())
    }
}
