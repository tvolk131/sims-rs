use anyhow::{Context, Result, ensure};
use sims_content::{
    Index, character_appearance,
    character_loading::{Presentation, Reader},
    save_storage::PersonReader,
    saves::House,
};
use sims_sim::pose::{animator::Settings, properties, refresh::Response, transform};
use sims_vm::{Object, ObjectId, Program, World};
use std::collections::BTreeMap;

/// Explicit diagnostic player input, matched to both the house and full prompt.
/// Unmatched choices remain pending; production dialog behavior is unchanged.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct DialogInput {
    source: String,
    title: String,
    body: String,
    result: u32,
}

/// An explicit counterfactual input, never a production/default neighborhood.
/// Separate output directories prevent these runs from replacing the evidence
/// for the loader whose original coordinate source remains unresolved.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct NeighborhoodExperiment {
    label: String,
    source: String,
    coordinates: Vec<[i32; 2]>,
}

impl NeighborhoodExperiment {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.label.is_empty()
                && self.label.len() <= 64
                && self
                    .label
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'),
            "experiment label must contain 1..64 lowercase letters, digits or hyphens"
        );
        ensure!(
            matches!(
                self.source.as_str(),
                "UserData/Houses/House07.iff"
                    | "UserData/Houses/House05.iff"
                    | "UserData2/Houses/House09.iff"
            ),
            "experiment must name one of the diagnostic original houses"
        );
        ensure!(
            self.coordinates.len() <= 32768,
            "coordinate input too large"
        );
        Ok(())
    }
}

#[derive(Default)]
struct Headless {
    visible: bool,
    notifications: usize,
    invalidations: usize,
    sounds: usize,
}
impl Presentation for Headless {
    fn retirement_references(
        &mut self,
        _: ObjectId,
    ) -> Result<sims_content::character_loading::RetirementReferences> {
        Ok(sims_content::character_loading::RetirementReferences {
            selected: false,
            focus: false,
            base_focus: false,
        })
    }
    fn retire(
        &mut self,
        _: ObjectId,
        _: sims_content::character_loading::RetirementEvent,
    ) -> Result<()> {
        // This renderer has no persistent GPU model, preview or selection owner.
        self.invalidations += 1;
        Ok(())
    }
    fn context(
        &mut self,
        object: ObjectId,
        placement: &transform::Placement,
    ) -> Result<properties::Context> {
        Ok(properties::Context {
            actor: object,
            sound_visible: false,
            selected: false,
            debug_sound: false,
            position_x: placement.base[0],
            appearance_mode: 0,
            snow: false,
            lot_kind: 1,
        })
    }
    fn viewport(&mut self) -> Result<[i32; 4]> {
        Ok([0, 0, 800, 600])
    }
    fn visibility(&mut self, _: ObjectId, _: [i32; 4]) -> Result<bool> {
        Ok(self.visible)
    }
    fn appearance(&mut self, _: ObjectId, request: &character_appearance::Request) -> Result<()> {
        use sims_sim::pose::appearance_selection::Request;
        match request {
            character_appearance::Request::Selection(Request::TextureNotification { .. }) => {
                self.notifications += 1
            }
            character_appearance::Request::Selection(Request::Tint(tint)) => {
                ensure!(*tint == [0; 3], "headless host needs tinted materials")
            }
            other => anyhow::bail!("unsupported headless appearance service {other:?}"),
        }
        Ok(())
    }
    fn saved_texture(&mut self, _: ObjectId) -> Result<()> {
        self.notifications += 1;
        Ok(())
    }
    fn invalidate(&mut self, _: ObjectId) -> Result<()> {
        self.invalidations += 1;
        Ok(())
    }
    fn property(&mut self, _: ObjectId, request: &properties::Request) -> Result<Response> {
        match request {
            properties::Request::Sound { .. } | properties::Request::TraceSound { .. } => {
                self.sounds += 1;
                Ok(Response::Done)
            }
            other => anyhow::bail!("unsupported headless property query {other:?}"),
        }
    }
}

struct Audited<'a> {
    reader: Reader<'a, Headless>,
    house: &'a House,
    initialized: BTreeMap<ObjectId, Object>,
    after_animator: BTreeMap<ObjectId, Object>,
    events: Vec<(ObjectId, &'static str)>,
}
impl PersonReader for Audited<'_> {
    fn initialize(
        &mut self,
        program: &mut Program,
        world: &mut World,
        id: ObjectId,
        object: Object,
    ) -> Result<Object> {
        ensure!(!world.positions.contains_key(&id));
        let before = Object::construct_person(
            object.definition.clone().context("person definition")?,
            program,
            object.slots.len(),
        )
        .map_err(anyhow::Error::msg)?;
        ensure!(
            object == before,
            "generic saved fields arrived before construction"
        );
        let object = self.reader.initialize(program, world, id, object)?;
        self.initialized.insert(id, object.clone());
        self.events.push((id, "initialized"));
        Ok(object)
    }
    fn read_animator(
        &mut self,
        program: &mut Program,
        world: &mut World,
        id: ObjectId,
        object: Object,
        saved: &sims_formats::saved_objects::Person,
    ) -> Result<Object> {
        let generic = &self.house.objects.instances[&(id as u16)];
        ensure!(object.data.values().copied().collect::<Vec<_>>() == generic.data);
        ensure!(
            world.positions[&id].x == generic.position[1]
                && world.positions[&id].y == generic.position[0]
        );
        let initialized = &self.initialized[&id];
        ensure!(
            object.person == initialized.person
                && object.motives == initialized.motives
                && object.motive_update == initialized.motive_update,
            "saved person fields/motives were installed before the animator read"
        );
        let object = self
            .reader
            .read_animator(program, world, id, object, saved)?;
        self.after_animator.insert(id, object.clone());
        self.events.push((id, "animator_read"));
        Ok(object)
    }
}

/// Full production storage loader, with original assets and no intermediate
/// initializer reports. Native component probes remain the differential oracle;
/// this verifies their composition at the actual person-read call sites.
#[test]
#[ignore = "requires private original houses and assets"]
fn original_houses_read_initialized_animators_before_person_fields() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut report = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = House::load(&index, source)?;
        let mut program = house.link_program(&index)?;
        let mut host = Audited {
            reader: Reader::new(
                &index,
                &house,
                Settings {
                    fade_enabled: true,
                    fade_duration: 200,
                    sampling_config: 1,
                },
                [0.; 3],
                0x51a5cafe,
                Headless::default(),
            )?,
            house: &house,
            initialized: BTreeMap::new(),
            after_animator: BTreeMap::new(),
            events: vec![],
        };
        let execution = house.prepare_execution_with_person_reader(&mut program, &mut host)?;
        ensure!(host.reader.is_complete());
        let expected: Vec<_> = house
            .objects
            .order
            .iter()
            .copied()
            .filter(|id| house.objects.instances[id].person.is_some())
            .flat_map(|id| [(id as i16, "initialized"), (id as i16, "animator_read")])
            .collect();
        ensure!(
            host.events == expected,
            "native person read ordering changed"
        );
        ensure!(execution.vm.tick == house.simulator.ticks as u64);
        ensure!(
            execution.vm.rng_state == house.simulator.global_rng
                && execution.vm.lot_rng_state == Some(house.simulator.lot_rng)
        );
        for resident in &house.household.as_ref().context("household")?.residents {
            let id = resident.object as i16;
            let saved = house.objects.instances[&resident.object]
                .person
                .as_ref()
                .context("saved person")?;
            let object = &execution.vm.world.objects[&id];
            let mut expected_fields = host.after_animator[&id].person.clone().unwrap();
            expected_fields.extend(
                saved
                    .data
                    .iter()
                    .enumerate()
                    .map(|(i, &value)| (i as i16, value)),
            );
            ensure!(object.person.as_ref() == Some(&expected_fields));
            ensure!(object.motives == Some(saved.motives.map(f32::to_bits)));
            ensure!(
                object.motive_update.as_ref().unwrap().previous
                    == saved.previous_motives.map(f32::to_bits)
            );
            let loaded = host
                .reader
                .characters
                .get(&id)
                .context("loaded character missing")?;
            ensure!(
                !loaded.resident.character.busy()
                    && loaded.resident.evaluated_tick == execution.vm.tick
            );
            loaded
                .resident
                .character
                .validate(&program)
                .map_err(anyhow::Error::msg)?;
            let state = (
                &loaded.resident,
                &loaded.model,
                &loaded.geometry,
                &host.reader.mesh_pool,
                loaded.avatar.snapshot(),
                &loaded.appearance_owner,
            );
            let restored: (
                sims_sim::character::Resident,
                sims_content::character_model::State,
                sims_content::character_geometry::CharacterGeometry,
                sims_content::character_geometry::MeshPool,
                sims_content::saved_avatar::BindingState,
                sims_sim::pose::appearance_reload::Owner,
            ) = serde_json::from_slice(&serde_json::to_vec(&state)?)?;
            ensure!(
                (
                    &restored.0,
                    &restored.1,
                    &restored.2,
                    &restored.3,
                    restored.4.clone(),
                    &restored.5
                ) == state
            );
            let avatar = sims_content::saved_avatar::BoundAvatar::restore(
                &index,
                &loaded.avatar.avatar.skeleton.name,
                id as u64,
                &restored.4,
            )?;
            ensure!(avatar.snapshot() == loaded.avatar.snapshot());
            report.push(
                serde_json::json!({"source":source,"name":resident.name,"object":id,
                "resident":loaded.resident,"model":loaded.model,"geometry":loaded.geometry,
                "mesh_pool":host.reader.mesh_pool,"asset_state":loaded.avatar.snapshot(),
                "owner":loaded.appearance_owner,"read_order":host.events}),
            );
        }
        ensure!(host.reader.characters.len() == host.initialized.len());
        eprintln!(
            "{source}: {} native person reads, {} invalidations, {} texture notifications",
            host.reader.characters.len(),
            host.reader.presentation.invalidations,
            host.reader.presentation.notifications
        );
    }
    ensure!(report.len() == 7);
    std::fs::write(
        root.join("cache/original-house-character-reads.json"),
        serde_json::to_vec(&report)?,
    )?;
    Ok(())
}

#[test]
#[ignore = "requires private original houses and assets"]
fn original_houses_complete_native_animator_post_load() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut report = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = House::load(&index, source)?;
        let mut reader = Reader::new(
            &index,
            &house,
            Settings {
                fade_enabled: true,
                fade_duration: 200,
                sampling_config: 1,
            },
            [0.; 3],
            0x51a5cafe,
            Headless::default(),
        )?;
        let frozen = house.load_frozen_with_native_characters(
            &index,
            sims_vm::loading::HouseContext {
                simulator: sims_vm::loading::SimulatorContext {
                    game_mode: 0,
                    retained_global25: 0,
                    global30: 1,
                },
                installed_expansions: 0xff,
                performance_class: 2,
            },
            &mut reader,
        )?;
        ensure!(
            frozen.animator_effects.is_empty(),
            "animator work left outstanding"
        );
        ensure!(reader.is_complete());
        ensure!(frozen.execution.vm.tick == house.simulator.ticks as u64);
        let vm = sims_vm::Snapshot::capture(&frozen.program, &frozen.execution.vm)
            .restore(&frozen.program)
            .map_err(anyhow::Error::msg)?;
        ensure!(vm == frozen.execution.vm);
        for resident in &house.household.as_ref().context("household")?.residents {
            let id = resident.object as i16;
            let loaded = reader
                .characters
                .get(&id)
                .context("post-load character missing")?;
            loaded
                .resident
                .character
                .validate(&frozen.program)
                .map_err(anyhow::Error::msg)?;
            ensure!(!loaded.resident.character.busy());
            ensure!(loaded.resident.controls.engaged == 0);
            ensure!(loaded.resident.controls.movement_anchors == [None; 2]);
            ensure!(loaded.resident.evaluated_tick == frozen.execution.vm.tick);
            ensure!(loaded.resident.source.as_ref() == Some(&loaded.avatar.avatar.skeleton));
            report.push(
                serde_json::json!({"source":source,"name":resident.name,"object":id,
                "skeleton":loaded.avatar.avatar.skeleton.name,"resident":loaded.resident,
                "model":loaded.model,"geometry":loaded.geometry,"mesh_pool":reader.mesh_pool,
                "asset_state":loaded.avatar.snapshot(),"owner":loaded.appearance_owner,
                "person":vm.world.objects[&id].person}),
            );
        }
        eprintln!(
            "{source}: {} populated animators completed ordered post-load at tick {}",
            reader.characters.len(),
            frozen.execution.vm.tick
        );
    }
    ensure!(report.len() == 7);
    std::fs::write(
        root.join("cache/original-house-character-post-load.json"),
        serde_json::to_vec(&report)?,
    )?;
    Ok(())
}

/// Diagnostic of the actual loader/character/scheduler handoff. Unsupported
/// world services stop the run; a recorded boundary is not playable activation.
#[test]
#[ignore = "requires private original houses and assets"]
fn original_houses_service_live_character_geometry() -> Result<()> {
    run_live_houses(sims_sim::lot_route::path::Policy::Strict, None)
}

#[test]
#[ignore = "requires private original homes; protected rectangle routing is explicitly inferred"]
fn original_houses_service_live_characters_with_documented_inferences() -> Result<()> {
    run_live_houses(sims_sim::lot_route::path::Policy::InferredRectangles, None)
}

#[test]
#[ignore = "requires private assets and an explicit counterfactual neighborhood input; not original-house progress"]
fn original_house_explores_supplied_neighborhood_coordinates() -> Result<()> {
    let path = std::env::var("SIMS_NEIGHBORHOOD_EXPERIMENT")
        .context("set SIMS_NEIGHBORHOOD_EXPERIMENT to an explicit JSON input file")?;
    let input: NeighborhoodExperiment = serde_json::from_slice(&std::fs::read(path)?)?;
    input.validate()?;
    run_live_houses(
        sims_sim::lot_route::path::Policy::InferredRectangles,
        Some(&input),
    )
}

fn run_live_houses(
    policy: sims_sim::lot_route::path::Policy,
    experiment: Option<&NeighborhoodExperiment>,
) -> Result<()> {
    use sims_sim::{Simulation, Snapshot, StepStop, lot_runtime};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = if let Some(input) = experiment {
        input.validate()?;
        eprintln!("COUNTERFACTUAL NEIGHBORHOOD INPUT: {input:?}; coordinate source is unverified");
        root.join("cache/neighborhood-input-experiments")
            .join(&input.label)
    } else {
        root.join("cache")
    };
    std::fs::create_dir_all(&output)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut report = vec![];
    let mut frames = vec![];
    let filter = std::env::var("SIMS_ORIGINAL_HOUSE_FILTER").ok();
    let dialog_inputs: Vec<DialogInput> = std::env::var("SIMS_ORIGINAL_DIALOG_INPUTS")
        .ok()
        .map(|path| -> Result<_> { Ok(serde_json::from_slice(&std::fs::read(path)?)?) })
        .transpose()?
        .unwrap_or_default();
    ensure!(
        dialog_inputs
            .iter()
            .all(|input| (2..=4).contains(&input.result)),
        "invalid scripted dialog result"
    );
    let tick_limit = std::env::var("SIMS_ORIGINAL_HOUSE_TICKS")
        .ok()
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(100);
    ensure!(
        (1..=1_000_000).contains(&tick_limit),
        "invalid live diagnostic tick limit"
    );
    let walk_budget = std::env::var("SIMS_ORIGINAL_WALK_BUDGET")
        .ok()
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(100_000);
    ensure!(walk_budget > 0, "invalid native walk instruction budget");
    let reach_budget = std::env::var("SIMS_ORIGINAL_REACH_BUDGET")
        .ok()
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(1);
    ensure!(reach_budget > 0, "invalid native Reach instruction budget");
    let one_instruction_after = std::env::var("SIMS_ORIGINAL_ONE_INSTRUCTION_AFTER")
        .ok()
        .map(|s| s.parse::<u64>())
        .transpose()?;
    // Focused movement runs can reuse the already-tested factory sequence.
    // The default still restores every factory service boundary.
    let factory_restore_limit = std::env::var("SIMS_ORIGINAL_FACTORY_RESTORES")
        .ok()
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(usize::MAX);
    let walk_restore_limit = std::env::var("SIMS_ORIGINAL_WALK_RESTORES")
        .ok()
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(8);
    let accessory_restore_limit = std::env::var("SIMS_ORIGINAL_ACCESSORY_RESTORES")
        .ok()
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(16);
    let retirement_restore_limit = std::env::var("SIMS_ORIGINAL_RETIREMENT_RESTORES")
        .ok()
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(64);
    let notification_restore_limit = std::env::var("SIMS_ORIGINAL_NOTIFICATION_RESTORES")
        .ok()
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(32);
    let stop_restore_limit = std::env::var("SIMS_ORIGINAL_STOP_RESTORES")
        .ok()
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(8);
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ]
    .into_iter()
    .filter(|source| filter.as_ref().is_none_or(|f| source.contains(f)))
    .filter(|source| experiment.is_none_or(|input| input.source == *source))
    {
        let setup_started = std::time::Instant::now();
        let house = House::load(&index, source)?;
        let mut reader = Reader::new(
            &index,
            &house,
            Settings {
                fade_enabled: true,
                fade_duration: 200,
                sampling_config: 1,
            },
            [0.; 3],
            0x51a5cafe,
            Headless {
                visible: true,
                ..Default::default()
            },
        )?;
        let mut loaded = house.load_frozen_with_native_characters(
            &index,
            sims_vm::loading::HouseContext {
                simulator: sims_vm::loading::SimulatorContext {
                    game_mode: 0,
                    retained_global25: 0,
                    global30: 1,
                },
                installed_expansions: 0xff,
                performance_class: 2,
            },
            &mut reader,
        )?;
        loaded.initialize_indicators(&index)?;
        // Explicit shipped zoning input for extracted, uninstalled neighborhoods.
        let zoning = sims_content::zoning::Zoning::load(
            &index,
            "GameData_ALL_Recursive/MasterLotZoning.iff",
            0xff,
        )?;
        loaded
            .execution
            .vm
            .world
            .person
            .as_mut()
            .unwrap()
            .lot_zoning = zoning.query_table();
        if let Some(input) = experiment {
            let order = sims_vm::nearby_houses::order(&input.coordinates, house.number.into())
                .map_err(anyhow::Error::msg)?;
            loaded
                .execution
                .vm
                .world
                .person
                .as_mut()
                .unwrap()
                .nearby_houses = Some(order);
        }
        let clock = loaded.execution.lot_clock(&house)?;
        let mut simulation = Simulation::from_loaded_lot(
            loaded.execution.vm,
            loaded.architecture,
            lot_runtime::State {
                placement: loaded.placement,
                membership: loaded.membership,
                indicators: loaded.indicators,
                observers: loaded.observers,
                diagnostics: sims_sim::lot_acceptance::person::Diagnostics {
                    error: 0,
                    blocker: None,
                },
                route: None,
                shoo: None,
                movement: None,
                snap: None,
                find_location: None,
                removal: None,
                snap_offsets: Default::default(),
                completion: None,
                route_paths: Default::default(),
                selected_person: None,
                room_effects: loaded
                    .resident_tails
                    .iter()
                    .flat_map(|tail| tail.room_effects.clone())
                    .collect(),
                routing_policy: policy,
            },
        )
        .map_err(anyhow::Error::msg)?;
        simulation.removals = loaded.execution.removals;
        let mut program = loaded.program;
        simulation
            .enable_household(&program, clock)
            .map_err(anyhow::Error::msg)?;
        simulation
            .enable_native_residential_autonomy(false)
            .map_err(anyhow::Error::msg)?;
        let mut host = reader.into_live(
            &mut simulation,
            sims_sim::character_update::Settings {
                terrain_scale: None,
                movement: sims_sim::pose::movement::Settings {
                    short_distance: 3.1,
                    turn_rate: 1.,
                    small_turn: 0.5,
                    medium_turn: 1.5,
                    large_turn: 3.,
                    minimum_weight: 0.5,
                    snap_distance: 0.5,
                    rate: sims_sim::pose::ordinary::RateSettings {
                        base: 1.2,
                        positive: 0.2,
                        negative: 0.4,
                        change: 0.2,
                    },
                },
                root_motion: 0,
                person: sims_sim::person_update::Context {
                    only_visible: true,
                    display_animation_names: false,
                    debug_recording: false,
                },
                floor_vector: [0.; 3],
            },
        )?;
        simulation
            .enable_registry_scheduler(i32::from(house.simulator.scheduler_cursor))
            .map_err(anyhow::Error::msg)?;
        // Explicit headless Live/Play input; preserve imported values until now.
        simulation.vm.world.globals.insert(17, 0);
        simulation.vm.world.globals.insert(19, 0);
        let start = simulation.vm.tick;
        host.notifications = Some(sims_content::notifications::Software::system_font(
            sims_sim::person_notification::currency::Locale {
                language: 1,
                real_currency: false,
            },
        )?);
        // Prepare actual inputs for the native room manager without pretending
        // that geometry-only totals fulfill a live Environment request.
        let room_tuning = sims_sim::lot_environment::Tuning::from_program(&program)
            .map_err(anyhow::Error::msg)?;
        let lot = simulation
            .architecture
            .as_ref()
            .unwrap()
            .original
            .as_ref()
            .unwrap();
        let mut room_geometry = std::collections::BTreeMap::new();
        for floor in &lot.floors {
            for (&room, metrics) in &floor.metrics {
                let totals = sims_sim::lot_environment::Totals::geometry(lot, room)
                    .map_err(anyhow::Error::msg)?;
                ensure!(
                    totals.half_tiles == metrics.half_tiles,
                    "room geometry area changed"
                );
                room_geometry.insert(room, totals);
            }
        }
        let mut room_objects = std::collections::BTreeMap::new();
        for &id in simulation.vm.world.objects.keys() {
            room_objects.insert(
                id,
                sims_sim::lot_environment::objects::Input::read(&simulation.vm.world, &program, id)
                    .map_err(anyhow::Error::msg)?,
            );
        }
        simulation
            .enable_original_environment(
                &program,
                sims_sim::lot_environment::manager::Settings {
                    daylight_multiplier: 1.,
                    light_range: 12.,
                    rooms_enabled: true,
                    // Headless renderer owns six logical model records. No GPU model
                    // allocation is required for their retained references/positions.
                    models: std::array::from_fn(|i| Some(i as u8)),
                },
            )
            .map_err(anyhow::Error::msg)?;
        let mut completed_services = 0;
        let mut factory_restores = 0;
        let mut rendered_factories = std::collections::BTreeSet::new();
        let mut rendered_walkers = std::collections::BTreeSet::new();
        let mut rendered_stops = std::collections::BTreeSet::new();
        let mut walking_positions = BTreeMap::new();
        let mut restored = false;
        let mut walk_restores = 0;
        let mut physical_restores = 0;
        let mut accessory_restores = 0;
        let mut deletion_reload_restores = 0;
        let mut deletion_reload_objects = std::collections::BTreeSet::new();
        let mut completed_deletion_reloads = std::collections::BTreeSet::new();
        let mut retirement_restores = 0;
        let mut notification_restores = 0;
        let mut notification_frames = Vec::new();
        let mut dialog_events = Vec::new();
        let mut pending_dialog_inputs: std::collections::VecDeque<_> = dialog_inputs
            .iter()
            .filter(|input| input.source == source)
            .cloned()
            .collect();
        let mut captured_notifications = std::collections::BTreeSet::new();
        let mut completed_retirements = std::collections::BTreeSet::new();
        let mut stop_restores = 0;
        let mut restored_stop_ticks = std::collections::BTreeSet::new();
        let mut stopping_routes = std::collections::BTreeSet::new();
        let mut rendered_accessories = std::collections::BTreeSet::new();
        let mut restored_censors = std::collections::BTreeSet::new();
        let mut rendered_censors = std::collections::BTreeSet::new();
        let mut walk_steps = 0;
        let mut reach_checkpoints = std::collections::BTreeSet::new();
        let mut burn_restores = 0;
        let mut reset_restores = 0;
        let mut completed_live_resets = std::collections::BTreeSet::new();
        let mut reach_frames = std::collections::BTreeSet::new();
        let mut steps = 0;
        let setup_seconds = setup_started.elapsed().as_secs_f64();
        let replay_started = std::time::Instant::now();
        let mut service_slices = 0u64;
        let boundary = loop {
            service_slices += 1;
            steps += 1;
            if steps == 1000 {
                std::fs::write(
                    output.join("original-live-character-service-limit.json"),
                    serde_json::to_vec(
                        &serde_json::json!({"source":source,"tick":simulation.vm.tick,
                        "start":start,"steps":steps,"simulation":Snapshot::capture(&program,&simulation),"assets":host.snapshot()}),
                    )?,
                )?;
                eprintln!(
                    "{source}: +{}, reached 1000 diagnostic slices in one tick; saved service-limit checkpoint",
                    simulation.vm.tick - start
                );
            }
            // Reach saves deliberately reduce the budget to one instruction,
            // including other objects in that lot pass. Retain a bounded stop
            // and its state, instead of losing the evidence on assertion failure.
            if steps >= 10_000 {
                break "10,000 diagnostic service slices in one tick".into();
            }
            if let Err(error) = host.update_contexts(&mut simulation) {
                break format!("asset context: {error:#}");
            }
            let walking = simulation
                .characters
                .values()
                .any(|r| r.native_walk.is_some());
            let reset_before = simulation
                .native_burn
                .as_ref()
                .and_then(|e| e.resetting_person());
            let budget = if one_instruction_after
                .is_some_and(|tick| simulation.vm.tick - start >= tick)
                || simulation.native_burn.is_some() && burn_restores < 64
                || reset_before.is_some() && reset_restores < 256
            {
                1
            } else if !simulation.native_reaches.is_empty() {
                reach_budget
            } else if walking {
                walk_budget
            } else {
                100_000
            };
            let before = (budget == 1).then(|| simulation.clone());
            let result = simulation.run_current_tick(&program, budget);
            let progressed = before.as_ref().is_some_and(|before| before != &simulation);
            let resetting = simulation
                .native_burn
                .as_ref()
                .and_then(|e| e.resetting_person());
            if simulation.native_burn.is_some() && burn_restores < 64
                || resetting.is_some() && reset_restores < 256
            {
                let saved = Snapshot::capture(&program, &simulation);
                let resumed = serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&saved)?)?
                    .restore(&program);
                if let Err(error) = &resumed {
                    std::fs::write(
                        output.join("original-live-character-restore-error.json"),
                        serde_json::to_vec(
                            &serde_json::json!({"error":error,"simulation":saved,"assets":host.snapshot()}),
                        )?,
                    )?;
                }
                let resumed = resumed.map_err(anyhow::Error::msg)?;
                if resetting.is_some() && reset_restores == 0 {
                    let mut broken = serde_json::to_value(&saved)?;
                    broken["state"]["native_burn"]["reset"] = serde_json::Value::Null;
                    ensure!(
                        serde_json::from_value::<Snapshot>(broken)?
                            .restore(&program)
                            .is_err(),
                        "Burn accepted a lost live Reset stack"
                    );
                }
                ensure!(
                    resumed == simulation,
                    "Burn continuation restore changed simulation"
                );
                let assets = host.snapshot();
                for (id, started) in simulation.pending_reset_character_services() {
                    if started {
                        let mut broken = assets.clone();
                        broken.reloads.remove(&id);
                        let mut broken_host = sims_content::character_loading::live::Host::restore(
                            &index,
                            Headless {
                                visible: true,
                                ..Default::default()
                            },
                            broken,
                        )?;
                        ensure!(
                            broken_host
                                .update_contexts(&mut simulation.clone())
                                .is_err(),
                            "Reset accepted a lost appearance asset job"
                        );
                    }
                }
                let mut resumed_host = sims_content::character_loading::live::Host::restore(
                    &index,
                    Headless {
                        visible: true,
                        ..Default::default()
                    },
                    serde_json::from_slice(&serde_json::to_vec(&assets)?)?,
                )?;
                ensure!(
                    resumed_host.snapshot() == assets,
                    "Burn continuation restore changed assets"
                );
                simulation = resumed;
                resumed_host.update_contexts(&mut simulation)?;
                host = resumed_host;
                burn_restores += 1;
                reset_restores += usize::from(resetting.is_some());
                eprintln!(
                    "{source}: +{}, restored Burn service boundary {burn_restores}",
                    simulation.vm.tick - start
                );
            }
            if let Some(id) = reset_before.filter(|_| resetting.is_none()) {
                if completed_live_resets.insert(id) {
                    let resident = &simulation.characters[&id];
                    let assets = &host.characters[&id];
                    ensure!(
                        !resident.character.busy(),
                        "completed Reset left its character busy"
                    );
                    frames.push(serde_json::json!({"source":source,"object":id,
                        "phase":"completed live person Reset", "tick":simulation.vm.tick,
                        "skeleton":assets.avatar.avatar.skeleton.name,"resident":resident,
                        "model":assets.model,"geometry":assets.geometry,"mesh_pool":host.mesh_pool,
                        "asset_state":assets.avatar.snapshot(),"owner":assets.appearance_owner}));
                }
            }
            if !simulation.native_reaches.is_empty() && reach_checkpoints.len() < 16 {
                let mut keys = Vec::new();
                for (&request, execution) in &simulation.native_reaches {
                    let value = serde_json::to_value(execution)?;
                    let thread = value["thread"].as_u64().context("Reach thread absent")?;
                    let phase = simulation.vm.threads[&thread]
                        .stack
                        .iter()
                        .find(|f| {
                            serde_json::to_value(&f.location).ok()
                                == Some(value["request"]["origin"].clone())
                        })
                        .map(|f| f.continuation);
                    let key = format!("{request}:{phase:?}:{}", value["stage"]);
                    if !reach_checkpoints.contains(&key) {
                        keys.push(key);
                    }
                }
                if !keys.is_empty() {
                    let saved = Snapshot::capture(&program, &simulation);
                    let resumed = serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&saved)?)?
                        .restore(&program)
                        .map_err(anyhow::Error::msg)?;
                    ensure!(
                        resumed == simulation,
                        "Reach continuation restore changed simulation"
                    );
                    let assets = host.snapshot();
                    let mut resumed_host = sims_content::character_loading::live::Host::restore(
                        &index,
                        Headless {
                            visible: true,
                            ..Default::default()
                        },
                        serde_json::from_slice(&serde_json::to_vec(&assets)?)?,
                    )?;
                    ensure!(
                        resumed_host.snapshot() == assets,
                        "Reach continuation restore changed assets"
                    );
                    simulation = resumed;
                    resumed_host.update_contexts(&mut simulation)?;
                    host = resumed_host;
                    eprintln!(
                        "{source}: +{}, restored Reach checkpoints {keys:?}",
                        simulation.vm.tick - start
                    );
                    reach_checkpoints.extend(keys);
                }
                for execution in simulation.native_reaches.values() {
                    let value = serde_json::to_value(execution)?;
                    let id = value["request"]["caller"]
                        .as_i64()
                        .context("Reach actor absent")? as i16;
                    let stage = value["stage"].to_string();
                    if reach_frames.len() < 4 && reach_frames.insert((id, stage.clone())) {
                        let assets = &host.characters[&id];
                        frames.push(serde_json::json!({"source":source,"object":id,"phase":format!("native Reach {stage}"),"tick":simulation.vm.tick,
                            "skeleton":assets.avatar.avatar.skeleton.name,"resident":simulation.characters[&id],
                            "model":assets.model,"geometry":assets.geometry,"mesh_pool":host.mesh_pool,
                            "asset_state":assets.avatar.snapshot(),"owner":assets.appearance_owner}));
                    }
                }
            }
            let walking_after = simulation
                .characters
                .values()
                .any(|r| r.native_walk.is_some());
            let stopping = simulation
                .characters
                .iter()
                .filter_map(|(&id, r)| {
                    r.native_walk
                        .as_ref()
                        .filter(|w| w.stopping.is_some())
                        .map(|w| (id, w.request))
                })
                .collect::<Vec<_>>();
            for &(id, request) in &stopping {
                if stopping_routes.insert((id, request)) {
                    let walk = simulation.characters[&id].native_walk.as_ref().unwrap();
                    eprintln!(
                        "{source}: +{}, native stop selected {:?}, actor {id}, request {request}",
                        simulation.vm.tick - start,
                        walk.stopping
                    );
                }
            }
            if !stopping.is_empty()
                && stop_restores < stop_restore_limit
                && restored_stop_ticks.insert(simulation.vm.tick)
            {
                let saved = Snapshot::capture(&program, &simulation);
                let resumed = serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&saved)?)?
                    .restore(&program)
                    .map_err(anyhow::Error::msg)?;
                ensure!(
                    resumed == simulation,
                    "native stop restore changed simulation"
                );
                let assets = host.snapshot();
                let resumed_host = sims_content::character_loading::live::Host::restore(
                    &index,
                    Headless {
                        visible: true,
                        ..Default::default()
                    },
                    serde_json::from_slice(&serde_json::to_vec(&assets)?)?,
                )?;
                ensure!(
                    resumed_host.snapshot() == assets,
                    "native stop restore changed assets"
                );
                simulation = resumed;
                host = resumed_host;
                stop_restores += 1;
                eprintln!(
                    "{source}: +{}, restored native stop checkpoint {stop_restores}",
                    simulation.vm.tick - start
                );
            }
            if walking_after {
                if walk_steps == 0 {
                    eprintln!(
                        "{source}: native animator route started at +{}",
                        simulation.vm.tick - start
                    );
                }
                walk_steps += 1;
            }
            for (&id, resident) in &simulation.characters {
                if resident.native_walk.is_none() {
                    continue;
                }
                if !rendered_stops.contains(&id)
                    && resident
                        .native_walk
                        .as_ref()
                        .is_some_and(|w| w.stopping.is_some())
                    && resident.ordinary_update.is_none()
                    && !resident.character.busy()
                    && resident.evaluated_tick == simulation.vm.tick
                    && let Some(assets) = host.characters.get(&id)
                {
                    frames.push(serde_json::json!({"source":source,"object":id,"phase":"native stopping","tick":simulation.vm.tick,
                        "skeleton":assets.avatar.avatar.skeleton.name,"resident":resident,"model":assets.model,
                        "geometry":assets.geometry,"mesh_pool":host.mesh_pool,"asset_state":assets.avatar.snapshot(),"owner":assets.appearance_owner}));
                    rendered_stops.insert(id);
                }
                let position = simulation.vm.world.positions[&id];
                walking_positions
                    .entry(id)
                    .or_insert((position, position))
                    .1 = position;
                if !rendered_walkers.contains(&id)
                    && resident.ordinary_update.is_none()
                    && !resident.character.busy()
                    && matches!(resident.controls.ordinary.mode, 9 | 10)
                    && let Some(assets) = host.characters.get(&id)
                {
                    frames.push(serde_json::json!({"source": source, "object": id,
                        "phase": "native walking", "tick": simulation.vm.tick,
                        "skeleton": assets.avatar.avatar.skeleton.name,
                        "resident": resident, "model": assets.model, "geometry": assets.geometry,
                        "mesh_pool": host.mesh_pool, "asset_state": assets.avatar.snapshot(),
                        "owner": assets.appearance_owner}));
                    rendered_walkers.insert(id);
                }
            }
            let physical_pending = simulation.characters.values().any(|r| {
                r.physical_movement.is_some()
                    || matches!(
                        r.controls.movement.pending_request(),
                        Some(
                            sims_sim::pose::movement::Request::AcceptStep { .. }
                                | sims_sim::pose::movement::Request::MoveStep { .. }
                        )
                    )
                    || r.native_walk.as_ref().is_some_and(|w| w.physical.is_some())
            });
            // Full restore fingerprints every loaded BHAV. Bound repeated
            // ordinary checkpoints while retaining separate physical callbacks.
            if walking_after
                && (walk_restores < walk_restore_limit
                    || (physical_pending && physical_restores < walk_restore_limit))
            {
                let checkpoint = Snapshot::capture(&program, &simulation);
                let resumed =
                    serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&checkpoint)?)?
                        .restore(&program)
                        .map_err(anyhow::Error::msg)?;
                ensure!(resumed == simulation, "native walk restore changed state");
                physical_restores += usize::from(physical_pending);
                simulation = resumed;
                walk_restores += 1;
                eprintln!(
                    "{source}: +{}, restored native walk checkpoint {walk_restores}, physical {physical_restores}",
                    simulation.vm.tick - start
                );
            }
            for (&id, factory) in &simulation.person_factories {
                if factory.finalized
                    && !rendered_factories.contains(&id)
                    && let (Some(resident), Some(assets)) =
                        (simulation.characters.get(&id), host.characters.get(&id))
                    && resident.ordinary_update.is_none()
                    && !resident.character.busy()
                {
                    frames.push(serde_json::json!({"source": source, "object": id,
                        "phase": "completed NPC factory", "skeleton": assets.avatar.avatar.skeleton.name,
                        "resident": resident, "model": assets.model, "geometry": assets.geometry,
                        "mesh_pool": host.mesh_pool, "asset_state": assets.avatar.snapshot(),
                        "owner": assets.appearance_owner}));
                    rendered_factories.insert(id);
                }
            }
            let pending = simulation.pending_character_services();
            // Save before real bind/unbind services and after a completed
            // censored pose. Never synthesize a binding to advance the run.
            let censor_pending: Vec<_> = simulation
                .characters
                .iter()
                .filter_map(|(&id, r)| match r.character.pending_request() {
                    Some(sims_sim::pose::refresh::Request::BindCensor { mask }) => {
                        Some((id, *mask, true))
                    }
                    Some(sims_sim::pose::refresh::Request::UnbindCensor { .. }) => {
                        Some((id, r.character.censor.mask, false))
                    }
                    _ => None,
                })
                .collect();
            let complete_censors: Vec<_> = simulation
                .characters
                .iter()
                .filter_map(|(&id, r)| {
                    (r.character.censor.binding.is_some()
                        && r.ordinary_update.is_none()
                        && !r.character.busy()
                        && r.evaluated_tick == simulation.vm.tick
                        && !rendered_censors.contains(&id))
                    .then_some(id)
                })
                .collect();
            if censor_pending
                .iter()
                .any(|&key| !restored_censors.contains(&key))
                || !complete_censors.is_empty()
            {
                let saved = Snapshot::capture(&program, &simulation);
                let mut resumed = serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&saved)?)?
                    .restore(&program)
                    .map_err(anyhow::Error::msg)?;
                let assets = host.snapshot();
                let mut resumed_host = sims_content::character_loading::live::Host::restore(
                    &index,
                    Headless {
                        visible: true,
                        ..Default::default()
                    },
                    serde_json::from_slice(&serde_json::to_vec(&assets)?)?,
                )?;
                resumed_host.update_contexts(&mut resumed)?;
                ensure!(
                    resumed == simulation && resumed_host.snapshot() == assets,
                    "censor checkpoint changed simulation or asset owners"
                );
                simulation = resumed;
                host = resumed_host;
                restored_censors.extend(censor_pending.iter().copied());
                eprintln!(
                    "{source}: +{}, restored censor boundary {:?}, complete {:?}",
                    simulation.vm.tick - start,
                    censor_pending,
                    complete_censors
                );
            }
            for id in complete_censors {
                let assets = &host.characters[&id];
                frames.push(serde_json::json!({"source":source,"object":id,"phase":"native censor", "tick":simulation.vm.tick,
                    "skeleton":assets.avatar.avatar.skeleton.name,"resident":simulation.characters[&id],
                    "model":assets.model,"geometry":assets.geometry,"mesh_pool":host.mesh_pool,
                    "asset_state":assets.avatar.snapshot(),"owner":assets.appearance_owner}));
                rendered_censors.insert(id);
            }
            if !restored && !pending.is_empty() {
                let saved = Snapshot::capture(&program, &simulation);
                let restored_sim =
                    serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&saved)?)?
                        .restore(&program)
                        .map_err(anyhow::Error::msg)?;
                ensure!(
                    restored_sim == simulation,
                    "live character scheduler restore changed state"
                );
                let assets = host.snapshot();
                let saved_assets = serde_json::from_slice(&serde_json::to_vec(&assets)?)?;
                let restored_host = sims_content::character_loading::live::Host::restore(
                    &index,
                    Headless {
                        visible: true,
                        ..Default::default()
                    },
                    saved_assets,
                )?;
                ensure!(
                    restored_host.snapshot() == assets,
                    "live character assets restore changed state"
                );
                simulation = restored_sim;
                host = restored_host;
                restored = true;
            }
            let deletion_reloads = simulation.pending_deletion_character_services();
            deletion_reload_objects.extend(deletion_reloads.iter().copied());
            let retiring: Vec<_> = simulation.retired_characters.keys().copied().collect();
            match host.service(&mut simulation, &mut program) {
                Ok(n) if n > 0 => {
                    completed_services += n;
                    if !host.pending_notifications().is_empty()
                        && notification_restores < notification_restore_limit
                    {
                        let checkpoint = Snapshot::capture(&program, &simulation);
                        let resumed =
                            serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&checkpoint)?)?
                                .restore(&program)
                                .map_err(anyhow::Error::msg)?;
                        ensure!(
                            resumed == simulation,
                            "notification restore changed simulation"
                        );
                        let assets = host.snapshot();
                        let mut restored_host =
                            sims_content::character_loading::live::Host::restore(
                                &index,
                                Headless {
                                    visible: true,
                                    ..Default::default()
                                },
                                serde_json::from_slice(&serde_json::to_vec(&assets)?)?,
                            )?;
                        ensure!(
                            restored_host.snapshot() == assets,
                            "notification restore changed presenter"
                        );
                        restored_host.update_contexts(&mut simulation)?;
                        let mut missing = assets.clone();
                        missing.notification_jobs.clear();
                        let mut broken = sims_content::character_loading::live::Host::restore(
                            &index,
                            Headless {
                                visible: true,
                                ..Default::default()
                            },
                            missing,
                        )?;
                        ensure!(
                            broken.update_contexts(&mut simulation.clone()).is_err(),
                            "missing notification job accepted"
                        );
                        host = restored_host;
                        simulation = resumed;
                        notification_restores += 1;
                    }
                    if let Some(presenter) = &host.notifications {
                        for (&id, entry) in &presenter.saved.entries {
                            let Some(cache) = &entry.cache else {
                                continue;
                            };
                            let Some(surface) = &entry.surface else {
                                continue;
                            };
                            if cache.active
                                && !host.pending_notifications().contains_key(&id)
                                && captured_notifications.insert((id, cache.clock))
                            {
                                let path = output.join(format!(
                                    "original-notification-{id}-{}.png",
                                    cache.clock
                                ));
                                presenter.image(id)?.save(&path)?;
                                notification_frames.push(serde_json::json!({"object":id,"tick":cache.clock,"cache":cache,"surface":surface,"path":path}));
                                eprintln!(
                                    "{source}: +{}, completed notification {id}: {:?}",
                                    simulation.vm.tick - start,
                                    String::from_utf8_lossy(&surface.text)
                                );
                            }
                        }
                    }
                    for id in retiring {
                        if !simulation.retired_characters.contains_key(&id) {
                            ensure!(
                                !host.characters.contains_key(&id),
                                "retired assets still owned"
                            );
                            completed_retirements.insert(id);
                            eprintln!(
                                "{source}: +{}, completed native character retirement for {id}",
                                simulation.vm.tick - start
                            );
                        }
                    }
                    if !simulation.retired_characters.is_empty()
                        && retirement_restores < retirement_restore_limit
                    {
                        let checkpoint = Snapshot::capture(&program, &simulation);
                        let resumed =
                            serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&checkpoint)?)?
                                .restore(&program)
                                .map_err(anyhow::Error::msg)?;
                        ensure!(
                            resumed == simulation,
                            "retirement restore changed simulation"
                        );
                        let assets = host.snapshot();
                        let mut restored_host =
                            sims_content::character_loading::live::Host::restore(
                                &index,
                                Headless {
                                    visible: true,
                                    ..Default::default()
                                },
                                serde_json::from_slice(&serde_json::to_vec(&assets)?)?,
                            )?;
                        ensure!(
                            restored_host.snapshot() == assets,
                            "retirement restore changed asset state"
                        );
                        simulation = resumed;
                        restored_host.update_contexts(&mut simulation)?;
                        let mut broken = assets.clone();
                        broken.retirements.clear();
                        let mut broken_host = sims_content::character_loading::live::Host::restore(
                            &index,
                            Headless {
                                visible: true,
                                ..Default::default()
                            },
                            broken,
                        )?;
                        ensure!(
                            broken_host
                                .update_contexts(&mut simulation.clone())
                                .is_err(),
                            "retirement accepted a missing asset continuation"
                        );
                        host = restored_host;
                        retirement_restores += 1;
                        eprintln!(
                            "{source}: +{}, restored retirement checkpoint {retirement_restores}",
                            simulation.vm.tick - start
                        );
                    }
                    for id in deletion_reloads {
                        if !simulation
                            .pending_deletion_character_services()
                            .contains(&id)
                        {
                            completed_deletion_reloads.insert(id);
                            eprintln!(
                                "{source}: +{}, completed native deletion appearance reload for {id}",
                                simulation.vm.tick - start
                            );
                        }
                    }
                    if !simulation.pending_deletion_character_services().is_empty()
                        && deletion_reload_restores < 32
                    {
                        let checkpoint = Snapshot::capture(&program, &simulation);
                        let resumed =
                            serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&checkpoint)?)?
                                .restore(&program)
                                .map_err(anyhow::Error::msg)?;
                        ensure!(
                            resumed == simulation,
                            "deletion appearance restore changed simulation"
                        );
                        let assets = host.snapshot();
                        let mut restored_host =
                            sims_content::character_loading::live::Host::restore(
                                &index,
                                Headless {
                                    visible: true,
                                    ..Default::default()
                                },
                                serde_json::from_slice(&serde_json::to_vec(&assets)?)?,
                            )?;
                        ensure!(
                            restored_host.snapshot() == assets,
                            "deletion appearance restore changed asset state"
                        );
                        simulation = resumed;
                        restored_host.update_contexts(&mut simulation)?;
                        let mut broken = assets.clone();
                        broken.reloads.clear();
                        let mut broken_host = sims_content::character_loading::live::Host::restore(
                            &index,
                            Headless {
                                visible: true,
                                ..Default::default()
                            },
                            broken,
                        )?;
                        ensure!(
                            broken_host
                                .update_contexts(&mut simulation.clone())
                                .is_err(),
                            "deletion reset accepted a missing appearance continuation"
                        );
                        host = restored_host;
                        deletion_reload_restores += 1;
                        eprintln!(
                            "{source}: +{}, restored deletion appearance checkpoint {deletion_reload_restores}",
                            simulation.vm.tick - start
                        );
                    }
                    if !host.pending_accessories().is_empty()
                        && accessory_restores < accessory_restore_limit
                    {
                        let asset_checkpoint = host.snapshot();
                        let checkpoint = Snapshot::capture(&program, &simulation);
                        let resumed =
                            serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&checkpoint)?)?
                                .restore(&program)
                                .map_err(anyhow::Error::msg)?;
                        ensure!(
                            resumed == simulation,
                            "accessory restore changed simulation"
                        );
                        let saved_assets =
                            serde_json::from_slice(&serde_json::to_vec(&asset_checkpoint)?)?;
                        let mut restored_host =
                            sims_content::character_loading::live::Host::restore(
                                &index,
                                Headless {
                                    visible: true,
                                    ..Default::default()
                                },
                                saved_assets,
                            )?;
                        ensure!(
                            restored_host.snapshot() == asset_checkpoint,
                            "accessory restore changed asset bindings"
                        );
                        simulation = resumed;
                        restored_host.update_contexts(&mut simulation)?;
                        host = restored_host;
                        accessory_restores += 1;
                        eprintln!(
                            "{source}: +{}, restored accessory checkpoint {accessory_restores}",
                            simulation.vm.tick - start
                        );
                    }
                    for (&id, assets) in &host.characters {
                        if !simulation.characters.contains_key(&id)
                            || rendered_accessories.contains(&id)
                            || host.pending_accessories().contains_key(&id)
                        {
                            continue;
                        }
                        if assets
                            .avatar
                            .records
                            .accessories
                            .iter()
                            .any(|&r| assets.avatar.records.entries[r].binding.is_some())
                        {
                            frames.push(serde_json::json!({"source":source,"object":id,"phase":"native accessory", "tick":simulation.vm.tick,
                                "skeleton":assets.avatar.avatar.skeleton.name,"resident":simulation.characters[&id],
                                "model":assets.model,"geometry":assets.geometry,"mesh_pool":host.mesh_pool,
                                "asset_state":assets.avatar.snapshot(),"owner":assets.appearance_owner}));
                            rendered_accessories.insert(id);
                        }
                    }
                    if simulation
                        .person_factories
                        .values()
                        .any(|s| !s.wrapper_finished)
                        && factory_restores < factory_restore_limit
                    {
                        let saved = Snapshot::capture(&program, &simulation);
                        let restored_sim =
                            serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&saved)?)?
                                .restore(&program)
                                .map_err(anyhow::Error::msg)?;
                        ensure!(
                            restored_sim == simulation,
                            "factory continuation restore changed simulation"
                        );
                        let assets = host.snapshot();
                        let mut restored_host =
                            sims_content::character_loading::live::Host::restore(
                                &index,
                                Headless {
                                    visible: true,
                                    ..Default::default()
                                },
                                serde_json::from_slice(&serde_json::to_vec(&assets)?)?,
                            )?;
                        ensure!(
                            restored_host.snapshot() == assets,
                            "factory continuation restore changed assets"
                        );
                        let mut restored_sim = restored_sim;
                        restored_host.update_contexts(&mut restored_sim)?;
                        if !assets.fresh.is_empty() || !assets.reloads.is_empty() {
                            let mut broken = assets.clone();
                            broken.fresh.clear();
                            broken.reloads.clear();
                            let mut broken_host =
                                sims_content::character_loading::live::Host::restore(
                                    &index,
                                    Headless {
                                        visible: true,
                                        ..Default::default()
                                    },
                                    broken,
                                )?;
                            ensure!(
                                broken_host
                                    .update_contexts(&mut restored_sim.clone())
                                    .is_err(),
                                "factory accepted a missing asset continuation"
                            );
                        }
                        simulation = restored_sim;
                        host = restored_host;
                        factory_restores += 1;
                        if factory_restores == 1 || factory_restores % 8 == 0 {
                            eprintln!(
                                "{source}: restored {factory_restores} live factory service boundaries"
                            );
                        }
                    }
                    continue;
                }
                Err(error) => break format!("asset host: {error:#}"),
                _ => (),
            }
            match result {
                Err(error) => break error,
                Ok(step) if step.stop == StepStop::Dialog => {
                    let dialogs = &simulation.vm.world.dialogs;
                    let id = dialogs.modal.context("Dialog stop without modal request")?;
                    let record = dialogs
                        .records
                        .iter()
                        .find(|r| r.id == id)
                        .context("modal dialog record missing")?
                        .clone();
                    let mut resumed = serde_json::from_slice::<Snapshot>(&serde_json::to_vec(
                        &Snapshot::capture(&program, &simulation),
                    )?)?
                    .restore(&program)
                    .map_err(anyhow::Error::msg)?;
                    ensure!(resumed == simulation, "dialog restore changed simulation");
                    let assets = host.snapshot();
                    let mut resumed_host = sims_content::character_loading::live::Host::restore(
                        &index,
                        Headless {
                            visible: true,
                            ..Default::default()
                        },
                        serde_json::from_slice(&serde_json::to_vec(&assets)?)?,
                    )?;
                    resumed_host.update_contexts(&mut resumed)?;
                    ensure!(
                        resumed == simulation && resumed_host.snapshot() == assets,
                        "dialog restore changed asset bindings"
                    );
                    let before = simulation.clone();
                    ensure!(
                        simulation
                            .run_current_tick(&program, 1000)
                            .map_err(anyhow::Error::msg)?
                            .stop
                            == StepStop::Dialog,
                        "waiting dialog consumed input"
                    );
                    ensure!(simulation == before, "waiting dialog advanced simulation");
                    ensure!(
                        simulation.finish_registry_pass().is_err() && simulation == before,
                        "dialog allowed clock advancement"
                    );
                    // The optional input trace selects exact prompts in order.
                    // Without one, only single-button notices are acknowledged.
                    let acknowledge = record.operands[5] & 127 == 0
                        && record.buttons.len() == 1
                        && record.buttons[0].result == 2;
                    let scripted = pending_dialog_inputs
                        .front()
                        .filter(|input| input.title == record.title && input.body == record.body);
                    let response = scripted
                        .map(|input| input.result)
                        .or(acknowledge.then_some(2));
                    ensure!(
                        response
                            .is_none_or(|result| record.buttons.iter().any(|b| b.result == result)),
                        "scripted response does not match an offered dialog button"
                    );
                    dialog_events.push(serde_json::json!({"tick":simulation.vm.tick,
                        "record":record,"response":response,"scripted":scripted.is_some()}));
                    eprintln!(
                        "{source}: +{}, modal {:?}, explicit test response={response:?}",
                        simulation.vm.tick - start,
                        record.title
                    );
                    if scripted.is_some() {
                        pending_dialog_inputs.pop_front();
                    }
                    let Some(response) = response else {
                        break format!("original dialog requires player choice: {}", record.title);
                    };
                    simulation
                        .vm
                        .answer_dialog(id, response)
                        .map_err(anyhow::Error::msg)?;
                    resumed
                        .vm
                        .answer_dialog(id, response)
                        .map_err(anyhow::Error::msg)?;
                    ensure!(
                        simulation.vm.answer_dialog(id, response).is_err(),
                        "dialog accepted duplicate input"
                    );
                    ensure!(
                        resumed == simulation,
                        "dialog response changed after restore"
                    );
                    simulation = resumed;
                    host = resumed_host;
                    continue;
                }
                Ok(step) if step.stop == StepStop::Budget => {
                    if (budget == 1 && progressed) || step.trace.len() >= budget {
                        continue;
                    }
                    std::fs::write(
                        output.join("original-live-character-stop.json"),
                        serde_json::to_vec(&serde_json::json!({
                            "source":source,"neighborhood_input_experiment":experiment,
                            "tick":simulation.vm.tick,"budget":budget,"trace_count":step.trace.len(),
                            "trace_tail":step.trace.iter().rev().take(12).collect::<Vec<_>>(),"events":step.events,
                            "simulation":Snapshot::capture(&program,&simulation),"assets":host.snapshot()
                        }))?,
                    )?;
                    break format!(
                        "pending live services ({} traced instructions): character updates={:?}; person={:?}; character refresh={:?}",
                        step.trace.len(),
                        simulation.pending_character_updates(),
                        simulation.pending_person_services(),
                        simulation.pending_character_services()
                    );
                }
                Ok(step) if step.stop == StepStop::Faulted => {
                    let faults: BTreeMap<_, _> = simulation
                        .vm
                        .threads
                        .iter()
                        .filter_map(|(&id, t)| {
                            if let sims_vm::Status::Faulted(fault) = &t.status {
                                Some((id, fault))
                            } else {
                                None
                            }
                        })
                        .collect();
                    break format!("VM fault: {faults:?}");
                }
                Ok(_) => {
                    host.present_notifications(
                        simulation.vm.tick as u32,
                        &simulation.characters.keys().copied().collect::<Vec<_>>(),
                    )?;
                    simulation
                        .finish_registry_pass()
                        .map_err(anyhow::Error::msg)?;
                    steps = 0;
                    if (simulation.vm.tick - start) % 100 == 0 {
                        eprintln!(
                            "{source}: native character replay +{} ticks, {completed_services} asset services",
                            simulation.vm.tick - start
                        );
                    }
                    // Explicit headless view update between completed ticks.
                    simulation
                        .update_original_environment(&program)
                        .map_err(anyhow::Error::msg)?;
                    if simulation.vm.tick - start == tick_limit {
                        break format!("{tick_limit}-tick diagnostic cap");
                    }
                }
            }
        };
        let replay_seconds = replay_started.elapsed().as_secs_f64();
        let replay_ticks = simulation.vm.tick - start;
        eprintln!(
            "{source}: setup {setup_seconds:.3}s; replay {replay_seconds:.3}s, {replay_ticks} ticks, {service_slices} slices, {:.2} ticks/s (includes diagnostic checkpoints and frame capture; excludes setup/final verification)",
            replay_ticks as f64 / replay_seconds
        );
        std::fs::write(
            output.join("original-live-character-final.json"),
            serde_json::to_vec(
                &serde_json::json!({"source":source,"neighborhood_input_experiment":experiment,
                "start":start,"tick":simulation.vm.tick,
                "program_modules":program.modules.keys().collect::<Vec<_>>(),
                "program_animations":program.animations.keys().collect::<Vec<_>>(),
                "boundary":boundary,"simulation":Snapshot::capture(&program,&simulation),"assets":host.snapshot()}),
            )?,
        )?;
        ensure!(
            simulation.animations.is_empty(),
            "native character allocated legacy Playback"
        );
        ensure!(
            simulation.routes.is_empty()
                && simulation.native_lot.as_ref().unwrap().movement.is_none(),
            "native character allocated tuned locomotion"
        );
        ensure!(
            restored && completed_services >= 6,
            "no verified live character service sequence"
        );
        let restored_sim = Snapshot::capture(&program, &simulation)
            .restore(&program)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            restored_sim == simulation,
            "completed geometry restore changed simulation"
        );
        let final_assets = host.snapshot();
        let mut final_host = sims_content::character_loading::live::Host::restore(
            &index,
            Headless {
                visible: true,
                ..Default::default()
            },
            serde_json::from_slice(&serde_json::to_vec(&final_assets)?)?,
        )?;
        ensure!(
            final_host.snapshot() == final_assets,
            "final live asset/presenter restore changed state"
        );
        final_host.update_contexts(&mut restored_sim.clone())?;
        if simulation
            .person_factories
            .values()
            .any(|s| !simulation.vm.lifecycles.contains_key(&s.factory.object()))
        {
            // Registration legitimately precedes the person's thread. It must
            // never become an ownerless object when restoring this boundary.
            ensure!(
                sims_vm::Snapshot::capture(&program, &simulation.vm)
                    .restore(&program)
                    .is_err(),
                "standalone VM accepted an unowned pre-thread person factory"
            );
            let mut broken = Snapshot::capture(&program, &simulation);
            broken.state.person_factories.clear();
            ensure!(
                broken.restore(&program).is_err(),
                "snapshot lost person factory continuation"
            );
        }
        ensure!(
            simulation
                .pending_person_services()
                .iter()
                .all(|(_, r)| !matches!(r, sims_sim::person_update::Request::Environment { .. })),
            "live original room lookup remained pending"
        );
        {
            // Keep a completed ordinary update even when the household now
            // stops in a later VM service rather than in the person's tail.
            let (&id, resident) = simulation
                .characters
                .iter()
                .find(|(id, r)| {
                    r.ordinary_update.is_none()
                        && !r.character.busy()
                        && (r.person_update.is_some()
                            || simulation.household.as_ref().unwrap().people[id]
                                .finished
                                .is_some())
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("no completed ordinary character for visual check")
                })?;
            let assets = &host.characters[&id];
            frames.push(serde_json::json!({"source": source, "object": id,
                "skeleton": assets.avatar.avatar.skeleton.name, "resident": resident,
                "model": assets.model, "geometry": assets.geometry, "mesh_pool": host.mesh_pool,
                "asset_state": assets.avatar.snapshot(), "owner": assets.appearance_owner}));
        }
        eprintln!(
            "{source}: live native {policy:?} +{}, {completed_services} services, restored={restored}; {boundary}",
            simulation.vm.tick - start
        );
        report.push(serde_json::json!({"source": source, "start": start, "tick": simulation.vm.tick,
            "timing": {"setup_seconds":setup_seconds,"replay_seconds":replay_seconds,
                "service_slices":service_slices,"ticks_per_second":replay_ticks as f64/replay_seconds,
                "scope":"Headless replay includes diagnostic checkpoints and frame capture; excludes setup, final verification and compilation."},
            "neighborhood_input_experiment": experiment,
            "environment_inputs": {"tuning":room_tuning,"geometry":room_geometry,"objects":room_objects},
            "environment":simulation.room_environment,
            "completed_services": completed_services, "restored": restored, "factory_restores": factory_restores,
            "walk_restores": walk_restores, "physical_restores": physical_restores,
            "accessory_restores": accessory_restores, "rendered_accessories": rendered_accessories,
            "reach_checkpoints": reach_checkpoints, "reach_instruction_budget":reach_budget,
            "burn_restores": burn_restores,
            "reset_restores":reset_restores,"completed_live_resets":completed_live_resets,
            "one_instruction_after":one_instruction_after,
            "deletion_reload_restores": deletion_reload_restores, "deletion_reload_objects": deletion_reload_objects,
            "completed_deletion_reloads": completed_deletion_reloads,
            "retirement_restores": retirement_restores, "completed_retirements": completed_retirements,
            "notification_restores": notification_restores, "notification_frames": notification_frames,
            "dialogs":dialog_events,"dialog_input_policy":"single-button acknowledgment plus optional exact-prompt input trace",
            "pending_dialog_inputs":pending_dialog_inputs,
            "stop_restores": stop_restores, "stopping_routes": stopping_routes,
            "rendered_stops": rendered_stops,
            "walking_positions": walking_positions, "rendered_walkers": rendered_walkers,
            "rendered_factories": rendered_factories, "routing_policy": policy, "boundary": boundary,
            "character_requests": simulation.pending_character_updates(), "person_requests": simulation.pending_person_services(),
            "refresh_requests": simulation.pending_character_services()}));
    }
    ensure!(
        !report.is_empty(),
        "live house filter selected no original homes"
    );
    let suffix = if policy == sims_sim::lot_route::path::Policy::InferredRectangles {
        "-inferred"
    } else {
        ""
    };
    std::fs::write(
        output.join(format!("original-live-character-prefix{suffix}.json")),
        serde_json::to_vec_pretty(&report)?,
    )?;
    std::fs::write(
        output.join(format!("original-live-character-frames{suffix}.json")),
        serde_json::to_vec(&frames)?,
    )?;
    Ok(())
}
