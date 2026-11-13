//! Original Snap placement, using the imported physical graph. The
//! primitive is synchronous: diagnostic budgets suspend callbacks, not lot time.
use crate::{Simulation, Step, lot_acceptance, lot_links::Descriptor, lot_placement};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, SnapRequest, Status, ThreadId, Vm, routing::Destination};
mod shoo;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
enum Failure {
    Check,
    Shoo(shoo::Execution),
    Complete(bool),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Plan {
    pub position: [i32; 2],
    pub level: i32,
    pub direction: u8,
    pub bypass_adjacency: bool,
    pub container: Option<(sims_vm::ObjectId, i32)>,
    /// Animator arguments as exact f32 bits, in native Y/X/heading order.
    pub offsets: [u32; 3],
}

pub fn contained_slot(r: &SnapRequest) -> Option<i32> {
    match &r.route.destination {
        Destination::FirstSlot => Some(0),
        Destination::Slot { slot, version, .. }
            if (4..=10).contains(version) && slot.kind == 3 && slot.snap_target >= 0 =>
        {
            Some(slot.snap_target)
        }
        _ => None,
    }
}

/// Contained branch of 0x454976. The native compensation uses cardinal-only
/// rotation and truncates each component, including with operand bit0 clear.
pub fn plan(
    r: &SnapRequest,
    target: [i32; 3],
    rotation: u8,
    facing: u8,
    slot_offset: Option<[f32; 2]>,
) -> Result<Plan, String> {
    if r.flags & !6 != 0 {
        return Err("native Snap compensation flags unsupported".into());
    }
    let Some(slot_index) = contained_slot(r) else {
        return floor_plan(r, target, rotation, facing);
    };
    if rotation > 7 || facing > 7 || !(1..=2).contains(&target[2]) {
        return Err("invalid native Snap coordinates/direction".into());
    }
    let (direction, bypass_adjacency) = match &r.route.destination {
        Destination::Slot { slot, .. } => {
            if slot_offset.is_none() {
                return Err("Snap containment slot absent (native error 0x20)".into());
            }
            (
                if slot.flags & 0x1000 == 0 {
                    facing
                } else {
                    (rotation + (0..7).find(|i| slot.flags & (1 << i) != 0).unwrap_or(0)) & 7
                },
                slot.flags & 0x800 != 0,
            )
        }
        _ => (facing, false),
    };
    let [x, y] = slot_offset.unwrap_or([0.; 2]);
    if !x.is_finite() || !y.is_finite() || x.abs() > 32768. || y.abs() > 32768. {
        return Err("invalid native Snap containment offset".into());
    }
    let [y, x] = match rotation {
        0 => [x, y],
        2 => [-y, x],
        4 => [-x, -y],
        6 => [y, -x],
        _ => [0., 0.],
    };
    Ok(Plan {
        position: [target[0], target[1]],
        level: target[2],
        direction,
        bypass_adjacency,
        container: Some((r.route.target, slot_index)),
        offsets: [(y as i32 as f32).to_bits(), (x as i32 as f32).to_bits(), 0],
    })
}
/// 0x454976 -> 0x454ecc/0x455070/0x454fb9. Unlike route candidate
/// enumeration, Snap always rotates its exact offset through the float matrix.
pub fn floor_plan(
    r: &SnapRequest,
    target: [i32; 3],
    rotation: u8,
    facing: u8,
) -> Result<Plan, String> {
    if rotation > 7 || facing > 7 || !(1..=2).contains(&target[2]) {
        return Err("invalid native Snap coordinates/direction".into());
    }
    match &r.route.destination {
        Destination::Front => {
            let (position, direction) =
                crate::routing::direct_snap([target[0], target[1]], rotation, facing, true)?;
            Ok(Plan {
                position,
                level: target[2],
                direction,
                bypass_adjacency: false,
                container: None,
                offsets: [0; 3],
            })
        }
        Destination::Slot { slot, version, .. }
            if (4..=10).contains(version) && slot.kind == 3 && slot.snap_target < 0 =>
        {
            let [x, y, _] = slot.offset;
            if !x.is_finite() || !y.is_finite() || x.abs() > 32768. || y.abs() > 32768. {
                return Err("invalid native Snap slot offset".into());
            }
            let angle = (f64::from(rotation) * f64::from(std::f32::consts::PI) * 0.25) as f32;
            let (sin, cos) = (f64::from(angle).sin() as f32, f64::from(angle).cos() as f32);
            let (x, y) = (x * 0.1875, y * 0.1875);
            let nx = (f64::from(y) * f64::from(cos) + f64::from(x) * f64::from(sin)) as f32;
            let ny = (f64::from(x) * f64::from(cos) - f64::from(y) * f64::from(sin)) as f32;
            let position = [
                target[0] as f32 * 0.1875 + nx,
                target[1] as f32 * 0.1875 + ny,
            ]
            .map(|v| (v * (16.0f32 / 3.0)).round() as i32);
            let direction = if slot.flags & 0x1000 == 0 {
                facing
            } else {
                (rotation + (0..7).find(|i| slot.flags & (1 << i) != 0).unwrap_or(0)) & 7
            };
            Ok(Plan {
                position,
                level: target[2],
                direction,
                bypass_adjacency: slot.flags & 0x800 != 0,
                container: None,
                offsets: [0; 3],
            })
        }
        _ => Err("native contained Snap adapter required".into()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Execution {
    pub thread: ThreadId,
    pub request: SnapRequest,
    target: [i32; 3],
    rotation: u8,
    facing: u8,
    slot_offset: Option<[u32; 2]>,
    pub plan: Plan,
    pub acceptance: Option<lot_acceptance::live::Execution>,
    pub placement: Option<lot_placement::live::Execution>,
    prechecked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure: Option<Failure>,
}
impl Execution {
    pub fn begin(
        s: &mut Simulation,
        p: &Program,
        thread: ThreadId,
        r: &SnapRequest,
    ) -> Result<Self, String> {
        let a = &s.vm.world.objects[&r.route.caller];
        let t = &s.vm.world.objects[&r.route.target];
        let at = s.vm.world.positions[&r.route.target];
        let target = [at.x, at.y, at.level];
        let rotation = u8::try_from(*t.data.get(&1).ok_or("Snap target direction absent")?)
            .map_err(|_| "Snap direction invalid")?;
        let facing = u8::try_from(*a.data.get(&1).ok_or("Snap actor direction absent")?)
            .map_err(|_| "Snap direction invalid")?;
        let slot_offset = if let Some(index) = contained_slot(r) {
            lot_acceptance::contained::slots(&s.vm.world, p, r.route.target)?
                .get(index as usize)
                .map(|s| [s.offset[0], s.offset[1]])
        } else {
            None
        };
        let plan = plan(r, target, rotation, facing, slot_offset)?;
        s.native_lot
            .as_mut()
            .ok_or("native Snap host absent")?
            .diagnostics = lot_acceptance::person::Diagnostics {
            error: 0,
            blocker: None,
        };
        Ok(Self {
            thread,
            request: r.clone(),
            target,
            rotation,
            facing,
            slot_offset: slot_offset.map(|v| v.map(f32::to_bits)),
            plan,
            acceptance: None,
            placement: None,
            prechecked: false,
            failure: None,
        })
    }
    fn destination(&self) -> Descriptor {
        Descriptor {
            on_lot: true,
            position: self.plan.position,
            level: self.plan.level,
            container: self.plan.container.map(|(id, _)| id),
            slot: self.plan.container.map_or(0, |(_, slot)| slot),
        }
    }
    fn operation(&self, s: &Simulation) -> Result<lot_placement::VirtualOperation, String> {
        let actor = self.request.route.caller;
        // 4550a0 calls virtual +58: cXMTObject dispatches to 422c2f.
        // People inherit the base +58; their walking +88 override is separate.
        if s.vm.world.objects[&actor].person.is_some() {
            Ok(lot_placement::VirtualOperation::Base(
                lot_placement::Operation::set(actor, self.destination(), true),
            ))
        } else {
            lot_placement::VirtualOperation::set(
                &s.vm.world,
                &s.native_lot
                    .as_ref()
                    .ok_or("Snap native lot absent")?
                    .placement
                    .links,
                actor,
                self.destination(),
                true,
            )
        }
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.acceptance
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self
                .placement
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || matches!(&self.failure, Some(Failure::Shoo(e)) if e.owns_callback(vm, p, thread, depth))
    }
    pub(crate) fn owns_deletion(&self, id: ObjectId) -> bool {
        matches!(&self.failure, Some(Failure::Shoo(e)) if e.owns_deletion(id))
    }
    pub fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        let t =
            s.vm.threads
                .get(&self.thread)
                .ok_or("native Snap thread absent")?;
        let bound = std::iter::once(&t.status)
            .chain(
                t.callback
                    .iter()
                    .flat_map(|b| b.chain())
                    .map(|b| &b.resume_status),
            )
            .any(|status| matches!(status,Status::AwaitingSnap(r) if r==&self.request));
        if !bound
            || s.visit.map(|v| v.0) != Some(self.thread)
            || self.prechecked != self.acceptance.is_some()
            || self.plan
                != plan(
                    &self.request,
                    self.target,
                    self.rotation,
                    self.facing,
                    self.slot_offset.map(|v| v.map(f32::from_bits)),
                )?
        {
            return Err("invalid native Snap continuation".into());
        }
        if let Some(a) = &self.acceptance {
            a.validate(&s.vm, p)?;
            if !self.prechecked
                || a.binding().0 != self.request.route.caller
                || a.binding().2 != self.plan.level
                || self.plan.container.is_none() && a.binding().1 != self.plan.position
                || a.container_binding() != self.plan.container
            {
                return Err("invalid Snap acceptance binding".into());
            }
        }
        if let Some(e) = &self.placement {
            e.validate(&s.vm, p, self.request.route.caller)?;
            if self
                .acceptance
                .as_ref()
                .and_then(|a| a.outcome())
                .is_none_or(|o| !o.accepted)
                || e.operation.binding() != (self.request.route.caller, Some(self.destination()))
                || matches!(e.operation, lot_placement::VirtualOperation::Group(_))
                    != s.vm.world.objects[&self.request.route.caller]
                        .multitile
                        .is_some()
            {
                return Err("invalid Snap placement binding".into());
            }
        }
        if self.failure.is_some() && self.placement.is_some() {
            return Err("Snap failure overlaps successful placement".into());
        }
        if let Some(Failure::Shoo(e)) = &self.failure {
            if self.request.flags & 2 == 0
                || s.vm.world.objects[&self.request.route.caller]
                    .person
                    .is_none()
                || self
                    .acceptance
                    .as_ref()
                    .and_then(|a| a.outcome())
                    .is_none_or(|o| o.accepted)
            {
                return Err("Snap shoo lacks failed person acceptance".into());
            }
            e.validate(s, p)?;
        }
        Ok(())
    }
}
impl Simulation {
    /// Optional 0x43ac10 catalog lookup for the imported placement-shoo owner.
    /// A missing loaded module is a host request, not proof of native absence.
    pub fn pending_snap_shoo_resource(&self, p: &Program) -> Result<bool, String> {
        let Some(n) = &self.native_lot else {
            return Ok(false);
        };
        let Some(e) = &n.snap else { return Ok(false) };
        if !matches!(e.failure, Some(Failure::Check))
            || p.modules.values().any(|m| {
                m.object_definition_words.values().any(|w| {
                    w.len() > 15 && (w[14] as u16 as u32 | ((w[15] as u16 as u32) << 16)) == 0x7c4
                })
            })
        {
            return Ok(false);
        }
        if !crate::contention::snap_shoo_allowed(
            self.vm.world.objects[&e.request.route.caller]
                .person
                .is_some(),
            e.request.flags,
            n.diagnostics.error,
            n.diagnostics
                .blocker
                .is_some_and(|b| self.vm.world.objects[&b].person.is_some()),
        ) {
            return Ok(false);
        }
        Ok(!shoo::has_move(self, n.diagnostics.blocker.unwrap())?)
    }
    /// The source catalog proved GUID 0x7c4 absent. Decode/provider errors must
    /// remain errors and must never be reported as this native null result.
    pub fn complete_missing_snap_shoo_resource(&mut self, p: &Program) -> Result<(), String> {
        if !self.pending_snap_shoo_resource(p)? {
            return Err("no pending Snap shoo resource lookup".into());
        }
        self.native_lot
            .as_mut()
            .unwrap()
            .snap
            .as_mut()
            .unwrap()
            .failure = Some(Failure::Complete(false));
        Ok(())
    }
    pub(crate) fn drive_native_snap(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let Some(mut e) = self.native_lot.as_mut().and_then(|n| n.snap.take()) else {
            return Ok(true);
        };
        let result = self.drive_snap(p, &mut e, remaining, step);
        if !result.as_ref().is_ok_and(|done| *done) {
            self.native_lot.as_mut().unwrap().snap = Some(e);
        }
        result
    }
    fn drive_snap(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let actor = e.request.route.caller;
        // The acceptance call has returned. Re-entering it here would overwrite
        // the live diagnostic globals changed by the shoo child's callbacks.
        if e.failure.is_some() {
            return self.finish_native_snap(p, e, false, remaining, step);
        }
        if !e.prechecked {
            if !self.snap_adjacency(actor, &e.plan)? {
                return self.finish_native_snap(p, e, false, remaining, step);
            }
            e.acceptance = Some(if let Some((parent, slot)) = e.plan.container {
                lot_acceptance::live::Execution::new_contained(
                    self,
                    actor,
                    parent,
                    slot,
                    e.plan.level,
                )?
            } else {
                lot_acceptance::live::Execution::new(self, actor, e.plan.position, e.plan.level)?
            });
            e.prechecked = true;
        }
        if !self.drive_native_acceptance(p, e.acceptance.as_mut().unwrap(), remaining, step)? {
            return Ok(false);
        }
        if !e.acceptance.as_ref().unwrap().outcome().unwrap().accepted {
            return self.finish_native_snap(p, e, false, remaining, step);
        }
        if e.placement.is_none() {
            if !contained_wall_gate(&e.plan, e.request.flags, |parent, direction| {
                self.snap_contained_crosses_wall(actor, parent, direction)
            })? {
                self.vm
                    .world
                    .objects
                    .get_mut(&actor)
                    .unwrap()
                    .data
                    .insert(54, 0);
                return self.finish_native_snap(p, e, false, remaining, step);
            }
            e.placement = Some(lot_placement::live::Execution::new_virtual(
                e.operation(self)?,
            ));
        }
        if !self.drive_live_placement(p, e.placement.as_mut().unwrap(), remaining, step)? {
            return Ok(false);
        }
        self.face(actor, 1 << e.plan.direction)?;
        if self.vm.world.objects[&actor].person.is_some() {
            // 4550a0 calls animator +20 after SetObject and facing publication.
            // This invalidates cached XY/heading, preserving existing offsets.
            if let Some(owner) = self.characters.get_mut(&actor) {
                owner.character.placement.invalidate();
            }
            // Retain the original primitive's Y/X/heading compensation. Its
            // +2c publication follows the result-word clear in the outer tail.
            self.native_lot
                .as_mut()
                .unwrap()
                .snap_offsets
                .insert(actor, e.plan.offsets);
        }
        self.finish_native_snap(p, e, true, remaining, step)
    }
    fn snap_contained_crosses_wall(
        &self,
        actor: sims_vm::ObjectId,
        parent: sims_vm::ObjectId,
        direction: u8,
    ) -> Result<bool, String> {
        let o = &self.vm.world.objects[&actor];
        let at = self.vm.world.positions[&parent];
        let d = o.definition.as_ref().ok_or("Snap actor resource absent")?;
        let architecture = self.architecture.as_ref().unwrap();
        let shape = architecture
            .footprints
            .get(&d.guid)
            .ok_or("Snap actor footprint absent")?;
        let person = |field| -> Result<i16, String> {
            o.person.as_ref().map_or(Ok(0), |p| {
                p.get(&field)
                    .copied()
                    .ok_or("Snap person footprint field absent".into())
            })
        };
        let [max_x, max_y, min_x, min_y] = crate::spatial::footprint(
            [at.x, at.y],
            o.person.is_some(),
            (person(68)? | person(84)?) & 1 != 0,
            shape.size,
            shape.insets,
            person(73)? as u16,
            direction,
        );
        architecture
            .original
            .as_ref()
            .unwrap()
            .wall_grid(at.level)?
            .blocks_bounds(crate::Rect {
                min: crate::Point::new(min_y, min_x),
                max: crate::Point::new(max_y, max_x),
            })
    }
    fn finish_native_snap(
        &mut self,
        p: &Program,
        e: &mut Execution,
        success: bool,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let actor = e.request.route.caller;
        if e.failure.is_none() {
            self.vm
                .world
                .objects
                .get_mut(&actor)
                .unwrap()
                .data
                .insert(52, 0);
            if !success {
                e.failure = Some(Failure::Check);
            }
        }
        if !success {
            loop {
                match e.failure.as_mut().unwrap() {
                    Failure::Check => {
                        let d = self.native_lot.as_ref().unwrap().diagnostics;
                        let allowed = crate::contention::snap_shoo_allowed(
                            self.vm.world.objects[&actor].person.is_some(),
                            e.request.flags,
                            d.error,
                            d.blocker
                                .is_some_and(|b| self.vm.world.objects[&b].person.is_some()),
                        );
                        let child = if allowed {
                            // 454976 supplies the full active action +a64,
                            // not its truncated person-data33 mirror.
                            let priority = self
                                .vm
                                .world
                                .actions
                                .as_ref()
                                .and_then(|a| a.actors.get(&actor))
                                .ok_or("Snap shoo caller action owner absent")?
                                .active
                                .as_ref()
                                .map_or(0, |a| a.priority);
                            shoo::Execution::begin(self, p, d.blocker.unwrap(), priority)?
                        } else {
                            None
                        };
                        e.failure = Some(child.map_or(Failure::Complete(false), Failure::Shoo));
                    }
                    Failure::Shoo(child) => {
                        let Some(accepted) =
                            self.drive_snap_shoo(p, child, e.thread, remaining, step)?
                        else {
                            return Ok(false);
                        };
                        e.failure = Some(Failure::Complete(accepted));
                    }
                    Failure::Complete(accepted) => {
                        // Re-read global error/blocker AFTER all synchronous
                        // callbacks. Failed shoo does not clear a callback's
                        // write to field 52; accepted shoo overwrites it with 1.
                        let d = self.native_lot.as_ref().unwrap().diagnostics;
                        let o = self.vm.world.objects.get_mut(&actor).unwrap();
                        if *accepted {
                            o.data.insert(52, 1);
                        }
                        if o.data[&52] == 0 && d.error == 1 {
                            o.data.insert(52, 2);
                        }
                        o.data.insert(54, d.blocker.unwrap_or(0));
                        break;
                    }
                }
            }
        }
        if success && self.vm.world.objects[&actor].person.is_some() {
            self.apply_snap_character_offsets(actor, e.plan.offsets)?;
        }
        self.vm
            .finish_snap(p, e.thread, e.request.route.id, success)?;
        self.event(
            &mut step.events,
            e.thread,
            format!(
                "native Snap #{} -> {:?} floor {}: {success}",
                actor, e.plan.position, e.plan.level
            ),
        );
        Ok(true)
    }
    /// 0x4550a0's pre-acceptance room and directional wall gates. SLOT bit0x800
    /// bypasses these; final floor acceptance still runs.
    fn snap_adjacency(&mut self, actor: ObjectId, plan: &Plan) -> Result<bool, String> {
        let n = self.native_lot.as_ref().unwrap();
        let lot = self
            .architecture
            .as_ref()
            .unwrap()
            .original
            .as_ref()
            .unwrap();
        let old = self.vm.world.positions[&actor];
        let mut error = n.diagnostics.error;
        let result = adjacency(
            lot.size,
            [old.x, old.y, old.level],
            self.vm.world.objects[&actor].data[&29],
            plan,
            &mut error,
            |request| {
                crate::lot_route::candidate::Check::world_reply(
                    request,
                    &self.vm.world,
                    &n.placement.links,
                    lot,
                )?
                .ok_or("Snap adjacency query returned no reply".into())
            },
        )?;
        self.native_lot.as_mut().unwrap().diagnostics.error = error;
        Ok(result)
    }
}

/// 0x4550a0 invokes this only after acceptance and before SetObject. The
/// temporary direction is used for the footprint but is restored before commit.
pub fn contained_wall_gate(
    plan: &Plan,
    flags: u8,
    mut crosses: impl FnMut(sims_vm::ObjectId, u8) -> Result<bool, String>,
) -> Result<bool, String> {
    if let Some((parent, _)) = plan.container
        && flags & 4 != 0
    {
        return Ok(!crosses(parent, plan.direction)?);
    }
    Ok(true)
}

/// Native 0x4550a0 pre-acceptance gates. Reads room and wall services lazily;
/// failed room/wall gates leave the diagnostic error unchanged.
pub fn adjacency(
    size: usize,
    old: [i32; 3],
    room: i16,
    plan: &Plan,
    error: &mut i32,
    mut query: impl FnMut(
        crate::lot_route::candidate::Request,
    ) -> Result<crate::lot_route::candidate::Reply, String>,
) -> Result<bool, String> {
    use crate::lot_route::candidate::{Reply, Request};
    if plan.bypass_adjacency {
        return Ok(true);
    }
    let [x, y] = [(plan.position[1] >> 4) as i8, (plan.position[0] >> 4) as i8];
    let inside =
        |x: i8, y: i8| x > 0 && y > 0 && (x as usize) < size - 1 && (y as usize) < size - 1;
    if !inside(x, y) {
        *error = 1;
        return Ok(false);
    }
    let Reply::Room(destination_room) = query(Request::Room([x, y, plan.level as i8]))? else {
        return Err("Snap room reply mismatch".into());
    };
    if destination_room != room {
        return Ok(false);
    }
    let old_tile = [(old[1] >> 4) as i8, (old[0] >> 4) as i8, old[2] as i8];
    if !inside(old_tile[0], old_tile[1]) || [old[0], old[1]] == plan.position {
        return Ok(true);
    }
    let direction = sims_vm::routing::direction([old[0], old[1]], plan.position) as u8;
    for (tile, rotation) in [
        (old_tile, direction),
        ([x, y, old[2] as i8], (direction + 4) & 7),
    ] {
        let Reply::Walls(mask) = query(Request::Walls { tile, rotation })? else {
            return Err("Snap wall reply mismatch".into());
        };
        if mask & 1 != 0 {
            return Ok(false);
        }
    }
    Ok(true)
}
