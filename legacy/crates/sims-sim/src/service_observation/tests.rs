use super::*;
use crate::{Snapshot, StepStop};
use sims_formats::behavior::Instruction;
use sims_vm::{Program, Status};

fn instruction(opcode: u16, operands: [u8; 8], next: u8) -> Instruction {
    Instruction {
        opcode,
        operands,
        on_true: next,
        on_false: 255,
    }
}

fn returned() -> Instruction {
    instruction(2, [0, 0, 0, 0, 0, 5, 8, 8], 254)
}

fn create(guid: u8, mode: u8, next: u8) -> Instruction {
    instruction(42, [guid, 0, 0, 0, mode, 0, 0, 0], next)
}

fn fixture(instructions: Vec<Instruction>) -> (Program, Simulation) {
    let (mut program, mut simulation) = crate::lot_runtime::tests::fixture();
    let module = program.modules.get_mut("objects").unwrap();
    let mut main = module.routines[&4096].clone();
    main.body.instructions = vec![returned()];
    module.routines.insert(4097, main);
    module.routines.get_mut(&4096).unwrap().body.instructions = instructions;
    let mut words = module.object_definition_words[&1].clone();
    words[14] = 2;
    module.object_definition_words.insert(2, words);
    let mut functions = module.object_functions[&1].clone();
    functions[1].action = 4097;
    module.object_functions.insert(2, functions);
    let mut words = module.object_definition_words[&1].clone();
    words[14] = 4;
    module.object_definition_words.insert(4, words);
    let mut functions = module.object_functions[&1].clone();
    functions[1].action = 4110;
    module.object_functions.insert(4, functions);
    let mut idle = module.routines[&4097].clone();
    idle.body.instructions = vec![returned()];
    module.routines.insert(4110, idle);
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
    for object in 1..=3 {
        let definition = simulation
            .vm
            .world
            .objects
            .get_mut(&object)
            .unwrap()
            .definition
            .as_mut()
            .unwrap();
        definition.object = 4;
        definition.guid = 4;
        simulation
            .vm
            .start_object_lifecycle(&program, object)
            .unwrap();
    }
    simulation.vm.world.globals.insert(34, 0);
    simulation.vm.world.refresh = Some(Default::default());
    let architecture = simulation.architecture.as_mut().unwrap();
    architecture.bind_footprints(&program).unwrap();
    architecture.publish_room_context(&mut simulation.vm.world);
    simulation
        .enable_household(
            &program,
            crate::clock::LotClock {
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
        .unwrap();
    select(&mut simulation, 1);
    (program, simulation)
}

fn callback(program: &mut Program, function: usize, instructions: Vec<Instruction>) {
    let module = program.modules.get_mut("objects").unwrap();
    let mut routine = module.routines[&4097].clone();
    routine.body.instructions = instructions;
    module.routines.insert(4100, routine);
    module.object_functions.get_mut(&4).unwrap()[function].action = 4100;
}

fn select(simulation: &mut Simulation, object: i16) {
    let thread = simulation.vm.object_threads[&4];
    simulation
        .vm
        .threads
        .get_mut(&thread)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .stack_object = object;
}

fn assert_cut(cut: ServiceCut, simulation: &Simulation) {
    match cut {
        ServiceCut::Creation {
            thread,
            request,
            object,
            phase,
            ..
        } => {
            assert!(simulation.vm.world.objects.contains_key(&object.object));
            assert_eq!(object, ObjectLifetime::at(simulation, object.object));
            assert!(
                matches!(&simulation.vm.threads[&thread].status, Status::AwaitingCreation(owner) if owner.id == request && owner.created == Some(object.object))
            );
            if simulation.native_creations.contains_key(&request) {
                assert_eq!(
                    simulation.native_creations[&request].binding(),
                    (request, object.object)
                );
            } else {
                assert_eq!(phase, CreationPhase::Factory);
            }
        }
        ServiceCut::FindLocation {
            thread, request, ..
        } => {
            let owner = simulation
                .native_lot
                .as_ref()
                .unwrap()
                .find_location
                .as_ref()
                .unwrap();
            assert_eq!(owner.binding().0, thread);
            assert_eq!(owner.binding().1.id, request);
        }
        ServiceCut::AwaitingAnimation {
            thread, request, ..
        } => {
            assert!(
                matches!(&simulation.vm.threads[&thread].status, Status::AwaitingAnimation(owner) if owner.id == request)
            );
        }
        ServiceCut::PrimitiveRemoval {
            thread, request, ..
        } => {
            let owner = simulation
                .native_lot
                .as_ref()
                .unwrap()
                .removal
                .as_ref()
                .unwrap();
            assert!(
                matches!(&owner.origin, crate::lot_removal::Origin::Primitive { thread: owner_thread, request: owner_request, .. } if *owner_thread == thread && owner_request.id == request)
            );
        }
        ServiceCut::QueuedCleanup {
            index,
            object,
            suppress_reset,
        } => {
            assert_eq!(simulation.removals[index].target, object.object);
            assert_eq!(simulation.removals[index].suppress_reset, suppress_reset);
        }
    }
}

fn finish(program: &Program, simulation: &mut Simulation, budget: usize) {
    for _ in 0..64 {
        if simulation.run_fixture_tick(program, budget).unwrap().stop != StepStop::Budget {
            return;
        }
    }
    panic!("authored fixture did not finish");
}

fn different_paths(
    before: &serde_json::Value,
    after: &serde_json::Value,
    path: String,
    paths: &mut Vec<String>,
) {
    if before == after {
        return;
    }
    match (before, after) {
        (serde_json::Value::Object(left), serde_json::Value::Object(right))
            if left.keys().eq(right.keys()) =>
        {
            for (key, value) in left {
                different_paths(value, &right[key], format!("{path}/{key}"), paths);
            }
        }
        (serde_json::Value::Array(left), serde_json::Value::Array(right))
            if left.len() == right.len() =>
        {
            for (index, (left, right)) in left.iter().zip(right).enumerate() {
                different_paths(left, right, format!("{path}/{index}"), paths);
            }
        }
        _ => paths.push(format!("{path}: {before} != {after}")),
    }
}

fn compare(program: &Program, initial: &Simulation, budget: usize) -> Vec<(ServiceCut, Vec<u8>)> {
    compare_replay(program, initial, budget, |_| true)
}

fn compare_replay(
    program: &Program,
    initial: &Simulation,
    budget: usize,
    replay_cut: fn(ServiceCut) -> bool,
) -> Vec<(ServiceCut, Vec<u8>)> {
    let mut observed = initial.clone();
    let mut ordinary = initial.clone();
    let mut cuts = vec![];
    for _ in 0..64 {
        let expected = ordinary.run_fixture_tick(program, budget).unwrap();
        let actual = observed
            .run_fixture_tick_with_service_observer(program, budget, &mut |cut, simulation| {
                assert_cut(cut, simulation);
                let bytes = serde_json::to_vec(&Snapshot::capture(program, simulation)).unwrap();
                let restored = Snapshot::from_json(&bytes, program)?;
                assert_eq!(&restored, simulation);
                cuts.push((cut, bytes));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            actual, expected,
            "trace, events and stop reason at budget {budget}"
        );
        assert_eq!(observed.vm.rng_state, ordinary.vm.rng_state);
        assert_eq!(observed.vm.lot_rng_state, ordinary.vm.lot_rng_state);
        assert_eq!(observed.vm.crt_rng_state, ordinary.vm.crt_rng_state);
        assert_eq!(observed, ordinary);
        if actual.stop != StepStop::Budget {
            for (cut, bytes) in &cuts {
                if !replay_cut(*cut) {
                    continue;
                }
                let mut replay = Snapshot::from_json(bytes, program).unwrap();
                finish(program, &mut replay, budget);
                let mut differences = vec![];
                different_paths(
                    &serde_json::to_value(&replay).unwrap(),
                    &serde_json::to_value(&ordinary).unwrap(),
                    String::new(),
                    &mut differences,
                );
                assert!(
                    differences.is_empty(),
                    "replay from {cut:?}: {differences:?}"
                );
            }
            return cuts;
        }
    }
    panic!("authored fixture did not finish");
}

#[test]
fn same_call_registration_is_observed_without_yielding_or_changing_execution() {
    let (program, initial) = fixture(vec![create(2, 6, 254)]);
    for budget in [1, 3, 17, 1000] {
        let cuts = compare(&program, &initial, budget);
        assert!(matches!(
            cuts[0].0,
            ServiceCut::Creation {
                moment: CreationMoment::Registered,
                phase: CreationPhase::Factory,
                object: ObjectLifetime { object: 5, .. },
                ..
            }
        ));
        if budget == 1000 {
            assert_eq!(cuts.len(), 1);
            let mut ordinary = initial.clone();
            assert_ne!(
                ordinary.run_fixture_tick(&program, budget).unwrap().stop,
                StepStop::Budget
            );
            assert!(
                !ordinary
                    .vm
                    .threads
                    .values()
                    .any(|thread| matches!(thread.status, Status::AwaitingCreation(_)))
            );
        }
    }
}

#[test]
fn rotation_installation_is_distinct_from_actual_registration() {
    let (program, initial) = fixture(vec![instruction(42, [2, 0, 0, 0, 6, 32, 0, 0], 254)]);
    for budget in [1, 17, 1000] {
        let cuts = compare(&program, &initial, budget);
        assert_eq!(
            cuts.iter()
                .filter(|(cut, _)| matches!(
                    cut,
                    ServiceCut::Creation {
                        moment: CreationMoment::Registered,
                        ..
                    }
                ))
                .count(),
            1
        );
        assert_eq!(
            cuts.iter()
                .filter(|(cut, _)| matches!(
                    cut,
                    ServiceCut::Creation {
                        moment: CreationMoment::RotationInstalled,
                        phase: CreationPhase::RotateOffLot,
                        ..
                    }
                ))
                .count(),
            1
        );
    }
}

#[test]
fn ordinary_registry_observation_preserves_rng_polls_and_replays_active_owners() {
    let random = |next| instruction(8, [0, 0, 8, 0, 19, 0, 7, 0], next);
    let (program, mut initial) = fixture(vec![random(1), create(2, 6, 2), random(254)]);
    initial.vm.world.scheduler_storage = Some(Default::default());
    initial.vm.world.globals.extend([(10, 1), (17, 0), (19, 2)]);
    for object in 1..=4 {
        let mut owner = initial.vm.world.objects.remove(&object).unwrap();
        owner.native_tree_error = Some(0);
        owner.native_flags = Some(0x240);
        initial
            .vm
            .world
            .register_scheduler_object(object, &mut owner)
            .unwrap();
        owner.idle.as_mut().unwrap().flags = 3;
        initial.vm.world.objects.insert(object, owner);
    }
    initial.vm.rng_state = 17;
    initial.vm.lot_rng_state = Some(1825128472);
    initial.vm.crt_rng_state = Some(23);
    initial.enable_registry_scheduler(0).unwrap();
    for budget in [1, 7, 1000] {
        let mut observed = initial.clone();
        let mut ordinary = initial.clone();
        let mut cuts = vec![];
        let mut completed = false;
        for _ in 0..64 {
            let mut ordinary_polls = vec![];
            let mut observed_polls = vec![];
            let expected = ordinary
                .run_current_tick_with_host_polls(&program, budget, &mut ordinary_polls)
                .unwrap();
            let actual = observed
                .run_current_tick_with_host_polls_and_service_observer(
                    &program,
                    budget,
                    Some(&mut observed_polls),
                    &mut |cut, simulation| {
                        assert_cut(cut, simulation);
                        let bytes =
                            serde_json::to_vec(&Snapshot::capture(&program, simulation)).unwrap();
                        assert_eq!(Snapshot::from_json(&bytes, &program)?, *simulation);
                        cuts.push(bytes);
                        Ok(())
                    },
                )
                .unwrap();
            assert_eq!(actual, expected);
            assert_eq!(observed, ordinary);
            let reports = |polls: Vec<HostPollTrace>| {
                polls
                    .into_iter()
                    .map(|poll| (poll.before_instruction, poll.instruction))
                    .collect::<Vec<_>>()
            };
            assert_eq!(reports(observed_polls), reports(ordinary_polls));
            if actual.stop != StepStop::Budget {
                completed = true;
                break;
            }
        }
        assert!(completed);
        assert_eq!(observed.vm.rng_state, 17);
        assert_ne!(observed.vm.lot_rng_state, initial.vm.lot_rng_state);
        assert!(!cuts.is_empty());
        for bytes in cuts {
            let mut replay = Snapshot::from_json(&bytes, &program).unwrap();
            let mut completed = false;
            for _ in 0..64 {
                if replay.run_current_tick(&program, budget).unwrap().stop != StepStop::Budget {
                    completed = true;
                    break;
                }
            }
            assert!(completed);
            assert_eq!(replay, ordinary);
        }
    }
}

#[test]
fn placed_creation_observes_only_retained_registration_and_reentry() {
    let (mut program, initial) = fixture(vec![create(2, 3, 254)]);
    let module = program.modules.get_mut("objects").unwrap();
    module.routines.get_mut(&4097).unwrap().body.instructions = vec![returned(); 4];
    for budget in [1, 7, 1000] {
        let cuts = compare(&program, &initial, budget);
        assert!(cuts.iter().any(|(cut, bytes)| {
            let ServiceCut::Creation {
                request,
                moment: CreationMoment::Registered,
                ..
            } = cut
            else {
                return false;
            };
            Snapshot::from_json(bytes, &program)
                .unwrap()
                .native_creations
                .contains_key(request)
        }));
        if budget == 1 {
            assert!(cuts.iter().any(|(cut, _)| matches!(
                cut,
                ServiceCut::Creation {
                    moment: CreationMoment::Reentry,
                    ..
                }
            )));
        }
    }
}

#[test]
fn native_find_location_installation_and_reentry_are_retained() {
    let (mut program, mut initial) =
        fixture(vec![instruction(0x10, [1, 0, 2, 0, 0, 0, 0, 0], 254)]);
    select(&mut initial, 1);
    callback(
        &mut program,
        10,
        vec![instruction(2, [34, 0, 1, 0, 0, 3, 6, 7], 1), returned()],
    );
    for budget in [1, 7, 1000] {
        let cuts = compare(&program, &initial, budget);
        assert!(cuts.iter().any(|(cut, _)| matches!(
            cut,
            ServiceCut::FindLocation {
                moment: ContinuationMoment::Installed,
                phase: FindLocationPhase::Detach,
                ..
            }
        )));
        if budget == 1 {
            assert!(cuts.iter().any(|(cut, _)| matches!(
                cut,
                ServiceCut::FindLocation {
                    moment: ContinuationMoment::Reentry,
                    ..
                }
            )));
        }
    }
}

#[test]
fn native_find_location_search_is_not_reported_as_detachment() {
    let (mut program, mut initial) =
        fixture(vec![instruction(0x10, [0, 0, 1, 0, 0, 0, 0, 0], 254)]);
    program
        .modules
        .get_mut("objects")
        .unwrap()
        .routines
        .get_mut(&4096)
        .unwrap()
        .body
        .locals = 1;
    let thread = initial
        .vm
        .reset_object_lifecycle(
            &program,
            4,
            sims_vm::construction::ObjectDate {
                year: 1997,
                month: 1,
                day: 1,
            },
        )
        .unwrap();
    initial
        .vm
        .threads
        .get_mut(&thread)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .locals[0] = 1;
    select(&mut initial, 1);
    for budget in [1, 17, 1000] {
        let cuts = compare(&program, &initial, budget);
        assert!(cuts.iter().any(|(cut, _)| matches!(
            cut,
            ServiceCut::FindLocation {
                moment: ContinuationMoment::Installed,
                phase: FindLocationPhase::Search,
                ..
            }
        )));
    }
}

#[test]
fn multipart_registration_retains_the_complete_group_factory() {
    let (mut program, mut initial) = fixture(vec![create(100, 6, 254)]);
    let module = program.modules.get_mut("objects").unwrap();
    for (number, index) in [(100, -1), (101, 0), (102, 1)] {
        let mut words = module.object_definition_words[&2].clone();
        words[10] = 7;
        words[11] = index;
        words[14] = number as i16;
        words[18] = 50;
        words[63] = i16::from(index == 0);
        module.object_definition_words.insert(number, words);
        module.object_definition_order.push(number);
        module
            .object_functions
            .insert(number, module.object_functions[&2].clone());
    }
    initial
        .architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&program)
        .unwrap();
    initial
        .vm
        .world
        .globals
        .extend([(15, 0), (32, 0), (33, 0), (36, 0)]);
    for budget in [1, 17, 1000] {
        let cuts = compare(&program, &initial, budget);
        assert_eq!(
            cuts.iter()
                .filter(|(cut, _)| matches!(
                    cut,
                    ServiceCut::Creation {
                        moment: CreationMoment::Registered,
                        phase: CreationPhase::GroupFactory,
                        ..
                    }
                ))
                .count(),
            1
        );
    }
}

#[test]
fn simulation_roundtrip_does_not_claim_full_save_envelope_validation() {
    let mut accepted_omissions = vec![];
    for operation in [
        create(2, 3, 254),
        instruction(0x10, [1, 0, 2, 0, 0, 0, 0, 0], 254),
        instruction(0x12, [1, 0, 0, 0, 0, 0, 0, 0], 254),
    ] {
        let (program, mut simulation) = fixture(vec![operation]);
        let mut calls = 0;
        let error = simulation
            .run_fixture_tick_with_service_observer(&program, 1000, &mut |cut, state| {
                calls += 1;
                let mut saved = serde_json::to_value(Snapshot::capture(&program, state)).unwrap();
                match cut {
                    ServiceCut::Creation { .. } => {
                        saved["state"]["native_creations"] = serde_json::json!({})
                    }
                    ServiceCut::FindLocation { .. } => {
                        saved["state"]["native_lot"]["find_location"] = serde_json::Value::Null
                    }
                    ServiceCut::PrimitiveRemoval { .. } => {
                        saved["state"]["native_lot"]["removal"] = serde_json::Value::Null
                    }
                    _ => panic!("unexpected cut {cut:?}"),
                }
                assert_cut(cut, state);
                assert_eq!(
                    Snapshot::capture(&program, state).restore(&program)?,
                    *state
                );
                if serde_json::from_value::<Snapshot>(saved)
                    .unwrap()
                    .restore(&program)
                    .is_ok()
                {
                    accepted_omissions.push(cut);
                }
                Err("missing-owner validation exercised".into())
            })
            .unwrap_err();
        assert_eq!(error, "missing-owner validation exercised");
        assert_eq!(calls, 1);
    }
    assert!(!accepted_omissions.is_empty());
}

#[test]
fn native_primitive_removal_and_queue_before_flush_are_observed() {
    let (mut program, mut initial) =
        fixture(vec![instruction(0x12, [1, 0, 0, 0, 0, 0, 0, 0], 254)]);
    select(&mut initial, 1);
    callback(
        &mut program,
        10,
        vec![instruction(2, [34, 0, 1, 0, 0, 3, 6, 7], 1), returned()],
    );
    for budget in [1, 7, 1000] {
        let cuts = compare(&program, &initial, budget);
        assert!(cuts.iter().any(|(cut, _)| matches!(
            cut,
            ServiceCut::PrimitiveRemoval {
                moment: ContinuationMoment::Installed,
                ..
            }
        )));
        assert!(cuts.iter().any(|(cut, _)| matches!(
            cut,
            ServiceCut::QueuedCleanup {
                object: ObjectLifetime { object: 1, .. },
                ..
            }
        )));
        if budget == 1 {
            assert!(cuts.iter().any(|(cut, _)| matches!(
                cut,
                ServiceCut::PrimitiveRemoval {
                    moment: ContinuationMoment::Reentry,
                    ..
                }
            )));
        }
    }
}

#[test]
fn animation_cut_is_a_genuine_resident_request_not_a_synthetic_poll() {
    let (program, initial) = fixture(vec![instruction(44, [0; 8], 254)]);
    for budget in [1, 17, 1000] {
        let cuts = compare(&program, &initial, budget);
        assert_eq!(
            cuts.iter()
                .filter(|(cut, _)| matches!(cut, ServiceCut::AwaitingAnimation { .. }))
                .count(),
            1
        );
    }
}

#[test]
fn callback_errors_preserve_the_exact_failure_and_installed_owner() {
    let (program, mut simulation) = fixture(vec![create(2, 3, 254)]);
    let mut calls = 0;
    let error = simulation
        .run_fixture_tick_with_service_observer(&program, 1000, &mut |cut, state| {
            calls += 1;
            assert_cut(cut, state);
            Err("authored observer failure".into())
        })
        .unwrap_err();
    assert_eq!(error, "authored observer failure");
    assert_eq!(calls, 1);
    assert_eq!(simulation.native_creations.len(), 1);
    let bytes = serde_json::to_vec(&Snapshot::capture(&program, &simulation)).unwrap();
    let mut restored = Snapshot::from_json(&bytes, &program).unwrap();
    finish(&program, &mut restored, 1000);
    finish(&program, &mut simulation, 1000);
    assert_eq!(restored, simulation);
}

#[test]
fn nested_creation_and_removal_are_suppressed_while_creation_owner_is_taken() {
    let (mut program, mut initial) = fixture(vec![create(2, 3, 254)]);
    let module = program.modules.get_mut("objects").unwrap();
    let mut words = module.object_definition_words[&2].clone();
    words[14] = 3;
    module.object_definition_words.insert(3, words);
    let mut functions = module.object_functions[&2].clone();
    functions[1].action = 4098;
    module.object_functions.insert(3, functions);
    let mut routine = module.routines[&4097].clone();
    routine.body.instructions = vec![returned()];
    module.routines.insert(4098, routine);
    module.routines.get_mut(&4097).unwrap().body.instructions = vec![
        create(3, 6, 1),
        instruction(0x12, [1, 0, 0, 0, 0, 0, 0, 0], 254),
    ];
    initial
        .architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&program)
        .unwrap();
    let cuts = compare_replay(&program, &initial, 1000, |cut| {
        !matches!(cut, ServiceCut::QueuedCleanup { .. })
    });
    assert_eq!(
        cuts.iter()
            .filter(|(cut, _)| matches!(cut, ServiceCut::Creation { .. }))
            .count(),
        1
    );
    assert!(
        !cuts
            .iter()
            .any(|(cut, _)| matches!(cut, ServiceCut::PrimitiveRemoval { .. }))
    );
    assert!(cuts.iter().any(|(cut, _)| matches!(
        cut,
        ServiceCut::QueuedCleanup {
            object: ObjectLifetime { object: 6, .. },
            ..
        }
    )));
    let (_, bytes) = cuts
        .iter()
        .find(|(cut, _)| matches!(cut, ServiceCut::QueuedCleanup { .. }))
        .unwrap();
    let mut replay = Snapshot::from_json(bytes, &program).unwrap();
    let mut ordinary = initial;
    finish(&program, &mut replay, 1000);
    finish(&program, &mut ordinary, 1000);
    let mut differences = vec![];
    different_paths(
        &serde_json::to_value(&replay).unwrap(),
        &serde_json::to_value(&ordinary).unwrap(),
        String::new(),
        &mut differences,
    );
    assert_eq!(differences.len(), 1);
    assert!(differences[0].starts_with("/fixture_driver/pass/end:"));
}

#[test]
fn nested_service_status_is_not_observed_while_removal_owner_is_taken() {
    let (mut program, mut initial) =
        fixture(vec![instruction(0x12, [1, 0, 0, 0, 0, 0, 0, 0], 254)]);
    select(&mut initial, 1);
    callback(
        &mut program,
        10,
        vec![
            instruction(2, [0, 0, 2, 0, 0, 5, 10, 7], 1),
            instruction(0x12, [1, 0, 0, 0, 0, 0, 0, 0], 254),
        ],
    );
    let mut observed = initial.clone();
    let mut ordinary = initial;
    let expected = ordinary.run_fixture_tick(&program, 1000).unwrap_err();
    let mut cuts = vec![];
    let actual = observed
        .run_fixture_tick_with_service_observer(&program, 1000, &mut |cut, simulation| {
            assert_cut(cut, simulation);
            Snapshot::capture(&program, simulation).restore(&program)?;
            cuts.push(cut);
            Ok(())
        })
        .unwrap_err();
    assert!(actual.contains("native placement callback host required: AwaitingRemoval"));
    assert_eq!(actual, expected);
    assert_eq!(observed, ordinary);
    assert_eq!(cuts.len(), 1);
    assert!(matches!(
        cuts[0],
        ServiceCut::PrimitiveRemoval {
            moment: ContinuationMoment::Installed,
            ..
        }
    ));
    assert!(observed.native_lot.as_ref().unwrap().removal.is_some());
}

#[test]
fn host_poll_sink_is_unchanged_by_service_observation() {
    let (initial, mut program, _, asset) = crate::character_animation::tests::fixture(0);
    program.animations.insert("test".into(), asset);
    let mut ordinary = initial.clone();
    let mut observed = initial;
    let mut ordinary_polls = vec![];
    let mut observed_polls = vec![];
    let expected = ordinary
        .run_scheduled_slice_observed(
            &program,
            1000,
            Simulation::next_fixture_thread,
            Some(&mut ordinary_polls),
            &mut Observer::default(),
        )
        .unwrap();
    let mut cuts = vec![];
    let actual = observed
        .run_scheduled_slice_observed(
            &program,
            1000,
            Simulation::next_fixture_thread,
            Some(&mut observed_polls),
            &mut Observer::new(&mut |cut, _| {
                cuts.push(cut);
                Ok(())
            }),
        )
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(observed, ordinary);
    assert_eq!(ordinary_polls.len(), 1);
    let reports = |polls: Vec<HostPollTrace>| {
        polls
            .into_iter()
            .map(|poll| (poll.before_instruction, poll.instruction))
            .collect::<Vec<_>>()
    };
    assert_eq!(reports(observed_polls), reports(ordinary_polls));
    assert!(!cuts.is_empty());
}

#[test]
fn service_observer_does_not_relax_original_scheduler_routing() {
    let (program, mut simulation) = fixture(vec![returned()]);
    let initial = simulation.clone();
    let mut calls = 0;
    assert!(
        simulation
            .run_current_tick_with_service_observer(&program, 1000, &mut |_, _| {
                calls += 1;
                Ok(())
            })
            .unwrap_err()
            .contains("original registry scheduler")
    );
    assert_eq!(calls, 0);
    assert_eq!(simulation, initial);
}
