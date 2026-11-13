//! Ordered original loading through the per-object post-load callbacks.
//! This produces frozen storage plus explicit remaining host work, not a live
//! household. Original files are read-only and ordinary Main/time stay frozen.
use crate::{Index, save_storage::Execution, saves::House};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sims_sim::{architecture::Architecture, lot_creation, lot_placement, lot_reset};
use sims_vm::{
    Definition, ObjectId, Program, Status,
    loading::{HouseContext, PostLoad},
};

#[derive(Debug, Clone, Serialize)]
pub struct Visit {
    pub id: ObjectId,
    pub definition: Definition,
    pub decision: PostLoad,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AnimatorEffect {
    Reset(ObjectId),
    ReloadAppearance(ObjectId),
    ResetPlacementCache(ObjectId),
    Rebuild(ObjectId),
}
type AnimatorHost<'a> = dyn FnMut(
        AnimatorEffect,
        &mut Execution,
        &mut Program,
        &Architecture,
        &mut lot_placement::State,
    ) -> Result<()>
    + 'a;
/// One owner services both deserialization and subsequent ordered post-load
/// effects. This prevents callback adapters from substituting reconstructed or
/// stale character state between those native phases.
pub trait NativeAnimatorHost:
    crate::save_storage::PersonReader + crate::house_startup::AssetHost
{
    ///4376f6 inside a nested indicator factory, before its Reset/Init.
    fn initialize_graphic(
        &mut self,
        vm: &sims_vm::Vm,
        program: &Program,
        id: ObjectId,
    ) -> Result<()>;
    fn post_load(
        &mut self,
        effect: AnimatorEffect,
        execution: &mut Execution,
        program: &mut Program,
        architecture: &Architecture,
        placement: &mut lot_placement::State,
    ) -> Result<()>;
}
enum AnimatorServices<'a> {
    Record,
    Effects(&'a mut AnimatorHost<'a>),
    Native(&'a mut dyn NativeAnimatorHost),
}

/// The catalog is borrowed only while the saved-person child is running. The
/// same caller-owned resource owner remains available to other load phases.
struct CatalogPersonReader<'s, 'c> {
    host: &'s mut dyn NativeAnimatorHost,
    catalog: &'s mut crate::object_catalog::Catalog<'c>,
    provider: &'s mut crate::autonomy_resources::CatalogProvider,
}
impl crate::save_storage::PersonReader for CatalogPersonReader<'_, '_> {
    fn process_input(&mut self, world: &mut sims_vm::World) -> Result<()> {
        self.host.process_input(world)
    }
    fn initialize(
        &mut self,
        program: &mut Program,
        world: &mut sims_vm::World,
        id: ObjectId,
        object: sims_vm::Object,
    ) -> Result<sims_vm::Object> {
        self.host
            .initialize_with_catalog(program, world, id, object, self.catalog, self.provider)
    }
    fn read_animator(
        &mut self,
        program: &mut Program,
        world: &mut sims_vm::World,
        id: ObjectId,
        object: sims_vm::Object,
        saved: &sims_formats::saved_objects::Person,
    ) -> Result<sims_vm::Object> {
        self.host.read_animator(program, world, id, object, saved)
    }
}
impl AnimatorServices<'_> {
    fn prepare(&mut self, house: &House, program: &mut Program) -> Result<Execution> {
        match self {
            Self::Native(host) => house.prepare_execution_with_person_reader(program, *host),
            _ => house.prepare_execution(program),
        }
    }
    fn graphic(&mut self, vm: &sims_vm::Vm, program: &Program, id: ObjectId) -> Result<()> {
        match self {
            Self::Native(host) => host.initialize_graphic(vm, program, id),
            _ => anyhow::bail!("nested indicator requires a native graphic owner"),
        }
    }
    fn effect(
        &mut self,
        effect: AnimatorEffect,
        execution: &mut Execution,
        program: &mut Program,
        architecture: &Architecture,
        placement: &mut lot_placement::State,
    ) -> Result<bool> {
        match self {
            Self::Record => return Ok(false),
            Self::Effects(host) => host(effect, execution, program, architecture, placement)?,
            Self::Native(host) => {
                host.post_load(effect, execution, program, architecture, placement)?
            }
        }
        Ok(true)
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct ResidentTail {
    pub id: ObjectId,
    pub room_effects: Vec<sims_vm::person_loading::RoomEffect>,
    pub scores: Vec<sims_vm::motives::Curve>,
}
/// All callback mutations are shared. Effects remain ordered within their
/// native service; the live adapter must consume them before publishing play.
pub struct FrozenLot {
    pub selected_family: i16,
    pub family_reset_requested: u8,
    pub residents_before_visits: Vec<ObjectId>,
    pub indicators: sims_vm::load_indicators::Indicators,
    pub program: Program,
    pub execution: Execution,
    pub architecture: Architecture,
    pub placement: lot_placement::State,
    pub membership: sims_vm::person_loading::RoomMembership,
    /// Ordered occupancy/presentation work from the pre-visit room pass and
    /// subsequent person visits, consumed by the retained environment owner.
    pub room_effects: Vec<sims_vm::person_loading::RoomEffect>,
    pub visits: Vec<Visit>,
    pub observers: Vec<lot_placement::Request>,
    pub animator_effects: Vec<AnimatorEffect>,
    pub resident_tails: Vec<ResidentTail>,
    pub initial_visit_count: usize,
}
impl House {
    /// Run the supported native load/reset profile. Failed callbacks/unsupported
    /// continuations abort the load; no partially loaded VM is returned as play.
    /// `context` supplies the caller's native simulator state, not demo defaults.
    pub fn load_frozen(&self, index: &Index, context: HouseContext) -> Result<FrozenLot> {
        self.load_frozen_inner(index, context, AnimatorServices::Record, None)
    }

    /// Consume native animator services at their actual post-load call sites.
    /// The host owns characters already restored through the serialized read
    /// tail. Returning Ok means the whole requested service completed. The host
    /// must fail if a service is unavailable, preventing later callbacks from
    /// observing a partially applied Reset. `load_frozen` is the separate
    /// diagnostic path that records outstanding animator work.
    /// ReloadAppearance includes its native profile writes, not only meshes.
    /// Errors abort before later callbacks; lot time remains frozen throughout.
    pub fn load_frozen_with_animators(
        &self,
        index: &Index,
        context: HouseContext,
        animator: &mut impl FnMut(
            AnimatorEffect,
            &mut Execution,
            &mut Program,
            &Architecture,
            &mut lot_placement::State,
        ) -> Result<()>,
    ) -> Result<FrozenLot> {
        self.load_frozen_inner(index, context, AnimatorServices::Effects(animator), None)
    }
    /// Complete character reads and animator post-load services with the same
    /// owner. Ordinary scheduling still belongs to household activation.
    pub fn load_frozen_with_native_characters(
        &self,
        index: &Index,
        context: HouseContext,
        host: &mut dyn NativeAnimatorHost,
    ) -> Result<FrozenLot> {
        self.load_frozen_inner(index, context, AnimatorServices::Native(host), None)
    }
    /// Continue a pre-read owner through resize, teardown and the saved read.
    pub fn load_frozen_with_retained_startup(
        &self,
        index: &Index,
        context: HouseContext,
        host: &mut dyn NativeAnimatorHost,
        incoming: crate::house_startup::Incoming,
    ) -> Result<FrozenLot> {
        self.load_frozen_inner(
            index,
            context,
            AnimatorServices::Native(host),
            Some(incoming),
        )
    }

    /// Continue one actual catalog owner through metadata inspection, resize,
    /// teardown and saved-person initialization. The caller retains it after
    /// return; this path never registers replacement files or installs a second
    /// Global provider. A live/managed resource bundle remains caller-owned.
    pub fn load_frozen_with_retained_catalog(
        &self,
        index: &Index,
        context: HouseContext,
        host: &mut dyn NativeAnimatorHost,
        incoming: crate::house_startup::Incoming,
        resources: &mut crate::object_catalog::Catalog<'_>,
        provider: &mut crate::autonomy_resources::CatalogProvider,
    ) -> Result<FrozenLot> {
        provider.validate_catalog(resources)?;
        host.validate_catalog_resources(resources, provider)?;
        let process = incoming
            .vm
            .world
            .autonomy_constants
            .as_ref()
            .context("retained process owner absent")?;
        ensure!(
            process.configuration.source.as_ref() == Some(&provider.binding()),
            "retained process and catalog Global sources differ"
        );
        self.load_frozen_in_catalog(
            index,
            context,
            AnimatorServices::Native(host),
            Some(incoming),
            resources,
            Some(provider),
        )
    }

    fn load_frozen_inner(
        &self,
        index: &Index,
        context: HouseContext,
        animator: AnimatorServices<'_>,
        incoming: Option<crate::house_startup::Incoming>,
    ) -> Result<FrozenLot> {
        let house = self;
        let mut resources = crate::object_catalog::Catalog::household(
            std::sync::Arc::new(index.clone()),
            &house.neighborhood_source,
            Default::default(),
        )?;
        self.load_frozen_in_catalog(index, context, animator, incoming, &mut resources, None)
    }

    fn load_frozen_in_catalog(
        &self,
        index: &Index,
        context: HouseContext,
        mut animator: AnimatorServices<'_>,
        incoming: Option<crate::house_startup::Incoming>,
        resources: &mut crate::object_catalog::Catalog<'_>,
        mut shared_global: Option<&mut crate::autonomy_resources::CatalogProvider>,
    ) -> Result<FrozenLot> {
        let house = self;
        let catalog = if shared_global.is_some() {
            resources.retained_global_controllers(index)?
        } else {
            resources.global_controllers(index)?
        };
        let mut p = house.link_program(index)?;
        crate::catalog::link_controllers(index, &mut p, &catalog)?;
        let storage_catalog = if shared_global.is_some() {
            resources.retained_resource_storage(index)?
        } else {
            resources.resource_storage(index)?
        };
        ensure!(
            !storage_catalog.iter().any(|d| d.reset_per_lot),
            "resource-global per-lot reset profile requires native caller-state mapping"
        );
        p.resource_storage = Some(storage_catalog);
        p.neighbor_resource_flags = Some(
            resources
                .definitions
                .guids
                .iter()
                .map(|(&guid, &i)| (guid, resources.definitions.entries[i].flags))
                .collect(),
        );
        let retained = incoming.is_some();
        let mut e = if let Some(incoming) = incoming {
            let guids: Vec<_> = incoming
                .vm
                .world
                .objects
                .values()
                .filter_map(|o| o.definition.as_ref().map(|d| d.guid))
                .collect();
            crate::assets::load_object_guids(index, &mut p, &guids)?;
            let expected =
                sims_vm::loading::house_resize_globals(&house.simulator.globals, context)
                    .map_err(anyhow::Error::msg)?;
            ensure!(
                incoming.vm.world.globals == expected,
                "retained resize globals differ from SIMI/context"
            );
            ensure!(
                incoming.vm.tick == u64::from(house.simulator.ticks)
                    && incoming.vm.rng_state == house.simulator.global_rng
                    && incoming.vm.lot_rng_state == Some(house.simulator.lot_rng),
                "retained owner is not at this house's post-SIMI boundary"
            );
            let vm = match &mut animator {
                AnimatorServices::Native(host) => incoming.prepare_read_with_assets(
                    house,
                    &p,
                    &catalog,
                    |_, _, _| Ok(()),
                    *host,
                )?,
                _ => {
                    incoming.prepare_read(house, &p, &catalog, |_, _, _| Ok(()), &mut |_, _| {
                        Err("retained startup requires an owning pending-asset service".into())
                    })?
                }
            };
            match &mut animator {
                AnimatorServices::Native(host) => {
                    if let Some(provider) = shared_global.as_deref_mut() {
                        let mut reader = CatalogPersonReader {
                            host: *host,
                            catalog: resources,
                            provider,
                        };
                        house.import_saved_execution_with_person_reader(&mut p, &mut reader, vm)?
                    } else {
                        house.import_saved_execution_with_person_reader(&mut p, *host, vm)?
                    }
                }
                _ => house.import_saved_execution(&p, vm)?,
            }
        } else {
            let mut e = animator.prepare(house, &mut p)?;
            e.load_simulator_context(house, context)?;
            e
        };
        // 42814b selects the constructor-owned family-zero record before the
        // inner house load. select_loaded_household binds the resident FAMI
        // later, before post-load visits (4282a2).
        if !retained {
            e.vm.world.magic_history = Some(sims_vm::magic_history::Context::constructed(0, true));
        }
        // Residential queries execute inside newly constructed controller Main,
        // before post-load callbacks or publication of the household session.
        // This loader explicitly uses the extracted installation's master input;
        // neighborhood/installer source selection remains a separate concern.
        let zoning = crate::zoning::Zoning::load(
            index,
            "GameData_ALL_Recursive/MasterLotZoning.iff",
            context.installed_expansions,
        )?;
        e.vm.world
            .person
            .as_mut()
            .context("loaded neighborhood absent")?
            .lot_zoning = zoning.query_table();
        e.reconcile_global_controllers(&p, &catalog)?;
        let grid = e.repair_object_grid(&house)?;
        e.restore_placement_links(&house, &grid)?;
        let geometry = if let Some(map) = e.vm.world.map_size {
            house.lot_geometry_in_active(
                &grid.floors,
                map.checked_size(house.width).map_err(anyhow::Error::msg)?,
            )?
        } else {
            //Explicit older diagnostic imports have no retained pre-resize
            //owner. Keep their existing full-stride geometry, not a guessed48.
            house.lot_geometry(&grid.floors)?
        };
        let a = Architecture::from_original(geometry, &p).map_err(anyhow::Error::msg)?;
        a.publish_room_context(&mut e.vm.world);
        let mut placement = lot_placement::State::new_for_world(
            e.placement_links.take().context("placement links absent")?,
            &a,
            &e.vm.world,
        )
        .map_err(anyhow::Error::msg)?;
        let tick = e.vm.tick;
        let mut visits = vec![];
        let mut observers = vec![];
        let mut resident_tails = vec![];
        let mut membership = sims_vm::person_loading::RoomMembership::constructed(&e.vm.world);
        let mut room_effects = vec![];
        let rooms = a.outdoors.keys().copied().collect();
        let mut animator_effects = vec![];
        let mut indicators = sims_vm::load_indicators::Indicators::default();
        // Keep the shared host service outside the VM so ordinary saved stacks
        // and lot time never advance while a child factory is being driven.
        let host = |e: &mut Execution,
                    p: &mut sims_vm::Program,
                    placement: &mut lot_placement::State,
                    thread,
                    observers: &mut Vec<lot_placement::Request>|
         -> Result<(), String> {
            match e.vm.threads[&thread].status.clone() {
                Status::AwaitingCreation(_) => {
                    let mut factory = e
                        .begin_off_lot_creation(&index, p, &mut placement.links, thread)
                        .map_err(|e| e.to_string())?;
                    if let Some(id) = factory.object() {
                        placement.postconstruct_bounds(&e.vm.world, p, id)?;
                    }
                    while let Some(request) = factory.advance(&mut e.vm, p, 100_000)? {
                        match request {
                            lot_creation::Request::Created(id) => {
                                observers.push(lot_placement::Request::Signal {
                                    event: 0xe2,
                                    value: id,
                                })
                            }
                            other => return Err(format!("factory: {other:?}")),
                        }
                    }
                }
                Status::AwaitingRemoval(request) => {
                    e.vm.defer_offlot_removal(p, &mut e.removals, thread, &request)?
                }
                status => return Err(format!("host: {status:?}")),
            }
            Ok(())
        };
        //415eff's F0 broadcast calls442659 before428041 selects the family
        //and publishes its budget. The two room passes precede43fc94 visits.
        e.update_loaded_rooms(
            &mut p,
            &a,
            &mut membership,
            0,
            &mut room_effects,
            &mut |e, p, thread| host(e, p, &mut placement, thread, &mut observers),
        )?;
        //428041 calls428679 immediately after415eff: discard the registry's
        //pending ID queue, including requests from retained startup/Init/Main.
        //The following post-load callbacks may enqueue new removals normally.
        e.removals.clear();
        e.select_loaded_household(&house)?;
        let people = e.vm.world.person.as_ref().unwrap();
        let selected_family = people.controlled_family;
        let family_reset_requested = u8::from(
            people
                .family_metadata
                .get(&selected_family)
                .context("loaded family metadata absent")?
                .flags
                & 4
                != 0,
        );
        let mut residents_before_visits = vec![];
        for &id in &people.person_order {
            if *e.vm.world.objects[&id]
                .person
                .as_ref()
                .and_then(|p| p.get(&32))
                .context("loaded person visitor field absent")?
                == 0
            {
                residents_before_visits.push(id);
            }
        }
        let initial_visit_count = people.object_order.len();
        let mut current = people.object_order.first().copied();
        while let Some(id) = current {
            let o = &e.vm.world.objects[&id];
            let definition = o.definition.as_ref().unwrap().clone();
            let person = o.person.is_some();
            let kind =
                p.modules[&definition.source].object_definition_words[&definition.object][9] as u16;
            let decision = PostLoad::new(
                kind,
                e.vm.world.changed_resources.contains(&definition.guid),
                true,
                false,
            );
            let result = (|| -> Result<(), String> {
                if person {
                    e.attach_resident_neighbor(&house, id)
                        .map_err(|e| e.to_string())?;
                }
                let update = e
                    .restore_object_position(&a, id)
                    .map_err(|e| e.to_string())?;
                placement
                    .bounds
                    .extend(update.objects.iter().map(|q| (q.object, q.bounds)));
                for effect in update.effects {
                    match effect {
                        sims_sim::lot_loading::Effect::Room(room) => {
                            e.vm.refresh_room_event(0xf3, i32::from(room as u16))?
                        }
                        sims_sim::lot_loading::Effect::Invalidate(object) => {
                            observers.push(lot_placement::Request::Invalidate {
                                object,
                                children_only: true,
                            })
                        }
                    }
                }
                for stage in 0..3 {
                    if stage == 1 {
                        if !decision.reset {
                            continue;
                        }
                        // These fixtures have no active/queued action-use frames.
                        // Do not substitute this narrow branch for general reset.
                        let people = e.vm.world.person.as_ref().unwrap().person_order.clone();
                        for person in people {
                            let actor =
                                e.vm.world
                                    .actions
                                    .as_mut()
                                    .unwrap()
                                    .actors
                                    .get_mut(&person)
                                    .unwrap();
                            let callbacks = actor.remove_object_references(id)?;
                            if !callbacks.is_empty() {
                                return Err("notification needs queued-action callbacks".into());
                            }
                        }
                        let now = sims_vm::construction::ObjectDate {
                            year: e.vm.world.globals[&8],
                            month: e.vm.world.globals[&7],
                            day: e.vm.world.globals[&1],
                        };
                        if person {
                            use sims_vm::person_reset::{Request, Reset};
                            let saved = house.objects.instances[&(id as u16)]
                                .person
                                .as_ref()
                                .unwrap();
                            // Native hosts consume animator work synchronously. The
                            // frozen diagnostic path records that unfinished boundary
                            // and applies only the represented VM profile writes.
                            let mut reset = Reset::new(
                                id,
                                if e.removals.iter().any(|r| r.target == id) {
                                    0x40
                                } else {
                                    0
                                },
                            );
                            if e.vm.world.objects[&id].native_flags.is_some() {
                                reset.bind_storage(&e.vm)?;
                            }
                            while let Some(request) = reset.next(&mut e.vm, &p)? {
                                match request {
                                    Request::ClearAnimator(object) => {
                                        let effect = AnimatorEffect::Reset(object);
                                        if !animator
                                            .effect(effect, &mut e, &mut p, &a, &mut placement)
                                            .map_err(|e| e.to_string())?
                                        {
                                            animator_effects.push(effect);
                                        }
                                    }
                                    Request::ReloadAppearance(object) => {
                                        let effect = AnimatorEffect::ReloadAppearance(object);
                                        if !animator
                                            .effect(effect, &mut e, &mut p, &a, &mut placement)
                                            .map_err(|e| e.to_string())?
                                        {
                                            animator_effects.push(effect);
                                            e.vm.world
                                                .objects
                                                .get_mut(&id)
                                                .unwrap()
                                                .accessories
                                                .clear();
                                            sims_vm::person_reset::reload_bound_profile(
                                                &mut e.vm, &p, id,
                                            )?;
                                        }
                                    }
                                    Request::ClearRouting(_) if saved.routing_frames == 0 => {}
                                    Request::ClearActionIcons(_)
                                        if e.vm.threads[&e.vm.object_threads[&id]]
                                            .stack
                                            .iter()
                                            .all(|f| !f.action_icon) => {}
                                    Request::Placement {
                                        marked_for_deletion,
                                        ..
                                    } => {
                                        let d = placement.links.capture(&mut e.vm.world, id)?;
                                        if d.container.is_some()
                                            || (!d.on_lot && !marked_for_deletion)
                                        {
                                            return Err(format!(
                                                "person relocation needs placement search: {d:?}"
                                            ));
                                        }
                                    }
                                    Request::Script(thread) => {
                                        let mut budget = 100_000;
                                        while e.vm.threads[&thread].callback.is_some() {
                                            if budget == 0 {
                                                return Err("person Reset diagnostic budget".into());
                                            }
                                            let slice = e.vm.run_slice(&p, thread, budget)?;
                                            budget -= slice.trace.len();
                                            if e.vm.threads[&thread].callback.is_some()
                                                && e.vm.threads[&thread].status != Status::Ready
                                            {
                                                host(
                                                    &mut e,
                                                    &mut p,
                                                    &mut placement,
                                                    thread,
                                                    &mut observers,
                                                )?;
                                            }
                                        }
                                    }
                                    other => return Err(format!("person reset host: {other:?}")),
                                }
                            }
                        } else {
                            let mut reset = lot_reset::Reset::new(id);
                            while let Some(request) = reset.advance(
                                &mut e.vm,
                                &p,
                                &a,
                                &mut placement,
                                now,
                                e.removals.iter().any(|r| r.target == id),
                            )? {
                                match request {
                                    lot_reset::Request::Placement(request) => {
                                        if let lot_placement::Request::ViewPlaced(id) = request {
                                            //Finish the actual44752e child before Reset's
                                            //placement returns. Its own saved guards order
                                            //initialization, flags, RTTI and manager input.
                                            let native =
                                                matches!(animator, AnimatorServices::Native(_));
                                            let mut graphic =
                                                |vm: &sims_vm::Vm, p: &Program, object| {
                                                    animator.graphic(vm, p, object)
                                                };
                                            placed_stored_indicators(
                                                index,
                                                &mut indicators,
                                                &mut e,
                                                &mut p,
                                                &mut placement,
                                                &mut observers,
                                                id,
                                                Some(context.installed_expansions),
                                                if native { Some(&mut graphic) } else { None },
                                            )
                                            .map_err(|e| format!("nested ViewPlaced #{id}: {e}"))?;
                                        }
                                        observers.push(request)
                                    }
                                    lot_reset::Request::Host { thread, .. } => host(
                                        &mut e,
                                        &mut p,
                                        &mut placement,
                                        thread,
                                        &mut observers,
                                    )?,
                                }
                            }
                        }
                    } else if (stage == 0 && decision.load_callback)
                        || (stage == 2 && decision.init_callback)
                    {
                        if let Some(thread) =
                            e.vm.begin_loaded_function(&p, id, if stage == 0 { 2 } else { 0 })?
                        {
                            let mut budget = 100_000;
                            while e.vm.threads[&thread].callback.is_some() {
                                if budget == 0 {
                                    return Err("callback diagnostic budget".into());
                                }
                                let slice = e.vm.run_slice(&p, thread, budget)?;
                                budget -= slice.trace.len();
                                if e.vm.threads[&thread].callback.is_some()
                                    && e.vm.threads[&thread].status != Status::Ready
                                {
                                    host(&mut e, &mut p, &mut placement, thread, &mut observers)?;
                                }
                            }
                        }
                    }
                }
                // End of base post-load, before the person's subclass tail.
                e.vm.refresh_registry_flags(&p, id)?;
                if person {
                    if house.objects.version < 50
                        || !house
                            .household
                            .as_ref()
                            .unwrap()
                            .residents
                            .iter()
                            .any(|r| r.object as i16 == id)
                    {
                        return Err(
                            "person post-load needs old-version/placeholder conversion".into()
                        );
                    }
                    let effects = membership.update(&e.vm.world, id, &rooms, false)?;
                    room_effects.extend(effects.clone());
                    // Native +0x24 -> 5893a9 -> 589361 resets cached XY/heading,
                    // not layer speed. Then 0x459d23
                    // reloads score curves, then +0x54(0) rebuilds graphics.
                    // Lighting and animator presentation stay explicit observers.
                    let reset = AnimatorEffect::ResetPlacementCache(id);
                    if !animator
                        .effect(reset, &mut e, &mut p, &a, &mut placement)
                        .map_err(|e| e.to_string())?
                    {
                        animator_effects.push(reset);
                    }
                    let scores = sims_vm::person_loading::load_score_curves(&e.vm.world, &p, id)?;
                    e.vm.world
                        .objects
                        .get_mut(&id)
                        .ok_or("post-load score person absent")?
                        .native_motive_scores =
                        Some(sims_vm::motives::score_owner::Owner::capture(&scores)?);
                    let rebuild = AnimatorEffect::Rebuild(id);
                    if !animator
                        .effect(rebuild, &mut e, &mut p, &a, &mut placement)
                        .map_err(|e| e.to_string())?
                    {
                        animator_effects.push(rebuild);
                    }
                    resident_tails.push(ResidentTail {
                        id,
                        room_effects: effects,
                        scores,
                    });
                }
                Ok(())
            })();
            result
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("post-load object #{id} ({})", definition.source))?;
            visits.push(Visit {
                id,
                definition,
                decision,
            });
            // The native iterator reads this object's +0xe0 AFTER its visit.
            // A new factory registration at the head is therefore not revisited.
            let order = &e.vm.world.person.as_ref().unwrap().object_order;
            current = order
                .iter()
                .position(|&v| v == id)
                .and_then(|i| order.get(i + 1))
                .copied();
            ensure!(e.vm.tick == tick, "frozen loading advanced lot time");
        }
        Ok(FrozenLot {
            selected_family,
            family_reset_requested,
            residents_before_visits,
            indicators,
            program: p,
            execution: e,
            architecture: a,
            placement,
            membership,
            room_effects,
            visits,
            observers,
            animator_effects,
            resident_tails,
            initial_visit_count,
        })
    }
}

impl FrozenLot {
    /// Original post-visit indicator factories, before selected-person/star
    /// attachment. Their E2 notifications remain explicit presentation work.
    pub fn initialize_indicators(&mut self, index: &Index) -> Result<()> {
        self.initialize_indicators_inner(index, None)
    }
    /// Service actual4376f6 graphics before440b2d continues with date/Reset/Init.
    /// The storage-only entry above is retained for explicit load diagnostics.
    pub fn initialize_indicators_with_graphics(
        &mut self,
        index: &Index,
        graphics: &mut GraphicHost<'_>,
    ) -> Result<()> {
        self.initialize_indicators_inner(index, Some(graphics))
    }
    fn initialize_indicators_inner(
        &mut self,
        index: &Index,
        graphics: Option<&mut GraphicHost<'_>>,
    ) -> Result<()> {
        initialize_stored_indicators(
            index,
            &mut self.indicators,
            &mut self.execution,
            &mut self.program,
            &mut self.placement,
            &mut self.observers,
            graphics,
        )
        .map_err(anyhow::Error::msg)?;
        self.architecture
            .bind_footprints(&self.program)
            .map_err(anyhow::Error::msg)?;
        Ok(())
    }
}
/// Borrow actual retained inputs. Cache guards read only the cache owner,
/// without eagerly dereferencing its object; manager bits never alias global20.
struct PlacedView<'a> {
    world: &'a sims_vm::World,
    indicators: &'a sims_vm::load_indicators::Indicators,
    manager_expansions: Option<u16>,
}
impl PlacedView<'_> {
    fn object(&self, id: ObjectId) -> Result<&sims_vm::Object, String> {
        self.world
            .objects
            .get(&id)
            .ok_or_else(|| "ViewPlaced object absent after initialization".into())
    }
}
impl sims_vm::placed_indicators::View for PlacedView<'_> {
    fn enabled_byte(&self) -> Result<u8, String> {
        Ok(u8::from(!self.world.registry_ui_disabled))
    }
    fn primary(&self) -> Result<Option<ObjectId>, String> {
        Ok(self.indicators.primary)
    }
    fn secondary(&self) -> Result<Option<ObjectId>, String> {
        Ok(self.indicators.secondary)
    }
    fn flags(&self, id: ObjectId) -> Result<u32, String> {
        self.object(id)?
            .native_flags
            .ok_or_else(|| "ViewPlaced native flags absent".into())
    }
    fn is_person(&self, id: ObjectId) -> Result<bool, String> {
        Ok(self.object(id)?.person.is_some())
    }
    fn person_data(&self, id: ObjectId, index: i16) -> Result<i16, String> {
        self.object(id)?
            .person
            .as_ref()
            .and_then(|p| p.get(&index))
            .copied()
            .ok_or_else(|| "ViewPlaced person field absent".into())
    }
    fn installed_expansions(&self) -> Result<u16, String> {
        self.manager_expansions
            .ok_or_else(|| "ViewPlaced game manager installation input absent".into())
    }
    fn lookup(&self, id: i16) -> Result<Option<ObjectId>, String> {
        Ok((id > 0 && self.world.objects.contains_key(&id)).then_some(id))
    }
    fn outside(&self, id: ObjectId) -> Result<bool, String> {
        let p = self
            .world
            .positions
            .get(&id)
            .ok_or("ViewPlaced position absent")?;
        let size = self
            .world
            .map_size
            .as_ref()
            .ok_or("ViewPlaced active world owner absent")?
            .active_size;
        Ok(sims_vm::presenter::outside(
            [(p.x >> 4) as i8, (p.y >> 4) as i8, p.level as i8],
            size,
        ))
    }
    fn slot(&self, id: ObjectId) -> Result<Option<sims_vm::placed_indicators::Slot>, String> {
        let object = self.object(id)?;
        if object.person.is_none() {
            return Err("ViewPlaced nonperson slot metadata requires owning physical host".into());
        }
        Ok(object
            .slots
            .get(1)
            .map(|occupant| sims_vm::placed_indicators::Slot {
                flags: sims_vm::person::BUILTIN_SLOT_FLAGS[1],
                occupant: occupant.unwrap_or(0),
            }))
    }
    fn position(&self, id: ObjectId) -> Result<[i32; 3], String> {
        let p = self
            .world
            .positions
            .get(&id)
            .ok_or("ViewPlaced position absent")?;
        Ok([p.x, p.y, p.level])
    }
    fn graphic(&self, _: ObjectId) -> Result<sims_vm::placed_indicators::Graphic, String> {
        Err("ViewPlaced graphic requires owning native host".into())
    }
    fn transform(&self, _: ObjectId) -> Result<sims_vm::placed_indicators::Graphic, String> {
        Err("ViewPlaced transform requires owning native host".into())
    }
    fn spare(&self) -> Result<Option<Option<ObjectId>>, String> {
        Err("ViewPlaced star pool requires owning native host".into())
    }
    fn pop_spare(&mut self) -> Result<(), String> {
        Err("ViewPlaced star pool requires owning native host".into())
    }
}

#[allow(clippy::too_many_arguments)]
fn placed_stored_indicators(
    index: &Index,
    indicators: &mut sims_vm::load_indicators::Indicators,
    execution: &mut Execution,
    program: &mut Program,
    placement: &mut lot_placement::State,
    observers: &mut Vec<lot_placement::Request>,
    id: ObjectId,
    manager_expansions: Option<u16>,
    mut graphics: Option<&mut GraphicHost<'_>>,
) -> Result<(), String> {
    use sims_vm::placed_indicators::{Placed, Reply, Request};
    let mut call = Placed::new((id != 0).then_some(id));
    loop {
        let request = call.advance(&mut PlacedView {
            world: &execution.vm.world,
            indicators,
            manager_expansions,
        })?;
        let Some(request) = request else {
            return Ok(());
        };
        match request {
            Request::Initialize => initialize_stored_indicators(
                index,
                indicators,
                execution,
                program,
                placement,
                observers,
                graphics.take(),
            )?,
            //Frozen import does not yet own the selection physical/pool/graphic
            //dispatcher. Keep its actual request, never acknowledge a no-op.
            other => {
                return Err(format!(
                    "frozen ViewPlaced requires owned service {other:?}"
                ));
            }
        }
        call.acknowledge(request, Reply::Done)?;
    }
}

#[cfg(test)]
mod placed_tests {
    use super::*;
    #[test]
    fn frozen_shared_placed_reads_manager_after_flags_without_aliasing_script_global() {
        let mut vm = sims_vm::Vm::new(Default::default(), 917);
        vm.world.objects.insert(
            7,
            sims_vm::Object {
                native_flags: Some(0x280),
                ..Default::default()
            },
        );
        let indicators = sims_vm::load_indicators::Indicators {
            primary: Some(1),
            secondary: None,
        };
        let run = |vm: &sims_vm::Vm, mask| {
            sims_vm::placed_indicators::Placed::new(Some(7)).advance(&mut PlacedView {
                world: &vm.world,
                indicators: &indicators,
                manager_expansions: mask,
            })
        };
        let before = vm.clone();
        assert_eq!(run(&vm, Some(255)), Ok(None));
        assert_eq!(vm, before);
        //447a32 dereferences the manager even for nonpersons, with either mask.
        assert_eq!(run(&vm, Some(0)), Ok(None));
        vm.world.globals.insert(20, -123);
        assert_eq!(
            run(&vm, None).unwrap_err(),
            "ViewPlaced game manager installation input absent"
        );
        vm.world.objects.get_mut(&7).unwrap().native_flags = None;
        assert_eq!(
            run(&vm, None).unwrap_err(),
            "ViewPlaced native flags absent"
        );
        vm.world.objects.remove(&7);
        assert_eq!(
            run(&vm, None).unwrap_err(),
            "ViewPlaced object absent after initialization"
        );
    }

    #[test]
    fn frozen_shared_placed_initialization_is_first_and_person_requests_are_not_noops() {
        use sims_vm::placed_indicators::{Placed, Request};
        let mut vm = sims_vm::Vm::new(Default::default(), 917);
        let mut indicators = sims_vm::load_indicators::Indicators::default();
        let mut call = Placed::new(Some(7));
        assert_eq!(
            call.advance(&mut PlacedView {
                world: &vm.world,
                indicators: &indicators,
                manager_expansions: None
            }),
            Ok(Some(Request::Initialize))
        );
        vm.world.registry_ui_disabled = true;
        assert_eq!(
            Placed::new(Some(7)).advance(&mut PlacedView {
                world: &vm.world,
                indicators: &indicators,
                manager_expansions: None
            }),
            Ok(None)
        );
        vm.world.registry_ui_disabled = false;
        vm.world.objects.insert(
            7,
            sims_vm::Object {
                native_flags: Some(2),
                person: Some(Default::default()),
                ..Default::default()
            },
        );
        indicators.primary = Some(1);
        let index = Index {
            root: Default::default(),
            resources: vec![],
            archives: 0,
            diagnostics: vec![],
        };
        let mut execution = Execution::from_retained(vm);
        let mut program = Program::default();
        let mut placement = lot_placement::State {
            links: sims_sim::lot_links::Links {
                size: 4,
                heads: vec![vec![0; 16]; 2],
                next: Default::default(),
            },
            bounds: Default::default(),
            flags: vec![vec![0; 16]; 2],
            spatial_dirty: Some(false),
        };
        let before = execution.vm.clone();
        let mut observers = vec![];
        assert_eq!(
            placed_stored_indicators(
                &index,
                &mut indicators,
                &mut execution,
                &mut program,
                &mut placement,
                &mut observers,
                7,
                Some(255),
                None
            )
            .unwrap_err(),
            "frozen ViewPlaced requires owned service Parent(1)"
        );
        assert_eq!(execution.vm, before);
        assert!(observers.is_empty());
        assert_eq!(indicators.primary, Some(1));
    }
}

type GraphicHost<'a> = dyn FnMut(&sims_vm::Vm, &Program, ObjectId) -> Result<()> + 'a;
fn initialize_stored_indicators(
    index: &Index,
    state: &mut sims_vm::load_indicators::Indicators,
    execution: &mut Execution,
    program: &mut Program,
    placement: &mut lot_placement::State,
    observers: &mut Vec<lot_placement::Request>,
    graphics: Option<&mut GraphicHost<'_>>,
) -> Result<(), String> {
    state.initialize(&mut IndicatorHost {
        execution,
        program,
        placement,
        observers,
        index,
        graphics,
    })
}
struct IndicatorHost<'a, 'b, 'c> {
    execution: &'a mut Execution,
    program: &'a mut Program,
    placement: &'a mut lot_placement::State,
    observers: &'a mut Vec<lot_placement::Request>,
    index: &'a Index,
    graphics: Option<&'b mut GraphicHost<'c>>,
}
impl sims_vm::load_indicators::Host for IndicatorHost<'_, '_, '_> {
    fn lookup(&mut self, guid: u32) -> Result<bool, String> {
        let present = self.program.modules.values().any(|m| {
            m.object_definition_words
                .values()
                .any(|w| (u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16)) == guid)
        });
        if !present {
            crate::assets::load_object_guids(self.index, &mut self.program, &[guid])
                .map_err(|e| e.to_string())?;
        }
        Ok(true)
    }
    fn create(&mut self, guid: u32) -> Result<Option<ObjectId>, String> {
        let vm = &mut self.execution.vm;
        let g = &vm.world.globals;
        let now = sims_vm::construction::ObjectDate {
            year: g[&8],
            month: g[&7],
            day: g[&1],
        };
        let time = [g[&0], g[&5]];
        let begin = if self.graphics.is_some() {
            lot_creation::OffLot::begin_resource_with_graphics
        } else {
            lot_creation::OffLot::begin_resource
        };
        let mut factory = begin(
            vm,
            &self.program,
            &mut self.placement.links,
            guid,
            now,
            time,
        )?;
        if let Some(id) = factory.object() {
            self.placement
                .postconstruct_bounds(&vm.world, &self.program, id)?;
        }
        while let Some(request) = factory.advance(vm, &self.program, 100_000)? {
            match request {
                lot_creation::Request::Graphic(id) => {
                    self.graphics
                        .as_mut()
                        .ok_or("ordinary graphic host absent")?(
                        vm, &self.program, id
                    )
                    .map_err(|e| format!("{e:#}"))?;
                    factory.complete_graphic(vm, &self.program, id)?;
                }
                lot_creation::Request::Created(id) => {
                    self.observers.push(lot_placement::Request::Signal {
                        event: 0xe2,
                        value: id,
                    })
                }
                lot_creation::Request::Host {
                    thread,
                    status: Status::AwaitingRemoval(request),
                } => vm.defer_offlot_removal(
                    &self.program,
                    &mut self.execution.removals,
                    thread,
                    &request,
                )?,
                other => return Err(format!("indicator factory host: {other:?}")),
            }
        }
        Ok(factory
            .object()
            .filter(|id| vm.world.objects.contains_key(id)))
    }
    fn hide(&mut self, object: ObjectId) -> Result<(), String> {
        let flags = self
            .execution
            .vm
            .world
            .objects
            .get_mut(&object)
            .and_then(|o| o.data.get_mut(&8))
            .ok_or("indicator flags absent")?;
        *flags |= 0x4000;
        Ok(())
    }
}
