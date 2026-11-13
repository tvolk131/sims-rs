//!42191c forgets represented furniture;42186b destroys it before clearing the
//!pointers. Both end at4a67de(NULL). Neither frees the cursor itself.
use super::{Placement, handle, id, placement, rotation};
use crate::{Simulation, Step, lot_acceptance, lot_placement, lot_removal::immediate::Removal};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, ThreadId, Vm};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    Forget,
    Discard,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Gate,
    Detach,
    Delete,
    Head,
    Clear,
    Check,
    Place,
    Focus,
    Complete,
}
///Validation-only record of a pointer held across44123f. Behavioral traversal
///continues to read the live cursor links, including after deletion callbacks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Reference {
    cursor: u32,
    furniture: u32,
    object: ObjectId,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Execution {
    tick: u64,
    #[serde(default, skip_serializing_if = "depth_zero")]
    deletion_depth: usize,
    cursor: u32,
    kind: Kind,
    phase: Phase,
    point: Option<Placement>,
    current: Option<u32>,
    seen: BTreeSet<u32>,
    detach: Option<rotation::Execution>,
    removal: Option<Removal>,
    deleted: bool,
    references: Vec<Reference>,
    check: Option<lot_acceptance::live::Execution>,
    placement: Option<placement::Execution>,
}
fn depth_zero(n: &usize) -> bool {
    *n == 0
}
fn cursor(s: &Simulation, h: u32) -> Result<&sims_vm::cursor::State, String> {
    s.vm.world.objects[&id(&s.vm, h)?]
        .cursor
        .as_ref()
        .ok_or("cleanup cursor absent".into())
}
impl Execution {
    pub(crate) fn call(s: &Simulation, h: u32, kind: Kind, depth: usize) -> Self {
        Self {
            tick: s.vm.tick,
            deletion_depth: depth,
            cursor: h,
            kind,
            phase: Phase::Gate,
            point: None,
            current: None,
            seen: BTreeSet::new(),
            detach: None,
            removal: None,
            deleted: false,
            references: vec![],
            check: None,
            placement: None,
        }
    }
    pub(crate) fn deletion_depth(&self) -> usize {
        self.deletion_depth
    }
    pub(crate) fn may_be_onlot(&self) -> bool {
        self.point.is_some() && matches!(self.phase, Phase::Place | Phase::Focus | Phase::Complete)
    }
    pub(crate) fn acknowledge_focus(&mut self) -> Result<(), String> {
        if !self.pending_focus_clear() {
            return Err("cursor focus clear reply out of order".into());
        }
        self.phase = Phase::Complete;
        Ok(())
    }

    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub fn pending_focus_clear(&self) -> bool {
        self.phase == Phase::Focus
    }
    pub fn cursor(&self) -> u32 {
        self.cursor
    }
    pub fn kind(&self) -> Kind {
        self.kind
    }
    pub(crate) fn owns_deletion(&self, object: ObjectId) -> bool {
        self.phase == Phase::Delete
            && self
                .removal
                .as_ref()
                .is_some_and(|e| e.owns_deletion(object))
    }
    pub(crate) fn owns_callback(&self, vm: &Vm, p: &Program, t: ThreadId, d: usize) -> bool {
        self.detach
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, t, d))
            || self
                .removal
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, t, d))
            || self
                .check
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
            .or_else(|| self.placement.as_ref().and_then(|e| e.graphic()))
            .or_else(|| {
                self.removal
                    .as_ref()
                    .filter(|e| !e.deleting)
                    .and_then(|e| e.placement.graphic())
            })
            .filter(|e| matches!(e.pending, Some(lot_placement::Request::RefreshAnimator(_))))
    }
    pub(crate) fn graphic_mut(&mut self) -> Option<&mut lot_placement::live::Execution> {
        if self.detach.as_ref().is_some_and(|e| e.graphic().is_some()) {
            return self.detach.as_mut().unwrap().graphic_mut();
        }
        if self
            .placement
            .as_ref()
            .is_some_and(|e| e.graphic().is_some())
        {
            return self.placement.as_mut().unwrap().graphic_mut();
        }
        self.removal
            .as_mut()
            .filter(|e| !e.deleting)
            .and_then(|e| e.placement.graphic_mut())
            .filter(|e| matches!(e.pending, Some(lot_placement::Request::RefreshAnimator(_))))
    }
    pub(crate) fn appearance(&self) -> Option<u32> {
        self.detach
            .as_ref()
            .and_then(|e| e.appearance())
            .or_else(|| self.placement.as_ref().and_then(|e| e.appearance()))
    }
    pub(crate) fn acknowledge_appearance(&mut self, h: u32) -> Result<(), String> {
        if self
            .detach
            .as_ref()
            .is_some_and(|e| e.appearance() == Some(h))
        {
            return self.detach.as_mut().unwrap().acknowledge_appearance(h);
        }
        self.placement
            .as_mut()
            .ok_or("cleanup appearance owner absent")?
            .acknowledge_appearance(h)
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        if self.deletion_depth != 0
            || s.cursor_factory.is_some()
            || s.cursor_rotation.is_some()
            || s.cursor_release.is_some()
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
            return Err("cursor cleanup overlaps synchronous work".into());
        }
        self.validate_call(s, p)
    }
    pub(crate) fn validate_call(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        if self.tick != s.vm.tick
            || !cursor(s, self.cursor)?.postconstructed
            || s.native_lot.is_none()
            || self.deletion_depth > s.deletion.len()
            || self.detach.is_some() && self.phase != Phase::Detach
            || self.check.is_some() && self.phase != Phase::Check
            || self.placement.is_some() && self.phase != Phase::Place
            || self.kind == Kind::Forget
                && (self.point.is_some()
                    || self.removal.is_some()
                    || !self.references.is_empty()
                    || !matches!(
                        self.phase,
                        Phase::Gate | Phase::Head | Phase::Clear | Phase::Focus | Phase::Complete
                    ))
            || self.phase == Phase::Clear && self.current.is_none()
            || self.deleted && self.removal.is_none()
            || self.removal.is_some()
                && !matches!(
                    self.phase,
                    Phase::Delete
                        | Phase::Head
                        | Phase::Clear
                        | Phase::Check
                        | Phase::Place
                        | Phase::Focus
                        | Phase::Complete
                )
            || self.removal.is_some() && self.deleted == (self.phase == Phase::Delete)
            || self.removal.is_none() && !self.references.is_empty()
            || matches!(
                self.phase,
                Phase::Detach | Phase::Delete | Phase::Check | Phase::Place
            ) && self.point.is_none()
        {
            return Err("invalid cursor cleanup owner".into());
        }
        if let Some(q) = self.point {
            if q.cursor != self.cursor || self.kind != Kind::Discard {
                return Err("cleanup point binding differs".into());
            }
        }
        if self.phase == Phase::Gate && self.point.is_some()
            || self.kind == Kind::Discard
                && matches!(
                    self.phase,
                    Phase::Head | Phase::Clear | Phase::Check | Phase::Place
                )
                && (self.point.is_none() || !self.deleted)
            || matches!(self.phase, Phase::Focus | Phase::Complete)
                && self.point.is_some() != self.deleted
        {
            return Err("cleanup continuation skipped its destructive caller".into());
        }
        if let Some(h) = self.current {
            cursor(s, h)?;
        }
        if let Some(e) = &self.detach {
            if !e.is_removal(self.cursor) {
                return Err("cleanup detach binding differs".into());
            }
            e.validate(s, p)?;
        }
        if let Some(e) = &self.removal {
            if e.depth() != self.deletion_depth {
                return Err("cleanup removal stack depth differs".into());
            }
            e.validate(s, p, self.deleted)?;
        }
        if let Some(e) = &self.check {
            let q = self.point.ok_or("cleanup check point absent")?;
            if e.binding() != (id(&s.vm, self.cursor)?, q.position, q.level)
                || e.container_binding().is_some()
            {
                return Err("cleanup check binding differs".into());
            }
            e.validate(&s.vm, p)?;
        }
        if let Some(e) = &self.placement {
            if Some(e.binding()) != self.point {
                return Err("cleanup placement binding differs".into());
            }
            e.validate(s, p)?;
        }
        let registry =
            s.vm.world
                .scheduler_storage
                .as_ref()
                .ok_or("cleanup registry absent")?;
        let mut unique = BTreeSet::new();
        let chain = sims_vm::multitile::chain(&s.vm.world, id(&s.vm, self.cursor)?)?;
        for r in &self.references {
            cursor(s, r.cursor)?;
            if !unique.insert(r.cursor)
                || !chain.contains(&id(&s.vm, r.cursor)?)
                || r.object <= 0
                || registry
                    .object_for_handle(r.furniture)
                    .is_some_and(|o| o != r.object)
                || registry.object_for_handle(r.furniture).is_none()
                    && !registry.retired_handle(r.furniture)
            {
                return Err("cleanup retained lifetime differs".into());
            }
        }
        if self.deleted
            && matches!(
                self.phase,
                Phase::Check | Phase::Place | Phase::Focus | Phase::Complete
            )
            && chain.iter().any(|id| {
                s.vm.world.objects[id]
                    .cursor
                    .as_ref()
                    .is_none_or(|c| c.underlying.is_some())
            })
        {
            return Err("cleanup advanced before clearing its furniture pointers".into());
        }
        if let Some(e) = &self.removal {
            if !self
                .references
                .iter()
                .any(|r| r.cursor == self.cursor && r.object == e.target)
            {
                return Err("cleanup deletion lost selected furniture binding".into());
            }
        }
        Ok(())
    }
    /// Only this validated synchronous caller may own dangling cursor pointers.
    /// A standalone VM restore, or a simulation without this owner, rejects them.
    pub(crate) fn retired_references(
        &self,
        s: &Simulation,
        p: &Program,
    ) -> Result<Vec<(ObjectId, u32)>, String> {
        self.validate(s, p)?;
        self.retired_references_call(s, p)
    }
    pub(crate) fn retired_references_call(
        &self,
        s: &Simulation,
        p: &Program,
    ) -> Result<Vec<(ObjectId, u32)>, String> {
        self.validate_call(s, p)?;
        let mut result = vec![];
        if self.removal.as_ref().is_some_and(|e| e.deleting)
            && matches!(self.phase, Phase::Delete | Phase::Head | Phase::Clear)
        {
            for r in &self.references {
                if cursor(s, r.cursor)?.underlying == Some(r.furniture)
                    && s.vm
                        .world
                        .scheduler_storage
                        .as_ref()
                        .unwrap()
                        .retired_handle(r.furniture)
                {
                    result.push((id(&s.vm, r.cursor)?, r.furniture));
                }
            }
        }
        Ok(result)
    }
    pub(crate) fn advance(
        &mut self,
        s: &mut Simulation,
        p: &Program,
        left: &mut usize,
        step: &mut Step,
    ) -> Result<(), String> {
        while *left > 0 && !matches!(self.phase, Phase::Focus | Phase::Complete) {
            if let Some(e) = &mut self.detach {
                if !e.advance(s, p, left, step)? {
                    return Ok(());
                }
                self.detach = None;
                self.phase = Phase::Delete;
                continue;
            }
            if self.phase == Phase::Delete {
                if self.removal.is_none() {
                    // The source reads the selected underlying pointer AFTER+50.
                    let target = id(
                        &s.vm,
                        cursor(s, self.cursor)?
                            .underlying
                            .ok_or("cleanup lost furniture after detach")?,
                    )?;
                    let members = if s.vm.world.objects[&target].multitile.is_some() {
                        sims_vm::multitile::chain(&s.vm.world, target)?
                    } else {
                        vec![target]
                    };
                    for c in sims_vm::multitile::chain(&s.vm.world, id(&s.vm, self.cursor)?)? {
                        if let Some(h) = s.vm.world.objects[&c]
                            .cursor
                            .as_ref()
                            .ok_or("cleanup chain class differs")?
                            .underlying
                        {
                            let object = id(&s.vm, h)?;
                            if members.contains(&object) {
                                self.references.push(Reference {
                                    cursor: handle(&s.vm, c)?,
                                    furniture: h,
                                    object,
                                });
                            }
                        }
                    }
                    self.removal = Some(Removal::at_depth(s, p, target, self.deletion_depth)?);
                }
                if !s.drive_immediate_removal(p, self.removal.as_mut().unwrap(), left, step)? {
                    return Ok(());
                }
                self.deleted = true;
                self.phase = Phase::Head;
                // Preserve a diagnostic/save boundary before clearing dangling pointers.
                *left = left.saturating_sub(1);
                continue;
            }
            if let Some(e) = &mut self.check {
                if !s.drive_native_acceptance(p, e, left, step)? {
                    return Ok(());
                }
                let accepted = e.outcome().unwrap().accepted;
                self.check = None;
                self.phase = if accepted { Phase::Place } else { Phase::Focus };
                continue;
            }
            if let Some(e) = &mut self.placement {
                if !e.advance(s, p, left, step)? {
                    return Ok(());
                }
                self.placement = None;
                self.phase = Phase::Focus;
                continue;
            }
            match self.phase {
                Phase::Gate => {
                    let c = cursor(s, self.cursor)?;
                    self.phase = if c.underlying.is_none() {
                        Phase::Focus
                    } else if self.kind == Kind::Forget {
                        Phase::Head
                    } else {
                        self.point = Some(Placement {
                            cursor: self.cursor,
                            position: c.position,
                            level: c.level,
                        });
                        Phase::Detach
                    };
                }
                Phase::Detach => self.detach = Some(rotation::Execution::remove(s, self.cursor)?),
                Phase::Head => {
                    let object = id(&s.vm, self.cursor)?;
                    let head = s.vm.world.objects[&object]
                        .multitile
                        .as_ref()
                        .ok_or("cleanup head absent")?
                        .head
                        .unwrap_or(object);
                    //635abe casts the head once; a failed RTTI cast skips the loop.
                    self.current = s.vm.world.objects[&head]
                        .cursor
                        .as_ref()
                        .map(|_| handle(&s.vm, head))
                        .transpose()?;
                    self.phase = if self.current.is_some() {
                        Phase::Clear
                    } else if self.kind == Kind::Discard {
                        Phase::Check
                    } else {
                        Phase::Focus
                    };
                }
                Phase::Clear => {
                    let h = self.current.unwrap();
                    if !self.seen.insert(h) {
                        return Err("cyclic cursor cleanup".into());
                    }
                    let object = id(&s.vm, h)?;
                    s.vm.world
                        .objects
                        .get_mut(&object)
                        .unwrap()
                        .cursor
                        .as_mut()
                        .ok_or("cleanup part class differs")?
                        .underlying = None;
                    // Read next after the write, as in42191c/42186b.
                    self.current = s.vm.world.objects[&object]
                        .multitile
                        .as_ref()
                        .ok_or("cleanup next absent")?
                        .next
                        .map(|o| handle(&s.vm, o))
                        .transpose()?;
                    self.phase = if self.current.is_some() {
                        Phase::Clear
                    } else if self.kind == Kind::Discard {
                        Phase::Check
                    } else {
                        Phase::Focus
                    };
                }
                Phase::Check => {
                    let q = self.point.unwrap();
                    self.check = Some(lot_acceptance::live::Execution::new(
                        s,
                        id(&s.vm, self.cursor)?,
                        q.position,
                        q.level,
                    )?);
                }
                Phase::Place => {
                    self.placement = Some(placement::Execution::new(
                        s,
                        self.point.unwrap(),
                        0,
                        true,
                        false,
                    )?)
                }
                Phase::Delete | Phase::Focus | Phase::Complete => unreachable!(),
            }
            *left -= 1;
        }
        Ok(())
    }
}
impl Simulation {
    pub fn begin_cursor_cleanup(
        &mut self,
        p: &Program,
        object: ObjectId,
        kind: Kind,
    ) -> Result<(), String> {
        if self.cursor_work_pending()
            || !self.deletion.is_empty()
            || self.cursor_has_active_vm_callback()
            || self.person_factories.values().any(|f| !f.finalized)
        {
            return Err("cursor cleanup overlaps synchronous work".into());
        }
        self.validate_native_lot()?;
        let e = Execution::call(self, handle(&self.vm, object)?, kind, 0);
        e.validate(self, p)?;
        self.cursor_cleanup = Some(e);
        Ok(())
    }
    pub fn advance_cursor_cleanup(&mut self, p: &Program, budget: usize) -> Result<Step, String> {
        if budget == 0 {
            return Err("cursor cleanup requires a positive budget".into());
        }
        self.cursor_cleanup
            .as_ref()
            .ok_or("cursor cleanup absent")?
            .validate(self, p)?;
        let mut e = self.cursor_cleanup.take().unwrap();
        let mut step = Step {
            stop: crate::StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        let mut left = budget;
        let result = e.advance(self, p, &mut left, &mut step);
        self.cursor_cleanup = Some(e);
        self.publish_live_placement_caches()?;
        self.validate_geometry()?;
        self.validate_native_lot()?;
        result?;
        self.cursor_cleanup.as_ref().unwrap().validate(self, p)?;
        Ok(step)
    }
    pub fn complete_cursor_cleanup_focus(&mut self) -> Result<(), String> {
        if let Some(e) = self
            .cursor_cleanup
            .as_mut()
            .filter(|e| e.pending_focus_clear())
        {
            return e.acknowledge_focus();
        }
        let e = self
            .deletion
            .iter_mut()
            .rev()
            .find_map(|d| d.cursor.as_mut().filter(|e| e.pending_focus_clear()))
            .ok_or("cursor focus clear owner absent")?;
        e.acknowledge_focus()
    }

    pub fn finish_cursor_cleanup(&mut self, p: &Program) -> Result<(), String> {
        let e = self
            .cursor_cleanup
            .as_ref()
            .ok_or("cursor cleanup absent")?;
        e.validate(self, p)?;
        if !e.complete() {
            return Err("cursor cleanup has not returned".into());
        }
        self.cursor_cleanup = None;
        Ok(())
    }
}
