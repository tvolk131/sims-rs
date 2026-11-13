//!421e72 and422050: the represented object and its cursor have distinct
//!acceptance/placement calls. Slot-search rotation is an explicit synchronous
//!421be4 request; it must not be replaced with a direction-field assignment.
use super::{
    Placement,
    acceptance::{Check, Query},
    handle, id,
};
use crate::{
    Simulation, Step, lot_acceptance,
    lot_links::{Cursor, Descriptor},
    lot_placement,
};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, ThreadId, Vm};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Request {
    Rotate { cursor: u32, notches: i32 },
    Appearance(u32),
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Start,
    Check,
    Search,
    Rotation,
    Budget,
    Furniture,
    Appearance,
    Cursor,
    Publish,
    Complete,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Search {
    mask: u16,
    iterator: Option<Cursor>,
    count: Option<usize>,
    slot: usize,
    seen: BTreeSet<ObjectId>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    input: Placement,
    selected: u32,
    cursor: u32,
    position: [i32; 2],
    level: i32,
    slot: i32,
    notify: bool,
    cheat: bool,
    phase: Phase,
    check: Option<Check>,
    search: Option<Search>,
    pub(crate) physical: Option<lot_placement::live::Execution>,
    physical_owner: Option<u32>,
    pending: Option<Request>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rotation: Option<Box<super::rotation::Execution>>,
}
fn cursor(vm: &Vm, h: u32) -> Result<&sims_vm::cursor::State, String> {
    vm.world
        .objects
        .get(&id(vm, h)?)
        .and_then(|o| o.cursor.as_ref())
        .ok_or("cursor placement receiver absent".into())
}
fn cursor_mut(vm: &mut Vm, h: u32) -> Result<&mut sims_vm::cursor::State, String> {
    let id = id(vm, h)?;
    vm.world
        .objects
        .get_mut(&id)
        .and_then(|o| o.cursor.as_mut())
        .ok_or("cursor placement receiver absent".into())
}
fn words<'a>(vm: &Vm, p: &'a Program, h: u32) -> Result<&'a [i16], String> {
    let d = vm.world.objects[&id(vm, h)?]
        .definition
        .as_ref()
        .ok_or("cursor placement definition absent")?;
    sims_vm::construction::definition_words(p, d)
}
///421e42, including the resource's first master in original OBJD order.
pub(super) fn cost(vm: &Vm, p: &Program, h: u32) -> Result<i32, String> {
    let Some(target) = cursor(vm, h)?.underlying else {
        return Ok(0);
    };
    let w = words(vm, p, target)?;
    if w[10] == 0 {
        return Ok(i32::from(w[18]));
    }
    let d = vm.world.objects[&id(vm, target)?]
        .definition
        .as_ref()
        .unwrap();
    let m = &p.modules[&d.source];
    for n in &m.object_definition_order {
        let candidate = m
            .object_definition_words
            .get(n)
            .ok_or("cursor catalog order differs")?;
        if candidate.get(10) == Some(&w[10]) && candidate.get(11) == Some(&-1) {
            return candidate
                .get(18)
                .map(|&v| i32::from(v))
                .ok_or("cursor catalog price absent".into());
        }
    }
    Err("cursor price needs its loaded catalog master".into())
}
impl Execution {
    pub fn new(
        s: &Simulation,
        input: Placement,
        slot: i32,
        notify: bool,
        cheat: bool,
    ) -> Result<Self, String> {
        let selected = id(&s.vm, input.cursor)?;
        cursor(&s.vm, input.cursor)?;
        let part = s.vm.world.objects[&selected]
            .multitile
            .as_ref()
            .ok_or("cursor multipart storage absent")?;
        let mut position = input.position;
        let mut level = input.level;
        let head = if let Some(head) = part.head {
            position[0] = position[0].wrapping_sub(part.offset[1].wrapping_mul(16));
            position[1] = position[1].wrapping_sub(part.offset[0].wrapping_mul(16));
            level = level.wrapping_sub(part.offset[2]);
            handle(&s.vm, head)?
        } else {
            input.cursor
        };
        Ok(Self {
            input,
            selected: input.cursor,
            cursor: head,
            position,
            level,
            slot,
            notify,
            cheat,
            phase: Phase::Start,
            check: None,
            search: None,
            physical: None,
            physical_owner: None,
            pending: None,
            rotation: None,
        })
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub(super) fn has_options(&self, slot: i32, notify: bool, cheat: bool) -> bool {
        self.slot == slot && self.notify == notify && self.cheat == cheat
    }
    pub fn binding(&self) -> Placement {
        self.input
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.rotation
            .as_ref()
            .is_some_and(|r| r.owns_callback(vm, p, thread, depth))
            || self
                .check
                .as_ref()
                .is_some_and(|c| c.owns_callback(vm, p, thread, depth))
            || self
                .physical
                .as_ref()
                .is_some_and(|c| c.owns_callback(vm, p, thread, depth))
    }
    pub(crate) fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        if self.pending != Some(request) {
            return Err("cursor placement service reply differs".into());
        }
        self.phase = match request {
            Request::Rotate { .. } => Phase::Budget,
            Request::Appearance(_) => Phase::Cursor,
        };
        self.pending = None;
        Ok(())
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
            return Err("cursor placement/rotation nesting limit".into());
        }
        cursor(&s.vm, self.selected)?;
        cursor(&s.vm, self.cursor)?;
        if self.selected != self.input.cursor
            || (self.phase == Phase::Check) != (self.check.is_some() && self.search.is_none())
            || self.phase == Phase::Rotation
                && !matches!(self.pending, Some(Request::Rotate { .. }))
            || self.phase == Phase::Furniture && self.physical.is_none()
            || self.complete()
                && (self.pending.is_some() || self.check.is_some() || self.physical.is_some())
            || self.physical.is_some() != self.physical_owner.is_some()
        {
            return Err("invalid cursor placement continuation".into());
        }
        if let Some(r) = self.pending {
            match r {
                Request::Rotate { cursor, .. }
                    if self.phase == Phase::Rotation && cursor == self.cursor =>
                {
                    ()
                }
                Request::Appearance(h)
                    if self.phase == Phase::Appearance
                        && cursor(&s.vm, self.cursor)?.underlying == Some(h) =>
                {
                    ()
                }
                _ => return Err("unbound cursor placement service".into()),
            }
        }
        if let Some(r) = &self.rotation {
            let Some(Request::Rotate { cursor, notches }) = self.pending else {
                return Err("cursor slot rotation lost caller".into());
            };
            if r.binding() != (cursor, notches) {
                return Err("cursor slot rotation binding differs".into());
            }
            r.validate_depth(s, p, depth + 1)?;
        }
        if let Some(check) = &self.check {
            if !matches!(self.phase, Phase::Check | Phase::Search)
                || check.binding().position != self.position
                || check.binding().level != self.level
                || cursor(&s.vm, self.cursor)?.underlying != Some(check.binding().object)
            {
                return Err("cursor furniture check binding differs".into());
            }
            check.validate(&s.vm, p)?;
        }
        if let Some(e) = &self.physical {
            if !matches!(self.phase, Phase::Furniture | Phase::Cursor) {
                return Err("cursor physical child has no caller".into());
            }
            e.validate(&s.vm, p, id(&s.vm, self.physical_owner.unwrap())?)?;
            let owner = if self.phase == Phase::Cursor {
                Some(self.cursor)
            } else {
                cursor(&s.vm, self.cursor)?.underlying
            };
            if owner != self.physical_owner
                || e.operation
                    .binding()
                    .1
                    .is_none_or(|d| d.position != self.position || d.level != self.level)
            {
                return Err("cursor physical destination or receiver differs".into());
            }
        }
        if self.search.is_some() != (self.phase == Phase::Search) {
            return Err("cursor slot search owner differs".into());
        }
        Ok(())
    }
    fn represented_query(&self, s: &Simulation) -> Result<Option<Query>, String> {
        let c = cursor(&s.vm, self.cursor)?;
        Ok(c.underlying.map(|object| Query {
            object,
            position: self.position,
            level: self.level,
            container: (c.container > 0 && s.vm.world.objects.contains_key(&c.container))
                .then_some((c.container, i32::from(c.slot))),
        }))
    }
    fn begin_physical(
        &mut self,
        s: &Simulation,
        object: u32,
        container: Option<ObjectId>,
        slot: i32,
    ) -> Result<(), String> {
        let links = &s
            .native_lot
            .as_ref()
            .ok_or("cursor placement lot absent")?
            .placement
            .links;
        let d = Descriptor {
            on_lot: links.on_lot(self.position),
            position: self.position,
            container,
            slot,
            level: self.level,
        };
        //For cursor self-placement this is the explicit422c2f base call inside
        //421e72, not a substitute for the cursor virtual+58 override.
        self.physical = Some(lot_placement::live::Execution::new_virtual(
            lot_placement::VirtualOperation::set(
                &s.vm.world,
                links,
                id(&s.vm, object)?,
                d,
                self.notify,
            )?,
        ));
        self.physical_owner = Some(object);
        Ok(())
    }
    pub(crate) fn advance(
        &mut self,
        s: &mut Simulation,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        self.advance_depth(s, p, remaining, step, 0)
    }
    pub(crate) fn advance_depth(
        &mut self,
        s: &mut Simulation,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
        depth: usize,
    ) -> Result<bool, String> {
        if depth > 64 {
            return Err("cursor placement/rotation nesting limit".into());
        }
        while *remaining > 0 && !self.complete() {
            if let Some(Request::Rotate { cursor, notches }) = self.pending {
                if self.rotation.is_none() {
                    self.rotation = Some(Box::new(super::rotation::Execution::new(
                        s, cursor, notches, self.cheat,
                    )?));
                }
                if !self.rotation.as_mut().unwrap().advance_depth(
                    s,
                    p,
                    remaining,
                    step,
                    depth + 1,
                )? {
                    return Ok(false);
                }
                self.rotation = None;
                self.acknowledge(Request::Rotate { cursor, notches })?;
                continue;
            }
            if self.pending.is_some() {
                return Ok(false);
            }
            if let Some(e) = &mut self.physical {
                if !s.drive_removal_placement(p, e, remaining, step)? {
                    return Ok(false);
                }
                self.physical = None;
                self.physical_owner = None;
                self.phase = if self.phase == Phase::Furniture {
                    Phase::Appearance
                } else {
                    Phase::Publish
                };
                continue;
            }
            match self.phase {
                Phase::Start => {
                    let c = cursor_mut(&mut s.vm, self.cursor)?;
                    if c.position == self.position && c.level == self.level {
                        c.slot = self.slot as i16;
                    } else {
                        c.container = 0;
                        c.slot = 0;
                    }
                    c.valid = i16::from(c.underlying.is_none());
                    if let Some(q) = self.represented_query(s)? {
                        self.check = Some(Check::new(s, q, self.cheat)?);
                        self.phase = Phase::Check;
                    } else {
                        self.phase = Phase::Cursor;
                    }
                    *remaining -= 1;
                }
                Phase::Check => {
                    let c = self.check.as_mut().ok_or("represented check absent")?;
                    if !c.advance(s, p, remaining, step)? {
                        return Ok(false);
                    }
                    let accepted = c.outcome().unwrap();
                    self.check = None;
                    if accepted {
                        self.phase = Phase::Budget;
                    } else if s.native_lot.as_ref().unwrap().diagnostics.error == 11
                        && self
                            .represented_query(s)?
                            .map(|q| words(&s.vm, p, q.object).map(|w| w[10] == 0))
                            .transpose()?
                            .unwrap_or(false)
                    {
                        let q = self.represented_query(s)?.unwrap();
                        let mask = s.vm.world.objects[&id(&s.vm, q.object)?]
                            .data
                            .get(&4)
                            .copied()
                            .ok_or("cursor slot height mask absent")?
                            as u16;
                        let head = s.native_lot.as_ref().unwrap().placement.links.head(
                            &s.vm.world,
                            [
                                (self.position[1] >> 4) as i8,
                                (self.position[0] >> 4) as i8,
                                self.level as i8,
                            ],
                        );
                        self.search = Some(Search {
                            mask,
                            iterator: head.map(Cursor::new),
                            count: None,
                            slot: 0,
                            seen: BTreeSet::new(),
                        });
                        self.phase = Phase::Search;
                    } else {
                        self.phase = Phase::Cursor;
                    }
                }
                Phase::Search => {
                    self.search_slots(s, p, remaining, step)?;
                }
                Phase::Rotation => return Ok(false),
                Phase::Budget => {
                    let balance = if *s
                        .vm
                        .world
                        .globals
                        .get(&32)
                        .ok_or("cursor budget mode absent")?
                        == 1
                    {
                        999999
                    } else {
                        s.vm.world
                            .budget
                            .as_ref()
                            .ok_or("cursor budget storage absent")?
                            .balance
                    };
                    let price = cost(&s.vm, p, self.cursor)?;
                    let c = cursor_mut(&mut s.vm, self.cursor)?;
                    if balance < price && price != 0 && c.purchase != 0 {
                        s.native_lot.as_mut().unwrap().diagnostics.error = 21;
                    } else {
                        c.valid = 1;
                    }
                    let q = self
                        .represented_query(s)?
                        .ok_or("cursor furniture disappeared before SetObject")?;
                    self.begin_physical(
                        s,
                        q.object,
                        q.container.map(|v| v.0),
                        q.container.map_or(0, |v| v.1),
                    )?;
                    self.phase = Phase::Furniture;
                    *remaining -= 1;
                }
                Phase::Furniture => return Err("cursor furniture placement child absent".into()),
                Phase::Appearance => {
                    let h = cursor(&s.vm, self.cursor)?
                        .underlying
                        .ok_or("cursor furniture disappeared before appearance reset")?;
                    let object = id(&s.vm, h)?;
                    if s.ordinary_graphic_required(p, object)? {
                        self.pending = Some(Request::Appearance(h));
                    } else {
                        self.phase = Phase::Cursor;
                    }
                    *remaining -= 1;
                }
                Phase::Cursor => {
                    self.begin_physical(s, self.cursor, None, 0)?;
                    *remaining -= 1;
                }
                Phase::Publish => {
                    let mut current = Some(id(&s.vm, self.cursor)?);
                    let mut seen = BTreeSet::new();
                    while let Some(object) = current {
                        if !seen.insert(object) {
                            return Err("cyclic cursor position publication".into());
                        }
                        let valid = cursor(&s.vm, self.cursor)?.valid;
                        let pos =
                            s.vm.world
                                .positions
                                .get(&object)
                                .copied()
                                .ok_or("cursor physical point absent")?;
                        let o =
                            s.vm.world
                                .objects
                                .get_mut(&object)
                                .ok_or("cursor member absent")?;
                        let c = o
                            .cursor
                            .as_mut()
                            .ok_or("cursor publication found ordinary member")?;
                        c.position = [pos.x, pos.y];
                        c.level = pos.level;
                        c.valid = valid;
                        c.field_1c0 = (-1f32).to_bits();
                        current = o
                            .multitile
                            .as_ref()
                            .ok_or("cursor publication links absent")?
                            .next;
                    }
                    self.phase = Phase::Complete;
                    *remaining -= 1;
                }
                Phase::Complete => break,
            }
        }
        Ok(self.complete())
    }
    pub(crate) fn graphic(&self) -> Option<&lot_placement::live::Execution> {
        self.rotation
            .as_ref()
            .and_then(|r| r.graphic())
            .or(self.physical.as_ref())
            .filter(|e| matches!(e.pending, Some(lot_placement::Request::RefreshAnimator(_))))
    }
    pub(crate) fn graphic_mut(&mut self) -> Option<&mut lot_placement::live::Execution> {
        if self
            .rotation
            .as_ref()
            .is_some_and(|r| r.graphic().is_some())
        {
            return self.rotation.as_mut().unwrap().graphic_mut();
        }
        self.physical
            .as_mut()
            .filter(|e| matches!(e.pending, Some(lot_placement::Request::RefreshAnimator(_))))
    }
    pub(crate) fn appearance(&self) -> Option<u32> {
        self.rotation
            .as_ref()
            .and_then(|r| r.appearance())
            .or_else(|| match self.pending {
                Some(Request::Appearance(h)) => Some(h),
                _ => None,
            })
    }
    pub(crate) fn acknowledge_appearance(&mut self, h: u32) -> Result<(), String> {
        if self
            .rotation
            .as_ref()
            .is_some_and(|r| r.appearance() == Some(h))
        {
            return self.rotation.as_mut().unwrap().acknowledge_appearance(h);
        }
        self.acknowledge(Request::Appearance(h))
    }
    fn search_slots(
        &mut self,
        s: &mut Simulation,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<(), String> {
        if let Some(check) = &mut self.check {
            if !check.advance(s, p, remaining, step)? {
                return Ok(());
            }
            if check.outcome() == Some(true) {
                let (parent, slot) = check
                    .binding()
                    .container
                    .ok_or("cursor candidate container absent")?;
                let c = cursor_mut(&mut s.vm, self.cursor)?;
                c.container = parent;
                c.slot = slot as i16;
                let furniture = c.underlying.ok_or("cursor slot furniture absent")?;
                let parent_handle = handle(&s.vm, parent)?;
                let mut direction = i32::from(s.vm.world.objects[&parent].data[&1]);
                if words(&s.vm, p, parent_handle)?[10] != 0 {
                    let origin = s.vm.world.positions[&parent];
                    let steps = [
                        [-16, 0],
                        [-16, 16],
                        [0, 16],
                        [16, 16],
                        [16, 0],
                        [16, -16],
                        [0, -16],
                        [-16, -16],
                    ];
                    let mut found = false;
                    for _ in 0..4 {
                        let v = steps[(direction & 7) as usize];
                        if s.native_lot
                            .as_ref()
                            .unwrap()
                            .placement
                            .links
                            .first_blocker(
                                &s.vm.world,
                                [origin.x.wrapping_add(v[0]), origin.y.wrapping_add(v[1])],
                                self.level,
                            )?
                            .is_none()
                        {
                            found = true;
                            break;
                        }
                        direction = direction.wrapping_add(2);
                    }
                    if !found {
                        direction = i32::from(s.vm.world.objects[&id(&s.vm, furniture)?].data[&1]);
                    }
                }
                let o = &s.vm.world.objects[&id(&s.vm, furniture)?];
                let notches = direction
                    .wrapping_sub(i32::from(o.data[&1]))
                    .checked_div(i32::from(o.data[&23]))
                    .ok_or("native cursor slot orientation divides by zero")?;
                self.check = None;
                self.search = None;
                self.phase = Phase::Rotation;
                self.pending = Some(Request::Rotate {
                    cursor: self.cursor,
                    notches,
                });
                return Ok(());
            }
            self.check = None;
            self.search.as_mut().unwrap().slot += 1;
        }
        let search = self.search.as_mut().ok_or("cursor slot search absent")?;
        let current = search.iterator.as_ref().and_then(|c| c.current);
        let Some(parent) = current else {
            s.native_lot.as_mut().unwrap().diagnostics.error =
                if search.mask == 8 { 36 } else { 11 };
            self.search = None;
            self.phase = Phase::Cursor;
            return Ok(());
        };
        if search.count.is_none() {
            if !search.seen.insert(parent) {
                return Err("cyclic cursor slot candidate traversal".into());
            }
            search.count = Some(
                s.vm.world
                    .objects
                    .get(&parent)
                    .ok_or("cursor slot candidate vanished")?
                    .slots
                    .len(),
            );
        }
        if search.slot >= search.count.unwrap() {
            search
                .iterator
                .as_mut()
                .unwrap()
                .advance(&s.native_lot.as_ref().unwrap().placement.links, &s.vm.world)?;
            search.slot = 0;
            search.count = None;
            *remaining -= 1;
            return Ok(());
        }
        let slot = search.slot;
        let metadata = lot_acceptance::contained::slots(&s.vm.world, p, parent)?;
        let eligible = metadata.get(slot).is_some_and(|d| {
            search.mask & (1u32.wrapping_shl(d.height.wrapping_sub(1) as u32) as u16) != 0
        });
        if eligible {
            let object = cursor(&s.vm, self.cursor)?
                .underlying
                .ok_or("cursor slot furniture disappeared")?;
            self.check = Some(Check::new(
                s,
                Query {
                    object,
                    position: self.position,
                    level: self.level,
                    container: Some((parent, slot as i32)),
                },
                self.cheat,
            )?);
        } else {
            search.slot += 1;
        }
        *remaining -= 1;
        Ok(())
    }
}
