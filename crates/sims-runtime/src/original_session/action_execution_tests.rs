//! Authored in-memory Session and real UpdateLot controls.
//! No private assets, saved checkpoints, native source admission or filesystem IO.
use super::super::*;
use super::*;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sims_content::{
    Resource,
    character_loading::live::{self, Host},
};
use sims_formats::behavior::{Behavior, Instruction};
use sims_sim::{
    Room, SimExecutionEvent,
    architecture::Architecture,
    lot_geometry::{FloorSource, Lot},
    lot_links::Links,
    lot_placement,
    lot_runtime::State as LotState,
};
use sims_vm::{
    Definition, Module, Object, Routine, Status, Vm, World, autonomy::Placement,
    person::WorldState, person_loading::RoomMembership, scheduler::clock as native_clock,
};
use std::{collections::BTreeMap, path::PathBuf};

fn bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    Ok(postcard::to_allocvec(value)?)
}

fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn resource(locator: &str, value: &[u8]) -> Resource {
    Resource {
        locator: locator.into(),
        name: locator.rsplit('/').next().unwrap().into(),
        source: PathBuf::from("/action-execution-must-not-open-payloads").join(locator),
        offset: 0,
        size: value.len() as u64,
        unpacked_size: value.len() as u64,
        archived: false,
        owned: Some(Arc::from(value)),
    }
}

fn session() -> Result<Session> {
    let index = Arc::new(Index {
        root: PathBuf::from("/action-execution-must-not-open-installation"),
        resources: vec![
            resource("UserData/Neighborhood.iff", b"authored neighborhood"),
            resource("UserData/Houses/House07.iff", b"authored house"),
        ],
        archives: 0,
        diagnostics: vec![],
    });
    let mut house: House = serde_json::from_value(json!({
        "source": "UserData/Houses/House07.iff",
        "number": 7,
        "neighborhood_source": "UserData/Neighborhood.iff",
        "sources": {}, "width": 3, "height": 3,
        "floors": [], "floor_map": [], "wall_map": [], "arrays": {},
        "house": {"version": 38, "name": "authored", "pairs": ([[0, 0]; 6]), "tail": []},
        "view": {
            "version": 38, "fields": [0, 0, 0, 0], "camera_direction": 0,
            "unknown_shorts": [0, 0], "unknown_word": 0, "roof": "", "tail": []
        },
        "simulator": {
            "version": 62, "globals": [], "ticks": 0, "scheduler_cursor": 0,
            "funds": 0, "lot_rng": 7, "global_rng": 11, "pending_funds": 0,
            "lot_value": 0, "objects_value": 0, "architecture_value": 0,
            "budget_days": [], "tail": []
        },
        "types": {"version": 2, "entries": []},
        "objects": {"version": 0, "order": [], "types_by_id": {}, "instances": {}, "tail": []},
        "definitions": {}, "neighbors": {"version": 0, "entries": [], "tail": []},
        "neighborhood": {
            "version": 20, "data": ([0; 16]), "inventories": [],
            "options": ([false; 11]), "option_bytes": [], "tail": []
        },
        "resource_storage": null, "families": {}, "household": null, "missing_tile_heads": []
    }))?;
    house.sources = index
        .resources
        .iter()
        .map(|resource| {
            (
                resource.locator.clone(),
                digest(resource.owned.as_ref().unwrap()),
            )
        })
        .collect();
    let assets = live::Snapshot {
        floors: BTreeMap::new(),
        neighborhood_source: house.neighborhood_source.clone(),
        initializer_settings: Settings {
            fade_enabled: false,
            fade_duration: 0,
            sampling_config: 0,
        },
        fresh: BTreeMap::new(),
        reloads: BTreeMap::new(),
        dependent_jobs: BTreeMap::new(),
        accessories: BTreeMap::new(),
        retirements: BTreeMap::new(),
        notification_jobs: BTreeMap::new(),
        notifications: None,
        characters: BTreeMap::new(),
        mesh_pool: Default::default(),
        clothing: Default::default(),
        skeletons: None,
        ordinary: Default::default(),
        autonomy_resources: None,
    };
    Ok(Session {
        loaded_input_origin: None,
        host: Host::restore(&index, View::default(), assets)?,
        index,
        managed_policy: managed::Policy::HistoricalSchema1,
        house,
        program: Program::default(),
        simulation: Simulation::new(
            Vm::new(World::default(), 0x1234),
            Room {
                width: 48,
                height: 48,
                obstacles: vec![],
            },
            BTreeMap::new(),
        )
        .map_err(anyhow::Error::msg)?,
        options: Options::default(),
    })
}

fn state(session: &Session) -> Result<Vec<u8>> {
    let owned: Vec<_> = session
        .index
        .resources
        .iter()
        .map(|resource| (&resource.locator, resource.owned.as_deref()))
        .collect();
    bytes(&(
        session.save(),
        &session.house,
        &session.program,
        &session.index.resources,
        owned,
        session.managed_policy == managed::Policy::CompletedOwnedSchema2,
    ))
}

fn instruction(opcode: u16, operands: [u8; 8], next: u8) -> Instruction {
    Instruction {
        opcode,
        operands,
        on_true: next,
        on_false: 255,
    }
}

fn registry_session() -> Result<Session> {
    let mut session = session()?;
    let returned = instruction(2, [0, 0, 0, 0, 0, 5, 8, 8], 254);
    let mut module = Module {
        source: "objects".into(),
        headline_catalog: Some(Default::default()),
        ..Default::default()
    };
    for (definition, main) in [(1, 4096), (2, 4097), (4, 4110)] {
        let mut words = vec![0; 128];
        words[9] = 4;
        words[14] = definition as i16;
        words[67] = 8;
        module.object_definition_words.insert(definition, words);
        let mut functions = vec![
            sims_formats::execution::ObjectFunction {
                action: 0,
                condition: 0,
            };
            27
        ];
        functions[1].action = main;
        module.object_functions.insert(definition, functions);
        module.routines.insert(
            main,
            Routine {
                name: "authored main".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 0,
                    locals: 0,
                    instructions: vec![returned.clone()],
                },
            },
        );
    }
    module.routines.get_mut(&4096).unwrap().body.instructions = vec![
        instruction(8, [0, 0, 8, 0, 19, 0, 7, 0], 1),
        instruction(42, [2, 0, 0, 0, 6, 0, 0, 0], 2),
        instruction(8, [0, 0, 8, 0, 19, 0, 7, 0], 254),
    ];
    for number in [1, 5, 6] {
        module.named_constants.insert(
            number,
            sims_formats::tuning::Constants {
                version: 1,
                entries: vec![],
            },
        );
    }
    for number in [501, 502, 503, 504] {
        module.strings.insert(number, vec![String::new(); 9]);
    }
    session.program = Program {
        globals: "objects".into(),
        modules: [("objects".into(), module)].into(),
        ..Default::default()
    };
    let floor = FloorSource {
        coverings: vec![0; 36],
        walls: vec![Default::default(); 36],
        object_heads: vec![0; 36],
        flags: vec![0; 36],
    };
    let lot = Lot::rebuild(
        6,
        vec![[0; 4]; 36],
        vec![0; 36],
        vec![255; 36],
        vec![floor.clone(), floor],
    )
    .map_err(anyhow::Error::msg)?;
    let mut architecture =
        Architecture::from_original(lot, &session.program).map_err(anyhow::Error::msg)?;
    architecture
        .bind_footprints(&session.program)
        .map_err(anyhow::Error::msg)?;
    let mut world = World {
        person: Some(WorldState {
            object_order: vec![2, 4, 1, 3],
            ..Default::default()
        }),
        scheduler_storage: Some(Default::default()),
        refresh: Some(Default::default()),
        ..Default::default()
    };
    world
        .globals
        .extend([(10, 1), (11, 0), (17, 0), (19, 2), (34, 0)]);
    for object in 1..=4 {
        let mut data: BTreeMap<_, _> = (0..72).map(|field| (field, 0)).collect();
        data.extend([
            (3, -1),
            (14, if object == 2 || object == 3 { 0 } else { -1 }),
            (
                26,
                match object {
                    2 => 3,
                    3 => 1,
                    _ => 0,
                },
            ),
            (29, if object == 4 { -5 } else { 0 }),
        ]);
        let mut owner = Object {
            definition: Some(Definition {
                source: "objects".into(),
                object: if object == 4 { 1 } else { 4 },
                guid: if object == 4 { 1 } else { 4 },
                slots: 0,
            }),
            data,
            native_tree_error: Some(0),
            native_flags: Some(0x240),
            ..Default::default()
        };
        world
            .register_scheduler_object(object, &mut owner)
            .map_err(anyhow::Error::msg)?;
        owner.idle.as_mut().unwrap().flags = 3;
        world.objects.insert(object, owner);
        world.positions.insert(
            object,
            if object == 4 {
                Placement {
                    x: -32,
                    y: -16,
                    level: 2,
                }
            } else {
                Placement {
                    x: 40,
                    y: 40,
                    level: 1,
                }
            },
        );
    }
    architecture.publish_room_context(&mut world);
    let mut heads = vec![vec![0; 36]; 2];
    heads[0][14] = 1;
    let links = Links {
        size: 6,
        heads,
        next: [(1, 3), (2, 0), (3, 2), (4, 0)].into(),
    };
    links
        .publish_tile_chains(&mut world)
        .map_err(anyhow::Error::msg)?;
    let native = LotState {
        placement: lot_placement::State::new(links, &architecture).map_err(anyhow::Error::msg)?,
        membership: RoomMembership::constructed(&world),
        indicators: Default::default(),
        observers: vec![],
        diagnostics: sims_sim::lot_acceptance::person::Diagnostics {
            error: 0,
            blocker: None,
        },
        route: None,
        shoo: None,
        routing_policy: Default::default(),
        movement: None,
        snap: None,
        find_location: None,
        removal: None,
        snap_offsets: Default::default(),
        completion: None,
        route_paths: Default::default(),
        selected_person: None,
        room_effects: vec![],
    };
    let mut vm = Vm::new(world, 17);
    vm.native_accessory_host = true;
    vm.lot_rng_state = Some(1825128472);
    vm.crt_rng_state = Some(23);
    for object in 1..=4 {
        vm.start_object_lifecycle(&session.program, object)
            .map_err(anyhow::Error::msg)?;
    }
    session.simulation =
        Simulation::from_loaded_lot(vm, architecture, native).map_err(anyhow::Error::msg)?;
    session
        .simulation
        .enable_household(
            &session.program,
            sims_sim::clock::LotClock {
                hour: 12,
                day: 1,
                period: 0,
                minute: 0,
                second: 0,
                month: 1,
                year: 1997,
                elapsed_days: 0,
                month_days: 30,
                ticks: 0,
                statistics: [[0; 8]; 6],
            },
        )
        .map_err(anyhow::Error::msg)?;
    session
        .simulation
        .enable_original_environment(
            &session.program,
            sims_sim::lot_environment::manager::Settings {
                daylight_multiplier: 1.,
                light_range: 12.,
                rooms_enabled: true,
                models: std::array::from_fn(|index| Some(index as u8)),
            },
        )
        .map_err(anyhow::Error::msg)?;
    session
        .simulation
        .enable_registry_scheduler(0)
        .map_err(anyhow::Error::msg)?;
    Ok(session)
}

fn attach_authored_clock(session: &mut Session) -> Result<()> {
    let mut transport = transport::State::from_completed_inputs(
        sims_vm::interface::state::State::constructed(500),
        Default::default(),
    );
    transport.timing = None;
    session.host.presentation.transport = Some(transport);
    session.host.presentation.clock = Some(clock::State {
        fields: native_clock::Fields {
            timing: Default::default(),
            period: 30,
            timer: native_clock::Timer {
                interval: 30,
                limit: 30,
                ..Default::default()
            },
            budget_d8: 60,
            budget_dc: 90,
            poll_messages: 0,
            publish: 0,
        },
        now: 100,
        stopping: 0,
        requests: vec![],
        call: None,
        captured_code: None,
    });
    session.begin_clock_frame()
}

#[test]
fn real_session_step_forks_preserve_trace_save_rng_and_service_order() -> Result<()> {
    let initial = registry_session()?;
    let mut ordinary = initial.fork_for_exploration()?;
    let mut disabled = initial.fork_for_exploration()?;
    let mut observed = initial.fork_for_exploration()?;
    let mut expected_trace = Vec::new();
    let mut disabled_trace = Vec::new();
    let mut actual_trace = Vec::new();
    let expected = ordinary.step_with_trace(&mut expected_trace)?;
    assert_eq!(
        disabled.step_with_trace_and_action_observer(&mut disabled_trace, &mut |_| {})?,
        expected
    );
    let mut events = Vec::new();
    assert_eq!(
        observed.step_with_trace_and_action_observer(&mut actual_trace, &mut |event| events
            .push(event))?,
        expected
    );
    assert_eq!(actual_trace, expected_trace);
    assert_eq!(disabled_trace, expected_trace);
    assert_eq!(state(&ordinary)?, state(&disabled)?);
    assert_eq!(state(&ordinary)?, state(&observed)?);
    assert!(!actual_trace.is_empty());
    let position = |owner| {
        events
            .iter()
            .position(|event| {
                matches!(event,
        SimExecutionEvent::Gap { owner: actual } if *actual == owner)
            })
            .unwrap()
    };
    assert!(position("Host::update_contexts") < position("Host::service"));
    assert!(position("Host::service") < position("Simulation::finish_registry_pass"));
    assert!(
        position("Simulation::finish_registry_pass")
            < position("Simulation::update_original_environment")
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, SimExecutionEvent::Vm(_)))
    );
    Ok(())
}

#[test]
fn existing_service_observer_callback_errors_are_not_dropped() -> Result<()> {
    let mut session = registry_session()?;
    let mut cuts = Vec::new();
    let error = session
        .step_with_service_observer(&mut |cut, _| {
            cuts.push(cut);
            anyhow::bail!("authored service observer stop")
        })
        .unwrap_err();
    assert_eq!(error.to_string(), "authored service observer stop");
    assert_eq!(cuts.len(), 1);
    assert!(matches!(cuts[0], sims_sim::ServiceCut::Creation { .. }));
    Ok(())
}

#[test]
fn pending_clock_guard_matches_ordinary_step_before_any_fact_or_mutation() -> Result<()> {
    let mut initial = registry_session()?;
    attach_authored_clock(&mut initial)?;
    let mut ordinary = initial.fork_for_exploration()?;
    let mut observed = initial.fork_for_exploration()?;
    let before = state(&initial)?;
    let expected = ordinary.step().unwrap_err().to_string();
    let mut events = Vec::new();
    let actual = observed
        .step_with_action_observer(&mut |event| events.push(event))
        .unwrap_err()
        .to_string();
    assert_eq!(actual, expected);
    assert!(actual.contains("elapsed frame owns"));
    assert_eq!(state(&ordinary)?, before);
    assert_eq!(state(&observed)?, before);
    assert!(events.is_empty());
    Ok(())
}

#[test]
fn elapsed_routes_through_real_update_lot_and_acknowledges_tick_or_partial_pass_once() -> Result<()>
{
    for paused in [false, true] {
        for budget in [0, 1, 2, 100] {
            let mut initial = registry_session()?;
            initial
                .simulation
                .vm
                .world
                .globals
                .insert(17, i16::from(paused));
            attach_authored_clock(&mut initial)?;
            let mut ordinary = initial.fork_for_exploration()?;
            let mut observed = initial.fork_for_exploration()?;
            let mut expected_trace = Vec::new();
            let mut actual_trace = Vec::new();
            let mut events = Vec::new();
            if budget == 0 {
                let before = state(&ordinary)?;
                let expected = ordinary.advance_clock(0).unwrap_err().to_string();
                assert_eq!(
                    observed
                        .advance_clock_with_action_observer(0, &mut |event| events.push(event))
                        .unwrap_err()
                        .to_string(),
                    expected
                );
                assert_eq!(state(&ordinary)?, before);
                assert_eq!(state(&observed)?, before);
                assert!(events.is_empty());
                continue;
            }
            let mut reached_household = false;
            for _ in 0..8 {
                let expected = ordinary.advance_clock_with_trace(budget, &mut expected_trace)?;
                let actual = observed.advance_clock_with_trace_and_action_observer(
                    budget,
                    &mut actual_trace,
                    &mut |event| events.push(event),
                )?;
                assert_eq!(actual, expected);
                assert_eq!(actual_trace, expected_trace);
                assert_eq!(state(&ordinary)?, state(&observed)?);
                if let clock::Progress::Household(progress) = actual {
                    assert!(matches!(
                        progress,
                        super::super::Progress::Tick | super::super::Progress::PartialPass
                    ));
                    let call = observed
                        .host
                        .presentation
                        .clock
                        .as_ref()
                        .unwrap()
                        .call
                        .as_ref()
                        .unwrap();
                    assert_eq!(call.updates, 1);
                    assert_eq!(call.tick, observed.simulation.vm.tick);
                    assert_eq!(call.frame.request(), None);
                    assert!(call.frame.at_completed_update());
                    reached_household = true;
                    break;
                }
            }
            assert!(reached_household);
            assert!(events.iter().any(|event| matches!(
                event,
                SimExecutionEvent::Gap {
                    owner: "clock::UpdateLot"
                }
            )));
            assert!(events.iter().any(|event| matches!(
                event,
                SimExecutionEvent::Gap {
                    owner: "Simulation::finish_registry_pass"
                }
            )));
        }
    }
    Ok(())
}

#[test]
fn elapsed_household_error_keeps_real_update_lot_unacknowledged() -> Result<()> {
    let mut initial = registry_session()?;
    attach_authored_clock(&mut initial)?;
    initial.host.presentation.awaiting_house_open = true;
    let mut ordinary = initial.fork_for_exploration()?;
    let mut observed = initial.fork_for_exploration()?;
    let expected = ordinary.advance_clock(100).unwrap_err().to_string();
    let mut events = Vec::new();
    assert_eq!(
        observed
            .advance_clock_with_action_observer(100, &mut |event| events.push(event))
            .unwrap_err()
            .to_string(),
        expected
    );
    assert!(expected.contains("house opening"));
    let call = observed
        .host
        .presentation
        .clock
        .as_ref()
        .unwrap()
        .call
        .as_ref()
        .unwrap();
    assert_eq!(call.updates, 0);
    assert_eq!(call.frame.request(), Some(native_clock::Request::UpdateLot));
    assert_eq!(state(&ordinary)?, state(&observed)?);
    assert_eq!(
        events,
        vec![SimExecutionEvent::Gap {
            owner: "clock::UpdateLot"
        }]
    );
    Ok(())
}

#[test]
fn elapsed_ui_handoff_neither_acknowledges_the_update_nor_claims_terminal_action() -> Result<()> {
    let mut initial = registry_session()?;
    initial
        .program
        .modules
        .get_mut("objects")
        .unwrap()
        .routines
        .get_mut(&4096)
        .unwrap()
        .body
        .instructions = vec![instruction(0x22, [0, 7, 0xdd, 7, 1, 0, 0, 0], 254)];
    attach_authored_clock(&mut initial)?;
    let mut ordinary = initial.fork_for_exploration()?;
    let mut observed = initial.fork_for_exploration()?;
    let expected = ordinary.advance_clock(100)?;
    let mut events = Vec::new();
    assert_eq!(
        observed.advance_clock_with_action_observer(100, &mut |event| events.push(event))?,
        expected
    );
    assert!(matches!(
        expected,
        clock::Progress::Household(super::super::Progress::UiEffect(_))
    ));
    let call = observed
        .host
        .presentation
        .clock
        .as_ref()
        .unwrap()
        .call
        .as_ref()
        .unwrap();
    assert_eq!(call.updates, 0);
    assert_eq!(call.frame.request(), Some(native_clock::Request::UpdateLot));
    assert_eq!(state(&ordinary)?, state(&observed)?);
    assert!(!events.iter().any(|event| matches!(
        event,
        SimExecutionEvent::Vm(sims_vm::action_observation::Fact::Returned { .. })
    )));
    Ok(())
}

#[test]
fn elapsed_unsupported_services_and_input_handoffs_keep_their_pending_owners() -> Result<()> {
    for mode in 0..3 {
        let mut initial = registry_session()?;
        attach_authored_clock(&mut initial)?;
        assert!(matches!(
            initial.advance_clock(100)?,
            clock::Progress::Household(
                super::super::Progress::Tick | super::super::Progress::PartialPass
            )
        ));
        let clock = initial.host.presentation.clock.as_mut().unwrap();
        let (owner, request) = match mode {
            0 => {
                clock.requests.push(NeighborhoodRequest {
                    id: 1,
                    neighbor: 1,
                    payload: 0,
                    kind: 0,
                    status: 0,
                    age: 0,
                });
                (
                    "clock::ProcessRequests",
                    native_clock::Request::ProcessRequests,
                )
            }
            1 => {
                clock.fields.publish = 1;
                ("clock::Publish", native_clock::Request::Publish)
            }
            _ => {
                clock.fields.poll_messages = 1;
                ("clock::PeekMessage", native_clock::Request::PeekMessage)
            }
        };
        let mut ordinary = initial.fork_for_exploration()?;
        let mut observed = initial.fork_for_exploration()?;
        let expected = ordinary
            .advance_clock(100)
            .map_err(|error| error.to_string());
        let mut events = Vec::new();
        let actual = observed
            .advance_clock_with_action_observer(100, &mut |event| events.push(event))
            .map_err(|error| error.to_string());
        assert_eq!(actual, expected);
        if mode < 2 {
            assert!(actual.is_err());
        } else {
            assert_eq!(actual.unwrap(), clock::Progress::PollInput);
        }
        let call = observed
            .host
            .presentation
            .clock
            .as_ref()
            .unwrap()
            .call
            .as_ref()
            .unwrap();
        assert_eq!(call.updates, 1);
        assert_eq!(call.frame.request(), Some(request));
        assert!(events.iter().any(|event| matches!(event,
            SimExecutionEvent::Gap { owner: actual } if *actual == owner)));
        assert_eq!(state(&ordinary)?, state(&observed)?);
    }
    Ok(())
}

#[test]
fn nonadmitted_parent_profile_only_checks_the_early_ordinary_entry_guard() -> Result<()> {
    let mut initial = registry_session()?;
    initial.simulation.vm.tick = 88448;
    attach_authored_clock(&mut initial)?;
    assert!(matches!(
        initial.advance_clock(100)?,
        clock::Progress::Household(
            super::super::Progress::Tick | super::super::Progress::PartialPass
        )
    ));
    let clock = initial.host.presentation.clock.as_mut().unwrap();
    clock.captured_code = Some(clock::CapturedCode::Goth88448DormantProtectedPoll20261029);
    initial.host.presentation.application_parent =
        Some(application_parent::State::from_completed_inputs(
            completed_application::Fields {
                in_update: 1,
                suppressed: 0,
                post_clock_flag: 1,
            },
            clock,
            initial.simulation.vm.tick,
        )?);
    let before = state(&initial)?;
    let mut ordinary = initial.fork_for_exploration()?;
    let mut observed = initial.fork_for_exploration()?;
    let expected = ordinary.step().unwrap_err().to_string();
    let mut events = Vec::new();
    assert_eq!(
        observed
            .step_with_action_observer(&mut |event| events.push(event))
            .unwrap_err()
            .to_string(),
        expected
    );
    assert!(expected.contains("retained application parent owns"));
    assert_eq!(state(&ordinary)?, before);
    assert_eq!(state(&observed)?, before);
    assert!(events.is_empty());
    Ok(())
}

mod scheduled_action_controls {
    use super::super::super::Progress as HouseholdProgress;
    use super::*;
    use sims_sim::{
        character::{Controls, Resident},
        character_update, head_look, pose,
    };
    use sims_vm::{
        Start, action,
        action_observation::{Evidence, Fact, FrameFingerprint, NativeReturn, Unavailable},
    };

    fn install_root_host(session: &mut Session) -> Result<()> {
        let avatar = sims_content::saved_avatar::BoundAvatar::empty(&session.index, "root", 1)?;
        let bound = pose::model::Bound::root(session.simulation.vm.crt_rng_state.as_mut().unwrap())
            .map_err(anyhow::Error::msg)?;
        let mut constructor = pose::construction::State::new(session.simulation.vm.tick as i32);
        constructor.carry_selection = Some(-1);
        let ordinary = constructor.prefix().map_err(anyhow::Error::msg)?;
        let head = constructor.head().map_err(anyhow::Error::msg)?;
        let generation = bound.skeleton.geometry.generation;
        let model = sims_content::character_model::State {
            dirty: bound.skeleton.geometry.dirty,
            evaluated_generation: bound.skeleton.geometry.evaluated_generation,
            bounds_generation: bound.skeleton.geometry.bounds_generation,
            bone_generations: bound.skeleton.geometry.bone_generations,
            wiggle: bound.skeleton.wiggle,
            flags: bound.skeleton.flags,
            named_bone_flags: bound.skeleton.named_bone_flags,
            local_bounds: bound.local_bounds,
            render_bounds: bound.render_bounds,
            bone_bounds: bound.bone_bounds,
            bone_bounds_generations: bound.bone_bounds_generations,
            previous_mode: constructor.previous_mode,
            phase_window: constructor.phase_window,
            stop_distance: constructor.stop_distance,
            clip_distances: constructor.clip_distances,
            needs_appearance: String::new(),
        };
        let character = pose::refresh::Character::new(
            bound.skeleton.skin,
            bound.skeleton.hierarchy,
            head,
            head_look::rig::Rig {
                head: None,
                hand: None,
                pelvis: None,
                forward: None,
                turn_bone: None,
            },
            bound.matrix.values,
            constructor.placement,
            bound.model,
            pose::refresh::Censor {
                binding: None,
                mask: 0,
            },
            constructor.clock,
        )
        .map_err(anyhow::Error::msg)?;
        let mut resident = Resident::new(
            character,
            session.simulation.vm.tick,
            Controls {
                ordinary,
                movement: pose::movement::Movement::new(constructor.movement),
                settings: Settings {
                    fade_enabled: false,
                    fade_duration: 0,
                    sampling_config: 0,
                },
                movement_anchors: [None; 2],
                geometry_generation: generation,
                animation_events: 0,
                engaged: 0,
            },
            pose::properties::Context {
                actor: 1,
                sound_visible: false,
                selected: false,
                debug_sound: false,
                position_x: 0.,
                appearance_mode: 0,
                snow: false,
                lot_kind: 1,
            },
        );
        resident.source = Some(avatar.avatar.skeleton.clone());
        let geometry = sims_content::character_geometry::CharacterGeometry::new(
            avatar.avatar.skeleton.bones.len(),
        );
        session.simulation.characters.insert(1, resident);
        session.host.characters.insert(
            1,
            live::Assets {
                model,
                geometry,
                avatar,
                appearance_owner: pose::appearance_reload::Owner::constructed(true),
            },
        );
        session.simulation.native_character_settings = Some(character_update::Settings {
            movement: pose::movement::Settings {
                short_distance: 3.1,
                turn_rate: 1.,
                small_turn: 0.5,
                medium_turn: 1.5,
                large_turn: 3.,
                minimum_weight: 0.5,
                snap_distance: 0.5,
                rate: pose::ordinary::RateSettings {
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
            terrain_scale: None,
        });
        session.host.update_contexts(&mut session.simulation)?;
        Ok(())
    }

    fn fixture() -> Result<(Session, u64, u32)> {
        let mut session = registry_session()?;
        let module = session.program.modules.get_mut("objects").unwrap();
        module.object_definition_words.get_mut(&1).unwrap()[9] = 2;
        module.object_functions.get_mut(&2).unwrap()[1].action = 4098;
        for (routine, opcode, operands) in [
            (4096, 17, [0, 0, 1, 0, 0, 0, 0, 0]),
            (4097, 2, [0, 0, 0, 0, 0, 2, 7, 7]),
            (4098, 0, [0; 8]),
        ] {
            module.routines.insert(
                routine,
                Routine {
                    name: format!("authored scheduled action {routine}"),
                    body: Behavior {
                        format: 0x8002,
                        kind: 0,
                        args: 4,
                        locals: 0,
                        instructions: vec![instruction(opcode, operands, 254)],
                    },
                },
            );
        }
        for table in [150, 152] {
            module.strings.insert(table, vec![String::new(); 40]);
        }
        module.object_interactions.insert(
            2,
            vec![sims_vm::program::MenuEntry {
                id: 19,
                flags: 1,
                action: 4097,
                check: 0,
                ..Default::default()
            }],
        );
        let lot = session
            .simulation
            .architecture
            .take()
            .unwrap()
            .original
            .unwrap();
        let mut architecture =
            Architecture::from_original(lot, &session.program).map_err(anyhow::Error::msg)?;
        architecture
            .bind_footprints(&session.program)
            .map_err(anyhow::Error::msg)?;
        let mut world = World {
            person: Some(WorldState {
                object_order: vec![1, 2],
                person_order: vec![1],
                ..Default::default()
            }),
            actions: Some(action::WorldState {
                actors: [(1, action::Actor::default())].into(),
                ..Default::default()
            }),
            scheduler_storage: Some(Default::default()),
            refresh: Some(Default::default()),
            ..Default::default()
        };
        world
            .globals
            .extend([(10, 1), (11, 0), (17, 0), (19, 2), (34, 0)]);
        for object in [1, 2] {
            let mut owner = Object {
                definition: Some(Definition {
                    source: "objects".into(),
                    object: object as u16,
                    guid: object as u32,
                    slots: 0,
                }),
                data: (0..80).map(|field| (field, 0)).collect(),
                slots: vec![None; 3],
                native_tree_error: Some(0),
                native_flags: Some(0x240),
                ..Default::default()
            };
            owner.data.insert(3, -1);
            if object == 1 {
                owner.person = Some((0..256).map(|field| (field, 0)).collect());
                owner.person.as_mut().unwrap().extend([(58, 30), (85, 100)]);
                owner.motives = Some([0; 16]);
            }
            world
                .register_scheduler_object(object, &mut owner)
                .map_err(anyhow::Error::msg)?;
            owner.idle.as_mut().unwrap().flags = 3;
            world.objects.insert(object, owner);
            world.positions.insert(
                object,
                Placement {
                    x: i32::from(object) * 16,
                    y: 16,
                    level: 1,
                },
            );
        }
        architecture.publish_room_context(&mut world);
        let mut heads = vec![vec![0; 36]; 2];
        heads[0][7] = 1;
        heads[0][8] = 2;
        let links = Links {
            size: 6,
            heads,
            next: [(1, 0), (2, 0)].into(),
        };
        links
            .publish_tile_chains(&mut world)
            .map_err(anyhow::Error::msg)?;
        let mut native = session.simulation.native_lot.take().unwrap();
        native.placement =
            lot_placement::State::new(links, &architecture).map_err(anyhow::Error::msg)?;
        native.membership = RoomMembership::constructed(&world);
        let mut vm = Vm::new(world, 17);
        vm.tick = 1;
        vm.native_accessory_host = true;
        vm.lot_rng_state = Some(1825128472);
        vm.crt_rng_state = Some(23);
        let mut actor_thread = None;
        for (object, bhav) in [(1, 4096), (2, 4098)] {
            let thread = vm
                .spawn(
                    &session.program,
                    Start {
                        owner: "objects",
                        bhav,
                        caller: object,
                        callee: object,
                        stack_object: 2,
                        args: vec![0; 4],
                        check: false,
                    },
                )
                .map_err(anyhow::Error::msg)?;
            vm.register_object_thread(object, thread)
                .map_err(anyhow::Error::msg)?;
            if object == 1 {
                actor_thread = Some(thread);
            }
        }
        let clock = session.simulation.household.as_ref().unwrap().clock.clone();
        let environment = session
            .simulation
            .room_environment
            .as_ref()
            .unwrap()
            .manager
            .settings;
        session.simulation =
            Simulation::from_loaded_lot(vm, architecture, native).map_err(anyhow::Error::msg)?;
        install_root_host(&mut session)?;
        session
            .simulation
            .enable_household(&session.program, clock)
            .map_err(anyhow::Error::msg)?;
        session
            .simulation
            .enable_original_environment(&session.program, environment)
            .map_err(anyhow::Error::msg)?;
        session
            .simulation
            .enable_registry_scheduler(0)
            .map_err(anyhow::Error::msg)?;
        let (serial, inserted) = session
            .simulation
            .vm
            .enqueue_action(
                &session.program,
                1,
                action::Action {
                    target: 2,
                    icon_owner: 2,
                    menu_id: 19,
                    priority: 100,
                    routine: 4097,
                    ..Default::default()
                },
            )
            .map_err(anyhow::Error::msg)?;
        assert!(inserted);
        assert_eq!(session.host.characters.len(), 1);
        assert_eq!(session.simulation.characters.len(), 1);
        assert_eq!(
            session.simulation.household.as_ref().unwrap().people.len(),
            1
        );
        assert!(session.program.animations.is_empty());
        assert!(session.simulation.bodies.contains_key(&1));
        assert!(session.simulation.bodies.contains_key(&2));
        assert!(session.simulation.native_character_settings.is_some());
        assert!(session.simulation.interactions.is_empty());
        Ok((session, actor_thread.unwrap(), serial))
    }

    fn assert_action_facts(
        initial: &Session,
        thread: u64,
        serial: u32,
        events: &[SimExecutionEvent],
    ) {
        let storage = initial
            .simulation
            .vm
            .world
            .scheduler_storage
            .as_ref()
            .unwrap();
        let actor_handle = storage.handle(1).unwrap();
        let target_handle = storage.handle(2).unwrap();
        let identity = |frame: &FrameFingerprint| {
            frame.thread == Some(thread)
                && frame.frame.location.source == "objects"
                && frame.frame.location.bhav == 4097
                && frame.frame.code_owner == "objects"
                && frame.caller.id == Some(1)
                && frame.caller.handle == Evidence::Known(actor_handle)
                && frame.callee.id == Some(2)
                && frame.callee.handle == Evidence::Known(target_handle)
                && frame.frame.action_target == Some(2)
                && frame.retained_lifetime_binding == Unavailable::NativeRecordHasNoLifetimeBinding
        };
        let bound = events
            .iter()
            .position(|event| {
                matches!(event,
                    SimExecutionEvent::Vm(Fact::FrameBound { frame, action })
                        if identity(frame)
                            && action.actor.id == Some(1)
                            && action.actor.handle == Evidence::Known(actor_handle)
                            && action.target.id == Some(2)
                            && action.target.handle == Evidence::Known(target_handle)
                            && action.serial == serial
                            && action.menu_id == 19
                            && action.routine == 4097
                            && action.retained_lifetime_binding
                                == Unavailable::NativeRecordHasNoLifetimeBinding
                )
            })
            .expect("real registry action frame must be bound");
        let returned = events.iter().position(|event| matches!(event,
            SimExecutionEvent::Vm(Fact::Returned { frame, branch, binding, .. })
                if identity(frame)
                    && *branch == NativeReturn::TrueFe
                    && *binding == Evidence::Unavailable(Unavailable::FrameHasNoSerialBinding)
        )).expect("actual action frame must return without acquiring a serial binding");
        assert!(bound < returned);
        let gap = |owner| {
            events
                .iter()
                .position(|event| {
                    matches!(event,
                        SimExecutionEvent::Gap { owner: actual } if *actual == owner
                    )
                })
                .unwrap()
        };
        assert!(gap("Host::update_contexts") < bound);
        assert!(returned < gap("Simulation::finish_registry_pass"));
        assert!(
            gap("Simulation::finish_registry_pass")
                < gap("Simulation::update_original_environment")
        );
    }

    #[test]
    fn session_step_identifies_scheduled_action_frames_without_terminal_binding() -> Result<()> {
        let (initial, thread, serial) = fixture()?;
        let mut ordinary = initial.fork_for_exploration()?;
        let mut disabled = initial.fork_for_exploration()?;
        let mut observed = initial.fork_for_exploration()?;
        let mut traced = initial.fork_for_exploration()?;
        let mut traced_observed = initial.fork_for_exploration()?;
        let expected = ordinary.step()?;
        assert!(matches!(
            expected,
            HouseholdProgress::Tick | HouseholdProgress::PartialPass
        ));
        assert_eq!(disabled.step_with_action_observer(&mut |_| {})?, expected);
        let mut events = Vec::new();
        assert_eq!(
            observed.step_with_action_observer(&mut |event| events.push(event))?,
            expected
        );
        let mut expected_trace = Vec::new();
        let mut actual_trace = Vec::new();
        let mut traced_events = Vec::new();
        assert_eq!(traced.step_with_trace(&mut expected_trace)?, expected);
        assert_eq!(
            traced_observed
                .step_with_trace_and_action_observer(&mut actual_trace, &mut |event| {
                    traced_events.push(event)
                },)?,
            expected
        );
        assert_eq!(actual_trace, expected_trace);
        assert_eq!(events, traced_events);
        assert!(
            actual_trace
                .iter()
                .any(|entry| { entry.thread == thread && entry.location.bhav == 4097 })
        );
        let expected_state = state(&ordinary)?;
        for actual in [&ordinary, &disabled, &observed, &traced, &traced_observed] {
            assert_eq!(state(actual)?, expected_state);
            let resident = &actual.simulation.characters[&1];
            assert_eq!(resident.evaluated_tick, initial.simulation.vm.tick);
            assert!(resident.ordinary_update.is_none());
            assert!(resident.person_update.is_none());
            assert!(!resident.character.busy());
            assert_eq!(
                actual.simulation.household.as_ref().unwrap().people[&1].finished,
                Some(initial.simulation.vm.tick)
            );
        }
        assert_action_facts(&initial, thread, serial, &events);
        Ok(())
    }

    #[test]
    fn elapsed_update_lot_identifies_action_frames_and_acknowledges_one_child() -> Result<()> {
        let (mut initial, thread, serial) = fixture()?;
        attach_authored_clock(&mut initial)?;
        let mut ordinary = initial.fork_for_exploration()?;
        let mut disabled = initial.fork_for_exploration()?;
        let mut observed = initial.fork_for_exploration()?;
        let mut traced = initial.fork_for_exploration()?;
        let mut traced_observed = initial.fork_for_exploration()?;
        let expected = ordinary.advance_clock(100)?;
        assert!(matches!(
            expected,
            clock::Progress::Household(HouseholdProgress::Tick | HouseholdProgress::PartialPass)
        ));
        assert_eq!(
            disabled.advance_clock_with_action_observer(100, &mut |_| {})?,
            expected
        );
        let mut events = Vec::new();
        assert_eq!(
            observed.advance_clock_with_action_observer(100, &mut |event| events.push(event))?,
            expected
        );
        let mut expected_trace = Vec::new();
        let mut actual_trace = Vec::new();
        let mut traced_events = Vec::new();
        assert_eq!(
            traced.advance_clock_with_trace(100, &mut expected_trace)?,
            expected
        );
        assert_eq!(
            traced_observed.advance_clock_with_trace_and_action_observer(
                100,
                &mut actual_trace,
                &mut |event| traced_events.push(event),
            )?,
            expected
        );
        assert_eq!(actual_trace, expected_trace);
        assert_eq!(events, traced_events);
        assert!(
            actual_trace
                .iter()
                .any(|entry| { entry.thread == thread && entry.location.bhav == 4097 })
        );
        let expected_state = state(&ordinary)?;
        for actual in [&ordinary, &disabled, &observed, &traced, &traced_observed] {
            assert_eq!(state(actual)?, expected_state);
            let resident = &actual.simulation.characters[&1];
            assert_eq!(resident.evaluated_tick, initial.simulation.vm.tick);
            assert!(resident.ordinary_update.is_none());
            assert!(resident.person_update.is_none());
            assert!(!resident.character.busy());
            assert_eq!(
                actual.simulation.household.as_ref().unwrap().people[&1].finished,
                Some(initial.simulation.vm.tick)
            );
        }
        assert!(matches!(
            events.first(),
            Some(SimExecutionEvent::Gap {
                owner: "clock::UpdateLot"
            })
        ));
        let call = observed
            .host
            .presentation
            .clock
            .as_ref()
            .unwrap()
            .call
            .as_ref()
            .unwrap();
        assert_eq!(call.updates, 1);
        assert_eq!(call.tick, observed.simulation.vm.tick);
        assert!(call.frame.at_completed_update());
        assert_ne!(call.frame.request(), Some(native_clock::Request::UpdateLot));
        assert_action_facts(&initial, thread, serial, &events);
        Ok(())
    }
}
