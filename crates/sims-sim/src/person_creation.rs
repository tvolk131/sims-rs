//! Original-lot person factories. The wrapper retains the native target
//! coordinates and runs floor acceptance, including synchronous collision
//! callbacks. Failed-placement cleanup remains an explicit boundary. Accepted
//! locations use native SetObject before returning the created object.
use crate::{Simulation, Step, household::PersonUpdate, lot_placement};
use serde::{Deserialize, Serialize};
use sims_vm::{
    Definition, ObjectId, Program, Status, ThreadId, create,
    person_factory::{
        ConstructionResetRequest as Reset, InitialResetRequest as Service,
        PostConstructRequest as Post,
        registered::{Factory, Request},
    },
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Presentation {
    BaseCaches,
    BindAnimator,
    ReloadAppearance,
    ClearAnimator,
}

/// Retain completed score/presentation effects as well as the continuation.
/// A headless host consumes the gameplay effects and leaves graphics work queued.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct State {
    pub factory: Factory,
    /// None for direct engine calls (e.g. portrait generation). Existing saved
    /// numeric parent IDs deserialize as Some without changing their encoding.
    pub parent: Option<ThreadId>,
    pub request: u64,
    pub wrapper_finished: bool,
    pub finalized: bool,
    /// Mode 6 and direct factories skip physical placement, but the wrapper
    /// still owns its later visitor callback and parameter writes.
    pub off_lot: bool,
    pub visitor: Option<Visitor>,
    pub active: bool,
    pub native_flags: u32,
    pub scores: Vec<sims_vm::motives::Curve>,
    pub presentation: Vec<Presentation>,
    pub placement: Option<lot_placement::live::Execution>,
    pub acceptance: Option<crate::lot_acceptance::live::Execution>,
    /// Cross-owner token: a live asset continuation must accompany this state.
    #[serde(default)]
    pub asset_service_started: bool,
    /// Actual44123f after a completed factory's virtual acceptance rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) rejection_cleanup: Option<crate::lot_removal::immediate::Removal>,
    /// Preserve the actual instance resource before destructor callbacks;
    /// never re-resolve a mutable neighbor slot after callbacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) rejected_definition: Option<Definition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Visitor {
    pub thread: ThreadId,
    pub depth: usize,
    pub owner: String,
    pub bhav: u16,
    pub complete: bool,
}
impl Visitor {
    pub fn owns_callback(&self, vm: &sims_vm::Vm, thread: ThreadId, depth: usize) -> bool {
        !self.complete
            && self.thread == thread
            && self.depth == depth
            && vm.threads.get(&thread).is_some_and(|t| {
                t.callback
                    .as_ref()
                    .is_some_and(|b| b.chain().any(|b| b.external && b.base_depth == depth))
            })
    }
}

impl State {
    /// Missing storage is permitted only while this exact rejected wrapper's
    /// retained44123f owns the already-unlinked person and its asset teardown.
    pub(crate) fn retired_creation(
        &self,
        s: &Simulation,
        p: &Program,
    ) -> Result<Option<(ThreadId, u64, ObjectId)>, String> {
        let id = self.factory.object();
        let missing = !s.vm.world.objects.contains_key(&id);
        let Some(c) = &self.rejection_cleanup else {
            if self.rejected_definition.is_some() {
                return Err("rejected definition has no cleanup owner".into());
            }
            return if missing {
                Err("person factory storage absent without rejection cleanup".into())
            } else {
                Ok(None)
            };
        };
        if c.target != id
            || !self.factory.is_complete()
            || self.factory.pending_request().is_some()
            || !self.wrapper_finished
            || self.finalized
            || self.off_lot
            || self.visitor.is_some()
            || self.placement.is_some()
            || self.asset_service_started
            || !self
                .acceptance
                .as_ref()
                .and_then(|e| e.outcome())
                .is_some_and(|o| !o.accepted)
        {
            return Err("invalid rejected person cleanup phase".into());
        }
        let definition = self
            .rejected_definition
            .as_ref()
            .ok_or("rejected person committed definition absent")?;
        let words = sims_vm::construction::definition_words(p, definition)?;
        if words[9] != 2
            || words[10] != 0
            || definition.guid
                != (u32::from(words[14] as u16) | (u32::from(words[15] as u16) << 16))
            || definition.slots != words[20] as u16
            || !missing && s.vm.world.objects[&id].definition.as_ref() != Some(definition)
        {
            return Err("rejected person committed definition differs".into());
        }
        c.validate(s, p, false)?;
        let thread = self.parent.ok_or("rejected person cleanup parent absent")?;
        let parent =
            s.vm.threads
                .get(&thread)
                .ok_or("rejected person cleanup parent stale")?;
        let request = std::iter::once(&parent.status)
            .chain(
                parent
                    .callback
                    .iter()
                    .flat_map(|b| b.chain())
                    .map(|b| &b.resume_status),
            )
            .chain(
                parent
                    .service
                    .iter()
                    .flat_map(|b| b.chain())
                    .map(|b| &b.resume_status),
            )
            .find_map(|status| match status {
                Status::AwaitingCreation(r) if r.id == self.request && r.created == Some(id) => {
                    Some(r)
                }
                _ => None,
            })
            .ok_or("rejected person cleanup request stale")?;
        let to = request
            .placement
            .as_ref()
            .ok_or("rejected person target absent")?;
        if !matches!(
            (request.operands[4], request.operands[5]),
            (8, 0x20) | (5, 4) | (5, 0x14)
        ) || self.acceptance.as_ref().unwrap().binding()
            != (id, [to.position[1], to.position[0]], to.level)
        {
            return Err("rejected person cleanup destination differs".into());
        }
        if missing
            && (!c.deleting
                || s.deletion.get(c.depth()).is_none_or(|d| {
                    d.target != id
                        || !d.storage_unlinked
                        || d.entry_definition.as_ref() != Some(definition)
                }))
        {
            return Err("retired rejected person lacks exact unlinked destructor".into());
        }
        Ok(missing.then_some((thread, self.request, id)))
    }
}

impl Simulation {
    fn register_person_factory(
        &mut self,
        factory: Factory,
        parent: Option<ThreadId>,
        request: u64,
        off_lot: bool,
    ) -> ObjectId {
        let id = factory.object();
        self.person_factories.insert(
            id,
            State {
                factory,
                parent,
                request,
                wrapper_finished: false,
                finalized: false,
                off_lot,
                visitor: None,
                active: false,
                native_flags: 0,
                scores: vec![],
                presentation: vec![],
                placement: None,
                acceptance: None,
                asset_service_started: false,
                rejection_cleanup: None,
                rejected_definition: None,
            },
        );
        // Publish the same native off-lot sentinel before Init/animator work.
        self.unplaced.insert(id);
        self.vm.world.positions.insert(
            id,
            sims_vm::autonomy::Placement {
                x: -16,
                y: -16,
                level: 1,
            },
        );
        let native = self.native_lot.as_mut().unwrap();
        native.placement.links.next.insert(id, 0);
        native.placement.bounds.insert(id, [0; 4]);
        if self.vm.world.registry_order.is_none() {
            native.placement.spatial_dirty = Some(true);
        }
        native.membership.0.insert(id, -5);
        self.household.as_mut().unwrap().people.insert(
            id,
            PersonUpdate {
                started: None,
                finished: None,
            },
        );
        id
    }

    /// Direct 440b2d person branch, with no fabricated SimAntics parent frame.
    /// The supplied resource must already pass the native readiness/name guards.
    /// Drive this owner and its real asset services synchronously before any
    /// ordinary scheduling. The caller separately owns subsequent placement.
    pub fn begin_direct_person_factory(
        &mut self,
        p: &Program,
        definition: Definition,
    ) -> Result<Option<ObjectId>, String> {
        if self.native_lot.is_none() || self.household.is_none() {
            return Err("direct person factory requires a native household".into());
        }
        if self.person_factories.values().any(|s| !s.finalized) {
            return Err("direct person factory overlaps active construction".into());
        }
        if self.vm.crt_rng_state.is_none() {
            return Err("person factory CRT random state not supplied".into());
        }
        let clock = &self.household.as_ref().unwrap().clock;
        let date = sims_vm::construction::ObjectDate {
            year: clock.year,
            month: clock.month,
            day: clock.day,
        };
        let Some(factory) = Factory::begin(
            &mut self.vm,
            p,
            definition,
            date,
            [clock.hour, clock.minute],
        )?
        else {
            return Ok(None);
        };
        Ok(Some(self.register_person_factory(factory, None, 0, true)))
    }

    pub fn drive_direct_person_factory(
        &mut self,
        p: &Program,
        id: ObjectId,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        self.validate_person_factories(p)?;
        let state = self
            .person_factories
            .get(&id)
            .ok_or("direct person factory absent")?;
        if state.parent.is_some() {
            return Err("person factory belongs to a SimAntics caller".into());
        }
        if state.finalized {
            return Ok(true);
        }
        let mut state = self.person_factories.remove(&id).unwrap();
        let result = self.drive_native_person_factory(p, id, &mut state, remaining, step);
        if result.as_ref().is_ok_and(|done| *done) {
            // These flags close the direct owner. 44e0c8's later trait copy,
            // facing and visitor logic are not part of direct 440b2d calls.
            state.wrapper_finished = true;
            state.finalized = true;
        }
        self.person_factories.insert(id, state);
        self.sync_spatial_world();
        self.validate_native_lot()?;
        result
    }

    pub fn pending_factory_autonomy_services(
        &self,
    ) -> Vec<(ObjectId, sims_vm::autonomy_constants::Request)> {
        self.person_factories
            .iter()
            .filter_map(|(&id, state)| {
                state
                    .factory
                    .autonomy_constants_request()
                    .cloned()
                    .map(|request| (id, request))
            })
            .collect()
    }
    pub fn factory_autonomy_table(
        &self,
        id: ObjectId,
    ) -> Option<&sims_vm::autonomy_constants::Table> {
        self.person_factories
            .get(&id)?
            .factory
            .autonomy_constants_table()
    }
    pub fn reply_factory_autonomy_constants(
        &mut self,
        id: ObjectId,
        reply: sims_vm::autonomy_constants::Reply,
    ) -> Result<(), String> {
        self.person_factories
            .get_mut(&id)
            .ok_or("person constants factory absent")?
            .factory
            .reply_autonomy_constants(reply)
    }

    pub fn pending_factory_character_services(&self) -> Vec<(ObjectId, Request)> {
        if self.native_character_settings.is_none() {
            return vec![];
        }
        self.person_factories
            .iter()
            .filter_map(|(&id, state)| {
                let request = state.factory.pending_request()?;
                matches!(
                    request,
                    Request::Post(Post::BindAnimator)
                        | Request::Reset(Reset::Service(Service::ReloadAppearance))
                )
                .then_some((id, request))
            })
            .collect()
    }

    /// Complete only actual asset-host work. The original factory remains
    /// suspended until its character and geometry owners are installed.
    pub fn complete_factory_character_service(
        &mut self,
        p: &Program,
        id: ObjectId,
        request: Request,
    ) -> Result<(), String> {
        if !self
            .pending_factory_character_services()
            .contains(&(id, request))
            || !self.person_factories[&id].asset_service_started
        {
            return Err("stale person factory character service".into());
        }
        let resident = self.characters.get(&id).ok_or("factory animator absent")?;
        if resident.evaluated_tick != self.vm.tick
            || resident.character.busy()
            || resident.source.is_none()
            || resident.ordinary_update.is_some()
            || resident.person_update.is_some()
        {
            return Err("factory animator is incomplete".into());
        }
        resident.character.validate(p)?;
        let effect = match request {
            Request::Post(Post::BindAnimator) => Presentation::BindAnimator,
            Request::Reset(Reset::Service(Service::ReloadAppearance)) => {
                Presentation::ReloadAppearance
            }
            _ => unreachable!(),
        };
        let state = self.person_factories.get_mut(&id).unwrap();
        state.factory.complete(&mut self.vm, p, request, None)?;
        state.asset_service_started = false;
        state.presentation.push(effect);
        Ok(())
    }

    pub(crate) fn validate_person_factories(&self, p: &Program) -> Result<(), String> {
        if !self.person_factories.is_empty()
            && (self.native_lot.is_none() || self.household.is_none())
        {
            return Err("person factories require a native household".into());
        }
        for (&id, state) in &self.person_factories {
            let retired = state.retired_creation(self, p)?.is_some();
            if !retired {
                state.factory.validate(&self.vm, p)?;
            }
            if state.asset_service_started
                && !self
                    .pending_factory_character_services()
                    .iter()
                    .any(|(owner, _)| *owner == id)
            {
                return Err("native factory asset token has no pending service".into());
            }
            if state.factory.object() != id
                || state.active != state.factory.activated()
                || (state.native_flags & 0x80 != 0) != state.active
                || state.presentation.len() > 8
                || (state.wrapper_finished && !state.factory.is_complete())
                || state.finalized && !state.wrapper_finished
                || state.off_lot && (state.placement.is_some() || state.acceptance.is_some())
                || state.scores.len() > 9
            {
                return Err("invalid native person factory effects".into());
            }
            if state.parent.is_none()
                && (!state.off_lot
                    || state.request != 0
                    || state.visitor.is_some()
                    || state.placement.is_some()
                    || state.acceptance.is_some()
                    || state.wrapper_finished != state.finalized
                    || !state.finalized && !self.unplaced.contains(&id))
            {
                return Err("invalid direct person factory ownership".into());
            }
            if let Some(v) = &state.visitor {
                let t = self
                    .vm
                    .threads
                    .get(&v.thread)
                    .ok_or("visitor thread absent")?;
                if self.vm.object_threads.get(&id) != Some(&v.thread)
                    || self.vm.resolve_named_tree(p, id, "init visitor")?
                        != Some((v.owner.clone(), v.bhav))
                    || !state.wrapper_finished
                    || !state.off_lot && state.placement.as_ref().is_none_or(|e| !e.complete)
                    || state.finalized && !v.complete
                {
                    return Err("invalid visitor callback binding".into());
                }
                if !v.complete {
                    let f = t.stack.get(v.depth).ok_or("visitor frame absent")?;
                    if !v.owns_callback(&self.vm, v.thread, v.depth)
                        || f.code_owner != v.owner
                        || f.location.bhav != v.bhav
                        || (f.caller, f.callee) != (id, id)
                    {
                        return Err("visitor callback owner mismatch".into());
                    }
                }
            }
            for curve in &state.scores {
                curve.validate()?;
            }
            if let Some(e) = &state.acceptance {
                if !retired {
                    e.validate(&self.vm, p)?;
                }
                if e.binding().0 != id
                    || !state.wrapper_finished
                    || state.placement.is_some() && !e.outcome().is_some_and(|o| o.accepted)
                {
                    return Err("invalid person factory acceptance binding".into());
                }
            }
            if let Some(placement) = &state.placement {
                placement.validate(&self.vm, p, id)?;
                if !state.wrapper_finished || placement.operation.binding().1.is_none() {
                    return Err("person placement before factory completion".into());
                }
            }
            if !state.finalized {
                let Some(parent_id) = state.parent else {
                    continue;
                };
                let parent = self
                    .vm
                    .threads
                    .get(&parent_id)
                    .ok_or("native person factory parent absent")?;
                let request = std::iter::once(&parent.status)
                    .chain(
                        parent
                            .callback
                            .iter()
                            .flat_map(|b| b.chain())
                            .map(|b| &b.resume_status),
                    )
                    .chain(
                        parent
                            .service
                            .iter()
                            .flat_map(|b| b.chain())
                            .map(|b| &b.resume_status),
                    )
                    .find_map(|status| match status {
                        Status::AwaitingCreation(r)
                            if r.id == state.request && r.created == Some(id) =>
                        {
                            Some(r)
                        }
                        _ => None,
                    })
                    .ok_or("native person factory parent binding mismatch")?;
                if state.off_lot != (request.operands[4] == 6)
                    || state.off_lot && request.placement.is_some()
                    || state.visitor.is_some() && request.operands[5] & 4 == 0
                {
                    return Err("native person factory wrapper mode mismatch".into());
                }
                if !state.wrapper_finished && !self.unplaced.contains(&id) {
                    return Err("unfinished native person factory is placed".into());
                }
                if let Some(e) = &state.acceptance {
                    let to = request
                        .placement
                        .as_ref()
                        .ok_or("native factory acceptance target absent")?;
                    if !matches!(
                        (request.operands[4], request.operands[5]),
                        (8, 0x20) | (5, 4) | (5, 0x14)
                    ) || e.binding() != (id, [to.position[1], to.position[0]], to.level)
                    {
                        return Err("native factory acceptance destination mismatch".into());
                    }
                }
                if let Some(e) = &state.placement {
                    let to = request
                        .placement
                        .as_ref()
                        .ok_or("native person placement target absent")?;
                    if !matches!(
                        (request.operands[4], request.operands[5]),
                        (8, 0x20) | (5, 4) | (5, 0x14)
                    ) || e.operation.binding().1
                        != Some(crate::lot_links::Descriptor {
                            on_lot: true,
                            position: [to.position[1], to.position[0]],
                            level: to.level,
                            container: None,
                            slot: -1,
                        })
                    {
                        return Err("native person placement destination mismatch".into());
                    }
                }
            }
        }
        Ok(())
    }

    /// None means this request belongs to the existing non-person dispatcher.
    pub(crate) fn service_native_person_creation(
        &mut self,
        p: &Program,
        thread: ThreadId,
        mut r: create::Request,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Option<bool>, String> {
        if self.native_lot.is_none()
            || !matches!(
                (r.operands[4], r.operands[5]),
                (6, 0 | 1 | 4) | (8, 0x20) | (5, 4) | (5, 0x14)
            )
        {
            return Ok(None);
        }
        if r.created.is_some_and(|id| {
            self.person_factories
                .get(&id)
                .is_some_and(|s| s.rejection_cleanup.is_some())
        }) {
            return self
                .drive_rejected_person_creation(p, thread, &r, remaining, step)
                .map(Some);
        }
        let guid = if let Some(id) = r.created {
            self.vm.world.objects[&id]
                .definition
                .as_ref()
                .ok_or("created definition absent")?
                .guid
        } else if let Some(guid) = self.vm.creation_guid(&r)? {
            guid
        } else {
            self.vm.finish_creation(p, thread, r.id, None)?;
            return Ok(Some(true));
        };
        if r.created.is_none() {
            let mut matches = p.modules.values().flat_map(|m| {
                m.object_definition_words
                    .iter()
                    .filter_map(move |(&number, w)| {
                        (w.len() > 20
                            && (u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16)) == guid)
                            .then_some((m, number, w))
                    })
            });
            let (module, number, words) = matches
                .next()
                .ok_or_else(|| format!("Create Object GUID {guid:08X} is not loaded"))?;
            if matches.next().is_some() {
                return Err("ambiguous Create Object GUID".into());
            }
            if words[9] != 2 {
                return Ok(None);
            }
            if r.operands[4] == 6
                && r.operands[5] == 1
                && crate::lot_creation::live::duplicate_at(
                    self,
                    [-1, -1, 1],
                    &module.source,
                    number,
                )?
            {
                self.vm.finish_creation(p, thread, r.id, None)?;
                return Ok(Some(true));
            }
            if matches!(r.operands[4], 5 | 8) {
                let frame = self.vm.threads[&thread]
                    .stack
                    .last()
                    .ok_or("creation frame absent")?;
                let target = if r.operands[4] == 5 {
                    r.caller
                } else {
                    let i = usize::try_from(r.operands[6] as i8)
                        .map_err(|_| "creation local invalid (native error 0x08)")?;
                    let target = *frame
                        .locals
                        .get(i)
                        .ok_or("creation local invalid (native error 0x08)")?;
                    if frame.opaque_locals.contains(&i) {
                        return Err("creation reads opaque local".into());
                    }
                    target
                };
                if !self.vm.world.objects.contains_key(&target) {
                    self.vm.finish_creation(p, thread, r.id, None)?;
                    return Ok(Some(true));
                }
                let position = *self
                    .vm
                    .world
                    .positions
                    .get(&target)
                    .ok_or("native creation reference position absent")?;
                if !self
                    .native_lot
                    .as_ref()
                    .unwrap()
                    .placement
                    .links
                    .on_lot([position.x, position.y])
                {
                    self.vm.finish_creation(p, thread, r.id, None)?;
                    return Ok(Some(true));
                }
                r.placement = Some(create::Placement {
                    target,
                    position: [position.y, position.x],
                    level: position.level,
                    room: -5,
                    wall_direction: None,
                });
            }
            // This process-owned stream is not saved in SIMI. The importing
            // runtime must choose it explicitly; never borrow a saved RNG word.
            if self.vm.crt_rng_state.is_none() {
                return Err("person factory CRT random state not supplied".into());
            }
            let clock = &self
                .household
                .as_ref()
                .ok_or("native person factory requires household clock")?
                .clock;
            let date = sims_vm::construction::ObjectDate {
                year: clock.year,
                month: clock.month,
                day: clock.day,
            };
            let definition = Definition {
                source: module.source.clone(),
                object: number,
                guid,
                slots: words[20] as u16,
            };
            let Some(factory) = Factory::begin(
                &mut self.vm,
                p,
                definition,
                date,
                [clock.hour, clock.minute],
            )?
            else {
                self.vm.finish_creation(p, thread, r.id, None)?;
                return Ok(Some(true));
            };
            let id = self.register_person_factory(factory, Some(thread), r.id, r.operands[4] == 6);
            r.created = Some(id);
            self.vm.threads.get_mut(&thread).unwrap().status = Status::AwaitingCreation(r.clone());
        }
        let id = r.created.unwrap();
        if !self.person_factories.contains_key(&id) {
            return Ok(None);
        }
        // Temporarily move the token out so nested VM services can borrow the
        // simulation. Reinsert it even on error: the stopped state stays resumable.
        let mut state = self.person_factories.remove(&id).unwrap();
        let result = self.drive_native_person_factory(p, id, &mut state, remaining, step);
        self.person_factories.insert(id, state);
        if !result? {
            return Ok(Some(false));
        }
        if !self.person_factories[&id].wrapper_finished {
            let person = self
                .vm
                .world
                .objects
                .get_mut(&id)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            self.vm
                .world
                .person
                .as_mut()
                .unwrap()
                .attach_person_neighbor(
                    person,
                    guid,
                    sims_vm::person_factory::NeighborAttachment::Traits,
                )?;
            if r.operands[5] & 0x20 != 0 {
                // Flag 0x20 copies STACK OBJECT facing, not the mode-8 local's.
                let stack_object = self.vm.threads[&thread]
                    .stack
                    .last()
                    .ok_or("creator frame absent after factory")?
                    .stack_object;
                if let Some(target) = self.vm.world.objects.get(&stack_object) {
                    let facing = *target.data.get(&1).ok_or("creation source facing absent")?;
                    self.vm
                        .world
                        .objects
                        .get_mut(&id)
                        .unwrap()
                        .data
                        .insert(1, facing);
                }
            }
            let person = self.vm.world.objects[&id].person.as_ref().unwrap();
            if person[&84] & 1 != 0 || person[&68] & 1 != 0 || person[&74] & 3 != 0 {
                // 44e70d uses the selected RESOURCE as 45810c's receiver,
                // then unconditionally marks the UI, even on a cache miss.
                self.vm.world.image_cache.invalidate(guid)?;
                self.vm.world.image_cache.invalidate_ui()?;
            }
            self.person_factories.get_mut(&id).unwrap().wrapper_finished = true;
        }
        // 44e0c8 skips only the physical block for mode 6. The visitor and
        // parameter tail below also run for off-lot neighbor construction.
        if !self.person_factories[&id].off_lot {
            if self.person_factories[&id].placement.is_none() {
                let to = r
                    .placement
                    .as_ref()
                    .ok_or("person creation position absent")?;
                if self.person_factories[&id].acceptance.is_none() {
                    let e = crate::lot_acceptance::live::Execution::new(
                        self,
                        id,
                        [to.position[1], to.position[0]],
                        to.level,
                    )?;
                    self.person_factories.get_mut(&id).unwrap().acceptance = Some(e);
                }
                if !self.drive_factory_acceptance(p, id, remaining, step)? {
                    return Ok(Some(false));
                }
                let outcome = self.person_factories[&id]
                    .acceptance
                    .as_ref()
                    .unwrap()
                    .outcome()
                    .ok_or("factory acceptance result absent")?;
                if !outcome.accepted {
                    let definition = self.vm.world.objects[&id]
                        .definition
                        .clone()
                        .ok_or("rejected person resource absent")?;
                    let lifecycle = self
                        .vm
                        .lifecycles
                        .get(&id)
                        .ok_or("rejected person lifecycle absent")?;
                    if lifecycle.owner != definition.source
                        || lifecycle.definition != definition.object
                    {
                        return Err("rejected person committed lifecycle resource differs".into());
                    }
                    let cleanup = crate::lot_removal::immediate::Removal::new(self, p, id)?;
                    let state = self.person_factories.get_mut(&id).unwrap();
                    state.rejected_definition = Some(definition);
                    state.rejection_cleanup = Some(cleanup);
                    return self
                        .drive_rejected_person_creation(p, thread, &r, remaining, step)
                        .map(Some);
                }
                self.person_factories.get_mut(&id).unwrap().placement = Some(
                    lot_placement::live::Execution::new(lot_placement::Operation::set(
                        id,
                        crate::lot_links::Descriptor {
                            on_lot: true,
                            position: [to.position[1], to.position[0]],
                            level: to.level,
                            container: None,
                            slot: -1,
                        },
                        true,
                    )),
                );
            }
            let mut placement = self
                .person_factories
                .get_mut(&id)
                .unwrap()
                .placement
                .take()
                .unwrap();
            let result = self.drive_live_placement(p, &mut placement, remaining, step);
            self.person_factories.get_mut(&id).unwrap().placement = Some(placement);
            if !result? {
                return Ok(Some(false));
            }
        }
        if self.person_factories[&id].visitor.is_none()
            && r.operands[5] & 4 != 0
            && self.vm.world.objects[&id].person.as_ref().unwrap()[&32] != 0
        {
            if let Some((owner, bhav)) = self.vm.resolve_named_tree(p, id, "init visitor")? {
                if self
                    .person_factories
                    .values()
                    .any(|s| s.visitor.as_ref().is_some_and(|v| !v.complete))
                {
                    return Err(
                        "nested person visitor construction requires a callback owner stack".into(),
                    );
                }
                if !self.vm.named_calls.is_empty() {
                    return Err("person visitor construction inside named execution requires a callback owner stack".into());
                }
                let child = self.vm.object_threads[&id];
                let depth = self.vm.threads[&child].stack.len();
                self.vm.begin_immediate_callback(
                    p,
                    child,
                    sims_vm::Start {
                        owner: &owner,
                        bhav,
                        caller: id,
                        callee: id,
                        stack_object: id,
                        args: self.vm.world.objects[&id].temps.to_vec(),
                        check: true,
                    },
                )?;
                self.person_factories.get_mut(&id).unwrap().visitor = Some(Visitor {
                    thread: child,
                    depth,
                    owner,
                    bhav,
                    complete: false,
                });
                self.event(
                    &mut step.events,
                    thread,
                    format!(
                        "created person #{id}: original init visitor begins {}",
                        if self.person_factories[&id].off_lot {
                            "off-lot"
                        } else {
                            "after placement"
                        }
                    ),
                );
            }
        }
        if !self.drive_person_visitor(p, id, remaining, step)? {
            return Ok(Some(false));
        }
        self.vm
            .finish_creation_with_parameters(p, thread, r.id, id)?;
        self.person_factories.get_mut(&id).unwrap().finalized = true;
        self.event(
            &mut step.events,
            thread,
            format!(
                "created person #{id}: wrapper completed {} and optional visitor callback",
                if self.person_factories[&id].off_lot {
                    "off-lot"
                } else {
                    "after placement"
                }
            ),
        );
        Ok(Some(true))
    }

    /// Resume the synchronous wrapper before generic end-of-tick deletion can
    /// consume its child and erase the still-suspended caller owner.
    pub(crate) fn drive_pending_rejected_person_cleanup(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let pending: Vec<_> = self
            .person_factories
            .iter()
            .filter_map(|(&id, s)| {
                s.rejection_cleanup
                    .as_ref()
                    .map(|_| (id, s.parent, s.request))
            })
            .collect();
        if pending.len() > 1 {
            return Err("overlapping rejected person cleanup owners".into());
        }
        for (id, parent, request) in pending {
            let thread = parent.ok_or("rejected person cleanup has no caller")?;
            let r = match &self
                .vm
                .threads
                .get(&thread)
                .ok_or("rejected person caller absent")?
                .status
            {
                Status::AwaitingCreation(r) if r.id == request && r.created == Some(id) => {
                    r.clone()
                }
                _ => {
                    return Err(
                        "rejected person cleanup caller is not suspended at creation".into(),
                    );
                }
            };
            if !self.drive_rejected_person_creation(p, thread, &r, remaining, step)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn drive_rejected_person_creation(
        &mut self,
        p: &Program,
        thread: ThreadId,
        request: &create::Request,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let id = request.created.ok_or("rejected person instance absent")?;
        let state = self
            .person_factories
            .get(&id)
            .ok_or("rejected person owner absent")?;
        state.retired_creation(self, p)?;
        if state.parent != Some(thread) || state.request != request.id {
            return Err("rejected person caller differs".into());
        }
        // Deletion unlinks the object and removes any completed factory token.
        // Keep this exact wrapper locally until its44123f child returns; on a
        // budget/asset yield the retained owner survives without object repair.
        let mut state = self.person_factories.remove(&id).unwrap();
        let result = self.drive_immediate_removal(
            p,
            state.rejection_cleanup.as_mut().unwrap(),
            remaining,
            step,
        );
        match result {
            Ok(true) => {
                state
                    .rejection_cleanup
                    .as_ref()
                    .unwrap()
                    .validate(self, p, true)?;
                self.vm.finish_creation(p, thread, request.id, None)?;
                self.event(
                    &mut step.events,
                    thread,
                    format!("created person #{id}: rejected placement removed synchronously"),
                );
                Ok(true)
            }
            other => {
                self.person_factories.insert(id, state);
                other
            }
        }
    }

    fn drive_person_visitor(
        &mut self,
        p: &Program,
        id: ObjectId,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        loop {
            let Some(v) = self.person_factories[&id].visitor.clone() else {
                return Ok(true);
            };
            if v.complete {
                return Ok(true);
            }
            if !v.owns_callback(&self.vm, v.thread, v.depth) {
                self.vm.threads[&v.thread]
                    .callback_result
                    .ok_or("visitor callback result absent")?;
                self.person_factories
                    .get_mut(&id)
                    .unwrap()
                    .visitor
                    .as_mut()
                    .unwrap()
                    .complete = true;
                self.event(
                    &mut step.events,
                    v.thread,
                    format!("created person #{id}: original init visitor returned"),
                );
                return Ok(true);
            }
            if *remaining == 0 {
                return Ok(false);
            }
            // The synchronous tree cannot advance Main or lot time. A single
            // instruction exposes its return before the suspended root runs.
            let thread = self.vm.named_calls.last().map_or(v.thread, |c| {
                if c.result.is_some() {
                    c.requester
                } else {
                    c.recipient
                }
            });
            match self.vm.threads[&thread].status.clone() {
                Status::Ready => {
                    let slice = self.vm.run_slice(p, v.thread, 1)?;
                    if slice.trace.is_empty() {
                        return Err("visitor callback made no progress".into());
                    }
                    *remaining -= slice.trace.len();
                    step.trace.extend(slice.trace);
                }
                Status::AwaitingCreation(r) => {
                    if !self.service_creation(p, thread, r, remaining, step)? {
                        return Ok(false);
                    }
                }
                Status::AwaitingTransfer(r) => {
                    self.service_transfer(p, thread, &r, &mut step.events)?
                }
                Status::AwaitingPlacement(r) => {
                    self.service_find_location(p, thread, &r, &mut step.events)?
                }
                Status::AwaitingRemoval(r) => {
                    if !self.service_removal(p, thread, &r, remaining, step)? {
                        return Ok(false);
                    }
                }
                Status::Faulted(f) => {
                    return Err(format!("visitor callback failed: {}", f.message));
                }
                s => {
                    return Err(format!(
                        "visitor callback host required (must finish synchronously): {s:?}"
                    ));
                }
            }
        }
    }

    pub(crate) fn drive_pending_person_visitors(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let pending: Vec<_> = self
            .person_factories
            .iter()
            .rev()
            .filter_map(|(&id, s)| s.visitor.as_ref().filter(|v| !v.complete).map(|_| id))
            .collect();
        if pending.len() > 1 {
            return Err("overlapping synchronous person visitors".into());
        }
        for id in pending {
            if !self.drive_person_visitor(p, id, remaining, step)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn drive_factory_acceptance(
        &mut self,
        p: &Program,
        id: ObjectId,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let mut query = self
            .person_factories
            .get_mut(&id)
            .unwrap()
            .acceptance
            .take()
            .ok_or("factory floor query absent")?;
        let result = self.drive_native_acceptance(p, &mut query, remaining, step);
        self.person_factories.get_mut(&id).unwrap().acceptance = Some(query);
        result
    }
    pub(crate) fn drive_pending_factory_acceptance(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let pending: Vec<_> = self
            .person_factories
            .iter()
            .filter_map(|(&id, s)| {
                s.acceptance
                    .as_ref()
                    .filter(|q| q.outcome().is_none())
                    .map(|_| id)
            })
            .collect();
        if pending.len() > 1 {
            return Err("overlapping synchronous factory floor checks".into());
        }
        for id in pending {
            if !self.drive_factory_acceptance(p, id, remaining, step)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn drive_native_person_factory(
        &mut self,
        p: &Program,
        id: ObjectId,
        state: &mut State,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        while let Some(request) = state.factory.next(&mut self.vm, p)? {
            let mut residential = None;
            match request {
                Request::Registered(object) => {
                    if object != id {
                        return Err("factory identity mismatch".into());
                    }
                }
                Request::Post(Post::Base) => {
                    self.vm.prepare_person_base_postconstruction(p, id)?;
                    self.native_lot
                        .as_mut()
                        .ok_or("person postconstructor native lot absent")?
                        .placement
                        .postconstruct_bounds(&self.vm.world, p, id)?;
                    state.presentation.push(Presentation::BaseCaches);
                }
                Request::Post(Post::BindAnimator) => {
                    if self.native_character_settings.is_some() {
                        // Imported live characters require the actual animator
                        // constructor before Init or the constructor's person
                        // update. The older diagnostic only records this work.
                        return Ok(false);
                    }
                    sims_vm::person_reset::reload_bound_profile(&mut self.vm, p, id)?;
                    state.presentation.push(Presentation::BindAnimator);
                }
                Request::Post(Post::AutonomyConstants) => {
                    //The constructor owns the saved producer child. Only a
                    //source-bound host reply permits it to continue; no VM
                    //instruction budget or script yield is consumed here.
                    return Ok(false);
                }
                Request::Post(Post::InitialScores) => {
                    state.scores = sims_vm::person_loading::initial_score_curves();
                    self.vm.world.objects.get_mut(&id)
                        .ok_or("initial score person absent")?.native_motive_scores =
                        Some(sims_vm::motives::score_owner::Owner::capture(&state.scores)?);
                }
                Request::Post(Post::RefreshResourceProfile) => {
                    // Program binds the immutable original body StringSet; resource
                    // cache reload has no further gameplay writes at this boundary.
                }
                Request::Reset(Reset::Script(child)) => {
                    if *remaining == 0 {
                        return Ok(false);
                    }
                    // Stop after each instruction so the Init callback boundary
                    // cannot accidentally run its suspended Main a second time.
                    self.vm.prepare_factory_object_visit(id)?;
                    let slice = self.vm.run_slice(p, child, 1)?;
                    if slice.trace.is_empty()
                        && self.vm.threads[&child].callback.is_some()
                        && matches!(self.vm.threads[&child].status, Status::Sleeping { .. })
                    {
                        return Err(
                            "native person Init suspended before synchronous completion".into()
                        );
                    }
                    *remaining -= slice.trace.len();
                    step.trace.extend(slice.trace);
                    match &self.vm.threads[&child].status {
                        Status::Ready | Status::Sleeping { .. } | Status::Returned(_) => {}
                        other => {
                            return Err(format!(
                                "native person factory script host required: {other:?}"
                            ));
                        }
                    }
                    continue;
                }
                Request::Reset(Reset::Service(Service::ReloadAppearance)) => {
                    if self.native_character_settings.is_some() {
                        return Ok(false);
                    }
                    self.vm
                        .world
                        .objects
                        .get_mut(&id)
                        .unwrap()
                        .accessories
                        .clear();
                    sims_vm::person_reset::reload_bound_profile(&mut self.vm, p, id)?;
                    state.presentation.push(Presentation::ReloadAppearance);
                }
                Request::Reset(Reset::Service(Service::AttachNeighbor(mode))) => {
                    let object = self.vm.world.objects.get_mut(&id).unwrap();
                    let guid = object.definition.as_ref().unwrap().guid;
                    self.vm
                        .world
                        .person
                        .as_mut()
                        .unwrap()
                        .attach_person_neighbor(object.person.as_mut().unwrap(), guid, mode)?;
                }
                Request::Reset(Reset::ImmediatePersonUpdate { object, tick }) => {
                    if object != id || tick != 0 {
                        return Err("invalid constructor update".into());
                    }
                    if self.native_character_settings.is_some() {
                        let thread = *self
                            .vm
                            .object_threads
                            .get(&id)
                            .ok_or("constructor thread absent")?;
                        if self.characters[&id].person_update.is_none() {
                            self.vm.finish_object_update(id, true)?;
                            self.characters.get_mut(&id).unwrap().person_update =
                                Some(crate::character_update::PersonPass {
                                    thread,
                                    execution: crate::person_update::Execution::new(0),
                                    constructor: true,
                                    notification_started: false,
                                });
                        }
                        if !self.drive_native_person_tail(p, id, thread, remaining, step)? {
                            return Ok(false);
                        }
                        self.vm.finish_factory_object_visit(id)?;
                        state.factory.complete(&mut self.vm, p, request, None)?;
                        continue;
                    }
                    self.vm.finish_object_update(id, true)?;
                    let o = self.vm.world.objects.get_mut(&id).unwrap();
                    let mut person = [0; 256];
                    for (&i, &v) in o.person.as_ref().unwrap() {
                        person[i as usize] = v;
                    }
                    let h = self.household.as_mut().unwrap();
                    o.motive_update.as_mut().unwrap().finish(
                        o.motives.as_mut().unwrap(),
                        &mut person,
                        o.data[&34] != 0,
                        tick,
                        self.vm.world.globals[&0],
                        &h.tuning,
                        None,
                        &o.motive_changes,
                    );
                    o.person.as_mut().unwrap().insert(27, person[27]);
                    h.people.get_mut(&id).unwrap().finished = Some(self.vm.tick);
                    self.vm.finish_factory_object_visit(id)?;
                }
                Request::Reset(Reset::Service(Service::IsResidential)) => {
                    let lot = *self
                        .vm
                        .world
                        .globals
                        .get(&10)
                        .ok_or("person factory current house absent")?;
                    let zoning = &self.vm.world.person.as_ref().unwrap().lot_zoning;
                    residential = Some(sims_vm::zoning::is_residential(zoning, lot));
                }
                Request::Reset(Reset::Service(Service::RandomizeTraits)) => {
                    self.vm.randomize_person_traits(id)?
                }
                Request::Reset(Reset::Service(Service::ClearAnimator)) => {
                    if self.native_character_settings.is_some() {
                        self.reset_character(id)?;
                    } else if self.animations.contains_key(&id)
                        || self.backgrounds.contains_key(&id)
                        || self.characters.contains_key(&id)
                    {
                        return Err("constructor animator unexpectedly active".into());
                    }
                    state.presentation.push(Presentation::ClearAnimator);
                }
                Request::Reset(Reset::Service(Service::LoadScores)) => {
                    state.scores =
                        sims_vm::person_loading::load_score_curves(&self.vm.world, p, id)?;
                    self.vm.world.objects.get_mut(&id)
                        .ok_or("reset score person absent")?.native_motive_scores =
                        Some(sims_vm::motives::score_owner::Owner::capture(&state.scores)?);
                }
                Request::Activate(_) => {
                    state.native_flags |= 0x80;
                    state.active = true;
                }
                Request::Created(object) => self.native_lot.as_mut().unwrap().observers.push(
                    lot_placement::Request::Signal {
                        event: 0xe2,
                        value: object,
                    },
                ),
                other => return Err(format!("person factory service required: {other:?}")),
            }
            state
                .factory
                .complete(&mut self.vm, p, request, residential)?;
        }
        Ok(true)
    }
}

#[cfg(test)]
#[path = "person_creation_tests.rs"]
mod tests;
