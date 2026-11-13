use super::*;
use crate::lot_route::departure::Scan;
use serde_json::json;

fn fixture() -> (Program, Simulation, RouteRequest, Potential, ThreadId) {
    let (mut program, mut simulation) = crate::lot_runtime::tests::fixture();
    simulation.vm.world.objects.get_mut(&2).unwrap().person =
        Some((0..100).map(|field| (field, 0)).collect());
    crate::lot_runtime::tests::contain_potential_actor(&mut program, &mut simulation, 2, 3, 0x11);
    let thread = simulation.vm.object_threads[&4];
    simulation.visit = Some((thread, None));
    let request = RouteRequest {
        id: 19209,
        origin: sims_vm::Location {
            source: "objects".into(),
            bhav: 4096,
            instruction: 0,
        },
        caller: 2,
        target: 3,
        destination: sims_vm::routing::Destination::Slot {
            source: "objects".into(),
            chunk: 0,
            index: 0,
            version: 10,
            slot: sims_vm::routing::Descriptor([
                0,
                0,
                0,
                1,
                1,
                0,
                0,
                0x801,
                u32::MAX,
                16,
                16,
                16,
                0.1875_f32.to_bits(),
                (-3_i32) as u32,
                16,
            ])
            .slot(),
        },
        no_failure_trees: true,
    };
    let settings = Potential {
        ignored_person: Some(1),
        ignore_people: false,
        ignored_word: 0x1234,
        blocked_object: 219,
        retained_result: 3,
    };
    (program, simulation, request, settings, thread)
}

fn step() -> Step {
    Step {
        stop: crate::StepStop::Budget,
        trace: vec![],
        events: vec![],
    }
}

#[test]
fn potential_ordinary_owner_rejected_before_driver_mutation() {
    let (program, mut simulation, request, settings, thread) = fixture();
    let execution =
        Execution::begin_potential(&mut simulation, &program, thread, &request, settings).unwrap();
    assert_eq!(
        execution.validate(&simulation, &program).unwrap_err(),
        "temporary potential cannot use ordinary route owner"
    );
    simulation.native_lot.as_mut().unwrap().route = Some(execution);
    let before = simulation.clone();
    let mut remaining = 1000;
    let mut output = step();
    assert_eq!(
        simulation
            .drive_native_route_prefix(&program, &mut remaining, &mut output)
            .unwrap_err(),
        "temporary potential cannot use ordinary route owner"
    );
    assert_eq!(simulation, before);
    assert_eq!(remaining, 1000);
    assert!(output.trace.is_empty() && output.events.is_empty());
}

#[test]
fn potential_departure_accepts_raw_seat_facing_and_wraps_each_probe() {
    for facing in [8, -1, 15, i16::MIN, i16::MAX] {
        let (program, mut simulation, mut request, settings, thread) = fixture();
        request.target = 1;
        simulation
            .vm
            .world
            .objects
            .get_mut(&3)
            .unwrap()
            .data
            .insert(1, facing);
        let execution =
            Execution::begin_potential(&mut simulation, &program, thread, &request, settings)
                .unwrap();
        let saved = serde_json::to_value(execution).unwrap();
        let mut scan: Scan = serde_json::from_value(saved["departure"]["scan"].clone()).unwrap();
        let first = scan.next(facing).unwrap();
        assert_eq!(first.direction, (facing & 7) as u8);
        scan.resume(first, 0).unwrap();
        let live_facing = facing.wrapping_add(1);
        let second = scan.next(live_facing).unwrap();
        assert_eq!(second.direction, ((i32::from(live_facing) + 4) & 7) as u8);
    }
    let (program, mut simulation, mut request, settings, thread) = fixture();
    request.target = 1;
    simulation
        .vm
        .world
        .objects
        .get_mut(&3)
        .unwrap()
        .data
        .remove(&1);
    let before = simulation.clone();
    assert_eq!(
        Execution::begin_potential(&mut simulation, &program, thread, &request, settings)
            .unwrap_err(),
        "departure seat facing absent"
    );
    assert_eq!(simulation, before);
}

#[test]
fn potential_mid_stand_zero_slot_predecessor_never_borrows_parent_controller() {
    let (program, mut simulation) = crate::lot_runtime::tests::routed_walking_fixture();
    simulation.native_lot.as_mut().unwrap().routing_policy = super::super::path::Policy::Strict;
    assert!(
        simulation
            .run_fixture_tick(&program, 1000)
            .unwrap_err()
            .starts_with("original-lot path construction required")
    );
    let previous = simulation
        .native_lot
        .as_mut()
        .unwrap()
        .route
        .take()
        .unwrap();
    let mut request = crate::contention::pending_route(
        &simulation.vm.threads[&previous.thread],
        previous.request,
    )
    .unwrap()
    .clone();
    let mut actor = simulation.vm.world.objects[&1].clone();
    actor.container = None;
    actor.data.extend([(1, 2), (14, 0), (26, 45)]);
    actor
        .person
        .as_mut()
        .unwrap()
        .extend([(0, 1), (37, 4), (72, 0), (73, 0)]);
    let mut target = simulation.vm.world.objects[&2].clone();
    target.data.extend([(14, -1), (26, 0)]);
    simulation.vm.world.objects.insert(16, actor);
    simulation.vm.world.objects.insert(45, target);
    for object in [16, 45] {
        simulation.vm.world.positions.insert(
            object,
            sims_vm::autonomy::Placement {
                x: 392,
                y: 280,
                level: 1,
            },
        );
    }
    let mut owner = simulation.vm.threads[&previous.thread].clone();
    let mut blocker = owner.clone();
    blocker.stack.last_mut().unwrap().location.bhav = 4102;
    blocker.stack.last_mut().unwrap().location.instruction = 8;
    owner.callback = None;
    simulation.vm.threads.insert(66, owner);
    simulation.vm.threads.insert(67, blocker);
    let mut parent_control = simulation.route_control[&request.id].clone();
    parent_control.caller = 18;
    parent_control.thread = 66;
    parent_control.deciding = true;
    parent_control.controller.result = 3;
    parent_control.original.as_mut().unwrap().blocked_object = 219;
    simulation
        .route_control
        .insert(19209, parent_control.clone());
    let mut blocker_control = parent_control;
    blocker_control.caller = 16;
    blocker_control.thread = 67;
    blocker_control.original.as_mut().unwrap().standing = Some(225);
    simulation.route_control.insert(19226, blocker_control);
    simulation.visit = Some((66, None));
    request.id = 19209;
    request.caller = 16;
    request.target = 45;
    let controllers = simulation.route_control.clone();
    let threads = simulation.vm.threads.clone();
    let positions = simulation.vm.world.positions.clone();
    let actor = simulation.vm.world.objects[&16].clone();
    let mut execution = Execution::begin_potential(
        &mut simulation,
        &program,
        66,
        &request,
        Potential {
            ignored_person: None,
            ignore_people: false,
            ignored_word: 0,
            blocked_object: 0,
            retained_result: 0,
        },
    )
    .unwrap();
    assert!(execution.departure.is_none() && execution.prepared);
    assert!(simulation.drive_potential_preparation(
        &program, &mut execution, &request, &mut 1000, &mut step(),
    ).unwrap());
    assert!(execution.goal.is_none() && execution.selected_seat.is_none());
    assert_eq!(simulation.route_control, controllers);
    assert_eq!(simulation.vm.threads, threads);
    assert_eq!(simulation.vm.world.positions, positions);
    assert_eq!(simulation.vm.world.objects[&16], actor);
}

#[test]
fn potential_departure_preserves_pointer_seed_and_delays_temporary_word() {
    for ignored_person in [None, Some(1)] {
        for ignored_word in [0, 0x1234] {
            let (program, mut simulation, request, mut settings, thread) = fixture();
            settings.ignored_person = ignored_person;
            settings.ignored_word = ignored_word;
            simulation
                .vm
                .world
                .objects
                .get_mut(&2)
                .unwrap()
                .person
                .as_mut()
                .unwrap()
                .insert(73, 73);
            let execution = Execution::begin_potential(
                &mut simulation,
                &program,
                thread,
                &request,
                settings.clone(),
            )
            .unwrap();
            assert!(!execution.prepared);
            assert_eq!(execution.potential.as_ref(), Some(&settings));
            assert_eq!(execution.input.ignored, ignored_person);
            assert_eq!(execution.blocked_object, 219);
            assert_eq!(execution.retained_result, 3);
            assert_eq!(
                simulation.vm.world.objects[&2].person.as_ref().unwrap()[&73],
                73
            );
            let saved = serde_json::to_value(&execution).unwrap();
            assert_eq!(
                saved["departure"]["input"]["ignored"],
                json!(ignored_person)
            );
            assert_eq!(saved["departure"]["blocked_object"], 219);
            let restored: Execution = serde_json::from_value(saved).unwrap();
            restored
                .validate_request(&simulation, &program, &request)
                .unwrap();
            assert_eq!(restored, execution);
        }
    }
}

#[test]
fn potential_departure_ties_latch_origin_target_mask_and_reread_seat_facing() {
    let (mut program, mut simulation, request, settings, thread) = fixture();
    let execution = Execution::begin_potential(
        &mut simulation,
        &program,
        thread,
        &request,
        settings.clone(),
    )
    .unwrap();
    let mut saved = serde_json::to_value(&execution).unwrap();
    let mut scan: Scan = serde_json::from_value(saved["departure"]["scan"].clone()).unwrap();
    let first = scan.next(0).unwrap();
    assert_eq!(first.position, [24, 40]);
    scan.resume(first, 5).unwrap();
    simulation
        .vm
        .world
        .objects
        .get_mut(&3)
        .unwrap()
        .data
        .insert(1, 2);
    program
        .modules
        .get_mut("objects")
        .unwrap()
        .object_definition_words
        .get_mut(&1)
        .unwrap()[0x84 / 2] = 255;
    let second = scan.next(simulation.vm.world.objects[&3].data[&1]).unwrap();
    assert_eq!(second.position, [40, 24]);
    scan.resume(second, 6).unwrap();
    assert!(scan.next(2).is_none());
    assert_eq!(scan.outcome().unwrap().position, crate::Point::new(24, 40));
    assert_eq!(scan.outcome().unwrap().flags, 1);
    saved["departure"]["scan"] = serde_json::to_value(scan).unwrap();
    saved["departure"]["blocked_object"] = json!(17);
    let mut execution: Execution = serde_json::from_value(saved).unwrap();
    simulation.vm.world.positions.get_mut(&2).unwrap().x = 44;
    simulation.vm.world.positions.get_mut(&3).unwrap().x = 48;
    simulation.bodies.get_mut(&2).unwrap().position.y = 44;
    simulation.bodies.get_mut(&3).unwrap().position.y = 48;
    simulation
        .vm
        .world
        .objects
        .get_mut(&2)
        .unwrap()
        .data
        .insert(1, 6);
    simulation.sync_script_directions().unwrap();
    let mut remaining = 0;
    let _ = simulation
        .drive_potential_preparation(
            &program,
            &mut execution,
            &request,
            &mut remaining,
            &mut step(),
        )
        .unwrap();
    assert!(execution.prepared);
    assert_eq!(execution.target_position, [48, 40]);
    assert_eq!(execution.target_facing, 2);
    assert_eq!(execution.saved_facing, 6);
    assert_eq!(
        execution.standing.as_ref().unwrap().request().preferred,
        [24, 40]
    );
    assert_eq!(execution.potential.as_ref(), Some(&settings));
    assert_eq!(
        simulation.vm.world.objects[&2].person.as_ref().unwrap()[&73],
        if execution.candidates.is_some() {
            0
        } else {
            settings.ignored_word
        }
    );
    execution
        .validate_request(&simulation, &program, &request)
        .unwrap();
}

#[test]
fn potential_preflight_rejects_before_capture_normalization_or_world_effects() {
    for corruption in 0..8 {
        let (mut program, mut simulation, mut request, settings, thread) = fixture();
        if !matches!(corruption, 4..=6) {
            simulation
                .vm
                .world
                .objects
                .get_mut(&2)
                .unwrap()
                .data
                .insert(14, 0);
        }
        match corruption {
            0 => {
                simulation
                    .vm
                    .world
                    .objects
                    .get_mut(&2)
                    .unwrap()
                    .person
                    .as_mut()
                    .unwrap()
                    .insert(0, 2);
            }
            1 => {
                request.caller = 300;
            }
            2 => {
                request.target = 300;
            }
            3 => {
                if let sims_vm::routing::Destination::Slot { version, .. } =
                    &mut request.destination
                {
                    *version = 3;
                }
            }
            4 => {
                simulation.vm.world.objects.get_mut(&3).unwrap().definition = None;
            }
            5 => {
                program
                    .modules
                    .get_mut("objects")
                    .unwrap()
                    .object_definition_words
                    .get_mut(&1)
                    .unwrap()
                    .truncate(66);
            }
            6 => {
                simulation
                    .vm
                    .world
                    .objects
                    .get_mut(&3)
                    .unwrap()
                    .data
                    .insert(1, 8);
            }
            7 => {
                simulation.vm.world.positions.remove(&request.caller);
            }
            _ => unreachable!(),
        }
        let before = simulation.clone();
        assert!(
            Execution::begin_potential(&mut simulation, &program, thread, &request, settings)
                .is_err(),
            "{corruption}"
        );
        assert_eq!(simulation, before, "{corruption}");
    }
}

#[test]
fn potential_departure_restore_rejects_forged_context_and_stage_without_effects() {
    let (program, mut simulation, request, settings, thread) = fixture();
    let execution =
        Execution::begin_potential(&mut simulation, &program, thread, &request, settings).unwrap();
    let saved = serde_json::to_value(execution).unwrap();
    for (path, value) in [
        (vec!["thread"], json!(67)),
        (vec!["candidate_limit"], json!(9)),
        (vec!["potential_request", "caller"], json!(18)),
        (vec!["potential", "ignored_person"], json!(3)),
        (vec!["potential", "blocked_object"], json!(18)),
        (vec!["departure", "input", "ignored"], json!(3)),
        (vec!["departure", "input", "flags"], json!(0)),
        (vec!["departure", "initial_blocked_object"], json!(18)),
        (vec!["prepared"], json!(true)),
        (vec!["posture_checked"], json!(true)),
    ] {
        let mut forged = saved.clone();
        let mut field = &mut forged;
        for component in path {
            field = &mut field[component];
        }
        *field = value;
        let mut execution: Execution = serde_json::from_value(forged).unwrap();
        let before_execution = execution.clone();
        let before = simulation.clone();
        let mut remaining = 1000;
        let mut output = step();
        assert!(
            execution
                .validate_request(&simulation, &program, &request)
                .is_err()
        );
        assert!(
            simulation
                .drive_potential_preparation(
                    &program,
                    &mut execution,
                    &request,
                    &mut remaining,
                    &mut output,
                )
                .is_err()
        );
        assert_eq!(execution, before_execution);
        assert_eq!(simulation, before);
        assert_eq!(remaining, 1000);
        assert!(output.trace.is_empty() && output.events.is_empty());
    }
}

#[test]
fn potential_departure_room_clamp_preserves_retained_eight_and_fresh_forty_eight() {
    for retained in [8, 48] {
        let (mut program, mut simulation, request, settings, thread) = fixture();
        program
            .modules
            .get_mut("objects")
            .unwrap()
            .object_definition_words
            .get_mut(&1)
            .unwrap()[0x84 / 2] = 0;
        simulation.vm.world.room_half_tiles.insert(0, 1000);
        let mut execution = Execution::begin_retained_potential(
            &mut simulation,
            &program,
            thread,
            &request,
            settings.clone(),
            retained,
        )
        .unwrap();
        assert_eq!(execution.potential_candidate_limit(), retained);
        simulation.vm.world.room_half_tiles.insert(0, 100);
        let mut fresh =
            Execution::begin_potential(&mut simulation, &program, thread, &request, settings)
                .unwrap();
        assert_eq!(fresh.potential_candidate_limit(), 48);
        for query in [&mut execution, &mut fresh] {
            let _ = simulation
                .drive_potential_preparation(&program, query, &request, &mut 0, &mut step())
                .unwrap();
            query
                .validate_request(&simulation, &program, &request)
                .unwrap();
        }
        assert_eq!(execution.potential_candidate_limit(), retained);
        assert_eq!(fresh.potential_candidate_limit(), 48);
    }
}
