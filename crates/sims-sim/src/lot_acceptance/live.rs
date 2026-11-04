//! Cooperative host for native floor acceptance. Ordinary lot
//! work remains suspended while function-5 callbacks run on their own objects.
use super::{Floor, Input, Outcome, Reply, Request, person};
use crate::{Simulation, Step};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, Status, ThreadId, Vm};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Callback {
    object: ObjectId,
    other: ObjectId,
    thread: ThreadId,
    depth: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Base {
    object: ObjectId,
    position: [i32; 2],
    level: i32,
    container: Option<(ObjectId, i32)>,
    floor: Floor,
    collision: Option<person::Collision>,
    callback: Option<Callback>,
}
impl Base {
    pub fn new(
        s: &Simulation,
        object: ObjectId,
        position: [i32; 2],
        level: i32,
    ) -> Result<Self, String> {
        let native = s
            .native_lot
            .as_ref()
            .ok_or("native acceptance host absent")?;
        let input = Input::for_object(
            &s.vm.world,
            &native.placement,
            s.architecture
                .as_ref()
                .ok_or("native acceptance architecture absent")?,
            object,
            position,
            level,
        )?;
        Ok(Self {
            object,
            position,
            level,
            container: None,
            floor: Floor::new(input, native.diagnostics.error)?,
            collision: None,
            callback: None,
        })
    }
    /// The public descriptor builder takes X/Y from the live parent, but the
    /// level argument still comes from the caller (SetObject later overrides it).
    pub fn new_contained(
        s: &Simulation,
        object: ObjectId,
        container: ObjectId,
        slot: i32,
        level: i32,
    ) -> Result<Self, String> {
        let at =
            s.vm.world
                .positions
                .get(&container)
                .ok_or("contained acceptance position absent")?;
        let mut e = Self::new(s, object, [at.x, at.y], level)?;
        e.container = Some((container, slot));
        e.floor.contained = true;
        Ok(e)
    }
    pub fn binding(&self) -> (ObjectId, [i32; 2], i32) {
        (self.object, self.position, self.level)
    }
    pub fn container_binding(&self) -> Option<(ObjectId, i32)> {
        self.container
    }
    pub fn outcome(&self) -> Option<Outcome> {
        self.floor.outcome()
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.callback.as_ref().is_some_and(|c| {
            let Some(t) = vm.threads.get(&thread) else {
                return false;
            };
            let Some(b) = t
                .callback
                .iter()
                .flat_map(|b| b.chain())
                .find(|b| b.external && b.base_depth == depth)
            else {
                return false;
            };
            let Some(f) = t.stack.get(depth) else {
                return false;
            };
            let Some(d) = vm
                .world
                .objects
                .get(&c.object)
                .and_then(|o| o.definition.as_ref())
            else {
                return false;
            };
            let bhav = p
                .modules
                .get(&d.source)
                .and_then(|m| m.object_functions.get(&d.object))
                .and_then(|v| v.get(5))
                .map(|f| f.action);
            c.thread == thread
                && c.depth == depth
                && b.external
                && f.caller == c.object
                && f.callee == c.object
                && f.code_owner == d.source
                && bhav == Some(f.location.bhav)
        })
    }
    pub fn validate(&self, vm: &Vm, p: &Program) -> Result<(), String> {
        if vm
            .world
            .objects
            .get(&self.object)
            .is_none_or(|o| o.person.is_some() != self.floor.input.person)
            || self.floor.input.position != self.position
            || self.floor.input.level != self.level
            || self.floor.contained != self.container.is_some()
            || self
                .container
                .is_some_and(|(id, _)| !vm.world.objects.contains_key(&id))
            || self.outcome().is_some() && (self.collision.is_some() || self.callback.is_some())
        {
            return Err("invalid live acceptance binding".into());
        }
        if let Some(collision) = &self.collision {
            if collision.binding() != self.binding()
                || self.floor.pending != Some(Request::Collision)
            {
                return Err("invalid live acceptance collision".into());
            }
        }
        if let Some(c) = &self.callback {
            if !self.owns_callback(vm, p, c.thread, c.depth)
                || self.collision.as_ref().and_then(|q| q.pending())
                    != Some(person::Request::Function {
                        object: c.object,
                        other: c.other,
                    })
            {
                return Err("invalid live acceptance callback".into());
            }
        }
        Ok(())
    }
}

impl Simulation {
    fn drive_base_acceptance(
        &mut self,
        p: &Program,
        e: &mut Base,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        loop {
            if let Some(outcome) = e.outcome() {
                self.native_lot.as_mut().unwrap().diagnostics.error = outcome.error;
                return Ok(true);
            }
            if let Some(c) = e.callback.clone() {
                if self.vm.threads[&c.thread].callback.is_some() {
                    if *remaining == 0 {
                        return Ok(false);
                    }
                    if self.vm.threads[&c.thread].status != Status::Ready {
                        return Err(format!(
                            "native acceptance callback host required: {:?}",
                            self.vm.threads[&c.thread].status
                        ));
                    }
                    let before = self.vm.threads[&c.thread].stack.clone();
                    let slice = self.vm.run_slice(p, c.thread, 1)?;
                    if slice.trace.is_empty()
                        && self.vm.threads[&c.thread].callback.is_some()
                        && self.vm.threads[&c.thread].stack == before
                        && self.vm.threads[&c.thread].status == Status::Ready
                    {
                        return Err("native acceptance callback made no progress".into());
                    }
                    *remaining -= slice.trace.len();
                    step.trace.extend(slice.trace);
                    self.sync_script_directions()?;
                    if self.vm.threads[&c.thread].callback.is_some() {
                        continue;
                    }
                }
                let allowed = self.vm.threads[&c.thread]
                    .callback_result
                    .ok_or("acceptance callback result absent")?;
                let request = person::Request::Function {
                    object: c.object,
                    other: c.other,
                };
                let diagnostics = self.native_lot.as_ref().unwrap().diagnostics;
                e.collision.as_mut().unwrap().resume(
                    request,
                    person::Reply::Function {
                        allowed,
                        diagnostics,
                    },
                )?;
                e.callback = None;
                continue;
            }
            if let Some(collision) = &mut e.collision {
                let native = self.native_lot.as_ref().unwrap();
                if let Some(request) = collision.next(
                    &self.vm.world,
                    &native.placement.links,
                    &native.placement.bounds,
                )? {
                    match request {
                        person::Request::Function { object, other } => {
                            let thread = *self
                                .vm
                                .object_threads
                                .get(&object)
                                .ok_or("collision callback object thread absent")?;
                            let depth = self.vm.threads[&thread].stack.len();
                            if let Some(thread) =
                                self.vm.begin_object_function(p, object, 5, other)?
                            {
                                e.callback = Some(Callback {
                                    object,
                                    other,
                                    thread,
                                    depth,
                                });
                            } else {
                                collision.resume(
                                    request,
                                    person::Reply::Function {
                                        allowed: false,
                                        diagnostics: native.diagnostics,
                                    },
                                )?;
                            }
                        }
                        person::Request::Walls {
                            bounds: [max_x, max_y, min_x, min_y],
                            level,
                        } => {
                            let blocked = self
                                .architecture
                                .as_ref()
                                .unwrap()
                                .original
                                .as_ref()
                                .unwrap()
                                .wall_grid(level)?
                                .blocks_bounds(crate::Rect {
                                    min: crate::Point::new(min_y, min_x),
                                    max: crate::Point::new(max_y, max_x),
                                })?;
                            collision.resume(request, person::Reply::Walls(blocked))?;
                        }
                        person::Request::WallObjects { object, other } => {
                            let window = |id| -> Result<bool, String> {
                                let d = self.vm.world.objects[&id]
                                    .definition
                                    .as_ref()
                                    .ok_or("wall object definition absent")?;
                                let marker = p
                                    .modules
                                    .get(&d.source)
                                    .and_then(|m| m.object_definition_words.get(&d.object))
                                    .and_then(|w| w.get(69))
                                    .ok_or("wall object catalog marker absent")?;
                                Ok(*marker == 2)
                            };
                            collision.resume(
                                request,
                                person::Reply::WallObjects {
                                    both_windows_or_neither: window(object)? == window(other)?,
                                },
                            )?;
                        }
                    }
                    continue;
                }
                let (accepted, diagnostics) = collision
                    .outcome()
                    .ok_or("native acceptance collision result absent")?;
                self.native_lot.as_mut().unwrap().diagnostics = diagnostics;
                e.floor.resume(
                    Request::Collision,
                    Reply::Geometry(Outcome {
                        accepted,
                        error: diagnostics.error,
                    }),
                )?;
                e.collision = None;
                continue;
            }
            match e.floor.next() {
                Some(Request::Containment) => {
                    let (container, slot) =
                        e.container.ok_or("containment acceptance binding absent")?;
                    let slots = super::contained::slots(&self.vm.world, p, container)?;
                    let mode = *self
                        .vm
                        .world
                        .globals
                        .get(&15)
                        .ok_or("native placement game mode absent")?;
                    let native = self.native_lot.as_mut().unwrap();
                    let result = super::contained::check(
                        &mut self.vm.world,
                        &native.placement.links,
                        e.object,
                        container,
                        slot,
                        &slots,
                        mode,
                        native.diagnostics.error,
                    )?;
                    native.diagnostics.error = result.error;
                    e.floor
                        .resume(Request::Containment, Reply::Geometry(result))?;
                }
                Some(Request::Collision) => {
                    let d = self.vm.world.objects[&e.object]
                        .definition
                        .as_ref()
                        .ok_or("accepted person definition absent")?;
                    let shape = self
                        .architecture
                        .as_ref()
                        .unwrap()
                        .footprints
                        .get(&d.guid)
                        .ok_or("accepted object footprint absent")?;
                    let native = self.native_lot.as_ref().unwrap();
                    e.collision = Some(person::Collision::begin_object(
                        &mut self.vm.world,
                        &native.placement.links,
                        e.object,
                        e.position,
                        e.level,
                        shape.size,
                        shape.insets,
                        native.diagnostics,
                    )?);
                }
                Some(Request::WallFit) => {
                    let object = &self.vm.world.objects[&e.object];
                    let wall = self
                        .architecture
                        .as_ref()
                        .unwrap()
                        .original
                        .as_ref()
                        .unwrap()
                        .wall_grid(e.level)?
                        .expanded((e.position[1] >> 4) as usize, (e.position[0] >> 4) as usize)?;
                    let result = crate::lot_walls::placement_fit(
                        &wall,
                        *object.data.get(&1).ok_or("wall-fit facing absent")?,
                        *object.data.get(&13).ok_or("wall-fit mask absent")? as u16,
                        self.native_lot.as_ref().unwrap().diagnostics.error,
                    )?;
                    self.native_lot.as_mut().unwrap().diagnostics.error = result.error;
                    e.floor.resume(Request::WallFit, Reply::Geometry(result))?;
                }
                Some(request) => {
                    return Err(format!(
                        "native acceptance environmental service required: {request:?}"
                    ));
                }
                None => {
                    let outcome = e.floor.outcome().ok_or("native acceptance result absent")?;
                    self.native_lot.as_mut().unwrap().diagnostics.error = outcome.error;
                    return Ok(true);
                }
            }
        }
    }
}

/// Virtual floor acceptance. Group wrappers invoke the base checker on each
/// sibling; they must not recursively dispatch that sibling's group override.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Execution {
    Base(Base),
    Group(Group),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Group {
    object: ObjectId,
    position: [i32; 2],
    level: i32,
    container: Option<(ObjectId, i32)>,
    check: super::group::Check,
    member: Option<Base>,
}
impl Execution {
    pub fn new(
        s: &Simulation,
        object: ObjectId,
        position: [i32; 2],
        level: i32,
    ) -> Result<Self, String> {
        Self::begin(s, object, position, level, None)
    }
    pub fn new_contained(
        s: &Simulation,
        object: ObjectId,
        container: ObjectId,
        slot: i32,
        level: i32,
    ) -> Result<Self, String> {
        let at =
            s.vm.world
                .positions
                .get(&container)
                .ok_or("contained acceptance position absent")?;
        Self::begin(s, object, [at.x, at.y], level, Some((container, slot)))
    }
    fn begin(
        s: &Simulation,
        object: ObjectId,
        position: [i32; 2],
        level: i32,
        container: Option<(ObjectId, i32)>,
    ) -> Result<Self, String> {
        if s.vm
            .world
            .objects
            .get(&object)
            .is_some_and(|o| o.multitile.is_some())
        {
            let native = s
                .native_lot
                .as_ref()
                .ok_or("group acceptance host absent")?;
            Ok(Self::Group(Group {
                object,
                position,
                level,
                container,
                check: super::group::Check::begin(
                    &s.vm.world,
                    &native.placement.links,
                    object,
                    position,
                    level,
                    container.is_some(),
                    container.map_or(0, |(_, slot)| slot),
                    native.diagnostics.error,
                )?,
                member: None,
            }))
        } else {
            Ok(Self::Base(if let Some((parent, slot)) = container {
                Base::new_contained(s, object, parent, slot, level)?
            } else {
                Base::new(s, object, position, level)?
            }))
        }
    }
    pub fn binding(&self) -> (ObjectId, [i32; 2], i32) {
        match self {
            Self::Base(b) => b.binding(),
            Self::Group(g) => (g.object, g.position, g.level),
        }
    }
    pub fn container_binding(&self) -> Option<(ObjectId, i32)> {
        match self {
            Self::Base(b) => b.container_binding(),
            Self::Group(g) => g.container,
        }
    }
    pub fn outcome(&self) -> Option<Outcome> {
        match self {
            Self::Base(b) => b.outcome(),
            Self::Group(g) => g.check.outcome(),
        }
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        match self {
            Self::Base(b) => b.owns_callback(vm, p, thread, depth),
            Self::Group(g) => g
                .member
                .as_ref()
                .is_some_and(|m| m.owns_callback(vm, p, thread, depth)),
        }
    }
    pub fn validate(&self, vm: &Vm, p: &Program) -> Result<(), String> {
        match self {
            Self::Base(b) => b.validate(vm, p),
            Self::Group(g) => {
                g.check.validate(&vm.world)?;
                if g.check.binding() != (g.object, g.position, g.level)
                    || g.check.contained() != g.container.is_some()
                    || g.member.is_some() && g.check.pending().is_none()
                    || g.container.is_some()
                        && g.outcome()
                            != Some(Outcome {
                                accepted: false,
                                error: 16,
                            })
                {
                    return Err("invalid live group acceptance binding".into());
                }
                if let Some(member) = &g.member {
                    member.validate(vm, p)?;
                    let q = g.check.pending().unwrap();
                    if member.binding() != (q.object, q.position, q.level)
                        || member.container.is_some()
                    {
                        return Err("invalid group acceptance member".into());
                    }
                }
                Ok(())
            }
        }
    }
}
impl Group {
    fn outcome(&self) -> Option<Outcome> {
        self.check.outcome()
    }
}
impl Simulation {
    pub(crate) fn drive_native_acceptance(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        match e {
            Execution::Base(base) => self.drive_base_acceptance(p, base, remaining, step),
            Execution::Group(group) => loop {
                if let Some(result) = group.check.outcome() {
                    self.native_lot.as_mut().unwrap().diagnostics.error = result.error;
                    return Ok(true);
                }
                if group.member.is_none() {
                    let Some(q) = group.check.next(&self.vm.world)? else {
                        continue;
                    };
                    group.member = Some(Base::new(self, q.object, q.position, q.level)?);
                }
                let member = group.member.as_mut().unwrap();
                if !self.drive_base_acceptance(p, member, remaining, step)? {
                    return Ok(false);
                }
                let result = member.outcome().unwrap();
                let lot = self
                    .architecture
                    .as_ref()
                    .and_then(|a| a.original.as_ref())
                    .ok_or("group acceptance lot absent")?;
                group.check.resume(
                    &self.vm.world,
                    lot.size,
                    &lot.heights,
                    group.check.pending().unwrap(),
                    result,
                )?;
                group.member = None;
            },
        }
    }
}
