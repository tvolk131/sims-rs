//! Original object factories and Create Object placement. Recovered ordering
//! is preserved across diagnostic budget slices; unsupported placement and
//! subclass services remain explicit boundaries.
use crate::{Body, Simulation, Step};
use sims_vm::{Definition, ObjectId, Program, Status, ThreadId, create::Request};

impl Simulation {
    pub(crate) fn service_creation(
        &mut self,
        p: &Program,
        thread: ThreadId,
        mut r: Request,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if let Some(done) =
            self.service_native_person_creation(p, thread, r.clone(), remaining, step)?
        {
            return Ok(done);
        }
        if self.native_creations.contains_key(&r.id) {
            return self.drive_native_creation(p, r.id, remaining, step);
        }
        if self.native_lot.is_some()
            && matches!(r.operands[4], 0 | 2 | 3 | 4 | 5 | 7 | 8 | 9)
            && r.operands[5] & !0x3b == 0
        {
            return self.service_native_placed_creation(p, thread, r, remaining, step);
        }
        let rotate_off_lot = self.native_lot.is_some() && r.operands[4] == 6 && r.operands[5] == 32;
        let check_off_lot = self.native_lot.is_some() && r.operands[4] == 6 && r.operands[5] == 1;
        if self.native_lot.is_some()
            && (r.operands[4] != 6 || r.operands[5] != 0)
            && !rotate_off_lot
            && !check_off_lot
        {
            return Err(format!(
                "original-lot creation placement adapter required: mode {} flags {:#x}",
                r.operands[4], r.operands[5]
            ));
        }
        let guid = u32::from_le_bytes(r.operands[..4].try_into().unwrap());
        let off_lot = r.operands[4] == 6;
        let floor = matches!(r.operands[4], 0 | 3 | 5 | 7 | 8 | 9);
        if !matches!(r.operands[4], 0 | 3 | 4 | 5 | 6 | 7 | 8 | 9)
            || (!rotate_off_lot
                && (r.operands[5] & !9 != 0 || (off_lot && r.operands[5] != 0 && !check_off_lot)))
        {
            return Err(format!(
                "Create Object placement mode {} flags {:#x} unsupported",
                r.operands[4], r.operands[5]
            ));
        }
        if r.created.is_none() {
            // Resource resolution precedes the placement-target lookup.
            let mut definitions = p.modules.values().flat_map(|m| {
                m.object_definition_words
                    .iter()
                    .filter_map(move |(&id, w)| {
                        (w.len() >= 0x84 / 2
                            && ((w[14] as u16 as u32) | ((w[15] as u16 as u32) << 16)) == guid)
                            .then_some((m, id, w))
                    })
            });
            let Some((module, mut number, mut words)) = definitions.next() else {
                // The content host must distinguish an absent GUID from an unloaded
                // resource. A partial asset catalog cannot truthfully return false.
                return Err(format!("Create Object GUID {guid:08X} is not loaded"));
            };
            if definitions.next().is_some() {
                return Err("ambiguous Create Object GUID".into());
            }
            // Mode 6 still performs the flag-1 duplicate query, using its
            // native off-lot tile (-1,-1,1), before entering the factory.
            if check_off_lot
                && crate::lot_creation::live::duplicate_at(
                    self,
                    [-1, -1, 1],
                    &module.source,
                    number,
                )?
            {
                self.vm.finish_creation(p, thread, r.id, None)?;
                return Ok(true);
            }
            let group = words[10] != 0;
            if group && self.native_lot.is_some() && off_lot {
                return self.service_native_group_creation(
                    p,
                    thread,
                    r,
                    &module.source,
                    number,
                    remaining,
                    step,
                );
            }
            if group {
                let plan = sims_vm::multitile::construction_plan(module, number)?;
                if plan.len() != 1 {
                    return Err("dynamic groups with multiple parts are not implemented".into());
                }
                number = plan[0];
                words = &module.object_definition_words[&number];
            }
            let placement_target = self.vm.creation_placement_target(thread, &r)?;
            if !off_lot && !self.vm.world.objects.contains_key(&placement_target) {
                self.vm.finish_creation(p, thread, r.id, None)?;
                return Ok(true);
            }
            if !off_lot {
                let parent_def = self.vm.world.objects[&placement_target]
                    .definition
                    .as_ref()
                    .ok_or("creation parent definition unavailable")?;
                if module
                    .object_functions
                    .get(&number)
                    .and_then(|f| f.get(9))
                    .is_some_and(|f| f.action != 0)
                    || p.modules[&parent_def.source]
                        .object_functions
                        .get(&parent_def.object)
                        .and_then(|f| f.get(9))
                        .is_some_and(|f| f.action != 0)
                {
                    return Err(
                        "dynamic creation with placement callbacks is not implemented".into(),
                    );
                }
                let mut target = self.body(placement_target)?.clone();
                if matches!(r.operands[4], 0 | 3) {
                    let xy = sims_vm::create::in_front(
                        [target.position.y, target.position.x],
                        target.facing.trailing_zeros() as u8,
                    );
                    target.position = crate::Point::new(xy[1], xy[0]);
                }
                let mut direction = None;
                if r.operands[4] == 9 {
                    let f = self.vm.threads[&thread].stack.last().unwrap();
                    let i = r.operands[6] as i8;
                    let rotation = *usize::try_from(i)
                        .ok()
                        .and_then(|i| f.locals.get(i))
                        .ok_or("creation rotation local invalid (native error 0x08)")?;
                    if f.opaque_locals.contains(&(i as usize)) {
                        return Err("creation reads opaque local".into());
                    }
                    let (xy, d) = sims_vm::create::relative_floor(
                        [target.position.y, target.position.x],
                        self.vm.world.objects[&r.caller].data[&1],
                        rotation,
                    );
                    target.position = crate::Point::new(xy[1], xy[0]);
                    direction = Some(d);
                }
                if direction.is_some_and(|d| {
                    self.architecture
                        .as_ref()
                        .is_some_and(|a| a.wall_mask(target.level, target.position, d) & 1 != 0)
                }) {
                    self.vm.finish_creation(p, thread, r.id, None)?;
                    return Ok(true);
                }
                let room = self
                    .architecture
                    .as_ref()
                    .map_or(self.vm.world.objects[&placement_target].data[&29], |a| {
                        a.room_at(target.level, target.position)
                    });
                r.placement = Some(sims_vm::create::Placement {
                    target: placement_target,
                    position: [target.position.x, target.position.y],
                    level: target.level,
                    room,
                    wall_direction: direction,
                });
                if floor
                    && (target.position.x < 0
                        || target.position.y < 0
                        || target.position.x >= self.room.width
                        || target.position.y >= self.room.height)
                {
                    self.vm.finish_creation(p, thread, r.id, None)?;
                    return Ok(true);
                }
                if r.operands[5] & 8 != 0
                    && self.bodies.iter().any(|(id, b)| {
                        self.vm.world.objects[id].container.is_none()
                            && b.level == target.level
                            && b.position.x / 16 == target.position.x / 16
                            && b.position.y / 16 == target.position.y / 16
                    })
                {
                    self.vm.finish_creation(p, thread, r.id, None)?;
                    return Ok(true);
                }
                if r.operands[5] & 1 != 0 {
                    for (&id, b) in &self.bodies {
                        if b.level == target.level
                            && b.position.x / 16 == target.position.x / 16
                            && b.position.y / 16 == target.position.y / 16
                            && self.vm.world.objects[&id].definition.is_some()
                            && sims_vm::multitile::is_type(&self.vm.world, p, id, guid)?
                        {
                            self.vm.finish_creation(p, thread, r.id, None)?;
                            return Ok(true);
                        }
                    }
                }
            }
            if [2, 8, 9].contains(&words[0x12 / 2]) {
                return Err("Create Object needs unsupported person/multitile subclass".into());
            }
            let Some(id) = (1..=i16::MAX).find(|id| !self.vm.world.objects.contains_key(id)) else {
                self.vm.finish_creation(p, thread, r.id, None)?;
                return Ok(true);
            };
            self.initialize_object_factory(p, id, &module.source, number, group)?;
            r.created = Some(id);
            self.vm.threads.get_mut(&thread).unwrap().status = Status::AwaitingCreation(r.clone());
            self.event(
                &mut step.events,
                thread,
                format!("registered #{id} off-lot; starting original Init/Main"),
            );
        }
        let id = r.created.unwrap();
        if !self.drive_object_factory(p, id, remaining, step)? {
            return Ok(false);
        }
        self.event(
            &mut step.events,
            thread,
            format!("factory completed #{id}; native creation signal 0xe2 before placement"),
        );
        if off_lot {
            // Mode 6 returns the registered object without a placement check.
            // Native physics remains at (-16,-16), level 1 and room -5; the
            // bounded host represents that position by absence of a lot body.
            if let Some(native) = &mut self.native_lot {
                native
                    .observers
                    .push(crate::lot_placement::Request::Signal {
                        event: 0xe2,
                        value: id,
                    });
            }
            if rotate_off_lot {
                return self.finish_native_rotated_creation(p, thread, r, remaining, step);
            }
            self.vm.finish_creation(p, thread, r.id, Some(id))?;
            return Ok(true);
        }
        if floor {
            // The authored collision profile currently supplies nonblocking
            // puddles. General native floor footprints/placement are separate.
            if ![0x5c67fc8f, 0x3e7470f6, 0x000007c4, 0xef046632, 0x3929aadc].contains(&guid) {
                return Err(format!(
                    "floor footprint {guid:08X} {} flags {} not in the authored household profile",
                    self.vm.world.objects[&id]
                        .definition
                        .as_ref()
                        .unwrap()
                        .source,
                    self.vm.world.objects[&id].data[&8]
                ));
            }
            let placement = r.placement.as_ref().ok_or("creation placement lost")?;
            let room = placement.room;
            let body = Body {
                level: placement.level,
                position: crate::Point::new(placement.position[0], placement.position[1]),
                facing: 1 << self.vm.world.objects[&id].data[&1],
                radius: 0,
                elevation: 0,
            };
            body.validate(&self.room)?;
            if guid == 0x3929aadc {
                // ReserveTile claims a physical footprint for the original
                // exhaustion routine. It must pass placement before committing.
                self.bodies.insert(id, body.clone());
                let accepted = self.clear(id, body.position, None);
                self.bodies.remove(&id);
                if !accepted {
                    self.discard_unplaced(id)?;
                    self.vm.finish_creation(p, thread, r.id, None)?;
                    return Ok(true);
                }
            }
            self.vm.world.objects.get_mut(&id).unwrap().data.extend([
                (2, 0),
                (3, -1),
                (14, -1),
                (26, 0),
                (29, room),
            ]);
            self.bodies.insert(id, body);
            self.unplaced.remove(&id);
            self.advertise_placed_object(id);
            self.vm.finish_creation(p, thread, r.id, Some(id))?;
            self.event(
                &mut step.events,
                thread,
                format!("created floor overlay #{id} at caller location"),
            );
            return Ok(true);
        }
        // Native checks placement after Init and its initial Main have run. This
        // bounded host implements containment slots; native floor placement is separate.
        let parent = &self.vm.world.objects[&r.target];
        if parent.slots.first() != Some(&None) {
            self.discard_unplaced(id)?;
            self.vm.finish_creation(p, thread, r.id, None)?;
            self.event(
                &mut step.events,
                thread,
                format!("placement failed; deleted newly created #{id}"),
            );
            return Ok(true);
        }
        let def = parent
            .definition
            .as_ref()
            .ok_or("creation container has no definition")?;
        p.modules[&def.source]
            .slots
            .get(&def.slots)
            .and_then(|s| s.entries.iter().find(|s| s.kind == 0))
            .ok_or("creation container lacks SLOT zero")?;
        let parent_body = self.body(r.target)?.clone();
        // Native 0x433340 places contained physics at the parent's position.
        // SLOT floating offsets are applied by presentation, not this integer body.
        let position = parent_body.position;
        let elevation = parent_body.elevation;
        let direction = self.vm.world.objects[&id].data[&1];
        let body = Body {
            level: parent_body.level,
            position,
            facing: 1 << direction,
            radius: 1,
            elevation,
        };
        body.validate(&self.room)?;
        let room = self.vm.world.objects[&r.target].data[&29];
        self.vm.world.objects.get_mut(&r.target).unwrap().slots[0] = Some(id);
        let object = self.vm.world.objects.get_mut(&id).unwrap();
        object.container = Some((r.target, 0));
        object
            .data
            .extend([(2, r.target), (3, 0), (14, 1), (26, r.target), (29, room)]);
        self.bodies.insert(id, body);
        self.unplaced.remove(&id);
        self.advertise_placed_object(id);
        self.vm.finish_creation(p, thread, r.id, Some(id))?;
        self.event(
            &mut step.events,
            thread,
            format!("created #{id} in #{} slot 0", r.target),
        );
        Ok(true)
    }

    pub(crate) fn validate_creation(&self, p: &Program) -> Result<(), String> {
        let mut native_objects = std::collections::BTreeSet::new();
        for (&key, e) in &self.native_creations {
            let (request, object) = e.binding();
            if key != request
                || !native_objects.insert(object)
                || e.parts()
                    .iter()
                    .any(|&id| id != object && !native_objects.insert(id))
            {
                return Err("duplicate or mismatched native creation continuation".into());
            }
            e.validate(self, p)?;
        }
        // Direct engine factories register off-lot storage before Init creates
        // a thread, just like script factories. Their saved owner is validated
        // by validate_person_factories rather than an AwaitingCreation frame.
        let mut pending: std::collections::BTreeSet<_> = self
            .person_factories
            .iter()
            .filter_map(|(&id, state)| (state.parent.is_none() && !state.finalized).then_some(id))
            .collect();
        if let Some(e) = self.selection_service() {
            e.validate(&self.vm, p)?;
            for id in e.constructing_objects() {
                if !pending.insert(id) {
                    return Err("registered factory part has multiple owners".into());
                }
            }
        }
        for (&thread, t) in &self.vm.threads {
            if let Status::AwaitingCreation(r) = &t.status {
                if self.native_creations.contains_key(&r.id) {
                    continue;
                }
                if self.native_lot.is_some()
                    && r.created.is_some()
                    && (matches!(r.operands[4], 0 | 2 | 3 | 4 | 5 | 7 | 8)
                        && !r
                            .created
                            .is_some_and(|id| self.person_factories.contains_key(&id))
                        || r.created
                            .and_then(|id| self.vm.world.objects.get(&id))
                            .is_some_and(|o| o.multitile.is_some()))
                {
                    return Err("orphan native creation continuation".into());
                }
                if let Some(id) = r.created {
                    if let Some(factory) = self.person_factories.get(&id) {
                        if !self.vm.world.objects.contains_key(&id)
                            && factory.retired_creation(self, p)? != Some((thread, r.id, id))
                        {
                            return Err("pending retired person factory binding differs".into());
                        }
                        if factory.request != r.id || factory.finalized || !pending.insert(id) {
                            return Err("invalid pending person factory".into());
                        }
                        continue;
                    }
                    if (r.operands[4] != 6) != r.placement.is_some() {
                        return Err("missing factory placement continuation".into());
                    }
                    if r.placement.as_ref().is_some_and(|p| p.level != 1) {
                        return Err("unsupported factory storey".into());
                    }
                    if !pending.insert(id)
                        || !self.unplaced.contains(&id)
                        || self.vm.world.objects[&id].container.is_some()
                        || self.vm.world.objects[&id]
                            .definition
                            .as_ref()
                            .is_none_or(|_| {
                                !sims_vm::multitile::is_type(
                                    &self.vm.world,
                                    p,
                                    id,
                                    u32::from_le_bytes(r.operands[..4].try_into().unwrap()),
                                )
                                .unwrap_or(false)
                            })
                    {
                        return Err("invalid pending object factory".into());
                    }
                }
            }
        }
        for id in &self.unplaced {
            if !pending.contains(id) {
                let object = &self.vm.world.objects[id];
                if object
                    .container
                    .is_some_and(|(parent, _)| !self.unplaced.contains(&parent))
                    || object.data.get(&29) != Some(&-5)
                    || !self.vm.object_threads.contains_key(id)
                        && !self
                            .cursor_factory
                            .as_ref()
                            .is_some_and(|f| f.owns_uninitialized(&self.vm, *id))
                {
                    return Err("invalid registered off-lot object".into());
                }
            }
        }
        Ok(())
    }

    pub(crate) fn discard_unplaced(&mut self, id: ObjectId) -> Result<(), String> {
        if !self.unplaced.remove(&id) {
            return Err("factory cleanup requires an off-lot object".into());
        }
        self.vm.world.unregister_scheduler_object(id)?;
        self.vm.world.objects.remove(&id);
        if let Some(thread) = self.vm.object_threads.remove(&id) {
            self.vm.threads.remove(&thread);
        }
        self.vm.lifecycles.remove(&id);
        if let Some(person) = &mut self.vm.world.person {
            person.object_order.retain(|&object| object != id);
            for order in person.resource_instances.values_mut() {
                order.retain(|&o| o != id);
            }
            for chain in &mut person.tile_chains {
                chain.retain(|&object| object != id);
            }
            person.tile_chains.retain(|chain| !chain.is_empty());
        }
        self.vm.world.invalidate_registry_order();
        if let Some(refresh) = &mut self.vm.world.refresh {
            refresh.draw.remove(&id);
            refresh.invalidated.remove(&id);
        }
        Ok(())
    }
}

impl Simulation {
    /// Common native factory path, also used by the routing engine's temporary
    /// GoHere object. Its Init/Main still execute on the original frame arena.
    pub(crate) fn initialize_object_factory(
        &mut self,
        p: &Program,
        id: ObjectId,
        source: &str,
        number: u16,
        group: bool,
    ) -> Result<(), String> {
        let module = &p.modules[source];
        let words = &module.object_definition_words[&number];
        let definition = Definition {
            source: module.source.clone(),
            object: number,
            guid: words[14] as u16 as u32 | ((words[15] as u16 as u32) << 16),
            slots: words[0x28 / 2] as u16,
        };
        let date = self
            .household
            .as_ref()
            .ok_or("dynamic factory requires lot calendar")?;
        let now = sims_vm::construction::ObjectDate {
            year: date.clock.year,
            month: date.clock.month,
            day: date.clock.day,
        };
        self.vm.begin_object_factory(
            p,
            id,
            definition,
            group,
            now,
            [date.clock.hour, date.clock.minute],
        )?;
        if let Some(native) = &mut self.native_lot {
            native.placement.links.next.insert(id, 0);
            native
                .placement
                .postconstruct_bounds(&self.vm.world, p, id)?;
        }
        self.unplaced.insert(id);
        Ok(())
    }
    pub(crate) fn drive_object_factory(
        &mut self,
        p: &Program,
        id: ObjectId,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if !self.drive_factory_update(p, id, remaining, step)? {
            return Ok(false);
        }
        self.vm.finish_object_factory(p, id)?;
        Ok(true)
    }
    /// Shared synchronous update, without the ordinary factory's later price
    /// store. Multipart factories set their catalog price before all Init calls.
    pub(crate) fn drive_factory_update(
        &mut self,
        p: &Program,
        id: ObjectId,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let child = self.vm.object_threads[&id];
        // reset(1): synchronous Init, then one immediate native object update.
        loop {
            if *remaining == 0 {
                return Ok(false);
            }
            self.vm.prepare_factory_object_visit(id)?;
            let budget = if self.vm.scheduled_object_visit.is_some()
                && self.vm.threads[&child].callback.is_some()
            {
                (*remaining).min(1)
            } else {
                *remaining
            };
            let run = self.vm.run_slice(p, child, budget)?;
            *remaining -= run.trace.len();
            self.publish_budget_statistics();
            step.trace.extend(run.trace);
            match self.vm.threads[&child].status.clone() {
                Status::Ready => continue,
                Status::Sleeping { .. } | Status::Returned(_) => break,
                Status::Faulted(f) => {
                    return Err(format!("created object Init/Main failed: {}", f.message));
                }
                Status::AwaitingCreation(request) => {
                    if !self.service_creation(p, child, request, remaining, step)? {
                        return Ok(false);
                    }
                }
                Status::AwaitingRemoval(request) => {
                    if !self.service_removal(p, child, &request, remaining, step)? {
                        return Ok(false);
                    }
                }
                Status::AwaitingPlacement(request) => {
                    if self.native_lot.is_some() {
                        return Err(
                            "original-lot factory requires native Find Location adapter".into()
                        );
                    }
                    self.service_find_location(p, child, &request, &mut step.events)?;
                }
                Status::AwaitingTransfer(request) => {
                    if self.native_lot.is_some() {
                        if !self.service_native_transfer(p, child, &request, remaining, step)? {
                            return Ok(false);
                        }
                        continue;
                    }
                    self.service_transfer(p, child, &request, &mut step.events)?;
                    self.sync_carry(p)?;
                }
                status => {
                    return Err(format!(
                        "created object's immediate update yielded unsupported host service: {status:?}"
                    ));
                }
            }
        }
        Ok(true)
    }
}
