//! Authored registry-service controls, not native payload or restored-continuation qualification.
use super::*;
use sims_formats::behavior::{Behavior, Instruction};
use sims_sim::{
    ServiceCut, Snapshot,
    architecture::Architecture,
    lot_geometry::{FloorSource, Lot},
    lot_links::Links,
    lot_placement,
    lot_runtime::State as LotState,
    service_observation::{CreationMoment, CreationPhase, ObjectLifetime},
};
use sims_vm::{
    Definition, Module, Object, Routine, Status, autonomy::Placement, person::WorldState,
    person_loading::RoomMembership,
};

mod poll_tests;

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

fn assert_creation(cut: ServiceCut, session: &Session) {
    let ServiceCut::Creation {
        moment,
        phase,
        thread,
        request,
        object,
    } = cut
    else {
        panic!("unexpected authored cut: {cut:?}");
    };
    assert_eq!(moment, CreationMoment::Registered);
    assert_eq!(phase, CreationPhase::Factory);
    assert_eq!(
        object,
        ObjectLifetime {
            object: 5,
            object_thread: session.simulation.vm.object_threads.get(&5).copied(),
        }
    );
    assert!(matches!(&session.simulation.vm.threads[&thread].status,
        Status::AwaitingCreation(owner) if owner.id == request && owner.created == Some(5)));
    session
        .simulation
        .native_scheduler
        .as_ref()
        .unwrap()
        .validate(&session.simulation)
        .unwrap();
}

#[test]
fn actual_registry_cut_preserves_full_saves_trace_and_native_rng_execution() -> Result<()> {
    let initial = registry_session()?;
    let mut reference = initial.fork_for_exploration()?;
    let mut expected = vec![];
    let ordinary_envelope = reference.save();
    let managed_envelope = reference.save_managed()?;
    let program = &reference.program;
    reference
        .simulation
        .run_current_tick_with_service_observer(program, 100_000, &mut |cut, simulation| {
            let mut ordinary = ordinary_envelope.clone();
            let mut managed = managed_envelope.clone();
            ordinary.simulation = Snapshot::capture(program, simulation);
            managed.simulation = ordinary.simulation.clone();
            expected.push((cut, bytes(&ordinary).unwrap(), bytes(&managed).unwrap()));
            Ok(())
        })
        .map_err(anyhow::Error::msg)?;
    let mut production = initial.fork_for_exploration()?;
    let mut observed = initial.fork_for_exploration()?;
    let mut ordinary_trace = vec![];
    let mut observed_trace = vec![];
    let ordinary_result = production.step_with_trace(&mut ordinary_trace)?;
    let mut captures = vec![];
    let observed_result =
        observed.step_with_trace_and_service_observer(&mut observed_trace, &mut |cut, view| {
            assert_creation(cut, view);
            let before = state(view)?;
            captures.push((cut, bytes(&view.save())?, bytes(&view.save_managed()?)?));
            assert_eq!(state(view)?, before);
            Ok(())
        })?;
    assert_eq!(observed_result, ordinary_result);
    assert_eq!(observed_trace, ordinary_trace);
    assert!(!ordinary_trace.is_empty());
    assert_eq!(captures, expected);
    assert_eq!(captures.len(), 1);
    assert_ne!(captures[0].1, bytes(&observed.save())?);
    assert_eq!(state(&observed)?, state(&production)?);
    assert_eq!(
        observed.simulation.vm.rng_state,
        initial.simulation.vm.rng_state
    );
    assert_ne!(
        observed.simulation.vm.lot_rng_state,
        initial.simulation.vm.lot_rng_state
    );
    assert_eq!(
        observed.simulation.vm.crt_rng_state,
        initial.simulation.vm.crt_rng_state
    );
    let mut dependent = initial.fork_for_exploration()?;
    let mut dependent_trace = vec![];
    let mut dependent_calls = 0;
    assert_eq!(
        dependent.step_with_dependent_observer(&mut dependent_trace, &mut |_| {
            dependent_calls += 1;
            Ok(())
        })?,
        ordinary_result
    );
    assert_eq!(dependent_calls, 0);
    assert_eq!(dependent_trace, ordinary_trace);
    assert_eq!(state(&dependent)?, state(&production)?);
    let mut untraced = initial.fork_for_exploration()?;
    let mut calls = 0;
    assert_eq!(
        untraced.step_with_service_observer(&mut |cut, view| {
            assert_creation(cut, view);
            calls += 1;
            Ok(())
        })?,
        ordinary_result
    );
    assert_eq!(calls, 1);
    assert_eq!(state(&untraced)?, state(&production)?);
    Ok(())
}

#[derive(Debug)]
struct CallbackFailure(Arc<()>);

impl std::fmt::Display for CallbackFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("authored service observer failure")
    }
}

impl std::error::Error for CallbackFailure {}

#[test]
fn callback_failure_retains_anyhow_identity_chain_and_exact_service_owner() -> Result<()> {
    let mut observed = registry_session()?;
    let mut expected = observed.fork_for_exploration()?;
    expected
        .simulation
        .run_current_tick_with_service_observer(&expected.program, 100_000, &mut |_, _| {
            Err("stop at the actual service cut".into())
        })
        .unwrap_err();
    let identity = Arc::new(());
    let mut calls = 0;
    let error = observed
        .step_with_service_observer(&mut |cut, view| {
            assert_creation(cut, view);
            calls += 1;
            Err(anyhow::Error::new(CallbackFailure(Arc::clone(&identity)))
                .context("retained callback context"))
        })
        .unwrap_err();
    assert_eq!(calls, 1);
    assert_eq!(error.to_string(), "retained callback context");
    assert!(format!("{error:#}").contains("authored service observer failure"));
    assert!(Arc::ptr_eq(
        &error.downcast_ref::<CallbackFailure>().unwrap().0,
        &identity
    ));
    assert_eq!(state(&observed)?, state(&expected)?);
    Ok(())
}

fn retained_clock_session() -> Result<Session> {
    let mut session = registry_session()?;
    application_parent(&mut session)?;
    let clock = session.host.presentation.clock.as_mut().unwrap();
    clock.fields.timing.elapsed = 60;
    clock.fields.timer.interval = 30;
    clock.fields.timer.limit = 30;
    Ok(session)
}

#[test]
fn actual_service_cut_keeps_retained_clock_and_application_parent_without_retiming() -> Result<()> {
    let initial = retained_clock_session()?;
    let mut production = initial.fork_for_exploration()?;
    let mut observed = initial.fork_for_exploration()?;
    let mut ordinary_trace = vec![];
    let mut observed_trace = vec![];
    let expected = production.advance_clock_with_trace(32, &mut ordinary_trace)?;
    let mut calls = 0;
    let actual = observed.advance_clock_with_trace_and_service_observer(
        32,
        &mut observed_trace,
        &mut |cut, view| {
            assert_creation(cut, view);
            calls += 1;
            let parent = view.host.presentation.application_parent.as_ref().unwrap();
            let clock = view.host.presentation.clock.as_ref().unwrap();
            parent.validate(Some(clock), view.simulation.vm.tick)?;
            clock.validate(view)?;
            assert!(parent.awaiting_elapsed());
            assert_eq!(view.simulation.vm.tick, 88448);
            assert_eq!(
                bytes(&view.save())?,
                bytes(&view.fork_for_exploration()?.save())?
            );
            view.save_managed()?;
            Ok(())
        },
    )?;
    assert_eq!(calls, 1);
    assert_eq!(actual, expected);
    assert!(matches!(actual, clock::Progress::Household(_)));
    assert_eq!(observed_trace, ordinary_trace);
    assert_eq!(state(&observed)?, state(&production)?);
    observed
        .host
        .presentation
        .clock
        .as_ref()
        .unwrap()
        .validate(&observed)?;
    let mut untraced = initial.fork_for_exploration()?;
    let mut calls = 0;
    assert_eq!(
        untraced.advance_clock_with_service_observer(32, &mut |cut, view| {
            assert_creation(cut, view);
            calls += 1;
            Ok(())
        })?,
        expected
    );
    assert_eq!(calls, 1);
    assert_eq!(state(&untraced)?, state(&production)?);
    Ok(())
}

#[test]
fn retained_clock_callback_error_does_not_acknowledge_its_household_return() -> Result<()> {
    let mut observed = retained_clock_session()?;
    let identity = Arc::new(());
    let mut at_cut = None;
    let error = observed
        .advance_clock_with_service_observer(32, &mut |cut, view| {
            assert_creation(cut, view);
            at_cut = Some(state(view)?);
            Err(anyhow::Error::new(CallbackFailure(Arc::clone(&identity))))
        })
        .unwrap_err();
    assert!(Arc::ptr_eq(
        &error.downcast_ref::<CallbackFailure>().unwrap().0,
        &identity
    ));
    assert_eq!(state(&observed)?, at_cut.unwrap());
    Ok(())
}

#[test]
fn observed_entries_reject_wrong_clock_epoch_or_application_owner_without_callbacks() -> Result<()>
{
    let mut wrong_epoch = retained_clock_session()?;
    wrong_epoch.simulation.vm.tick += 1;
    let mut wrong_owner = retained_clock_session()?;
    wrong_owner
        .host
        .presentation
        .clock
        .as_mut()
        .unwrap()
        .captured_code = Some(clock::CapturedCode::Goth89073DormantProtectedPoll20261031);
    for (mut session, message) in [
        (wrong_epoch, "unacknowledged lot tick"),
        (wrong_owner, "source profiles differ"),
    ] {
        let before = state(&session)?;
        let mut calls = 0;
        let error = session
            .advance_clock_with_service_observer(32, &mut |_, _| {
                calls += 1;
                Ok(())
            })
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error:#}");
        assert_eq!(calls, 0);
        assert_eq!(state(&session)?, before);
    }
    let mut session = retained_clock_session()?;
    let before = state(&session)?;
    let mut calls = 0;
    assert!(
        session
            .step_with_service_observer(&mut |_, _| {
                calls += 1;
                Ok(())
            })
            .unwrap_err()
            .to_string()
            .contains("application parent owns")
    );
    assert_eq!(calls, 0);
    assert_eq!(state(&session)?, before);
    Ok(())
}

#[test]
fn actual_cut_rejects_a_foreign_simulation_slot_before_invoking_the_runtime_observer() -> Result<()>
{
    let mut session = registry_session()?;
    let mut calls = 0;
    with_view(
        &mut session,
        &mut |cut, view| {
            assert_creation(cut, view);
            calls += 1;
            Ok(())
        },
        |program, simulation, observe| {
            simulation
                .run_current_tick_with_service_observer(program, 100_000, &mut |cut, simulation| {
                    let foreign = simulation.clone();
                    let error = observe(cut, &foreign).unwrap_err();
                    assert!(error.to_string().contains("does not belong"));
                    observe(cut, simulation).map_err(|error| format!("{error:#}"))
                })
                .map_err(anyhow::Error::msg)
        },
    )?;
    assert_eq!(calls, 1);
    Ok(())
}

#[test]
fn actual_cut_propagates_existing_external_source_rejection_without_running_later_services()
-> Result<()> {
    let mut session = registry_session()?;
    session.managed_policy = managed::Policy::CompletedOwnedSchema2;
    let source = ExternalSource::new(b"authored behavior owner")?;
    let mut external = resource("objects", b"authored behavior owner");
    external.owned = None;
    external.source = source.path();
    Arc::make_mut(&mut session.index).resources.push(external);
    session
        .program
        .modules
        .get_mut("objects")
        .unwrap()
        .content_hash = digest(b"authored behavior owner");
    session.save_managed()?;
    std::fs::write(source.path(), b"modified behavior owner")?;
    let expected_error = session.save_managed().unwrap_err().to_string();
    assert!(expected_error.contains("current source version differs"));
    let mut at_cut = None;
    let error = session
        .step_with_service_observer(&mut |cut, view| {
            assert_creation(cut, view);
            at_cut = Some(state(view)?);
            view.save_managed()?;
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error.to_string(), expected_error);
    assert_eq!(state(&session)?, at_cut.unwrap());
    assert!(session.index.resources.last().unwrap().owned.is_none());
    Ok(())
}
