use crate::{Simulation, geometry};
use serde::{Deserialize, Serialize};
use sims_vm::{Program, Status};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub version: u32,
    pub program_hash: String,
    pub state: Simulation,
}
impl Snapshot {
    pub fn capture(program: &Program, state: &Simulation) -> Self {
        Self {
            // Native owner/checkpoint layout is unchanged. Explicit fixture
            // passes need their own continuation schema; never infer it from
            // an old bare-thread checkpoint.
            version: if state.native_scheduler.is_some() {
                55
            } else {
                56
            },
            program_hash: program.fingerprint(),
            state: state.clone(),
        }
    }
    pub fn restore(self, program: &Program) -> Result<Simulation, String> {
        if self.version == 55 && self.state.native_scheduler.is_none() {
            if self.state.vm.lot_rng_scope.is_some() {
                return Err("saved lot RNG scope has no registry-pass owner".into());
            }
            return Err("legacy simulation snapshot has no original registry continuation; recreate the authored fixture or reload the original house".into());
        }
        if !matches!(self.version, 55 | 56) || self.program_hash != program.fingerprint() {
            return Err("room snapshot version/content mismatch".into());
        }
        // Reuse all VM stack, instruction, request and containment validation.
        let factories: Vec<_> = self
            .state
            .person_factories
            .values()
            .filter(|s| {
                s.parent.is_some()
                    && self
                        .state
                        .vm
                        .world
                        .objects
                        .contains_key(&s.factory.object())
            })
            .map(|s| &s.factory)
            .collect();
        let direct: Vec<_> = self
            .state
            .person_factories
            .values()
            .filter(|s| s.parent.is_none())
            .map(|s| &s.factory)
            .collect();
        let mut retired = self
            .state
            .native_creations
            .values()
            .map(|e| e.retired_creation(&self.state, program))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        for state in self.state.person_factories.values() {
            if let Some(binding) = state.retired_creation(&self.state, program)? {
                retired.push(binding);
            }
        }
        let mut retired_cursors = self
            .state
            .cursor_cleanup
            .as_ref()
            .map(|e| e.retired_references(&self.state, program))
            .transpose()?
            .unwrap_or_default();
        for (index, d) in self.state.deletion.iter().enumerate() {
            if !d.storage_unlinked {
                if let Some(e) = &d.cursor {
                    retired_cursors.extend(e.retired_references(&self.state, program, index)?);
                }
            }
        }
        let groups: Vec<_> = self
            .state
            .native_selection
            .as_ref()
            .and_then(crate::lot_selection::Execution::group_construction)
            .into_iter()
            .collect();
        sims_vm::Snapshot::capture(program, &self.state.vm).restore_with_group_construction(
            program,
            &factories,
            &direct,
            &retired,
            &retired_cursors,
            &groups,
        )?;
        let s = &self.state;
        if let Some(scheduler) = &s.native_scheduler {
            scheduler.validate(s)?;
        } else if s.vm.lot_rng_scope.is_some() {
            return Err("saved lot RNG scope has no registry-pass owner".into());
        } else if s.vm.scheduled_object_visit.is_some()
            && !s
                .cursor_factory
                .as_ref()
                .is_some_and(|f| f.owns_visit(&s.vm))
        {
            return Err("simulation native visit has no registry-pass owner".into());
        }
        if let Some(driver) = &s.fixture_driver {
            driver.validate(s)?;
        }
        s.validate_characters(program)?;
        s.validate_retired_characters(program)?;
        s.validate_retired_graphics(program)?;
        if s.visited.iter().any(|id| !s.vm.threads.contains_key(id))
            || s.visit.is_some_and(|(id, actor)| {
                !s.visited.contains(&id)
                    || s.vm
                        .threads
                        .get(&id)
                        .is_none_or(|t| t.stack.first().is_some_and(|f| Some(f.caller) != actor))
            })
        {
            return Err("invalid cooperative scheduler continuation".into());
        }
        s.validate_architecture(program)?;
        s.validate_geometry()?;
        s.validate_native_lot()?;
        if let Some(e) = &s.native_selection {
            e.validate(&s.vm, program)?;
        }
        s.validate_selection_notification_mode()?;
        s.validate_original_environment(program)?;
        s.validate_native_routing(program)?;
        if let Some(n) = &s.native_lot {
            if let Some(e) = &n.completion {
                e.validate(s, program)?;
            }
            if n.route_paths.iter().any(|(id, path)| {
                s.vm.world
                    .objects
                    .get(id)
                    .is_none_or(|o| o.person.is_none())
                    || path.len() > 65536
            }) {
                return Err("invalid retained native route path".into());
            }
            if let Some(e) = &n.find_location {
                e.validate(s, program)?;
            }
            if let Some(e) = &n.snap {
                e.validate(s, program)?;
            }
            if n.snap_offsets.iter().any(|(id, offset)| {
                s.vm.world
                    .objects
                    .get(id)
                    .is_none_or(|o| o.person.is_none())
                    || offset[2] != 0
                    || offset[..2].iter().any(|bits| {
                        let v = f32::from_bits(*bits);
                        !v.is_finite() || v.abs() > 32768. || v.fract() != 0.
                    })
            }) {
                return Err("invalid native Snap animator compensation".into());
            }
        }
        let mut spatial = s.clone();
        spatial.sync_spatial_world();
        if s.architecture.is_some()
            && (spatial.vm.world.positions != s.vm.world.positions
                || spatial.vm.world.room_outdoors != s.vm.world.room_outdoors
                || spatial.vm.world.room_half_tiles != s.vm.world.room_half_tiles
                || s.vm
                    .world
                    .objects
                    .iter()
                    .any(|(id, o)| o.data.get(&29) != spatial.vm.world.objects[id].data.get(&29)))
        {
            return Err("saved spatial cache mismatch".into());
        }

        s.validate_person_factories(program)?;
        if let Some(e) = &s.portrait_person {
            e.validate(s, program)?;
        }
        if let Some(e) = &s.synchronous_removal {
            e.validate(s, program)?;
        }
        if let Some(e) = &s.object_placement {
            e.validate(s, program)?;
        }
        if let Some(e) = &s.object_pickup {
            e.validate(s, program)?;
        }
        if let Some(e) = &s.ground_placement {
            e.validate(s, program)?;
        }
        if let Some(e) = &s.controller_reconciliation {
            e.validate(s, program)?;
        }
        if let Some(f) = &s.cursor_factory {
            f.validate(s, program)?;
        }
        if let Some(r) = &s.cursor_rotation {
            r.validate(s, program)?;
        }
        if let Some(r) = &s.cursor_release {
            r.validate(s, program)?;
        }
        if let Some(e) = &s.cursor_cleanup {
            e.validate(s, program)?;
        }
        s.validate_immediate(program)?;
        if let Some(c) = &s.ui_wait_callback {
            c.validate(s, program)?;
        }
        s.validate_cancellation(program)?;
        s.validate_creation(program)?;
        if let Some(e) = &s.native_burn {
            e.validate(s, program)?;
        }
        for t in s.vm.threads.values() {
            for status in std::iter::once(&t.status)
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
            {
                if let Status::AwaitingBurn(r) = status {
                    if r.started && s.native_burn.as_ref().is_none_or(|e| e.request() != r) {
                        return Err("orphan live Burn continuation".into());
                    }
                }
            }
        }
        for (&key, e) in &s.native_transfers {
            e.validate(s, program, key)?;
        }
        for (&key, e) in &s.native_reaches {
            e.validate(s, program, key)?;
        }
        s.validate_removals(program)?;
        s.validate_carry(program)?;
        s.validate_contention(program)?;
        let portals = s.portal_nodes();
        for (&actor, cache) in &s.portal_cache {
            if s.vm
                .world
                .objects
                .get(&actor)
                .is_none_or(|o| o.person.is_none())
            {
                return Err("portal cache without person".into());
            }
            cache.validate(&portals)?;
        }
        for state in s.route_control.values() {
            if state
                .last_portal
                .is_some_and(|id| !portals.iter().any(|p| p.id == id))
            {
                return Err("route bound to missing portal".into());
            }
        }
        for (&id, layer) in &s.backgrounds {
            if s.vm
                .world
                .objects
                .get(&id)
                .is_none_or(|o| o.person.is_none())
                || layer.started_tick > s.vm.tick
                || program
                    .animations
                    .get(&layer.clip)
                    .is_none_or(|a| a.clip.frames == 0 || !a.clip.events.is_empty())
            {
                return Err("invalid background animation layer".into());
            }
        }
        s.validate_animations(program)?;
        s.validate_reaches(program)?;
        s.validate_turns(program)?;
        s.validate_interactions(program)?;
        s.validate_household(program)?;
        if s.routes.len() > s.vm.threads.len()
            || s.reservations.len() > s.vm.world.objects.len()
            || s.pending_actions.len() > s.vm.threads.len()
        {
            return Err("oversized room snapshot collections".into());
        }
        for (&request_id, feedback) in &s.route_feedback {
            let thread =
                s.vm.threads
                    .get(&feedback.thread)
                    .ok_or("orphaned route feedback")?;
            let mut service = thread.service.as_ref();
            let mut bound = None;
            while let Some(call) = service {
                if matches!(&call.resume_status, Status::AwaitingRoute(r) if r.id==request_id) {
                    bound = Some(call);
                    break;
                }
                service = call.parent.as_deref();
            }
            let call = bound.ok_or("route feedback without service continuation")?;
            let Status::AwaitingRoute(request) = &call.resume_status else {
                unreachable!()
            };
            let named =
                s.vm.resolve_named_tree(program, request.caller, "Route failure feedback")?;
            if feedback.code == 0
                || request.no_failure_trees
                || call.callee != request.target
                || named.is_none_or(|(_, bhav)| bhav != call.bhav)
            {
                return Err("route feedback binding mismatch".into());
            }
        }
        for (&id, r) in &s.routes {
            let Some(t) = s.vm.threads.get(&id) else {
                return Err("orphaned route".into());
            };
            let request = crate::contention::pending_route(t, r.request)
                .ok_or("route without waiting continuation")?;
            let pending_movement = s
                .native_lot
                .as_ref()
                .and_then(|n| n.movement.as_ref())
                .filter(|m| m.thread == id);
            if request.id != r.request
                || s.native_lot.is_some()
                    && s.route_control
                        .get(&r.request)
                        .is_none_or(|c| c.original.is_none())
                || request.caller != r.caller
                || request.target != r.target
                || s.architecture.is_none() && s.reservations.get(&r.target) != Some(&id)
                || r.next == 0
                || r.next > r.path.len()
                || r.path.len() > 65536
                || r.last_tick > s.vm.tick
                || r.started_tick > r.last_tick
                || r.last_tick - r.started_tick != u64::from(r.locomotion.elapsed)
                || (pending_movement.is_none_or(|m| m.placement.is_none())
                    && s.body(r.caller)?.position != r.locomotion.anchor(&r.path))
                || r.path.last() != Some(&r.goal.position)
            {
                return Err("invalid route progress".into());
            }
            r.target_body.validate(&s.room)?;
            for p in &r.path {
                if !(0..s.room.width).contains(&p.x)
                    || !(0..s.room.height).contains(&p.y)
                    || !s.room.contains(s.body(r.caller)?.bounds_at(*p))
                {
                    return Err("route outside room".into());
                }
            }
            if r.path
                .windows(2)
                .any(|w| geometry::segment_distance(w[0], w[1]).is_none())
            {
                return Err("zero-length saved path segment".into());
            }
            let mut expected = crate::locomotion::Locomotion::plan(
                program,
                &r.path,
                r.locomotion.initial_facing,
                r.goal.facing,
            )?;
            expected.elapsed = r.locomotion.elapsed;
            if expected != r.locomotion
                || expected.elapsed > expected.duration()
                || expected.next() != r.next
                || pending_movement.map_or(expected.facing(), |m| m.locomotion.facing())
                    != s.body(r.caller)?.facing
            {
                return Err("invalid locomotion timeline".into());
            }
            if let Some(native) = &s.native_lot {
                // Imported paths use original fixed/dynamic arrays and live
                // floor acceptance, not authored-demo wall bounds. Protected
                // geometry remains explicitly experimental even in a save.
                if native.routing_policy != crate::lot_route::path::Policy::InferredRectangles {
                    return Err("native movement without inference policy".into());
                }
                let (slot, version) = crate::lot_route::live::descriptor(&request.destination)?;
                if !(4..=10).contains(&version)
                    || slot.kind != 3
                    || slot.offset[2] != 0.
                    || slot.height != 0
                    || slot.snap_target >= 0
                {
                    return Err("unsupported saved original point SLOT".into());
                }
                let valid = if slot.flags & 255 == 0 {
                    let point = crate::routing::PointCandidate::prepare(
                        [r.target_body.position.y, r.target_body.position.x],
                        [slot.offset[0], slot.offset[1]],
                        r.target_body.facing.trailing_zeros() as u8,
                        slot.flags,
                    )?;
                    r.goal.position == crate::Point::new(point.position[1], point.position[0])
                        && r.goal.entry_flags == 0
                } else {
                    let mut actor = s.body(r.caller)?.clone();
                    actor.facing = r.goal.facing;
                    let descriptor =
                        crate::routing::standing_slot(&slot, version, &r.target_body, &actor)?;
                    // Validate membership in the descriptor's geometric lattice.
                    // Earlier clearance callback outcomes are not rerun on restore.
                    let set = crate::routing::posture_candidates(
                        &descriptor,
                        slot.sitting,
                        &mut 0,
                        |_| None,
                        |_, _| 6,
                    )?
                    .0;
                    set.candidates.iter().any(|c| {
                        crate::Point::new(c.position[1], c.position[0]) == r.goal.position
                            && c.flags as i16 == r.goal.entry_flags
                    })
                };
                let original = r
                    .original_path
                    .as_ref()
                    .ok_or("native search path absent")?;
                let mut simplified = original.clone();
                simplified.dedup();
                if original.len() > 65536 || simplified != r.path {
                    return Err("native search path differs from locomotion path".into());
                }
                let points: Vec<_> = original.iter().map(|p| [p.y, p.x]).collect();
                let look = crate::routing::endpoint_look(
                    &points,
                    [r.target_body.position.y, r.target_body.position.x],
                    r.target_body.facing.trailing_zeros() as u8,
                    slot.flags,
                    slot.facing,
                    r.locomotion.initial_facing.trailing_zeros() as u8,
                )?
                .ok_or("saved native route lacks endpoint look")?;
                let mut retained = points.clone();
                retained.push(look);
                if native.route_paths.get(&r.caller) != Some(&retained) {
                    return Err("native retained path differs from active walk".into());
                }
                let facing =
                    1 << sims_vm::routing::direction([r.goal.position.y, r.goal.position.x], look);
                if !valid
                    || r.goal.facing != facing
                    || r.target_body.level != s.body(r.caller)?.level
                {
                    return Err("original route goal does not match descriptor".into());
                }
                continue;
            }
            if r.original_path.is_some() {
                return Err("native search path without original lot".into());
            }
            if r.path.windows(2).any(|w| {
                !geometry::segment_clear(w[0], w[1], |p| {
                    let bounds = s.collision_bounds(r.caller, p);
                    s.room.contains(bounds)
                        && (s.bodies[&r.caller].level != 1
                            || !s.room.obstacles.iter().any(|o| o.overlaps(bounds)))
                        && s.architecture.as_ref().is_none_or(|a| {
                            a.bounds_clear(
                                s.bodies[&r.caller].level,
                                bounds,
                                &s.active_portals(r.caller),
                            )
                        })
                })
            }) {
                return Err("saved route cuts through static geometry".into());
            }
            let mut initial_body = s.body(r.caller)?.clone();
            initial_body.facing = r.locomotion.initial_facing;
            initial_body.position = r.path[0];
            // A persisted locomotion route is a standing candidate; seated
            // candidates execute the original Sit service before locomotion.
            let mut standing = request.destination.clone();
            if let sims_vm::routing::Destination::Slot { slot, .. } = &mut standing {
                slot.sitting = 0;
            }
            // Lot routes prefer native SLOT enumeration even when the older
            // laboratory geometry can also describe that SLOT. Nonzero offsets
            // must not collapse a native candidate lattice into a Snap point.
            let goals = if let sims_vm::routing::Destination::Slot { slot, version, .. } = &standing
                && s.architecture.is_some()
                && slot.flags & 255 != 0
                && let Ok(context) =
                    crate::routing::standing_slot(slot, *version, &r.target_body, &initial_body)
            {
                crate::routing::standing_candidates(&context, &mut 0, |_, _| 6)?
                    .candidates
                    .iter()
                    .map(|c| crate::routing::candidate_destination(&context, c))
                    .collect()
            } else {
                crate::routing::destinations(&standing, &r.target_body, &initial_body)?
            };
            if !goals.contains(&r.goal) {
                return Err(format!(
                    "route goal does not match destination descriptor: thread {id}, goal {:?}, descriptor {:?}",
                    r.goal, standing
                ));
            }
        }
        for (&id, object) in &s.vm.world.objects {
            if let Some((parent, index)) = object.container {
                let parent_object = &s.vm.world.objects[&parent];
                let slot = if s.native_lot.is_some() {
                    let slots =
                        crate::lot_acceptance::contained::slots(&s.vm.world, program, parent)?;
                    if slots
                        .get(usize::from(index))
                        .is_none_or(|slot| slot.occupant != id)
                    {
                        return Err("native containment slot binding mismatch".into());
                    }
                    None
                } else if parent_object.person.is_some() {
                    if index as usize >= sims_vm::person::BUILTIN_SLOT_HEIGHTS.len()
                        || object.person.is_some()
                    {
                        return Err("unsupported built-in person containment slot".into());
                    }
                    None
                } else {
                    let def = parent_object
                        .definition
                        .as_ref()
                        .ok_or("container without definition")?;
                    Some(
                        program
                            .modules
                            .get(&def.source)
                            .and_then(|m| m.slots.get(&def.slots))
                            .and_then(|slots| {
                                slots
                                    .entries
                                    .iter()
                                    .filter(|slot| slot.kind == 0)
                                    .nth(index as usize)
                            })
                            .ok_or("missing containment slot")?,
                    )
                };
                if s.unplaced.contains(&id) || s.unplaced.contains(&parent) {
                    if !s.unplaced.contains(&id) || !s.unplaced.contains(&parent) {
                        return Err(
                            "contained object and parent disagree on off-lot placement".into()
                        );
                    }
                    continue;
                }
                let body = s.body(id)?;
                // Native physical coordinates remain at the parent anchor;
                // containment offsets belong to the animator/render transform.
                let (position, elevation) = if object.person.is_none() || s.native_lot.is_some() {
                    let parent = s.body(parent)?;
                    (parent.position, parent.elevation)
                } else {
                    let slot = slot.ok_or("person cannot be carried in built-in slot")?;
                    let height = match slot.height {
                        0 | 1 | 5 => 0,
                        7 => 7,
                        _ => return Err("unsupported saved containment height".into()),
                    };
                    (geometry::offset(slot, s.body(parent)?)?, height)
                };
                if body.position != position
                    || body.elevation != elevation
                    || body.level != s.body(parent)?.level
                {
                    return Err("saved containment pose disagrees with SLOT".into());
                }
            }
        }
        for (&target, &thread) in &s.reservations {
            let t =
                s.vm.threads
                    .get(&thread)
                    .ok_or("reservation without thread")?;
            // A route may target Stack Object while executing another object's
            // behavior (food cleanup routes to its newly created trash pile).
            // The reservation also outlives that route instruction/frame.
            if !s.vm.world.objects.contains_key(&target)
                || (t.check
                    && !s.immediate.as_ref().is_some_and(|e| e.thread == thread)
                    && !s
                        .reach_callback
                        .as_ref()
                        .is_some_and(|e| e.thread == thread)
                    && !s.cancellation.as_ref().is_some_and(|e| e.thread == thread))
                || matches!(t.status, Status::Returned(_) | Status::Faulted(_))
            {
                return Err(format!(
                    "invalid reservation owner: target {target}, thread {thread}, tick {}, check {}, callees {:?}",
                    s.vm.tick,
                    t.check,
                    t.stack.iter().map(|f| f.callee).collect::<Vec<_>>()
                ));
            }
        }
        for (&id, action) in &s.pending_actions {
            let t =
                s.vm.threads
                    .get(&id)
                    .ok_or("queued action without check thread")?;
            if !t.check
                || !s.vm.world.objects.contains_key(&action.actor)
                || !s.vm.world.objects.contains_key(&action.target)
            {
                return Err("invalid queued action".into());
            }
            let (_, routine) = program.resolve(&action.owner, action.bhav)?;
            if action.args.len() > usize::from(routine.body.args).max(4) {
                return Err("invalid queued action arguments".into());
            }
            if let Some(f) = t.stack.first()
                && (f.caller != action.actor
                    || f.callee != action.target
                    || f.code_owner != action.owner)
            {
                return Err("queued action/check context mismatch".into());
            }
        }
        Ok(self.state)
    }
    pub fn from_json(bytes: &[u8], program: &Program) -> Result<Simulation, String> {
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("room snapshot exceeds 16 MiB limit".into());
        }
        serde_json::from_slice::<Self>(bytes)
            .map_err(|e| e.to_string())?
            .restore(program)
    }
}
