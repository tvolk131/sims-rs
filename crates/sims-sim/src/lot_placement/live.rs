//! Synchronous base placement in the live native lot. Ordinary scheduling is
//! frozen until this continuation and its function callbacks have completed.
use super::{Operation, Request, VirtualOperation};
use crate::{Body, Point, Simulation, Step, lot_adjacency::Adjacency};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, Status, ThreadId, Vm};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Callback {
    object: ObjectId,
    function: usize,
    thread: ThreadId,
    depth: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    pub operation: VirtualOperation,
    pub pending: Option<Request>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub graphic_started: bool,
    adjacency: Option<Adjacency>,
    callback: Option<Callback>,
    pub complete: bool,
}
impl Execution {
    pub fn new(operation: Operation) -> Self {
        Self::new_virtual(VirtualOperation::Base(operation))
    }
    pub fn new_virtual(operation: VirtualOperation) -> Self {
        Self {
            operation,
            pending: None,
            graphic_started: false,
            adjacency: None,
            callback: None,
            complete: false,
        }
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
                .and_then(|v| v.get(c.function))
                .map(|f| f.action);
            c.thread == thread
                && c.depth == depth
                && b.external
                && b.base_depth == depth
                && f.caller == c.object
                && f.callee == c.object
                && f.code_owner == d.source
                && bhav == Some(f.location.bhav)
        })
    }
    pub fn validate(&self, vm: &Vm, p: &Program, object: ObjectId) -> Result<(), String> {
        self.validate_kind(vm, p, object, false)
    }
    pub(crate) fn validate_rotation(
        &self,
        vm: &Vm,
        p: &Program,
        object: ObjectId,
    ) -> Result<(), String> {
        self.validate_kind(vm, p, object, true)
    }
    fn validate_kind(
        &self,
        vm: &Vm,
        p: &Program,
        object: ObjectId,
        rotation: bool,
    ) -> Result<(), String> {
        let finished = self.operation.validate_progress(&vm.world, rotation)?;
        self.validate_state(vm, p, object, finished)
    }
    pub(crate) fn validate_native_rotation(
        &self,
        vm: &Vm,
        p: &Program,
        object: ObjectId,
        notches: i32,
    ) -> Result<(), String> {
        let finished = self.operation.validate_native_rotation(object, notches)?;
        self.validate_state(vm, p, object, finished)
    }
    fn validate_state(
        &self,
        vm: &Vm,
        p: &Program,
        object: ObjectId,
        finished: bool,
    ) -> Result<(), String> {
        if self.operation.binding().0 != object
            || self.graphic_started && !matches!(self.pending, Some(Request::RefreshAnimator(_)))
            || !vm.world.objects.contains_key(&object)
            || self.complete != finished
            || self.complete
                && (self.pending.is_some() || self.adjacency.is_some() || self.callback.is_some())
        {
            return Err("invalid live placement binding".into());
        }
        if let Some(c) = &self.callback {
            if !self.owns_callback(vm, p, c.thread, c.depth) {
                return Err("invalid live placement callback".into());
            }
            let bound = matches!(self.pending,Some(Request::Function{object,function}) if object==c.object && function==c.function)
                || matches!(self.pending,Some(Request::Walls(id)) if id==c.object && c.function==6)
                || matches!(self.pending, Some(Request::Adjacency(_)))
                    && self
                        .adjacency
                        .as_ref()
                        .is_some_and(|a| a.callback_object() == Some(c.object))
                    && c.function == 9;
            if !bound {
                return Err("unbound live placement callback".into());
            }
        }
        if self.adjacency.is_some() && !matches!(self.pending, Some(Request::Adjacency(_))) {
            return Err("unbound placement adjacency traversal".into());
        }
        Ok(())
    }
    fn begin_function(
        &mut self,
        vm: &mut Vm,
        p: &Program,
        object: ObjectId,
        function: usize,
    ) -> Result<(), String> {
        let depth = vm
            .object_threads
            .get(&object)
            .and_then(|t| vm.threads.get(t))
            .map_or(0, |t| t.stack.len());
        if let Some(thread) = vm.begin_loaded_function(p, object, function)? {
            self.callback = Some(Callback {
                object,
                function,
                thread,
                depth,
            });
        }
        Ok(())
    }
}

impl Simulation {
    /// Copy presentation/routing caches from native storage, never the reverse.
    pub(crate) fn publish_live_placement_caches(&mut self) -> Result<(), String> {
        for (&id, q) in &self.vm.world.positions {
            if self.cursor_constructor_pending(id) {
                self.validate_cursor_constructor(id)?;
                continue;
            }
            if q.x < 0 || q.y < 0 {
                self.bodies.remove(&id);
                self.unplaced.insert(id);
            } else {
                let direction = *self.vm.world.objects[&id]
                    .data
                    .get(&1)
                    .ok_or("native placed direction absent")?;
                let b = self.bodies.entry(id).or_insert(Body {
                    position: Point::new(q.y, q.x),
                    level: q.level,
                    facing: 1 << (direction as u16 & 7),
                    radius: 0,
                    elevation: 0,
                });
                b.position = Point::new(q.y, q.x);
                b.level = q.level;
                b.facing = 1 << (direction as u16 & 7);
                self.unplaced.remove(&id);
            }
        }
        self.native_lot
            .as_ref()
            .ok_or("native placement host absent")?
            .placement
            .links
            .publish_tile_chains(&mut self.vm.world)?;
        self.sync_spatial_world();
        Ok(())
    }
    pub(crate) fn drive_live_placement(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        self.drive_live_placement_with_graphics(p, e, remaining, step, &Default::default())
    }
    /// Retained ordinary graphics have a real +120 owner. Leave that native
    /// refresh pending for the asset host instead of taking the null branch.
    pub(crate) fn drive_live_placement_with_graphics(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
        ordinary_graphics: &std::collections::BTreeSet<ObjectId>,
    ) -> Result<bool, String> {
        loop {
            if e.complete {
                return Ok(true);
            }
            if let Some(c) = e.callback.clone() {
                let t = &self.vm.threads[&c.thread];
                if t.callback.is_some() {
                    if *remaining == 0 {
                        return Ok(false);
                    }
                    if !matches!(t.status, Status::Ready) {
                        return Err(format!(
                            "native placement callback host required: {:?}",
                            t.status
                        ));
                    }
                    let before = t.stack.clone();
                    let slice = self.vm.run_slice(p, c.thread, 1)?;
                    if slice.trace.is_empty()
                        && self.vm.threads[&c.thread].callback.is_some()
                        && self.vm.threads[&c.thread].stack == before
                        && matches!(self.vm.threads[&c.thread].status, Status::Ready)
                    {
                        return Err("native placement callback made no progress".into());
                    }
                    *remaining -= slice.trace.len();
                    step.trace.extend(slice.trace);
                    if self.vm.threads[&c.thread].callback.is_some() {
                        continue;
                    }
                }
                self.vm.threads[&c.thread]
                    .callback_result
                    .ok_or("placement callback result absent")?;
                e.callback = None;
                if e.adjacency.is_none() {
                    e.pending = None;
                }
                self.publish_live_placement_caches()?;
                continue;
            }
            if let Some(scan) = &mut e.adjacency {
                let native = self.native_lot.as_ref().unwrap();
                if let Some(update) = scan.next(&native.placement.links, &mut self.vm.world, p)? {
                    if update.callback {
                        e.begin_function(&mut self.vm, p, update.object, 9)?;
                    }
                    continue;
                }
                e.adjacency = None;
                e.pending = None;
                continue;
            }
            if e.pending.is_none() {
                let native = self
                    .native_lot
                    .as_mut()
                    .ok_or("native placement host absent")?;
                e.pending = e.operation.next(
                    &mut self.vm.world,
                    p,
                    self.architecture
                        .as_ref()
                        .ok_or("native placement architecture absent")?,
                    &mut native.placement,
                )?;
                self.publish_live_placement_caches()?;
                if e.pending.is_none() {
                    e.complete = true;
                    return Ok(true);
                }
            }
            match e.pending.clone().unwrap() {
                Request::Function { object, function } => {
                    e.begin_function(&mut self.vm, p, object, function)?;
                    if e.callback.is_none() {
                        e.pending = None;
                    }
                }
                Request::Adjacency(tile) => {
                    e.adjacency = Adjacency::begin(
                        &self.native_lot.as_ref().unwrap().placement.links,
                        &self.vm.world,
                        p,
                        tile,
                    )?;
                    if e.adjacency.is_none() {
                        e.pending = None;
                    }
                }
                Request::Walls(object) => {
                    let old = self.vm.world.objects[&object].data[&5];
                    let flags = super::wall_flags(
                        &self.vm.world,
                        p,
                        self.architecture.as_ref().unwrap(),
                        object,
                    )?;
                    if old != flags {
                        self.vm
                            .world
                            .objects
                            .get_mut(&object)
                            .unwrap()
                            .data
                            .insert(5, flags);
                        e.begin_function(&mut self.vm, p, object, 6)?;
                    }
                    if e.callback.is_none() {
                        e.pending = None;
                    }
                }
                Request::PlacementError(error) => {
                    self.native_lot.as_mut().unwrap().diagnostics.error = error;
                    e.pending = None;
                }
                Request::Signal { event, value } => {
                    self.vm.refresh_room_event(event, i32::from(value))?;
                    e.pending = None;
                }
                Request::RefreshAnimator(object) => {
                    if ordinary_graphics.contains(&object) {
                        return Ok(false);
                    }
                    // 0x433125 queries virtual +0x2c (0x4215c3); its +0x120
                    // pointer is null for these ordinary objects. Only a live
                    // animator calls virtual +0x54(1). That updates the skin
                    // and placement transforms; it does not clear the clip.
                    // Person/skin refresh is a separate service, not an unconditional RemoveFromLot
                    // failure for furniture and off-lot controllers.
                    if self.characters.contains_key(&object) {
                        if !self.drive_detached_character(p, object, remaining)? {
                            return Ok(false);
                        }
                    } else {
                        self.refresh_detached_animator(object)?;
                    }
                    e.pending = None;
                }
                effect => {
                    self.native_lot.as_mut().unwrap().observers.push(effect);
                    e.pending = None;
                }
            }
        }
    }
    pub(crate) fn refresh_detached_animator(&mut self, object: ObjectId) -> Result<(), String> {
        let o = &self.vm.world.objects[&object];
        if let Some(person) = &o.person {
            // 0x58da12 retains foreground/background pointers. This branch
            // covers an ended foreground or an already-updated foreground at
            // the current clock, without head look, mood overlays or attached
            // skin. The latter runs 0x597c4c with zero elapsed time: progress,
            // completion and consumed event cursors survive even for a loop.
            // advance_animations owns that clock update before ordinary script
            // execution; a refresh must not advance it twice. The renderer
            // derives transforms from current placement. Nontrivial secondary
            // layers/head clocks and stale active foregrounds need an adapter.
            if [30, 41, 42, 43, 44, 45]
                .iter()
                .any(|i| person.get(i) != Some(&0))
                || self.animations.get(&object).is_some_and(|a| {
                    (!a.completed || !a.ended || !a.pending.is_empty())
                        && a.last_tick != self.vm.tick
                })
                || self.backgrounds.contains_key(&object)
                || self.carry.get(&object).is_some_and(|c| c.layer.is_some())
                || !o.accessories.is_empty()
            {
                return Err(format!(
                    "live placement active-layer/head-look animator refresh required: object #{object}, tick {}, head {:?}, foreground {:?}, background {}, carry {:?}, accessories {}",
                    self.vm.tick,
                    [30, 41, 42, 43, 44, 45].map(|i| person.get(&i)),
                    self.animations.get(&object).map(|a| (
                        a.last_tick,
                        a.completed,
                        a.ended,
                        a.pending.len()
                    )),
                    self.backgrounds.contains_key(&object),
                    self.carry.get(&object),
                    o.accessories.len()
                ));
            }
            self.native_lot
                .as_mut()
                .unwrap()
                .observers
                .push(Request::RefreshAnimator(object));
        } else if self.animations.contains_key(&object)
            || self.backgrounds.contains_key(&object)
            || !o.accessories.is_empty()
        {
            return Err("live placement non-person animator refresh required".into());
        }
        Ok(())
    }
}
