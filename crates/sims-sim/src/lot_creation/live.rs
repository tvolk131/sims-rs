//! Live 0x44e0c8 floor/contained creation: factory, acceptance, SetObject, or synchronous
//! destruction. This continuation freezes ordinary scheduling, not Init/Main
//! or placement callbacks. Original resources determine the created behavior.
use crate::{Simulation, Step, lot_acceptance, lot_links::Descriptor, lot_placement, removal};
use serde::{Deserialize, Serialize};
use sims_vm::{
    Program, Status, ThreadId, Vm,
    create::{Placement, Request},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Factory,
    GroupFactory,
    GroupRotate,
    RotateOffLot,
    RotatePlaced,
    BeginPlacement,
    Check,
    Place,
    Dynamic,
    Detach,
    Destroy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    thread: ThreadId,
    request: Request,
    phase: Phase,
    acceptance: Option<lot_acceptance::live::Execution>,
    placement: Option<lot_placement::live::Execution>,
    after_delete_room: Option<i16>,
    group: Option<sims_vm::group_factory::Factory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dynamic: Option<crate::lot_dynamic::live::Execution>,
}
impl Execution {
    pub(crate) fn binding(&self) -> (u64, sims_vm::ObjectId) {
        (self.request.id, self.request.created.unwrap_or(0))
    }
    pub(crate) fn parts(&self) -> &[sims_vm::ObjectId] {
        self.group.as_ref().map_or(&[], |g| &g.parts)
    }
    pub(crate) fn retired_creation(
        &self,
        s: &Simulation,
        p: &Program,
    ) -> Result<Option<(ThreadId, u64, sims_vm::ObjectId)>, String> {
        let Some(id) = self.request.created else {
            return Ok(None);
        };
        if s.vm.world.objects.contains_key(&id) {
            return Ok(None);
        }
        self.validate(s, p)?;
        let group = self
            .group
            .as_ref()
            .ok_or("retired creation has no multipart factory")?;
        let deletion = s
            .deletion
            .first()
            .and_then(|d| d.group.as_ref())
            .ok_or("retired creation has no group destructor")?;
        if self.phase != Phase::Destroy
            || deletion.marked != group.parts
            || deletion.active == Some(id)
                && !s
                    .deletion
                    .iter()
                    .any(|d| d.target == id && d.storage_unlinked)
            || deletion.next == Some(id)
        {
            return Err("retired creation group ownership mismatch".into());
        }
        Ok(Some((self.thread, self.request.id, id)))
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.acceptance
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self
                .placement
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self
                .dynamic
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        let r = &self.request;
        let id = r.created.ok_or("native creation instance absent")?;
        let t =
            s.vm.threads
                .get(&self.thread)
                .ok_or("native creator thread absent")?;
        let bound = std::iter::once(&t.status)
            .chain(
                t.callback
                    .iter()
                    .flat_map(|b| b.chain())
                    .map(|b| &b.resume_status),
            )
            .chain(
                t.service
                    .iter()
                    .flat_map(|b| b.chain())
                    .map(|b| &b.resume_status),
            )
            .any(|status| matches!(status, Status::AwaitingCreation(q) if q==r));
        if matches!(self.phase, Phase::GroupFactory | Phase::GroupRotate) {
            let group = self.group.as_ref().ok_or("multipart factory absent")?;
            group.validate(&s.vm, p)?;
            let w = &p.modules[&group.source].object_definition_words[&group.catalog];
            if s.native_lot.is_none()
                || !bound
                || id != group.head()
                || if r.operands[4] == 6 {
                    ![0, 1, 32].contains(&r.operands[5]) || r.placement.is_some()
                } else {
                    self.phase == Phase::GroupRotate
                        || !matches!(r.operands[4], 0 | 2 | 3 | 4 | 5 | 7 | 8 | 9)
                        || r.operands[5] & !0x3b != 0
                        || r.placement.is_none()
                }
                || self.acceptance.is_some()
                || self.phase == Phase::GroupFactory && self.placement.is_some()
                || self.after_delete_room.is_some()
                || self.dynamic.is_some()
                || self.phase == Phase::GroupRotate && !group.complete()
                || (w[14] as u16 as u32 | ((w[15] as u16 as u32) << 16))
                    != u32::from_le_bytes(r.operands[..4].try_into().unwrap())
                || group.parts.iter().any(|id| {
                    !s.unplaced.contains(id) || s.removals.iter().any(|r| r.target == *id)
                })
            {
                return Err("invalid native multipart creation continuation".into());
            }
            if let Some(rotation) = &self.placement {
                rotation.validate_rotation(&s.vm, p, id)?;
            }
            return Ok(());
        }
        if let Some(group) = &self.group {
            if !group.complete() || group.head() != id || r.operands[4] == 6 {
                return Err("invalid completed placed group factory".into());
            }
            let module = p
                .modules
                .get(&group.source)
                .ok_or("placed group source absent")?;
            let plan = sims_vm::multitile::construction_plan(module, group.catalog)?;
            let words = &module.object_definition_words[&group.catalog];
            if plan.len() != group.parts.len()
                || (words[14] as u16 as u32 | ((words[15] as u16 as u32) << 16))
                    != u32::from_le_bytes(r.operands[..4].try_into().unwrap())
                || group.parts.iter().zip(plan).any(|(id, number)| {
                    s.vm.world.objects.get(id).is_some_and(|o| {
                        o.definition
                            .as_ref()
                            .is_none_or(|d| d.source != group.source || d.object != number)
                    })
                })
            {
                return Err("placed group factory resource mismatch".into());
            }
            // After dynamic joins the original factory parts need not form the
            // whole group. During destruction they can already be unregistered.
            if !matches!(self.phase, Phase::Dynamic | Phase::Destroy) {
                group.validate(&s.vm, p)?;
            }
        }
        if self.phase == Phase::RotateOffLot {
            let object =
                s.vm.world
                    .objects
                    .get(&id)
                    .ok_or("rotating creation absent")?;
            let definition = object
                .definition
                .as_ref()
                .ok_or("rotating definition absent")?;
            let words = p
                .modules
                .get(&definition.source)
                .and_then(|m| m.object_definition_words.get(&definition.object))
                .ok_or("rotating OBJD absent")?;
            if s.native_lot.is_none()
                || !bound
                || r.operands[4] != 6
                || r.operands[5] != 32
                || r.placement.is_some()
                || self.acceptance.is_some()
                || self.after_delete_room.is_some()
                || !s.unplaced.contains(&id)
                || object.person.is_some()
                || object.multitile.is_some()
                || self.group.is_some()
                || self.dynamic.is_some()
                || words.get(10) != Some(&0)
                || definition.guid != u32::from_le_bytes(r.operands[..4].try_into().unwrap())
                || s.removals.iter().any(|q| q.target == id)
            {
                return Err("invalid off-lot rotation factory".into());
            }
            return self
                .placement
                .as_ref()
                .ok_or("creation rotation absent")?
                .validate_rotation(&s.vm, p, id);
        }
        let at = r
            .placement
            .as_ref()
            .ok_or("native creation destination absent")?;
        if s.native_lot.is_none()
            || !bound
            || !matches!(r.operands[4], 0 | 2 | 3 | 4 | 5 | 7 | 8 | 9)
            || r.operands[5] & !0x3b != 0
            || self.phase == Phase::RotatePlaced && r.operands[5] & 0x20 == 0
            || match r.operands[4] {
                0 | 2 | 5 | 9 => at.target != r.caller,
                // The frame selector is captured before callbacks, not reread
                // on restoration. The request and controller must agree above.
                7 | 8 => at.target <= 0,
                _ => at.target != r.target,
            }
            || matches!(r.operands[4], 0 | 2 | 5 | 7 | 8 | 9) && at.room != -5
            || r.operands[4] != 9 && at.wall_direction.is_some()
            || !s.vm.world.objects.contains_key(&id) && self.phase != Phase::Destroy
            // Floor placement uses the destination captured before Init; the
            // original wrapper does not reread its selected object afterward.
            // Contained placement additionally requires a surviving parent.
            || matches!(r.operands[4], 2 | 4)
                && !s.vm.world.objects.contains_key(&at.target)
            || s.removals.iter().any(|q| q.target == id)
        {
            return Err("unbound native placed factory".into());
        }
        if self.phase == Phase::Destroy {
            if self.deletion_binding(s, id).is_err()
                || self.dynamic.is_some()
                || !self
                    .acceptance
                    .as_ref()
                    .and_then(|e| e.outcome())
                    .is_some_and(|o| !o.accepted)
                || !self
                    .placement
                    .as_ref()
                    .is_some_and(|e| e.complete && e.operation.binding() == (id, None))
            {
                return Err("native creation cleanup continuation missing".into());
            }
            return Ok(());
        }
        let object = &s.vm.world.objects[&id];
        let definition = object
            .definition
            .as_ref()
            .ok_or("native factory definition absent")?;
        let words = p
            .modules
            .get(&definition.source)
            .and_then(|m| m.object_definition_words.get(&definition.object))
            .filter(|w| w.len() > 15)
            .ok_or("native factory OBJD absent")?;
        let guid = self.group.as_ref().map_or(definition.guid, |g| {
            let w = &p.modules[&g.source].object_definition_words[&g.catalog];
            w[14] as u16 as u32 | ((w[15] as u16 as u32) << 16)
        });
        if guid != u32::from_le_bytes(r.operands[..4].try_into().unwrap())
            || object.person.is_some()
            || (words[10] != 0) != self.group.is_some()
            || matches!(
                self.phase,
                Phase::Factory | Phase::RotatePlaced | Phase::BeginPlacement | Phase::Check
            ) && !s.unplaced.contains(&id)
        {
            return Err("native contained factory resource or physical state mismatch".into());
        }
        let has_check = !matches!(
            self.phase,
            Phase::Factory | Phase::RotatePlaced | Phase::BeginPlacement
        );
        if self.acceptance.is_some() != has_check
            || self.placement.is_some()
                != matches!(
                    self.phase,
                    Phase::RotatePlaced
                        | Phase::Place
                        | Phase::Dynamic
                        | Phase::Detach
                        | Phase::Destroy
                )
            || self.dynamic.is_some() != (self.phase == Phase::Dynamic)
        {
            return Err("native creation phase mismatch".into());
        }
        if let Some(e) = &self.acceptance {
            e.validate(&s.vm, p)?;
            if e.binding().0 != id
                || e.binding().2 != at.level
                || e.container_binding() != destination(r).container.map(|id| (id, 0))
                || !matches!(r.operands[4], 2 | 4) && e.binding().1 != destination(r).position
                || self.phase != Phase::Check && e.outcome().is_none()
                || matches!(self.phase, Phase::Place | Phase::Dynamic)
                    && !e.outcome().is_some_and(|o| o.accepted)
                || matches!(self.phase, Phase::Detach | Phase::Destroy)
                    && !e.outcome().is_some_and(|o| !o.accepted)
            {
                return Err("native creation acceptance mismatch".into());
            }
        }
        if let Some(e) = &self.placement {
            if self.phase == Phase::RotatePlaced {
                e.validate_rotation(&s.vm, p, id)?;
            } else {
                e.validate(&s.vm, p, id)?;
            }
            let expected = if matches!(self.phase, Phase::Place | Phase::Dynamic) {
                Some(destination(r))
            } else {
                None
            };
            if e.operation.binding() != (id, expected)
                || self.phase == Phase::Destroy && !e.complete
            {
                return Err("native creation placement mismatch".into());
            }
        }
        if let Some(e) = &self.dynamic {
            e.validate(&s.vm, p)?;
            if e.binding() != id || !self.placement.as_ref().is_some_and(|e| e.complete) {
                return Err("dynamic creation update binding differs".into());
            }
        }
        if self.phase == Phase::Destroy && s.deletion.first().map(|d| d.target) != Some(id) {
            return Err("native creation cleanup continuation missing".into());
        }
        Ok(())
    }
    fn deletion_binding(&self, s: &Simulation, id: sims_vm::ObjectId) -> Result<(), String> {
        if s.deletion.first().map(|d| d.target) != Some(id) {
            return Err("native creation deletion owner absent".into());
        }
        Ok(())
    }
}
/// Native mode 2 uses the caller and leaves the room filter at -5; mode 4
/// uses Stack Object and captures its room. Both select slot zero before Init.
fn contained_destination(world: &sims_vm::World, r: &Request) -> Result<Option<Placement>, String> {
    let target = match r.operands[4] {
        2 => r.caller,
        4 => r.target,
        _ => return Err("not a native contained creation mode".into()),
    };
    let Some(object) = world.objects.get(&target) else {
        return Ok(None);
    };
    let q = world
        .positions
        .get(&target)
        .ok_or("native creation parent position absent")?;
    Ok(Some(Placement {
        target,
        position: [q.y, q.x],
        level: q.level,
        room: if r.operands[4] == 2 {
            -5
        } else {
            *object.data.get(&29).ok_or("creation parent room absent")?
        },
        wall_direction: None,
    }))
}

/// 0/5 use the caller; 7/8 select an argument/local. Only mode 3 keeps the
/// selected object's room. Everything is captured before the factory runs.
fn placed_destination(vm: &Vm, thread: ThreadId, r: &Request) -> Result<Option<Placement>, String> {
    let world = &vm.world;
    if matches!(r.operands[4], 2 | 4) {
        return contained_destination(world, r);
    }
    if !matches!(r.operands[4], 0 | 3 | 5 | 7 | 8 | 9) {
        return Err("unsupported native placed creation mode".into());
    }
    let target = vm.creation_placement_target(thread, r)?;
    let Some(object) = world.objects.get(&target) else {
        return Ok(None);
    };
    let q = world
        .positions
        .get(&target)
        .ok_or("creation target position absent")?;
    let mut wall_direction = None;
    let position = if r.operands[4] == 9 {
        let f = vm
            .threads
            .get(&thread)
            .and_then(|t| t.stack.last())
            .ok_or("relative creation frame absent")?;
        let index = r.operands[6] as i8;
        let rotation = *usize::try_from(index)
            .ok()
            .and_then(|i| f.locals.get(i))
            .ok_or("creation rotation local invalid (native error 0x08)")?;
        if f.opaque_locals.contains(&(index as usize)) {
            return Err("creation reads opaque local".into());
        }
        let (position, direction) = sims_vm::create::relative_floor(
            [q.x, q.y],
            *object
                .data
                .get(&1)
                .ok_or("relative creation direction absent")?,
            rotation,
        );
        // Native -1 suppresses the wall query. Other signed remainders are
        // masked by the existing original wall-direction routine.
        wall_direction = (direction != u8::MAX).then_some(direction);
        position
    } else if matches!(r.operands[4], 5 | 7 | 8) {
        [q.x, q.y]
    } else {
        sims_vm::create::in_front(
            [q.x, q.y],
            *object
                .data
                .get(&1)
                .ok_or("creation target direction absent")? as u8,
        )
    };
    Ok(Some(Placement {
        target,
        position: [position[1], position[0]],
        level: q.level,
        room: if r.operands[4] == 3 {
            *object.data.get(&29).ok_or("creation target room absent")?
        } else {
            -5
        },
        wall_direction,
    }))
}

fn destination(r: &Request) -> Descriptor {
    let at = r.placement.as_ref().unwrap();
    let contained = matches!(r.operands[4], 2 | 4);
    Descriptor {
        on_lot: true,
        position: [at.position[1], at.position[0]],
        container: contained.then_some(at.target),
        slot: if contained { 0 } else { -1 },
        level: at.level,
    }
}

fn center_dynamic_destination(at: &mut Placement) {
    at.position = at.position.map(|coordinate| (coordinate & !7) | 8);
}

pub(crate) fn copied_rotation(
    vm: &Vm,
    thread: ThreadId,
    created: sims_vm::ObjectId,
) -> Result<Option<lot_placement::live::Execution>, String> {
    let target = vm
        .threads
        .get(&thread)
        .and_then(|t| t.stack.last())
        .ok_or("rotation creator frame absent")?
        .stack_object;
    let Some(source) = vm.world.objects.get(&target) else {
        return Ok(None);
    };
    let desired = *source
        .data
        .get(&1)
        .ok_or("creation source direction absent")?
        & 6;
    Ok(Some(lot_placement::live::Execution::new_virtual(
        lot_placement::VirtualOperation::copy_direction(&vm.world, created, desired)?,
    )))
}

pub(crate) fn duplicate_at(
    s: &Simulation,
    tile: [i8; 3],
    source: &str,
    number: u16,
) -> Result<bool, String> {
    let links = &s
        .native_lot
        .as_ref()
        .ok_or("creation native host absent")?
        .placement
        .links;
    if let Some(head) = links.head(&s.vm.world, tile) {
        for id in links.walk(&s.vm.world, head)? {
            // 44e0c8 compares resource pointers, including contained children.
            if s.vm.world.objects[&id]
                .definition
                .as_ref()
                .is_some_and(|d| d.source == source && d.object == number)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn preflight(
    s: &Simulation,
    r: &Request,
    at: &Placement,
    source: &str,
    number: u16,
) -> Result<bool, String> {
    let [x, y] = [at.position[1], at.position[0]];
    let links = &s
        .native_lot
        .as_ref()
        .ok_or("creation native host absent")?
        .placement
        .links;
    let head = links.head(
        &s.vm.world,
        [(y >> 4) as i8, (x >> 4) as i8, at.level as i8],
    );
    if r.operands[5] & 1 != 0 {
        if duplicate_at(
            s,
            [(y >> 4) as i8, (x >> 4) as i8, at.level as i8],
            source,
            number,
        )? {
            return Ok(false);
        }
    }
    Ok(!(r.operands[5] & 8 != 0 && head.is_some()
        || !links.on_lot([x, y])
        || at.room != -5
            && s.architecture
                .as_ref()
                .ok_or("creation architecture absent")?
                .primary_room_at(at.level, crate::Point::new(y, x))
                != at.room
        || at.wall_direction.is_some_and(|direction| {
            s.architecture
                .as_ref()
                .is_some_and(|a| a.wall_mask(at.level, crate::Point::new(y, x), direction) & 1 != 0)
        })))
}

impl Simulation {
    /// The factory's Created signal has already been emitted. Native 0x44e64d
    /// now reads the current frame's Stack Object, rather than the request's
    /// earlier placement target. The remaining rotation may invoke callbacks.
    pub(crate) fn finish_native_rotated_creation(
        &mut self,
        p: &Program,
        thread: ThreadId,
        r: Request,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let id = r.created.ok_or("rotation factory instance absent")?;
        let target = self.vm.threads[&thread]
            .stack
            .last()
            .ok_or("creation parent frame absent")?
            .stack_object;
        let Some(source) = self.vm.world.objects.get(&target) else {
            self.vm.finish_creation(p, thread, r.id, Some(id))?;
            return Ok(true);
        };
        let desired = *source
            .data
            .get(&1)
            .ok_or("creation source direction absent")?
            & 6;
        let operation = lot_placement::Operation::copy_direction(id, desired)?;
        self.native_creations.insert(
            r.id,
            Execution {
                thread,
                request: r.clone(),
                phase: Phase::RotateOffLot,
                acceptance: None,
                placement: Some(lot_placement::live::Execution::new(operation)),
                after_delete_room: None,
                group: None,
                dynamic: None,
            },
        );
        self.drive_native_creation(p, r.id, remaining, step)
    }
    pub(crate) fn owns_immediate_creation_deletion(&self) -> bool {
        self.deletion.first().is_some_and(|d| {
            self.person_factories.values().any(|s| {
                s.rejection_cleanup
                    .as_ref()
                    .is_some_and(|c| c.owns_deletion(d.target))
            }) || self
                .cursor_cleanup
                .as_ref()
                .is_some_and(|e| e.owns_deletion(d.target))
                || self
                    .native_creations
                    .values()
                    .any(|e| e.phase == Phase::Destroy && e.request.created == Some(d.target))
                || self
                    .native_burn
                    .as_ref()
                    .is_some_and(|e| e.owns_deletion(d.target))
                || self
                    .native_lot
                    .as_ref()
                    .and_then(|n| n.shoo.as_ref())
                    .is_some_and(|s| s.owns_deletion(d.target))
                || self
                    .native_lot
                    .as_ref()
                    .and_then(|n| n.snap.as_ref())
                    .is_some_and(|s| s.owns_deletion(d.target))
                || self
                    .portrait_person
                    .as_ref()
                    .is_some_and(|e| e.owns_deletion(d.target))
                || self
                    .synchronous_removal
                    .as_ref()
                    .is_some_and(|e| e.owns_deletion(d.target))
        })
    }
    pub(crate) fn drive_pending_native_creations(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        // Nested synchronous factories have newer request IDs. Finish the inner
        // call before re-entering its parent's suspended Init/Main frame.
        while let Some((&key, _)) = self.native_creations.last_key_value() {
            if !self.drive_native_creation(p, key, remaining, step)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(crate) fn service_native_group_creation(
        &mut self,
        p: &Program,
        thread: ThreadId,
        r: Request,
        source: &str,
        catalog: u16,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if r.created.is_some() || r.placement.is_some() {
            return Err("orphan multipart factory request".into());
        }
        self.begin_group_factory(p, thread, r, source, catalog, remaining, step)
    }
    fn begin_group_factory(
        &mut self,
        p: &Program,
        thread: ThreadId,
        mut r: Request,
        source: &str,
        catalog: u16,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let date = &self
            .household
            .as_ref()
            .ok_or("group factory calendar absent")?
            .clock;
        let group = sims_vm::group_factory::Factory::begin(
            &mut self.vm,
            p,
            source,
            catalog,
            sims_vm::construction::ObjectDate {
                year: date.year,
                month: date.month,
                day: date.day,
            },
            [date.hour, date.minute],
        )?;
        let native = self
            .native_lot
            .as_mut()
            .ok_or("group factory host absent")?;
        for id in &group.parts {
            native.placement.links.next.insert(*id, 0);
            native.placement.bounds.insert(*id, [0; 4]);
            self.unplaced.insert(*id);
        }
        r.created = Some(group.head());
        self.vm.threads.get_mut(&thread).unwrap().status = Status::AwaitingCreation(r.clone());
        self.event(
            &mut step.events,
            thread,
            format!(
                "registered multipart group {:?}; starting original ordered Init/Main",
                group.parts
            ),
        );
        self.native_creations.insert(
            r.id,
            Execution {
                thread,
                request: r.clone(),
                phase: Phase::GroupFactory,
                acceptance: None,
                placement: None,
                after_delete_room: None,
                group: Some(group),
                dynamic: None,
            },
        );
        self.drive_native_creation(p, r.id, remaining, step)
    }
    pub(crate) fn service_native_placed_creation(
        &mut self,
        p: &Program,
        thread: ThreadId,
        mut r: Request,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if !self.native_creations.contains_key(&r.id) {
            if r.created.is_some() || r.placement.is_some() {
                return Err("orphan native creation continuation".into());
            }
            let guid = u32::from_le_bytes(r.operands[..4].try_into().unwrap());
            let mut definitions = p.modules.values().flat_map(|m| {
                m.object_definition_words
                    .iter()
                    .filter_map(move |(&id, w)| {
                        (w.len() > 15
                            && u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16) == guid)
                            .then_some((m, id))
                    })
            });
            let (module, number) = definitions
                .next()
                .ok_or_else(|| format!("Create Object GUID {guid:08X} is not loaded"))?;
            if definitions.next().is_some() {
                return Err("ambiguous Create Object GUID".into());
            }
            let Some(at) = placed_destination(&self.vm, thread, &r)? else {
                self.vm.finish_creation(p, thread, r.id, None)?;
                return Ok(true);
            };
            if !preflight(self, &r, &at, &module.source, number)? {
                self.vm.finish_creation(p, thread, r.id, None)?;
                return Ok(true);
            }
            let words = &module.object_definition_words[&number];
            if words.len() < 68 || [2, 8, 9].contains(&words[9]) {
                return Err("native placed creation requires ordinary or multipart factory".into());
            }
            if words[10] != 0 {
                r.placement = Some(at);
                return self.begin_group_factory(
                    p,
                    thread,
                    r,
                    &module.source,
                    number,
                    remaining,
                    step,
                );
            }
            let Some(id) = (1..=i16::MAX).find(|id| !self.vm.world.objects.contains_key(id)) else {
                self.vm.finish_creation(p, thread, r.id, None)?;
                return Ok(true);
            };
            self.initialize_object_factory(p, id, &module.source, number, false)?;
            r.created = Some(id);
            r.placement = Some(at);
            self.vm.threads.get_mut(&thread).unwrap().status = Status::AwaitingCreation(r.clone());
            self.event(
                &mut step.events,
                thread,
                format!("registered #{id} off-lot; starting original Init/Main"),
            );
            self.native_creations.insert(
                r.id,
                Execution {
                    thread,
                    request: r.clone(),
                    phase: Phase::Factory,
                    acceptance: None,
                    placement: None,
                    after_delete_room: None,
                    group: None,
                    dynamic: None,
                },
            );
        }
        self.drive_native_creation(p, r.id, remaining, step)
    }
    pub(crate) fn drive_native_creation(
        &mut self,
        p: &Program,
        key: u64,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let mut e = self
            .native_creations
            .remove(&key)
            .ok_or("native creation continuation missing")?;
        let result = self.drive_native_creation_inner(p, &mut e, remaining, step);
        if !matches!(result, Ok(true)) {
            self.native_creations.insert(key, e);
        }
        result
    }
    fn drive_native_creation_inner(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let id = e.request.created.unwrap();
        if e.phase == Phase::GroupFactory {
            let group = e.group.as_mut().unwrap();
            while let Some(part) = group.current(&mut self.vm, p)? {
                if !self.drive_factory_update(p, part, remaining, step)? {
                    return Ok(false);
                }
                group.finish_current(&mut self.vm, p)?;
            }
            self.native_lot
                .as_mut()
                .unwrap()
                .observers
                .push(lot_placement::Request::Signal {
                    event: 0xe2,
                    value: id,
                });
            self.event(
                &mut step.events,
                e.thread,
                format!("multipart factory completed #{id}; native creation signal 0xe2"),
            );
            e.phase = if e.request.operands[4] == 6 {
                Phase::GroupRotate
            } else if e.request.operands[5] & 0x20 != 0 {
                Phase::RotatePlaced
            } else {
                Phase::BeginPlacement
            };
        }
        if e.phase == Phase::GroupRotate {
            if e.request.operands[5] == 32 {
                if e.placement.is_none() {
                    let target = self.vm.threads[&e.thread]
                        .stack
                        .last()
                        .ok_or("group creator frame absent")?
                        .stack_object;
                    if let Some(source) = self.vm.world.objects.get(&target) {
                        let desired =
                            *source.data.get(&1).ok_or("group direction source absent")? & 6;
                        e.placement = Some(lot_placement::live::Execution::new_virtual(
                            lot_placement::VirtualOperation::copy_direction(
                                &self.vm.world,
                                id,
                                desired,
                            )?,
                        ));
                    }
                }
                if let Some(rotation) = &mut e.placement {
                    if !self.drive_live_placement(p, rotation, remaining, step)? {
                        return Ok(false);
                    }
                }
            }
            self.vm
                .finish_creation(p, e.thread, e.request.id, Some(id))?;
            return Ok(true);
        }
        if e.phase == Phase::RotateOffLot {
            if !self.drive_live_placement(p, e.placement.as_mut().unwrap(), remaining, step)? {
                return Ok(false);
            }
            self.vm
                .finish_creation(p, e.thread, e.request.id, Some(id))?;
            return Ok(true);
        }
        loop {
            match e.phase {
                Phase::RotateOffLot | Phase::GroupFactory | Phase::GroupRotate => unreachable!(),
                Phase::Factory => {
                    if !self.drive_object_factory(p, id, remaining, step)? {
                        return Ok(false);
                    }
                    self.native_lot.as_mut().unwrap().observers.push(
                        lot_placement::Request::Signal {
                            event: 0xe2,
                            value: id,
                        },
                    );
                    self.event(
                        &mut step.events,
                        e.thread,
                        format!(
                            "factory completed #{id}; native creation signal 0xe2 before placement"
                        ),
                    );
                    e.phase = if e.request.operands[5] & 0x20 != 0 {
                        Phase::RotatePlaced
                    } else {
                        Phase::BeginPlacement
                    };
                }
                Phase::RotatePlaced => {
                    // 44e64d reads the parent's current Stack Object after
                    // Init/Main, independently of the earlier destination.
                    // Capture its masked direction once. Native virtual Rotate
                    // callbacks may alter the frame or source before placement.
                    if e.placement.is_none() {
                        e.placement = copied_rotation(&self.vm, e.thread, id)?;
                    }
                    if let Some(rotation) = &mut e.placement {
                        if !self.drive_live_placement(p, rotation, remaining, step)? {
                            return Ok(false);
                        }
                    }
                    e.placement = None;
                    e.phase = Phase::BeginPlacement;
                }
                Phase::BeginPlacement => {
                    // 44e0c8 centers a dynamic multipart destination after
                    // Init/Main, before acceptance. Keep the captured target
                    // and publish the same request in the suspended VM frame.
                    if sims_vm::multitile::dynamic(&self.vm.world, p, id)? {
                        let at = e.request.placement.as_mut().unwrap();
                        center_dynamic_destination(at);
                        self.vm.threads.get_mut(&e.thread).unwrap().status =
                            Status::AwaitingCreation(e.request.clone());
                    }
                    let at = e.request.placement.as_ref().unwrap();
                    e.acceptance = Some(if matches!(e.request.operands[4], 2 | 4) {
                        lot_acceptance::live::Execution::new_contained(
                            self, id, at.target, 0, at.level,
                        )?
                    } else {
                        lot_acceptance::live::Execution::new(
                            self,
                            id,
                            [at.position[1], at.position[0]],
                            at.level,
                        )?
                    });
                    e.phase = Phase::Check;
                }
                Phase::Check => {
                    let a = e.acceptance.as_mut().unwrap();
                    if !self.drive_native_acceptance(p, a, remaining, step)? {
                        return Ok(false);
                    }
                    let accepted = a.outcome().unwrap().accepted;
                    let links = &self.native_lot.as_ref().unwrap().placement.links;
                    let operation = if accepted {
                        lot_placement::VirtualOperation::set(
                            &self.vm.world,
                            links,
                            id,
                            destination(&e.request),
                            true,
                        )?
                    } else {
                        self.native_removal_tree(p, id)?;
                        e.after_delete_room =
                            crate::lot_removal::immediate_room(&self.vm.world, id)?;
                        lot_placement::VirtualOperation::remove(&self.vm.world, links, id, true)?
                    };
                    e.placement = Some(lot_placement::live::Execution::new_virtual(operation));
                    e.phase = if accepted {
                        Phase::Place
                    } else {
                        Phase::Detach
                    };
                }
                Phase::Place | Phase::Detach => {
                    if !self.drive_live_placement(
                        p,
                        e.placement.as_mut().unwrap(),
                        remaining,
                        step,
                    )? {
                        return Ok(false);
                    }
                    if e.phase == Phase::Place {
                        if e.group.is_some() && sims_vm::multitile::dynamic(&self.vm.world, p, id)?
                        {
                            e.dynamic = Some(crate::lot_dynamic::live::Execution::begin(self, id)?);
                            e.phase = Phase::Dynamic;
                            continue;
                        }
                        self.vm
                            .finish_creation_with_parameters(p, e.thread, e.request.id, id)?;
                        self.event(
                            &mut step.events,
                            e.thread,
                            format!("created #{id} at {:?}", destination(&e.request)),
                        );
                        return Ok(true);
                    }
                    if !self.deletion.is_empty() {
                        return Err(
                            "nested synchronous deletion requires a destructor stack boundary"
                                .into(),
                        );
                    }
                    let mut d = removal::Deletion::new(id);
                    d.after_delete_room = e.after_delete_room;
                    self.deletion.push(d);
                    e.phase = Phase::Destroy;
                }
                Phase::Dynamic => {
                    if !self.drive_dynamic_update(
                        p,
                        e.dynamic.as_mut().unwrap(),
                        remaining,
                        step,
                    )? {
                        return Ok(false);
                    }
                    self.vm
                        .finish_creation_with_parameters(p, e.thread, e.request.id, id)?;
                    self.event(
                        &mut step.events,
                        e.thread,
                        format!(
                            "created dynamic group #{id} at {:?}",
                            destination(&e.request)
                        ),
                    );
                    return Ok(true);
                }
                Phase::Destroy => {
                    if !self.drive_deletion_stack(p, remaining, step, false)? {
                        return Ok(false);
                    }
                    self.vm.finish_creation(p, e.thread, e.request.id, None)?;
                    return Ok(true);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn dynamic_centering_and_update_gate_match_original_wrapper() {
        let report: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/create-dynamic.json")).unwrap();
        assert_eq!(report["cases"].as_array().unwrap().len(), 32);
        for row in report["cases"].as_array().unwrap() {
            let c = &row["input"];
            let calls = row["original"]["calls"].as_array().unwrap();
            let bounds = calls.iter().find(|v| v[0] == "bounds").unwrap();
            let check = calls.iter().find(|v| v[0] == "check").unwrap();
            let mut at = Placement {
                target: 1,
                position: [
                    bounds[1][1].as_i64().unwrap() as i32,
                    bounds[1][0].as_i64().unwrap() as i32,
                ],
                level: 1,
                room: -5,
                wall_direction: None,
            };
            if c["dynamic"] == true {
                center_dynamic_destination(&mut at);
            }
            assert_eq!(json!([at.position[1], at.position[0]]), check[1], "{c}");
            let accepted = c["placement"] == true;
            assert_eq!(
                calls.iter().any(|v| v[0] == "dynamic_update"),
                accepted && c["after_dynamic"] == true,
                "{c}"
            );
            assert_eq!(calls.iter().any(|v| v[0] == "delete"), !accepted, "{c}");
        }
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn relative_placed_destination_matches_original_caller_local_and_wall_capture() {
        let report: Value = serde_json::from_str(include_str!(
            "../../../sims-vm/tests/fixtures/home-create.json"
        ))
        .unwrap();
        assert_eq!(report["cases"].as_array().unwrap().len(), 64);
        for row in report["cases"].as_array().unwrap() {
            let c = &row["input"];
            let (_, mut s) = crate::lot_runtime::tests::fixture();
            let thread = s.vm.object_threads[&4];
            let frame =
                s.vm.threads
                    .get_mut(&thread)
                    .unwrap()
                    .stack
                    .last_mut()
                    .unwrap();
            frame.locals = vec![c["local"].as_i64().unwrap() as i16];
            let r = Request {
                id: 1,
                origin: frame.location.clone(),
                caller: 4,
                target: 1,
                operands: [0, 0, 0, 0, 9, 1, 0, 0],
                created: None,
                placement: None,
            };
            s.vm.world.positions.insert(
                4,
                sims_vm::autonomy::Placement {
                    x: 32,
                    y: 48,
                    level: 1,
                },
            );
            s.vm.world
                .objects
                .get_mut(&4)
                .unwrap()
                .data
                .insert(1, c["facing"].as_i64().unwrap() as i16);
            let at = placed_destination(&s.vm, thread, &r).unwrap().unwrap();
            let calls = row["original"]["calls"].as_array().unwrap();
            assert_eq!(
                json!([at.position[1], at.position[0]]),
                calls.iter().find(|v| v[0] == "check").unwrap()[1]
            );
            assert_eq!(
                json!(at.wall_direction),
                calls.iter().find(|v| v[0] == "wall").unwrap()[1]
            );
            assert_eq!((at.target, at.room), (4, -5));
        }
    }
    #[test]
    fn creation_room_filter_uses_primary_tile_on_a_diagonal_split() {
        let (_, mut s) = crate::lot_runtime::tests::fixture();
        s.vm.world.objects.get_mut(&1).unwrap().data.insert(29, 7);
        let architecture = s.architecture.as_mut().unwrap();
        let floor = &mut architecture.original.as_mut().unwrap().floors[0];
        floor.rooms.primary.fill(7);
        floor.rooms.primary[13] = -5;
        floor.source.walls[13].segments = 16;
        // Imported grids are column-major; the architecture projection is
        // row-major, as in Architecture::from_original.
        architecture.floors[0].rooms.fill(7);
        architecture.floors[0].rooms[8] = -5;
        let point = crate::Point::new(40, 24);
        assert_eq!(architecture.room_at(1, point), 7);
        assert_eq!(architecture.primary_room_at(1, point), -5);
        let r = Request {
            id: 1,
            origin: sims_vm::Location {
                source: "objects".into(),
                bhav: 4096,
                instruction: 0,
            },
            caller: 4,
            target: 1,
            operands: [0, 0, 0, 0, 3, 0, 0, 0],
            created: None,
            placement: None,
        };
        let at = placed_destination(&s.vm, 0, &r).unwrap().unwrap();
        assert_eq!(at.position, [40, 24]);
        assert!(!preflight(&s, &r, &at, "objects", 1).unwrap());
    }

    #[cfg(feature = "private-fixtures")]
    #[test]
    fn floor_destinations_and_preallocation_guards_match_original_wrapper() {
        let report: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/create-floor.json")).unwrap();
        assert_eq!(report["cases"].as_array().unwrap().len(), 480);
        let frame_report: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/create-frame-floor.json"))
                .unwrap();
        assert_eq!(frame_report["cases"].as_array().unwrap().len(), 700);
        for row in report["cases"]
            .as_array()
            .unwrap()
            .iter()
            .chain(frame_report["cases"].as_array().unwrap())
        {
            let c = &row["input"];
            let n = &row["original"];
            let value = |key: &str, default: i64| c[key].as_i64().unwrap_or(default);
            let (p, mut s) = crate::lot_runtime::tests::fixture();
            s.vm.world.objects.clear();
            s.vm.world.positions.clear();
            for (id, key, x, y, direction) in [
                (1, "caller", 32, 48, value("facing", 0)),
                (2, "target", 80, 96, value("rotation", 2)),
            ] {
                s.vm.world.objects.insert(
                    id,
                    sims_vm::Object {
                        definition: Some(sims_vm::Definition {
                            source: "objects".into(),
                            object: if c["duplicate"] == true { 1 } else { 2 },
                            guid: 0,
                            slots: 0,
                        }),
                        data: [
                            (1, direction as i16),
                            (29, value(&format!("{key}_room"), -5) as i16),
                        ]
                        .into(),
                        ..Default::default()
                    },
                );
                s.vm.world.positions.insert(
                    id,
                    sims_vm::autonomy::Placement {
                        x,
                        y,
                        level: value(&format!("{key}_level"), 1) as i32,
                    },
                );
            }
            s.architecture = Some(
                crate::architecture::Architecture::with_floors(
                    1024,
                    1024,
                    vec![
                        crate::architecture::FloorPlan {
                            walls: vec![],
                            supported: vec![true; 4096]
                        };
                        2
                    ],
                    &p,
                )
                .unwrap(),
            );
            for floor in &mut s.architecture.as_mut().unwrap().floors {
                floor.rooms.fill(7);
            }
            let mut operands = [0; 8];
            operands[4] = value("mode", 0) as u8;
            operands[5] = value("flags", 0) as u8;
            operands[6] = value("index", 0) as u8;
            let mut r = Request {
                id: 1,
                origin: sims_vm::Location {
                    source: "objects".into(),
                    bhav: 4096,
                    instruction: 0,
                },
                caller: 1,
                target: value("target", 2) as i16,
                operands,
                created: None,
                placement: None,
            };
            let calls = n["calls"].as_array().unwrap();
            if c["resource"] == false {
                assert!(n["error"].is_null(), "{c}");
                assert_eq!(calls.len(), 1, "{c}");
                continue;
            } // Resource lookup is before destination selection.
            let thread = s.vm.object_threads[&4];
            if matches!(operands[4], 7 | 8) {
                let f =
                    s.vm.threads
                        .get_mut(&thread)
                        .unwrap()
                        .stack
                        .last_mut()
                        .unwrap();
                f.args = vec![1; value("frame_args", 4) as usize];
                f.locals = vec![1; value("frame_locals", 4) as usize];
                let selected = if operands[4] == 7 {
                    f.args.first_mut()
                } else {
                    f.locals.get_mut(usize::from(operands[6]))
                };
                if let Some(selected) = selected {
                    *selected = value("selected", 2) as i16;
                }
            }
            let at = placed_destination(&s.vm, thread, &r);
            if n["error"] == 8 {
                assert!(at.unwrap_err().contains("0x08"), "{c}");
                assert_eq!(calls.len(), 1, "{c}");
                continue;
            }
            let at = at.unwrap_or_else(|e| panic!("{c}: {e}"));
            let Some(at) = at else {
                assert!(
                    !calls
                        .iter()
                        .any(|call| call[0] == "bounds" || call[0] == "factory"),
                    "{c}"
                );
                continue;
            };
            let links = &mut s.native_lot.as_mut().unwrap().placement.links;
            links.size = 64;
            links.heads = vec![vec![0; 4096]; 2];
            links.next = [(1, 0), (2, 0)].into();
            if c["occupied"] == true {
                let cell = (at.position[0] >> 4) as usize * 64 + (at.position[1] >> 4) as usize;
                links.heads[at.level as usize - 1][cell] = 2;
            }
            if c["outside"] == true {
                links.size = 1;
            }
            assert_eq!(
                preflight(&s, &r, &at, "objects", 1).unwrap(),
                calls.iter().any(|call| call[0] == "factory"),
                "{c}"
            );
            r.placement = Some(at);
            if c["factory_change"] == "unregister" {
                s.vm.world.objects.remove(&2);
                s.vm.world.positions.remove(&2);
            } else if c["factory_change"] == "move" {
                s.vm.world.positions.insert(
                    2,
                    sims_vm::autonomy::Placement {
                        x: 112,
                        y: 128,
                        level: 2,
                    },
                );
            }
            if matches!(c["factory_change"].as_str(), Some("move" | "unregister")) {
                let f =
                    s.vm.threads
                        .get_mut(&thread)
                        .unwrap()
                        .stack
                        .last_mut()
                        .unwrap();
                if operands[4] == 7 {
                    f.args[0] = -123;
                } else {
                    f.locals[usize::from(operands[6])] = -123;
                }
            }
            let descriptor = destination(&r);
            if let Some(check) = calls.iter().find(|call| call[0] == "check") {
                assert_eq!(json!(descriptor.position), check[1], "{c}");
                assert_eq!(json!(descriptor.level), check[2], "{c}");
                assert!(descriptor.container.is_none());
                assert_eq!(check[3], 0);
                // The wrapper passes slot zero with a null container. The
                // physical floor descriptor normalizes its unused slot to -1.
                assert_eq!(check[4], 0);
                assert_eq!(descriptor.slot, -1);
            }
        }
    }

    #[test]
    fn frame_selected_creation_rejects_opaque_local_without_using_placeholder_id() {
        let (_, mut s) = crate::lot_runtime::tests::fixture();
        let thread = s.vm.object_threads[&4];
        let f =
            s.vm.threads
                .get_mut(&thread)
                .unwrap()
                .stack
                .last_mut()
                .unwrap();
        f.locals = vec![1];
        f.opaque_locals.insert(0);
        let r = Request {
            id: 1,
            origin: f.location.clone(),
            caller: 4,
            target: 1,
            operands: [0, 0, 0, 0, 8, 0, 0, 0],
            created: None,
            placement: None,
        };
        assert!(
            placed_destination(&s.vm, thread, &r)
                .unwrap_err()
                .contains("opaque local")
        );
    }

    #[cfg(feature = "private-fixtures")]
    #[test]
    fn contained_creation_modes_match_original_destination_selection() {
        let report: Value = serde_json::from_str(include_str!(
            "../../../sims-vm/tests/fixtures/create-contained-modes.json"
        ))
        .unwrap();
        assert_eq!(report["cases"].as_array().unwrap().len(), 32);
        for row in report["cases"].as_array().unwrap() {
            let c = &row["input"];
            let n = &row["original"];
            let mut world = sims_vm::World::default();
            for (id, x, y, level) in [
                (1, 32, 48, c["caller_level"].as_i64().unwrap()),
                (2, 80, 96, c["target_level"].as_i64().unwrap()),
            ] {
                world.objects.insert(
                    id,
                    sims_vm::Object {
                        data: [(29, -5)].into(),
                        ..Default::default()
                    },
                );
                world.positions.insert(
                    id,
                    sims_vm::autonomy::Placement {
                        x,
                        y,
                        level: level as i32,
                    },
                );
            }
            let mut operands = [0; 8];
            operands[4] = c["mode"].as_u64().unwrap() as u8;
            let r = Request {
                id: 1,
                origin: sims_vm::Location {
                    source: "test".into(),
                    bhav: 4096,
                    instruction: 0,
                },
                caller: 1,
                target: c["target"].as_i64().unwrap() as i16,
                operands,
                created: None,
                placement: None,
            };
            let actual = contained_destination(&world, &r).unwrap();
            let expected = n["calls"]
                .as_array()
                .unwrap()
                .iter()
                .find(|call| call[0] == "check");
            assert_eq!(actual.is_some(), expected.is_some(), "{c}");
            if let (Some(at), Some(expected)) = (actual, expected) {
                let mut r = r.clone();
                r.placement = Some(at);
                let physical = destination(&r);
                assert_eq!(json!(physical.position), expected[1], "{c}");
                assert_eq!(json!(physical.level), expected[2], "{c}");
                assert_eq!(json!(physical.container), expected[3], "{c}");
                assert_eq!(json!(physical.slot), expected[4], "{c}");
            }
            if operands[4] == 2 {
                // Mode 2 never reads the caller's room as a placement filter.
                world.objects.get_mut(&1).unwrap().data.remove(&29);
                assert_eq!(contained_destination(&world, &r).unwrap().unwrap().room, -5);
            }
        }
    }
}
