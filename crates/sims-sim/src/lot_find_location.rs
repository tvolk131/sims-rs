//! Native Find Location search and physical detachment on imported lots. Search,
//! repeated acceptance and ordered commit are synchronous across budget slices.
use crate::{
    Simulation, Step, lot_acceptance,
    lot_links::Descriptor,
    lot_placement,
    placement::{RandomOrigin, SearchTile, Spiral, search_tile_allowed},
};
use serde::{Deserialize, Serialize};
use sims_vm::{Program, Status, ThreadId, Vm, placement::Request};

/// 44b3c8 checks the selected object's master word, runtime group type, and
/// function 8 before virtual removal. A split is a separate operation: never
/// silently remove the whole group when the original would first unlink a part.
pub fn detachment_operation(
    world: &sims_vm::World,
    p: &Program,
    links: &crate::lot_links::Links,
    target: sims_vm::ObjectId,
) -> Result<lot_placement::VirtualOperation, String> {
    if requires_dynamic_split(world, p, target)? {
        return Err("native dynamic-group split before detachment required".into());
    }
    validate_detachment_tree(world, p, target)?;
    if world.objects[&target].multitile.is_some() {
        lot_placement::VirtualOperation::remove(world, links, target, true)
    } else {
        // cXPerson's RemoveFromLot override is the shared base operation too.
        Ok(lot_placement::VirtualOperation::Base(
            lot_placement::Operation::remove(target, true),
        ))
    }
}

fn requires_dynamic_split(
    world: &sims_vm::World,
    p: &Program,
    target: sims_vm::ObjectId,
) -> Result<bool, String> {
    let object = world
        .objects
        .get(&target)
        .ok_or("detachment target absent")?;
    let definition = object
        .definition
        .as_ref()
        .ok_or("detachment definition absent")?;
    let module = p
        .modules
        .get(&definition.source)
        .ok_or("detachment module absent")?;
    let words = module
        .object_definition_words
        .get(&definition.object)
        .ok_or("detachment definition words absent")?;
    Ok(*words.get(10).ok_or("detachment master word absent")? != 0
        && object.multitile.is_some()
        && module
            .object_functions
            .get(&definition.object)
            .ok_or("detachment functions absent")?
            .get(8)
            .is_some_and(|f| f.action != 0))
}

fn validate_detachment_tree(
    world: &sims_vm::World,
    p: &Program,
    target: sims_vm::ObjectId,
) -> Result<(), String> {
    let members = if world
        .objects
        .get(&target)
        .is_some_and(|o| o.multitile.is_some())
    {
        sims_vm::multitile::chain(world, target)?
    } else {
        vec![target]
    };
    let mut pending: Vec<_> = members.into_iter().map(|id| (id, true)).collect();
    let mut seen = std::collections::BTreeSet::new();
    while let Some((id, member)) = pending.pop() {
        if !seen.insert(id) {
            return Err("cyclic native detachment subtree".into());
        }
        let object = world.objects.get(&id).ok_or("detachment object absent")?;
        let definition = object
            .definition
            .as_ref()
            .ok_or("detachment definition absent")?;
        let words = p
            .modules
            .get(&definition.source)
            .and_then(|m| m.object_definition_words.get(&definition.object))
            .ok_or("detachment definition words absent")?;
        if words.get(9).copied().ok_or("detachment type word absent")? == 9
            || object.multitile.is_some() && object.person.is_some()
            || !member && (object.multitile.is_some() || words.get(10).copied() != Some(0))
        {
            return Err("native special-object/contained-group detachment adapter required".into());
        }
        pending.extend(object.slots.iter().flatten().map(|&id| (id, false)));
    }
    Ok(())
}

/// Person virtual +0x74 (0x45e519): clear word 64 after the animator reset,
/// then select pool posture only inside the native signed-byte interior bounds.
pub fn refresh_pool_profile(size: usize, position: [i32; 3], pool: &[u8]) -> Result<i16, String> {
    if !(3..=64).contains(&size) || pool.len() != size * size {
        return Err("invalid appearance pool grid".into());
    }
    let [x, y] = [(position[1] >> 4) as i8, (position[0] >> 4) as i8];
    Ok(i16::from(
        position[2] as i8 == 1
            && x > 0
            && y > 0
            && (x as usize) < size - 1
            && (y as usize) < size - 1
            && pool[x as usize * size + y as usize] < 16,
    ))
}

/// Detachment owns the same synchronous placement callbacks as SetObject,
/// but it performs no search, appearance reset, or deferred object deletion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Execution {
    Search(Search),
    Detach {
        thread: ThreadId,
        request: Request,
        placement: lot_placement::live::Execution,
    },
    DynamicDetach {
        thread: ThreadId,
        request: Request,
        split: crate::lot_dynamic::live::Execution,
        placement: Option<lot_placement::live::Execution>,
    },
}
impl Execution {
    pub fn begin(
        s: &mut Simulation,
        p: &Program,
        thread: ThreadId,
        r: &Request,
    ) -> Result<Self, String> {
        if r.operands[0] != 1 {
            return Search::begin(s, thread, r).map(Self::Search);
        }
        if r.operands[2] & !7 != 0 {
            return Err("unsupported native detachment flags".into());
        }
        if requires_dynamic_split(&s.vm.world, p, r.target)? {
            validate_detachment_tree(&s.vm.world, p, r.target)?;
            return Ok(Self::DynamicDetach {
                thread,
                request: r.clone(),
                split: crate::lot_dynamic::live::Execution::begin_split(s, r.target)?,
                placement: None,
            });
        }
        let native = s
            .native_lot
            .as_ref()
            .ok_or("Find Location native host absent")?;
        let operation = detachment_operation(&s.vm.world, p, &native.placement.links, r.target)?;
        // No acceptance query runs in this branch. 0x422022 initializes a
        // local tile coordinate, not the global placement error/blocker.
        Ok(Self::Detach {
            thread,
            request: r.clone(),
            placement: lot_placement::live::Execution::new_virtual(operation),
        })
    }
    pub fn binding(&self) -> (ThreadId, &Request) {
        match self {
            Self::Search(e) => (e.thread, &e.request),
            Self::Detach {
                thread, request, ..
            }
            | Self::DynamicDetach {
                thread, request, ..
            } => (*thread, request),
        }
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        match self {
            Self::Search(e) => e.owns_callback(vm, p, thread, depth),
            Self::Detach { placement, .. } => placement.owns_callback(vm, p, thread, depth),
            Self::DynamicDetach {
                split, placement, ..
            } => {
                split.owns_callback(vm, p, thread, depth)
                    || placement
                        .as_ref()
                        .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            }
        }
    }
    pub fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        let (thread, request, placement) = match self {
            Self::Search(search) => return search.validate(s, p),
            Self::Detach {
                thread,
                request,
                placement,
            } => (thread, request, Some(placement)),
            Self::DynamicDetach {
                thread,
                request,
                split,
                placement,
            } => {
                split.validate(&s.vm, p)?;
                if split.binding() != request.target || placement.is_some() && !split.complete() {
                    return Err("invalid split-before-detachment ordering".into());
                }
                (thread, request, placement.as_ref())
            }
        };
        let t = s.vm.threads.get(thread).ok_or("detachment thread absent")?;
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
            .any(|status| matches!(status, Status::AwaitingPlacement(r) if r == request));
        if !bound
            || s.visit.map(|v| v.0) != Some(*thread)
            || request.operands[0] != 1
            || request.operands[2] & !7 != 0
            || placement.is_some_and(|e| e.operation.binding() != (request.target, None))
        {
            return Err("invalid native Find Location detachment continuation".into());
        }
        // The split gate has already executed; callbacks must not repeat it.
        validate_detachment_tree(&s.vm.world, p, request.target)?;
        let Some(placement) = placement else {
            return Ok(());
        };
        if s.vm.world.objects[&request.target].multitile.is_some()
            != matches!(
                placement.operation,
                lot_placement::VirtualOperation::Group(_)
            )
        {
            return Err("native detachment lost virtual group operation".into());
        }
        placement.validate(&s.vm, p, request.target)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Randomize,
    Search,
    Recheck,
    Commit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Search {
    pub thread: ThreadId,
    pub request: Request,
    center: [i32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    random_origin: Option<RandomOrigin>,
    level: i32,
    size: i32,
    pass: u8,
    room: i16,
    direction: i32,
    cursor: Spiral,
    phase: Phase,
    position: Option<[i32; 2]>,
    pub acceptance: Option<lot_acceptance::live::Execution>,
    pub placement: Option<lot_placement::live::Execution>,
}
impl Search {
    pub fn begin(s: &mut Simulation, thread: ThreadId, r: &Request) -> Result<Self, String> {
        if ![0, 3, 4, 5].contains(&r.operands[0])
            || r.operands[0] != 5 && r.operands[2] & 1 == 0
            || r.operands[2] & !7 != 0
        {
            return Err(format!(
                "native Find Location detach/alternate-mode adapter required: {r:?}"
            ));
        }
        let at = if r.operands[2] & 1 == 0 {
            sims_vm::autonomy::Placement {
                // 422022(1,1) converts tile coordinates to physical units.
                x: 16,
                y: 16,
                level: 1,
            }
        } else {
            *s.vm
                .world
                .positions
                .get(&r.reference)
                .ok_or("Find Location reference position absent")?
        };
        if !(1..=2).contains(&at.level) {
            return Err("native Find Location reference floor unsupported".into());
        }
        let lot = s
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .ok_or("Find Location original lot absent")?;
        let size = lot.size as i32;
        // The original full-size view retains extrema0..63 under rotation.
        // Smaller-world4791a6 bound reconstruction is not yet integrated.
        if r.operands[0] == 5 && size != 64 {
            return Err("random Find Location requires original full-size view bounds".into());
        }
        let center = [at.x, at.y];
        let direction = if matches!(r.operands[0], 0 | 5) {
            -1
        } else {
            let source = *s.vm.world.objects[&r.reference]
                .data
                .get(&1)
                .ok_or("Find Location reference facing absent")? as i32;
            if r.operands[0] == 4 {
                (source + 2) % 8
            } else {
                source
            }
        };
        let cursor = Spiral::new_direction(size, center, direction)?;
        let [x, y] = cursor.center_tile();
        let room = lot.floor(at.level).unwrap().rooms.primary[x as usize * lot.size + y as usize];
        s.native_lot
            .as_ref()
            .ok_or("Find Location native host absent")?;
        Ok(Self {
            thread,
            request: r.clone(),
            center,
            random_origin: (r.operands[0] == 5).then(|| RandomOrigin::new(center)),
            level: at.level,
            size,
            pass: 0,
            room,
            direction,
            cursor,
            phase: if r.operands[0] == 5 {
                Phase::Randomize
            } else {
                Phase::Search
            },
            position: None,
            acceptance: None,
            placement: None,
        })
    }
    fn destination(&self) -> Descriptor {
        Descriptor {
            on_lot: true,
            position: self.position.unwrap(),
            level: self.level,
            container: None,
            slot: 0,
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
    }
    pub fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        let t =
            s.vm.threads
                .get(&self.thread)
                .ok_or("Find Location thread absent")?;
        let bound = std::iter::once(&t.status)
            .chain(
                t.callback
                    .iter()
                    .flat_map(|b| b.chain())
                    .map(|b| &b.resume_status),
            )
            .any(|st| matches!(st,Status::AwaitingPlacement(r) if r==&self.request));
        if !bound
            || s.visit.map(|v| v.0) != Some(self.thread)
            || self.pass > 1
            || !(1..=2).contains(&self.level)
            || ![0, 3, 4, 5].contains(&self.request.operands[0])
            || matches!(self.request.operands[0], 0 | 5) && self.direction != -1
            || matches!(self.request.operands[0], 3 | 4) && self.pass != 0
            || (self.request.operands[0] == 5) != self.random_origin.is_some()
            || self.random_origin.is_some() && self.size != 64
            || self.phase == Phase::Randomize && self.random_origin.is_none()
            || !(-32768..=32767).contains(&self.direction)
            || self.request.operands[0] == 4 && !(-7..=7).contains(&self.direction)
            || self.cursor.direction() != self.direction
            || self.request.operands[0] != 5 && self.request.operands[2] & 1 == 0
            || self.request.operands[2] & !7 != 0
            || s.architecture
                .as_ref()
                .and_then(|a| a.original.as_ref())
                .is_none_or(|a| a.size as i32 != self.size)
            || matches!(self.phase, Phase::Recheck | Phase::Commit) && self.position.is_none()
            || self.phase == Phase::Randomize && self.position.is_some()
            || self.placement.is_some() && self.phase != Phase::Commit
            || self.acceptance.is_some() && self.position.is_none()
            || self.phase == Phase::Commit
                && self
                    .acceptance
                    .as_ref()
                    .and_then(|a| a.outcome())
                    .is_none_or(|o| !o.accepted)
        {
            return Err("invalid native Find Location continuation".into());
        }
        self.cursor.validate(self.size, self.center)?;
        if let Some(pos) = self.position {
            if self.cursor.last_point() != Some(pos)
                || pos
                    .iter()
                    .any(|v| *v < 24 || *v >= (self.size - 1) * 16 || v & 15 != 8)
            {
                return Err("invalid Find Location candidate".into());
            }
        }
        if let Some(a) = &self.acceptance {
            a.validate(&s.vm, p)?;
            if a.binding() != (self.request.target, self.position.unwrap(), self.level) {
                return Err("invalid Find Location acceptance binding".into());
            }
        }
        if let Some(e) = &self.placement {
            e.validate(&s.vm, p, self.request.target)?;
            if self
                .acceptance
                .as_ref()
                .and_then(|a| a.outcome())
                .is_none_or(|o| !o.accepted)
                || e.operation.binding() != (self.request.target, Some(self.destination()))
            {
                return Err("invalid Find Location placement binding".into());
            }
        }
        Ok(())
    }
}
impl Simulation {
    /// Virtual +0x74 calls the view's +0x24(0): 5893a9 -> 589361
    /// invalidates placement caches, not animation layers or root compensation.
    /// The person override then selects the pool profile from the new position.
    pub(crate) fn refresh_find_location_appearance(
        &mut self,
        object: sims_vm::ObjectId,
    ) -> Result<(), String> {
        let at = self.vm.world.positions[&object];
        let lot = self
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .ok_or("Find Location original architecture absent")?;
        let profile = refresh_pool_profile(lot.size, [at.x, at.y, at.level], &lot.pool)?;
        if let Some(owner) = self.characters.get_mut(&object) {
            owner.character.placement.invalidate();
        }
        if let Some(person) = self
            .vm
            .world
            .objects
            .get_mut(&object)
            .unwrap()
            .person
            .as_mut()
        {
            person.insert(64, profile);
        }
        self.native_lot
            .as_mut()
            .ok_or("Find Location native host absent")?
            .observers
            .push(lot_placement::Request::RefreshAppearance(object));
        Ok(())
    }
    pub(crate) fn drive_native_find_location(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        self.drive_native_find_location_observed(
            p,
            remaining,
            step,
            crate::service_observation::ContinuationMoment::Reentry,
            &mut crate::service_observation::Observer::default(),
        )
    }
    pub(crate) fn drive_native_find_location_observed(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
        moment: crate::service_observation::ContinuationMoment,
        observer: &mut crate::service_observation::Observer<'_>,
    ) -> Result<bool, String> {
        if let Some(retained) = self
            .native_lot
            .as_ref()
            .and_then(|n| n.find_location.as_ref())
        {
            let (thread, request) = retained.binding();
            let phase = match retained {
                Execution::Search(_) => crate::service_observation::FindLocationPhase::Search,
                Execution::Detach { .. } => crate::service_observation::FindLocationPhase::Detach,
                Execution::DynamicDetach { .. } => {
                    crate::service_observation::FindLocationPhase::DynamicDetach
                }
            };
            observer.emit(
                crate::ServiceCut::FindLocation {
                    moment,
                    phase,
                    thread,
                    request: request.id,
                    object: crate::service_observation::ObjectLifetime::at(self, request.target),
                },
                self,
            )?;
        }
        let Some(mut e) = self
            .native_lot
            .as_mut()
            .and_then(|n| n.find_location.take())
        else {
            return Ok(true);
        };
        let result = match &mut e {
            Execution::Search(search) => self.drive_find_location(p, search, remaining, step),
            Execution::DynamicDetach {
                thread,
                request,
                split,
                placement,
            } => (|| {
                if !self.drive_dynamic_update(p, split, remaining, step)? {
                    return Ok(false);
                }
                if placement.is_none() {
                    validate_detachment_tree(&self.vm.world, p, request.target)?;
                    let links = &self.native_lot.as_ref().unwrap().placement.links;
                    *placement = Some(lot_placement::live::Execution::new_virtual(
                        lot_placement::VirtualOperation::remove(
                            &self.vm.world,
                            links,
                            request.target,
                            true,
                        )?,
                    ));
                }
                if !self.drive_live_placement(p, placement.as_mut().unwrap(), remaining, step)? {
                    return Ok(false);
                }
                self.vm.finish_find_location(p, *thread, request.id, true)?;
                self.event(
                    &mut step.events,
                    *thread,
                    format!("native Find Location detached #{} from lot", request.target),
                );
                Ok(true)
            })(),
            Execution::Detach {
                thread,
                request,
                placement,
            } => match self.drive_live_placement(p, placement, remaining, step) {
                Ok(true) => self
                    .vm
                    .finish_find_location(p, *thread, request.id, true)
                    .map(|()| {
                        self.event(
                            &mut step.events,
                            *thread,
                            format!("native Find Location detached #{} from lot", request.target),
                        );
                        true
                    }),
                result => result,
            },
        };
        if !result.as_ref().is_ok_and(|done| *done) {
            self.native_lot.as_mut().unwrap().find_location = Some(e);
        }
        result
    }
    fn drive_find_location(
        &mut self,
        p: &Program,
        e: &mut Search,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        loop {
            if e.phase == Phase::Randomize {
                if *remaining == 0 {
                    return Ok(false);
                }
                *remaining -= 1;
                let Some(center) = e.random_origin.as_mut().unwrap().advance(
                    e.size,
                    [0, 63, 0, 63],
                    i32::from(self.vm.world.camera_rotation),
                    &mut self.vm.rng_state,
                ) else {
                    continue;
                };
                e.center = center;
                e.cursor = Spiral::new(e.size, center)?;
                let [x, y] = e.cursor.center_tile();
                let lot = self
                    .architecture
                    .as_ref()
                    .unwrap()
                    .original
                    .as_ref()
                    .unwrap();
                e.room =
                    lot.floor(e.level).unwrap().rooms.primary[x as usize * lot.size + y as usize];
                e.phase = Phase::Search;
            }
            if e.phase == Phase::Commit {
                if e.placement.is_none() {
                    e.placement = Some(lot_placement::live::Execution::new_virtual(
                        lot_placement::VirtualOperation::set(
                            &self.vm.world,
                            &self.native_lot.as_ref().unwrap().placement.links,
                            e.request.target,
                            e.destination(),
                            true,
                        )?,
                    ));
                }
                if !self.drive_live_placement(p, e.placement.as_mut().unwrap(), remaining, step)? {
                    return Ok(false);
                }
                if !matches!(e.request.operands[0], 0 | 5) {
                    return self.finish_native_find_location(p, e, true, step);
                }
                self.refresh_find_location_appearance(e.request.target)?;
                return self.finish_native_find_location(p, e, true, step);
            }
            if e.position.is_none() {
                let lot = self
                    .architecture
                    .as_ref()
                    .unwrap()
                    .original
                    .as_ref()
                    .unwrap();
                let floor = lot.floor(e.level).unwrap();
                let n = self.native_lot.as_ref().unwrap();
                for point in e.cursor.by_ref() {
                    let cell = (point[1] >> 4) as usize * lot.size + (point[0] >> 4) as usize;
                    let tile = SearchTile {
                        segments: floor.source.walls[cell].segments,
                        head: n.placement.links.heads[e.level as usize - 1][cell],
                        room: floor.rooms.primary[cell],
                        pool: lot.pool[cell],
                        ground_flags: n.placement.flags[0][cell],
                    };
                    // Like the current imported route host, headless execution
                    // explicitly uses global 0x6e4ad4 = false.
                    if search_tile_allowed(
                        tile,
                        e.level,
                        e.request.operands[2],
                        (e.pass == 0).then_some(e.room),
                        false,
                    ) {
                        e.position = Some(point);
                        break;
                    }
                }
                if e.position.is_none() {
                    if e.pass == 1 || !matches!(e.request.operands[0], 0 | 5) {
                        return self.finish_native_find_location(p, e, false, step);
                    }
                    e.pass = 1;
                    if let Some(origin) = &mut e.random_origin {
                        origin.reset();
                        e.phase = Phase::Randomize;
                        continue;
                    }
                    e.cursor = Spiral::new(e.size, e.center)?;
                    let [x, y] = e.cursor.center_tile();
                    e.room = floor.rooms.primary[x as usize * lot.size + y as usize];
                    continue;
                }
            }
            if e.acceptance.is_none() {
                e.acceptance = Some(lot_acceptance::live::Execution::new(
                    self,
                    e.request.target,
                    e.position.unwrap(),
                    e.level,
                )?);
            }
            if !self.drive_native_acceptance(p, e.acceptance.as_mut().unwrap(), remaining, step)? {
                return Ok(false);
            }
            let accepted = e.acceptance.as_ref().unwrap().outcome().unwrap().accepted;
            if e.phase == Phase::Recheck {
                if !accepted {
                    return self.finish_native_find_location(p, e, false, step);
                }
                e.phase = Phase::Commit;
            } else {
                e.acceptance = None;
                if accepted {
                    e.phase = Phase::Recheck;
                } else {
                    e.position = None;
                }
            }
        }
    }
    fn finish_native_find_location(
        &mut self,
        p: &Program,
        e: &Search,
        success: bool,
        step: &mut Step,
    ) -> Result<bool, String> {
        self.vm
            .finish_find_location(p, e.thread, e.request.id, success)?;
        self.event(
            &mut step.events,
            e.thread,
            format!(
                "native Find Location #{} -> {:?} floor {}: {success}",
                e.request.target, e.position, e.level
            ),
        );
        Ok(true)
    }
}
