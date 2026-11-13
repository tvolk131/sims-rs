//! Deserialize linked original state into VM storage, before native post-load.
//! Saved ordinary frame storage can be registered for load callbacks. This does
//! not run Init, repair the grid, or resume native animation/routing continuations.
use crate::saves::House;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sims_vm::{Definition, Object, ObjectId, Program, World, loading::PostLoad};
use std::collections::{BTreeMap, BTreeSet};

/// Services inside the native person reader, before script frames or post-load
/// callbacks. The object is not yet published in the saved registry. A host may
/// temporarily register it while servicing its initialized animator; it must
/// return the object and preserve all shared world mutations.
pub trait PersonReader {
    fn process_input(&mut self, _world: &mut World) -> Result<()> {
        Ok(())
    }

    fn initialize(
        &mut self,
        program: &mut Program,
        world: &mut World,
        id: ObjectId,
        object: Object,
    ) -> Result<Object>;
    /// Explicit retained-catalog path. Legacy readers must not silently service
    /// the same native Global through an independent owned provider.
    fn validate_catalog_resources(
        &self,
        _catalog: &crate::object_catalog::Catalog<'_>,
        _provider: &crate::autonomy_resources::CatalogProvider,
    ) -> Result<()> {
        anyhow::bail!("person reader does not support retained catalog resources")
    }
    fn initialize_with_catalog(
        &mut self,
        _program: &mut Program,
        _world: &mut World,
        _id: ObjectId,
        _object: Object,
        _catalog: &mut crate::object_catalog::Catalog<'_>,
        _provider: &mut crate::autonomy_resources::CatalogProvider,
    ) -> Result<Object> {
        anyhow::bail!("person reader does not support retained catalog initialization")
    }
    /// Generic fields and physical position have been read. Saved motives and
    /// person words have NOT: 45cf2d calls the animator reader before those.
    fn read_animator(
        &mut self,
        program: &mut Program,
        world: &mut World,
        id: ObjectId,
        object: Object,
        saved: &sims_formats::saved_objects::Person,
    ) -> Result<Object>;
}

enum ReadProgram<'a> {
    Diagnostic(&'a Program),
    Native(&'a mut Program, &'a mut dyn PersonReader),
}
impl ReadProgram<'_> {
    fn process_input(&mut self, world: &mut World) -> Result<()> {
        if let Self::Native(_, host) = self {
            host.process_input(world)?;
        }
        Ok(())
    }
    fn get(&self) -> &Program {
        match self {
            Self::Diagnostic(program) => program,
            Self::Native(program, _) => program,
        }
    }
    fn initialize(&mut self, world: &mut World, id: ObjectId, object: Object) -> Result<Object> {
        match self {
            Self::Diagnostic(_) => Ok(object),
            Self::Native(program, host) => host.initialize(program, world, id, object),
        }
    }
    fn read_animator(
        &mut self,
        world: &mut World,
        id: ObjectId,
        object: Object,
        saved: &sims_formats::saved_objects::Person,
    ) -> Result<Object> {
        match self {
            Self::Diagnostic(_) => Ok(object),
            Self::Native(program, host) => host.read_animator(program, world, id, object, saved),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Storage {
    pub world: World,
    pub post_load: BTreeMap<ObjectId, PostLoad>,
    /// References survive parsing even when the saved registry does not yet
    /// contain them. Global reconciliation precedes native reference repair.
    pub unresolved: Vec<Reference>,
}
/// Pre-activation VM: the caller must service every post-load callback/reset
/// before advancing ordinary object threads. This is not a playable snapshot.
pub struct Execution {
    pub vm: sims_vm::Vm,
    pub post_load: BTreeMap<ObjectId, PostLoad>,
    pub removals: Vec<sims_vm::removal::Pending>,
    /// Physical tile/slot links, installed after registry reconciliation and
    /// ground-head repair, before the ordered post-load visits.
    pub placement_links: Option<sims_sim::lot_links::Links>,
    /// A recycled ID must not inherit old sentinel data, even when its new
    /// definition happens to match the saved object's definition.
    constructed_during_load: BTreeSet<ObjectId>,
}
#[derive(Debug, Default, Serialize)]
pub struct GlobalReconciliation {
    pub created: Vec<ObjectId>,
    pub animation_tester: Option<ObjectId>,
    pub instructions: usize,
    pub deferred_removals: Vec<sims_vm::removal::Pending>,
}
#[derive(Debug, Serialize)]
pub struct RepairedGrid {
    pub floors: Vec<Vec<u16>>,
    pub repairs: Vec<sims_vm::loading::GridHeadRepair>,
}
impl Execution {
    pub fn from_retained(vm: sims_vm::Vm) -> Self {
        Self {
            vm,
            post_load: BTreeMap::new(),
            removals: vec![],
            placement_links: None,
            constructed_during_load: BTreeSet::new(),
        }
    }

    /// Resolve a requested factory resource lazily during frozen activation.
    /// A missing resource remains an explicit catalog error, rather than an
    /// invented successful object or an unverified native allocation failure.
    pub fn begin_off_lot_creation(
        &mut self,
        index: &crate::Index,
        p: &mut Program,
        links: &mut sims_sim::lot_links::Links,
        thread: sims_vm::ThreadId,
    ) -> Result<sims_sim::lot_creation::OffLot> {
        let sims_vm::Status::AwaitingCreation(request) = &self.vm.threads[&thread].status else {
            anyhow::bail!("loaded thread is not awaiting creation");
        };
        let guid = u32::from_le_bytes(request.operands[..4].try_into().unwrap());
        let loaded = p.modules.values().any(|m| {
            m.object_definition_words.values().any(|w| {
                w.len() > 15 && (u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16)) == guid
            })
        });
        if !loaded {
            crate::assets::load_object_guids(index, p, &[guid])?;
        }
        let g = &self.vm.world.globals;
        let now = sims_vm::construction::ObjectDate {
            year: g[&8],
            month: g[&7],
            day: g[&1],
        };
        let time = [g[&0], g[&5]];
        sims_sim::lot_creation::OffLot::begin(&mut self.vm, p, links, thread, now, time)
            .map_err(anyhow::Error::msg)
    }
    /// Preserve saved sentinel links and the repaired native grid order. New
    /// controllers (including recycled IDs) have an empty physical slot zero.
    /// This is initialization, not a reconstruction to repeat after callbacks.
    pub fn restore_placement_links(&mut self, house: &House, grid: &RepairedGrid) -> Result<()> {
        ensure!(
            self.placement_links.is_none(),
            "physical placement links already restored"
        );
        let links = sims_sim::lot_links::Links {
            size: house.width,
            heads: grid
                .floors
                .iter()
                .map(|f| f.iter().map(|&id| id as i16).collect())
                .collect(),
            next: self
                .vm
                .world
                .objects
                .iter()
                .map(|(&id, object)| {
                    let saved = house.objects.instances.get(&(id as u16)).filter(|s| {
                        !self.constructed_during_load.contains(&id)
                            && object.definition.as_ref().is_some_and(|d| d.guid == s.guid)
                    });
                    (
                        id,
                        saved.and_then(|s| s.slots.first()).map_or(0, |s| s.object),
                    )
                })
                .collect(),
        };
        links.validate_arrays().map_err(anyhow::Error::msg)?;
        links
            .publish_tile_chains(&mut self.vm.world)
            .map_err(anyhow::Error::msg)?;
        self.placement_links = Some(links);
        Ok(())
    }
    /// Placement-triggered 0x433612. Each flag write and synchronous function-9
    /// callback precedes advancing the live physical-slot iterator. Ordinary
    /// Main threads and lot time remain suspended during loading.
    pub fn refresh_loaded_seat_adjacency(
        &mut self,
        program: &Program,
        tile: [i8; 3],
    ) -> Result<Vec<sims_sim::lot_adjacency::Update>> {
        let links = self
            .placement_links
            .as_ref()
            .context("loaded placement links absent")?;
        let Some(mut scan) =
            sims_sim::lot_adjacency::Adjacency::begin(links, &self.vm.world, program, tile)
                .map_err(anyhow::Error::msg)?
        else {
            return Ok(vec![]);
        };
        let mut updates = vec![];
        while let Some(update) = scan
            .next(links, &mut self.vm.world, program)
            .map_err(anyhow::Error::msg)?
        {
            if update.callback {
                links
                    .publish_tile_chains(&mut self.vm.world)
                    .map_err(anyhow::Error::msg)?;
                if let Some(thread) = self
                    .vm
                    .begin_loaded_function(program, update.object, 9)
                    .map_err(anyhow::Error::msg)?
                {
                    self.vm
                        .run_slice(program, thread, 100_000)
                        .map_err(anyhow::Error::msg)?;
                    let state = &self.vm.threads[&thread];
                    ensure!(
                        state.callback.is_none() && state.status == sims_vm::Status::Ready,
                        "loaded adjacency callback for {} needs host service: {:?}",
                        update.object,
                        state.status
                    );
                }
            }
            updates.push(update);
        }
        links
            .publish_tile_chains(&mut self.vm.world)
            .map_err(anyhow::Error::msg)?;
        Ok(updates)
    }
    /// Spatial prefix of one original base post-load visit. Do not call this as
    /// an eager all-object pass in the live loader: Load/Reset/Init must follow
    /// for this object before advancing to the next registry entry.
    pub fn restore_object_position(
        &mut self,
        architecture: &sims_sim::architecture::Architecture,
        id: ObjectId,
    ) -> Result<sims_sim::lot_loading::PlacementUpdate> {
        let mut position = *self
            .vm
            .world
            .positions
            .get(&id)
            .context("loaded object position absent")?;
        if position.level == 0 {
            position.level = 1;
        }
        sims_sim::lot_loading::restore_position(&mut self.vm.world, architecture, id, position)
            .map_err(anyhow::Error::msg)
    }
    ///442659, reached by415eff's post-read F0 broadcast. Publish every
    ///matching object's room before any function-7 callback. Each pass follows
    ///the live registry links; callbacks may insert objects at the head.
    ///Occupancy/presentation effects retain their order for the environment
    ///host, as in the existing frozen person-postload adapter.
    pub fn update_loaded_rooms(
        &mut self,
        program: &mut Program,
        architecture: &sims_sim::architecture::Architecture,
        membership: &mut sims_vm::person_loading::RoomMembership,
        level: i32,
        effects: &mut Vec<sims_vm::person_loading::RoomEffect>,
        host: &mut dyn FnMut(&mut Self, &mut Program, sims_vm::ThreadId) -> Result<(), String>,
    ) -> Result<Vec<ObjectId>> {
        let rooms = architecture.outdoors.keys().copied().collect();
        let tick = self.vm.tick;
        let mut callbacks = vec![];
        for pass in 0..2 {
            let mut current = self
                .vm
                .world
                .person
                .as_ref()
                .context("room registry absent")?
                .object_order
                .first()
                .copied();
            let mut seen = BTreeSet::new();
            while let Some(id) = current {
                ensure!(seen.insert(id), "room pass revisited an object");
                let position = self
                    .vm
                    .world
                    .positions
                    .get_mut(&id)
                    .context("room object position absent")?;
                if pass == 0 && position.level == 0 {
                    position.level = 1;
                }
                if level == 0 || position.level == level {
                    if pass == 0 {
                        let room = architecture
                            .room_at(position.level, sims_sim::Point::new(position.y, position.x));
                        let object = self
                            .vm
                            .world
                            .objects
                            .get_mut(&id)
                            .context("room object absent")?;
                        object.data.insert(29, room);
                        if object.person.is_some() {
                            let selected =
                                object.native_flags.context("room person flags absent")? & 2 != 0;
                            effects.extend(
                                membership
                                    .update(&self.vm.world, id, &rooms, selected)
                                    .map_err(anyhow::Error::msg)?,
                            );
                        }
                    } else {
                        callbacks.push(id);
                        if let Some(thread) = self
                            .vm
                            .begin_loaded_function(program, id, 7)
                            .map_err(anyhow::Error::msg)?
                        {
                            let mut budget = 100_000;
                            while self.vm.threads[&thread].callback.is_some() {
                                ensure!(budget > 0, "room callback diagnostic budget");
                                let slice = self
                                    .vm
                                    .run_slice(program, thread, budget)
                                    .map_err(anyhow::Error::msg)?;
                                budget -= slice.trace.len();
                                if self.vm.threads[&thread].callback.is_some()
                                    && self.vm.threads[&thread].status != sims_vm::Status::Ready
                                {
                                    host(self, program, thread).map_err(anyhow::Error::msg)?;
                                }
                            }
                        }
                    }
                }
                let order = &self.vm.world.person.as_ref().unwrap().object_order;
                let at = order
                    .iter()
                    .position(|&other| other == id)
                    .context("room callback removed its live iterator")?;
                current = order.get(at + 1).copied();
                ensure!(self.vm.tick == tick, "room callback advanced lot time");
            }
        }
        Ok(callbacks)
    }
    /// Run after global reconciliation: recycled IDs may now name a different,
    /// off-lot controller, so existence alone cannot validate a saved grid head.
    pub fn repair_object_grid(&self, house: &House) -> Result<RepairedGrid> {
        ensure!(
            house.width == house.height && !house.floors.is_empty(),
            "unsupported original grid dimensions"
        );
        let mut floors: Vec<_> = house
            .floors
            .iter()
            .map(|f| f.object_heads.clone())
            .collect();
        let active = self
            .vm
            .world
            .map_size
            .map(|map| map.checked_size(house.width))
            .transpose()
            .map_err(anyhow::Error::msg)?
            .unwrap_or(house.width);
        let repairs = sims_vm::loading::repair_ground_object_heads_in_active(
            house.width,
            active,
            &mut floors[0],
            &self.vm.world,
        )
        .map_err(anyhow::Error::msg)?;
        Ok(RepairedGrid { floors, repairs })
    }
    /// The native factory prefix uses the loaded calendar and registry before
    /// family selection. Original files and ordinary saved stacks remain idle.
    /// An error leaves this pre-activation VM incomplete; discard it, rather
    /// than handing it to the live scheduler as a partially loaded household.
    pub fn reconcile_global_controllers(
        &mut self,
        program: &Program,
        catalog: &[crate::catalog::Controller],
    ) -> Result<GlobalReconciliation> {
        use sims_vm::global_load::{self, Instance, Resource, Service};
        ensure!(
            self.placement_links.is_none(),
            "reconcile controllers before restoring physical links"
        );
        ensure!(
            self.vm.world.budget.is_some(),
            "load simulator context before global reconciliation"
        );
        let resources: Vec<_> = catalog
            .iter()
            .map(|c| {
                let words = &program.modules[&c.definition.source].object_definition_words
                    [&c.definition.chunk];
                Resource {
                    definition: Definition {
                        source: c.definition.source.clone(),
                        object: c.definition.chunk,
                        guid: c.guid,
                        slots: words[0x28 / 2] as u16,
                    },
                    kind: words[0x12 / 2],
                    global: words[0x54 / 2],
                }
            })
            .collect();
        let instances: Vec<_> = self
            .vm
            .world
            .person
            .as_ref()
            .context("missing loaded registry")?
            .object_order
            .iter()
            .map(|&id| {
                let def = self.vm.world.objects[&id].definition.as_ref().unwrap();
                Instance {
                    id,
                    definition: def.clone(),
                    global: program.modules[&def.source].object_definition_words[&def.object]
                        [0x54 / 2],
                }
            })
            .collect();
        let read = |i| {
            self.vm
                .world
                .globals
                .get(&i)
                .copied()
                .context("missing factory calendar global")
        };
        let now = sims_vm::construction::ObjectDate {
            year: read(8)?,
            month: read(7)?,
            day: read(1)?,
        };
        let time = [read(0)?, read(5)?];
        let mut report = GlobalReconciliation::default();
        report.animation_tester =
            global_load::reconcile(&resources, &instances, |service| match service {
                Service::Delete(id) => Err(format!(
                    "global duplicate #{id} needs the native deletion host"
                )),
                Service::Create(definition) => {
                    let Some(id) =
                        (1..=i16::MAX).find(|id| !self.vm.world.objects.contains_key(id))
                    else {
                        return Ok(0);
                    };
                    let group = program.modules[&definition.source].object_definition_words
                        [&definition.object][10]
                        != 0;
                    if group {
                        return Err("global controller needs a multipart factory".into());
                    }
                    self.vm.begin_object_factory(
                        program,
                        id,
                        definition.clone(),
                        false,
                        now,
                        time,
                    )?;
                    self.constructed_during_load.insert(id);
                    let thread = self.vm.object_threads[&id];
                    let mut remaining = 100_000;
                    loop {
                        if remaining == 0 {
                            return Err(format!(
                                "global controller {:08x} exceeded Init/Main diagnostic budget",
                                definition.guid
                            ));
                        }
                        let slice = self.vm.run_slice(program, thread, remaining)?;
                        if slice.trace.is_empty()
                            && self.vm.threads[&thread].status == sims_vm::Status::Ready
                        {
                            return Err(format!(
                                "global controller {:08x} requires a pending host callback",
                                definition.guid
                            ));
                        }
                        remaining -= slice.trace.len();
                        report.instructions += slice.trace.len();
                        match self.vm.threads[&thread].status.clone() {
                            sims_vm::Status::Ready => (),
                            sims_vm::Status::Sleeping { .. } | sims_vm::Status::Returned(_) => {
                                break;
                            }
                            sims_vm::Status::AwaitingRemoval(request) => {
                                self.vm.defer_offlot_removal(
                                    program,
                                    &mut self.removals,
                                    thread,
                                    &request,
                                )?;
                            }
                            status => {
                                return Err(format!(
                                    "global controller {} #{} ({:08x}) Init/Main: {status:?}",
                                    definition.source, definition.object, definition.guid
                                ));
                            }
                        }
                    }
                    self.vm.finish_object_factory(program, id)?;
                    report.created.push(id);
                    Ok(id)
                }
            })
            .map_err(anyhow::Error::msg)?;
        report.deferred_removals = self.removals.clone();
        Ok(report)
    }
    /// Calendar and statistics after the SIMI runtime overlay. Native serializes
    /// the current day first, followed by the five history blocks; the clock
    /// stores those in their native in-memory order (history, then current).
    pub fn lot_clock(&self, house: &House) -> Result<sims_sim::clock::LotClock> {
        let g = &self.vm.world.globals;
        let read = |i| g.get(&i).copied().context("missing loaded calendar global");
        ensure!(
            house.simulator.budget_days.len() == 6,
            "missing loaded daily statistics"
        );
        let mut statistics = [[0; 8]; 6];
        statistics[5] = house.simulator.budget_days[0].unwrap_or([0; 8]);
        for (dest, source) in statistics[..5]
            .iter_mut()
            .zip(&house.simulator.budget_days[1..])
        {
            *dest = source.unwrap_or([0; 8]);
        }
        let clock = sims_sim::clock::LotClock {
            hour: read(0)?,
            day: read(1)?,
            period: read(4)?,
            minute: read(5)?,
            second: read(6)?,
            month: read(7)?,
            year: read(8)?,
            elapsed_days: read(28)?,
            month_days: read(29)?,
            ticks: self.vm.tick as u32,
            statistics,
        };
        clock.validate().map_err(anyhow::Error::msg)?;
        Ok(clock)
    }
    /// Select the saved household after raw loading/reconciliation and before
    /// visiting post-load callbacks. No suspended original interaction runs.
    pub fn select_household(
        &mut self,
        house: &House,
        context: sims_vm::loading::HouseContext,
    ) -> Result<()> {
        self.load_simulator_context(house, context)?;
        self.select_loaded_household(house)
    }
    /// SIMI and enclosing house-context setup precede global reconciliation;
    /// family selection is a distinct later step. Controllers see installation
    /// globals and family zero, with the loaded lot budget still intact.
    pub fn load_simulator_context(
        &mut self,
        house: &House,
        context: sims_vm::loading::HouseContext,
    ) -> Result<()> {
        self.vm.world.globals = sims_vm::loading::house_globals(&house.simulator.globals, context)
            .map_err(anyhow::Error::msg)?;
        // Free will is a running-engine input, not a value restored from SIMI.
        // Keep its native byte's truth value with the separately loaded global
        // so the live host does not publish the synthetic world's false default.
        self.vm
            .world
            .person
            .as_mut()
            .context("person registry missing")?
            .free_will = context.simulator.global30 != 0;
        let mut expenses = [0; 9];
        expenses[..8].copy_from_slice(
            &house
                .simulator
                .budget_days
                .first()
                .copied()
                .flatten()
                .unwrap_or([0; 8]),
        );
        self.vm.world.budget = Some(sims_vm::budget::State {
            balance: house
                .simulator
                .funds
                .wrapping_add(house.simulator.pending_funds),
            expenses,
            notifications: vec![],
        });
        self.vm.world.refresh = Some(sims_vm::refresh::RefreshState {
            visible_level: 1,
            draw: self
                .vm
                .world
                .objects
                .keys()
                .map(|&id| {
                    (
                        id,
                        sims_vm::refresh::DrawState::constructed(
                            self.vm.world.positions[&id].level,
                        ),
                    )
                })
                .collect(),
            ..Default::default()
        });
        Ok(())
    }
    pub fn select_loaded_household(&mut self, house: &House) -> Result<()> {
        let family = house
            .household
            .as_ref()
            .context("original lot has no resident household")?;
        // 4282a2 binds application+10 to the selected FAMI before globals and
        // post-load visits. This is distinct from the family-zero owner used
        // by42814b during the earlier inner house read/resize.
        let magic = family.family.magic_history()?;
        self.vm.world.magic_history = Some(sims_vm::magic_history::Context::Retained {
            enabled: magic.is_some(),
            family: i32::from(family.id),
            registry_present: true,
            values: magic.unwrap_or([0; 6]),
        });
        self.vm
            .world
            .select_loaded_family(family.id.try_into()?, house.number, family.family.budget)
            .map_err(anyhow::Error::msg)?;
        Ok(())
    }

    /// Person virtual +0x44 attaches neighborhood data before base post-load.
    /// The caller invokes this in saved registry order, not as a batch merge.
    pub fn attach_resident_neighbor(&mut self, _house: &House, id: ObjectId) -> Result<()> {
        let object = self
            .vm
            .world
            .objects
            .get_mut(&id)
            .context("loaded person missing")?;
        let guid = object
            .definition
            .as_ref()
            .context("loaded person definition missing")?
            .guid;
        let person = object
            .person
            .as_mut()
            .context("neighbor attachment requires a person")?;
        self.vm
            .world
            .person
            .as_mut()
            .context("neighborhood storage missing")?
            .attach_person_neighbor(
                person,
                guid,
                sims_vm::person_factory::NeighborAttachment::Persistent,
            )
            .map(|_| ())
            .map_err(anyhow::Error::msg)
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Reference {
    pub object: ObjectId,
    pub field: String,
    pub target: ObjectId,
}
fn words(values: &[i16]) -> BTreeMap<i16, i16> {
    values
        .iter()
        .enumerate()
        .map(|(i, &v)| (i as i16, v))
        .collect()
}

impl House {
    pub fn prepare_execution(&self, program: &Program) -> Result<Execution> {
        let storage = self.prepare_storage(program)?;
        self.execution_from_storage(program, storage)
    }
    /// Restore native person animators at the original stream boundaries, then
    /// install saved script stacks. Post-load Reset remains the caller's work.
    pub fn prepare_execution_with_person_reader(
        &self,
        program: &mut Program,
        reader: &mut dyn PersonReader,
    ) -> Result<Execution> {
        let storage = self.prepare_storage_with_person_reader(program, reader)?;
        self.execution_from_storage(program, storage)
    }
    fn execution_from_storage(&self, program: &Program, storage: Storage) -> Result<Execution> {
        ensure!(
            storage.unresolved.is_empty(),
            "original references need reconciliation before execution"
        );
        // Restore the raw saved streams before the caller's activation policy.
        // Residential native loading keeps both; the nonresidential timer
        // reseeding branch is a separate, explicitly supplied context service.
        let mut vm = sims_vm::Vm::new(storage.world, u64::from(self.simulator.global_rng));
        vm.lot_rng_state = Some(self.simulator.lot_rng);
        vm.tick = u64::from(self.simulator.ticks);
        self.restore_saved_execution(program, vm, storage.post_load)
    }

    /// ObjM read into the surviving registry owner. SIMI, controller creation,
    /// teardown and TATT are earlier phases; none are replayed here.
    pub fn import_saved_execution(
        &self,
        program: &Program,
        mut execution: Execution,
    ) -> Result<Execution> {
        let vm = &mut execution.vm;
        let storage = self.import_saved_storage_inner(
            ReadProgram::Diagnostic(program),
            std::mem::take(&mut vm.world),
        )?;
        ensure!(
            storage.unresolved.is_empty(),
            "original references need reconciliation before execution"
        );
        vm.world = storage.world;
        let mut imported =
            self.restore_saved_execution(program, execution.vm, storage.post_load)?;
        imported.removals = execution.removals;
        Ok(imported)
    }

    pub fn import_saved_execution_with_person_reader(
        &self,
        program: &mut Program,
        reader: &mut dyn PersonReader,
        mut execution: Execution,
    ) -> Result<Execution> {
        let vm = &mut execution.vm;
        let storage = self.import_saved_storage_inner(
            ReadProgram::Native(program, reader),
            std::mem::take(&mut vm.world),
        )?;
        ensure!(
            storage.unresolved.is_empty(),
            "original references need reconciliation before execution"
        );
        vm.world = storage.world;
        let mut imported =
            self.restore_saved_execution(program, execution.vm, storage.post_load)?;
        imported.removals = execution.removals;
        Ok(imported)
    }

    fn restore_saved_execution(
        &self,
        program: &Program,
        mut vm: sims_vm::Vm,
        post_load: BTreeMap<ObjectId, PostLoad>,
    ) -> Result<Execution> {
        ensure!(
            vm.threads.is_empty() && vm.object_threads.is_empty() && vm.lifecycles.is_empty(),
            "saved import requires completed thread teardown"
        );
        let mut actions = vm.world.actions.take().unwrap_or_default();
        ensure!(
            actions.actors.is_empty(),
            "saved import requires completed action teardown"
        );
        for &id in &self.objects.order {
            let saved = &self.objects.instances[&id];
            if let Some(person) = &saved.person {
                ensure!(
                    person.queue.len() <= sims_vm::action::CAPACITY,
                    "saved person {id} exceeds native action queue capacity"
                );
                let restore = |a: &sims_formats::saved_objects::Interaction| {
                    ensure!(
                        a.caller == id as i16 || (a.target == 0 && a.caller == 0),
                        "saved action has unsupported owner"
                    );
                    ensure!(
                        (a.target == 0 || vm.world.objects.contains_key(&a.target))
                            && (a.icon == 0 || vm.world.objects.contains_key(&a.icon)),
                        "saved action references missing object"
                    );
                    Ok(sims_vm::action::Action {
                        id: a.uid as u32,
                        target: a.target,
                        icon_owner: a.icon,
                        menu_id: a.table_index as u32,
                        priority: a.priority,
                        flags: a.flags,
                        args: a.args,
                        routine: a.action,
                        attenuation_bits: a.attenuation.to_bits(),
                        label: None,
                    })
                };
                let active = restore(&person.active)?;
                let previous = restore(&person.last)?;
                let pending = person.queue.iter().map(restore).collect::<Result<_>>()?;
                // Original 0x41f1c5 raises the signed global action serial for
                // every deserialized record, including the previous action.
                for a in [&person.active, &person.last]
                    .into_iter()
                    .chain(&person.queue)
                {
                    if a.uid > actions.serial as i32 {
                        actions.serial = a.uid as u32;
                    }
                }
                actions.actors.insert(
                    id as i16,
                    sims_vm::action::Actor {
                        empty_action_icon: if active.target == 0 {
                            active.icon_owner
                        } else {
                            0
                        },
                        active: (active.target != 0).then_some(active),
                        previous: (previous.target != 0).then_some(previous),
                        queue: sims_vm::action::Queue { pending },
                    },
                );
            }
            let frames = saved
                .stack
                .iter()
                .map(|f| {
                    let typ = self
                        .types
                        .entries
                        .iter()
                        .find(|t| i32::from(t.id) == i32::from(f.code_owner_type))
                        .context("original frame code owner missing")?;
                    Ok(sims_vm::loading::SavedFrame {
                        owner: self.definitions[&typ.guid].source.clone(),
                        tree: f.tree,
                        node: f.node,
                        stack_object: f.stack_object,
                        args: f.parameters.clone(),
                        locals: f.locals.clone(),
                        primitive_state: f.primitive_state as u32,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let uses = saved
                .person
                .as_ref()
                .map(|person| {
                    person
                        .object_uses
                        .iter()
                        .map(|record| {
                            Ok(sims_vm::loading::SavedUse {
                                target: record.target,
                                depth: record
                                    .stack_depth
                                    .try_into()
                                    .context("negative action-use depth")?,
                                icon: record.unknown != 0,
                            })
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default();
            vm.restore_object_stack_with_uses(
                program,
                id as i16,
                &frames,
                saved.stack_capacity_bytes.try_into()?,
                post_load[&(id as i16)].reset,
                &uses,
            )
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("restore original object {id}"))?;
            vm.world.objects.get_mut(&(id as i16)).unwrap().idle = Some(
                sims_vm::idle::IdleState::from_registry_word(saved.registry_word as u32),
            );
        }
        vm.world.actions = Some(actions);
        Ok(Execution {
            vm,
            post_load,
            removals: vec![],
            placement_links: None,
            constructed_during_load: BTreeSet::new(),
        })
    }
    pub fn prepare_storage(&self, program: &Program) -> Result<Storage> {
        self.prepare_storage_inner(ReadProgram::Diagnostic(program))
    }
    pub fn prepare_storage_with_person_reader(
        &self,
        program: &mut Program,
        reader: &mut dyn PersonReader,
    ) -> Result<Storage> {
        self.prepare_storage_inner(ReadProgram::Native(program, reader))
    }
    fn prepare_storage_inner(&self, program: ReadProgram<'_>) -> Result<Storage> {
        let mut world = self.prepare_neighborhood(program.get())?;
        if let Some(saved) = &self.resource_storage {
            world.resource_storage.load(saved);
        }
        self.import_saved_storage_inner(program, world)
    }

    /// Construct neighborhood/catalog storage before any lot objects are read.
    /// A retained load calls this at neighborhood reload, never again at ObjM.
    pub fn prepare_neighborhood(&self, program: &Program) -> Result<World> {
        let mut world = World {
            scheduler_storage: Some(Default::default()),
            registry_order: Some(sims_vm::presenter::RegistryOrder::constructed()),
            globals: words(&self.simulator.globals),
            neighborhood: words(&self.neighborhood.data),
            ..Default::default()
        };
        if let Some(catalog) = &program.resource_storage {
            world.resource_storage = sims_vm::resource_storage::State::construct(catalog);
        }
        let mut people = sims_vm::person::WorldState {
            neighbor_slots: Some(
                self.neighbors
                    .entries
                    .iter()
                    .map(|n| n.as_ref().map(|n| n.id))
                    .collect(),
            ),
            ..Default::default()
        };
        for n in self.neighbors.entries.iter().flatten() {
            let mut person: BTreeMap<_, _> = (0..256).map(|i| (i, 0)).collect();
            person.extend(words(&n.person_data));
            let mut relationships = BTreeMap::new();
            for r in &n.relationships {
                let id: i16 = r
                    .target
                    .try_into()
                    .context("neighbor relationship key exceeds native ID")?;
                ensure!(
                    relationships.insert(id, r.values.clone()).is_none(),
                    "duplicate neighbor relationship"
                );
            }
            let inventory = self
                .neighborhood
                .inventories
                .iter()
                .flatten()
                .filter(|v| v.neighbor == n.id)
                .flat_map(|v| &v.items)
                .map(|item| {
                    Ok(sims_vm::person::InventoryEntry {
                        kind: item
                            .kind
                            .try_into()
                            .context("inventory kind exceeds native byte")?,
                        guid: item.guid,
                        count: item.count as i16,
                    })
                })
                .collect::<Result<_>>()?;
            ensure!(
                people
                    .neighbors
                    .insert(
                        n.id,
                        sims_vm::person::Neighbor {
                            guid: n.guid,
                            person_revision: n.person_mode,
                            person,
                            relationships,
                            inventory,
                            // 430ad6 uses default ctor423736; the NBRS reader
                            // does not overwrite its +20 cache or +24 dirty byte.
                            friend_count: Some(0),
                            dirty: true,
                        }
                    )
                    .is_none(),
                "duplicate neighbor ID {}",
                n.id
            );
        }
        let families = self
            .families
            .iter()
            .map(|(&id, f)| {
                Ok(sims_vm::neighborhood::Family {
                    id: id
                        .try_into()
                        .context("family ID exceeds native signed range")?,
                    number: f.number,
                    house: f.house.try_into()?,
                    flags: f.flags,
                    friends: f.friends,
                    members: f.members.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let ordered = people
            .prepare_family_reload(&families)
            .map_err(anyhow::Error::msg)?;
        // Neighborhood reload (0x425244) reconstructs this word from FAMI;
        // saved NBRS family values do not survive as authoritative membership.
        people
            .rebuild_family_links(
                ordered
                    .iter()
                    .map(|family| (family.id, family.members.as_slice())),
            )
            .map_err(anyhow::Error::msg)?;
        // 45962b attaches the live neighborhood before saved person words are
        // read. Publish it now so constructors share additions/cache mutations.
        world.person = Some(people);
        Ok(world)
    }

    fn import_saved_storage_inner(
        &self,
        mut program: ReadProgram<'_>,
        mut world: World,
    ) -> Result<Storage> {
        ensure!(
            world.objects.is_empty() && world.positions.is_empty(),
            "ObjM import requires an empty, synchronously cleared registry"
        );
        let people = world
            .person
            .as_ref()
            .context("ObjM import requires retained neighborhood")?;
        ensure!(
            people.object_order.is_empty()
                && people.person_order.is_empty()
                && people.tile_chains.is_empty()
                && people.resource_instances.values().all(Vec::is_empty),
            "ObjM import requires completed registry unlinking"
        );
        program.process_input(&mut world)?;
        let compatibility = self.compatibility(program.get())?;
        //43a516 overlays each listed resource's compatibility bit.43dd76
        //clears its saved type index, but never clears unlisted resources'
        //bits. Keep those process-owned inputs through the same World.
        for resource in &compatibility {
            if resource.changed {
                world.changed_resources.insert(resource.guid);
            } else {
                world.changed_resources.remove(&resource.guid);
            }
        }
        let mut post_load = BTreeMap::new();
        for &raw_id in &self.objects.order {
            let saved = &self.objects.instances[&raw_id];
            let id: ObjectId = raw_id.try_into()?;
            let linked = &self.definitions[&saved.guid];
            let module = &program.get().modules[&linked.source];
            let definition_words = module.object_definition_words[&linked.chunk].clone();
            let definition = Definition {
                source: linked.source.clone(),
                object: linked.chunk,
                guid: saved.guid,
                slots: definition_words[0x28 / 2] as u16,
            };
            // Physical slot zero is the native tile-chain sentinel, not a
            // script containment slot. Saved slots retain the prepared layout.
            let resource_slots = module
                .slots
                .get(&definition.slots)
                .map_or(0, |s| s.entries.iter().filter(|s| s.kind == 0).count());
            let prepared_slots = resource_slots
                + if saved.person.is_some() {
                    sims_vm::person::BUILTIN_SLOT_HEIGHTS.len()
                } else {
                    0
                };
            let slot_count = prepared_slots.max(saved.slots.len().saturating_sub(1));
            let mut object = if saved.person.is_some() {
                Object::construct_person(definition, program.get(), slot_count)
            } else {
                Object::construct(definition, program.get(), slot_count)
            }
            .map_err(anyhow::Error::msg)?;
            if saved.person.is_some() {
                object = program
                    .initialize(&mut world, id, object)
                    .with_context(|| format!("initialize original person {id}"))?;
            }
            //43fe0a sets+114 bit7 immediately after each virtual+3c
            //post-constructor, before the second pass reads saved fields.
            //It is not serialized by the supported ObjM object reader.
            object.activate_native().map_err(anyhow::Error::msg)?;
            ensure!(
                saved.attributes.len() <= object.attributes.len(),
                "saved attributes exceed loaded allocation for object {id}"
            );
            object.attributes[..saved.attributes.len()].copy_from_slice(&saved.attributes);
            object.temps = saved.temps.as_slice().try_into()?;
            object.data = words(&saved.data);
            let dynamic_count =
                usize::try_from(definition_words.get(0x82 / 2).copied().unwrap_or(0))
                    .context("negative dynamic sprite count")?;
            object.dynamic_sprite_flags = vec![0; dynamic_count.max(saved.dynamic_sprites.len())];
            object.dynamic_sprite_flags[..saved.dynamic_sprites.len()]
                .copy_from_slice(&saved.dynamic_sprites);
            for (slot, saved) in object.slots.iter_mut().zip(saved.slots.iter().skip(1)) {
                *slot = (saved.object != 0).then_some(saved.object);
            }
            if saved.data[2] != 0 {
                ensure!(
                    saved.data[3] >= 0,
                    "object {id} has a container with negative script slot"
                );
                object.container = Some((saved.data[2], saved.data[3] as u16));
            }
            for r in saved.relationships.iter().filter(|r| r.present != 0) {
                let target: i16 = r
                    .target
                    .try_into()
                    .context("object relationship key exceeds native ID")?;
                ensure!(
                    object
                        .insert_relationship(target, r.values.clone())
                        .is_none(),
                    "duplicate object relationship"
                );
            }
            // The stream stores +0xf8 then +0xf4. VM placements use native
            // +0xf4/+0xf8 order; simulator Point uses the other convention.
            world.positions.insert(
                id,
                sims_vm::autonomy::Placement {
                    x: saved.position[1],
                    y: saved.position[0],
                    level: saved.position[2],
                },
            );
            if let Some(person) = &saved.person {
                object = program
                    .read_animator(&mut world, id, object, person)
                    .with_context(|| format!("read original animator {id}"))?;
                object
                    .person
                    .as_mut()
                    .context("animator reader removed person storage")?
                    .extend(words(&person.data));
                object.motives = Some(person.motives.map(f32::to_bits));
                object.motive_update.as_mut().unwrap().previous =
                    person.previous_motives.map(f32::to_bits);
                object.motive_changes = person
                    .motive_deltas
                    .iter()
                    .map(|d| {
                        Ok(sims_vm::motives::Change {
                            motive: d.motive.try_into().context("negative motive delta index")?,
                            delta: d.delta.to_bits(),
                            target: d.stop_at.to_bits(),
                        })
                    })
                    .collect::<Result<_>>()?;
                world.person.as_mut().unwrap().person_order.push(id);
            }
            if let Some(part) = &saved.multipart {
                object.multitile = Some(sims_vm::multitile::Part {
                    head: (part.parent != 0).then_some(part.parent),
                    next: None,
                    origin: part.group,
                    offset: part.offset,
                });
            }
            world.person.as_mut().unwrap().object_order.push(id);
            world.invalidate_registry_order();
            //4434cd appends to an existing lazy resource vector, including an
            //empty cache retained across44279a. Unqueried GUIDs stay absent:
            //their first lookup still scans numeric registry IDs.
            if let Some(order) = world
                .person
                .as_mut()
                .unwrap()
                .resource_instances
                .get_mut(&saved.guid)
            {
                order.push(id);
            }
            let changed = compatibility
                .iter()
                .find(|c| c.type_id == saved.type_id)
                .unwrap()
                .changed;
            post_load.insert(id, PostLoad::new(linked.kind, changed, true, false));
            world
                .register_scheduler_object(id, &mut object)
                .map_err(anyhow::Error::msg)?;
            if let Some(refresh) = &mut world.refresh {
                refresh.draw.insert(
                    id,
                    sims_vm::refresh::DrawState::constructed(world.positions[&id].level),
                );
            }
            world.objects.insert(id, object);
        }
        let people = world
            .person
            .take()
            .context("saved reader lost neighborhood")?;
        // 0x423223 -> 0x4228a9 appends each secondary to its saved head in
        // deserialization order, independently of the part's numeric ID.
        for &id in &people.object_order {
            let Some(head) = world.objects[&id].multitile.as_ref().and_then(|p| p.head) else {
                continue;
            };
            let mut cursor = head;
            let mut seen = BTreeSet::new();
            loop {
                ensure!(
                    seen.insert(cursor) && cursor != id,
                    "cyclic saved multipart chain"
                );
                let p = world
                    .objects
                    .get_mut(&cursor)
                    .and_then(|o| o.multitile.as_mut())
                    .context("saved multipart head is missing")?;
                if let Some(next) = p.next {
                    cursor = next;
                } else {
                    p.next = Some(id);
                    break;
                }
            }
        }
        for &id in &people.object_order {
            if world.objects[&id].multitile.is_some() {
                sims_vm::multitile::chain(&world, id).map_err(anyhow::Error::msg)?;
            }
        }
        let mut unresolved = Vec::new();
        for (&id, object) in &world.objects {
            let mut check = |field: String, target: i16| {
                if target != 0 && !world.objects.contains_key(&target) {
                    unresolved.push(Reference {
                        object: id,
                        field,
                        target,
                    });
                }
            };
            for (i, child) in object.slots.iter().enumerate() {
                if let Some(child) = child {
                    check(format!("slot {i}"), *child);
                }
            }
            if let Some((parent, _)) = object.container {
                check("container".into(), parent);
            }
            check(
                "next on tile".into(),
                self.objects.instances[&(id as u16)]
                    .slots
                    .first()
                    .map_or(0, |s| s.object),
            );
        }
        world.person = Some(people);
        Ok(Storage {
            world,
            post_load,
            unresolved,
        })
    }
}
