//! Authored real-registry controls; validation receipts are recorded separately.
use sims_formats::behavior::{Behavior, Instruction};
use sims_sim::{Room, SimExecutionEvent, Simulation, Snapshot, StepStop};
use sims_vm::{Module, Object, Program, Routine, Start, Vm, World};
use std::collections::BTreeMap;

fn fixture(cursor: i32) -> (Program, Simulation, [u64; 2]) {
    let program = Program {
        globals: "test".into(),
        modules: BTreeMap::from([(
            "test".into(),
            Module {
                source: "test".into(),
                routines: BTreeMap::from([(
                    4096,
                    Routine {
                        name: "Sleep".into(),
                        body: Behavior {
                            format: 0x8002,
                            kind: 0,
                            args: 4,
                            locals: 0,
                            instructions: vec![Instruction {
                                opcode: 0,
                                on_true: 254,
                                on_false: 255,
                                operands: [0; 8],
                            }],
                        },
                    },
                )]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut world = World {
        scheduler_storage: Some(Default::default()),
        globals: [(10, 7), (17, 0), (19, 2)].into(),
        ..Default::default()
    };
    for id in [1, 2] {
        let mut object = Object {
            native_tree_error: Some(0),
            data: [(1, 0), (25, 9)].into(),
            ..Default::default()
        };
        world.register_scheduler_object(id, &mut object).unwrap();
        object.idle.as_mut().unwrap().flags = if id == 1 { 3 } else { 1 };
        world.objects.insert(id, object);
    }
    let mut vm = Vm::new(world, 1);
    vm.lot_rng_state = Some(1825128472);
    vm.tick = 500;
    let mut threads = [0; 2];
    for id in [1, 2] {
        let thread = vm
            .spawn(
                &program,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: id,
                    callee: id,
                    stack_object: id,
                    args: vec![10, 0, 0, 0],
                    check: false,
                },
            )
            .unwrap();
        vm.register_object_thread(id, thread).unwrap();
        threads[id as usize - 1] = thread;
    }
    let mut sim = Simulation::from_placements(
        vm,
        Room {
            width: 64,
            height: 64,
            obstacles: vec![],
        },
        [1, 2]
            .map(|id| {
                (
                    id,
                    sims_sim::Body {
                        position: sims_sim::Point::new(i32::from(id) * 16, 16),
                        level: 1,
                        facing: 1,
                        radius: 1,
                        elevation: 0,
                    },
                )
            })
            .into(),
        Default::default(),
        None,
    )
    .unwrap();
    sim.enable_registry_scheduler(cursor).unwrap();
    (program, sim, threads)
}
fn restore(program: &Program, sim: &mut Simulation) {
    let saved = serde_json::to_vec(&Snapshot::capture(program, sim)).unwrap();
    *sim = serde_json::from_slice::<Snapshot>(&saved)
        .unwrap()
        .restore(program)
        .unwrap();
}

fn snapshot(program: &Program, simulation: &Simulation) -> Vec<u8> {
    serde_json::to_vec(&Snapshot::capture(program, simulation)).unwrap()
}

#[test]
fn registry_budgets_recording_and_fresh_sinks_preserve_visits_cursor_rng_and_polls() {
    for cursor in [0, 1, 2] {
        for budget in [0, 1, 2, 1000] {
            let (program, initial, threads) = fixture(cursor);
            let mut ordinary = initial.clone();
            let mut disabled = initial.clone();
            let mut observed = initial.clone();
            let mut visits = Vec::new();
            let mut completed = false;
            for iteration in 0..64 {
                let slice = if budget == 0 && iteration > 0 {
                    1
                } else {
                    budget
                };
                let mut expected_polls = Vec::new();
                let mut disabled_polls = Vec::new();
                let mut actual_polls = Vec::new();
                let expected =
                    ordinary.run_current_tick_with_host_polls(&program, slice, &mut expected_polls);
                let without_recording = disabled
                    .run_current_tick_with_host_polls_and_action_observer(
                        &program,
                        slice,
                        Some(&mut disabled_polls),
                        &mut |_| {},
                    );
                let mut events = Vec::new();
                let actual = observed.run_current_tick_with_host_polls_and_action_observer(
                    &program,
                    slice,
                    Some(&mut actual_polls),
                    &mut |event| events.push(event),
                );
                assert_eq!(actual, expected);
                assert_eq!(without_recording, expected);
                assert_eq!(polls(&expected_polls), polls(&actual_polls));
                assert_eq!(polls(&expected_polls), polls(&disabled_polls));
                assert_eq!(snapshot(&program, &ordinary), snapshot(&program, &disabled));
                assert_eq!(snapshot(&program, &ordinary), snapshot(&program, &observed));
                assert_eq!(ordinary.vm.rng_state, observed.vm.rng_state);
                assert_eq!(ordinary.vm.lot_rng_state, observed.vm.lot_rng_state);
                assert_eq!(ordinary.native_scheduler, observed.native_scheduler);
                assert_eq!(
                    ordinary.vm.scheduled_object_visit,
                    observed.vm.scheduled_object_visit
                );
                assert!(events.iter().any(|event| matches!(
                    event,
                    SimExecutionEvent::Gap {
                        owner: "Simulation::drive_cancellation",
                    }
                )));
                let step = actual.unwrap();
                assert!(step.trace.len() <= slice);
                visits.extend(step.trace.iter().map(|trace| trace.thread));
                if step.stop != StepStop::Budget {
                    assert!(events.iter().any(|event| matches!(
                        event,
                        SimExecutionEvent::Gap {
                            owner: "Simulation::flush_removals",
                        }
                    )));
                    completed = true;
                    break;
                }
                restore(&program, &mut ordinary);
                restore(&program, &mut disabled);
                restore(&program, &mut observed);
            }
            assert!(completed);
            if cursor < 2 {
                assert!(!visits.is_empty());
            }
            assert!(visits.iter().all(|thread| threads.contains(thread)));
            assert_eq!(
                ordinary.finish_registry_pass(),
                observed.finish_registry_pass()
            );
            assert_eq!(snapshot(&program, &ordinary), snapshot(&program, &observed));
        }
    }
}

#[test]
fn native_error_reentry_declares_gap_before_real_registry_host_failure() {
    let (program, mut initial, _) = fixture(0);
    let object = initial.vm.world.objects.get_mut(&1).unwrap();
    object.native_tree_error = Some(8);
    object.idle.as_mut().unwrap().flags = 4;
    let mut ordinary = initial.clone();
    let mut observed = initial;
    for _ in 0..2 {
        let expected = ordinary.run_current_tick(&program, 100);
        let mut events = Vec::new();
        let actual = observed
            .run_current_tick_with_action_observer(&program, 100, &mut |event| events.push(event));
        assert_eq!(actual, expected);
        assert!(actual.unwrap_err().contains("439a6f"));
        assert!(events.iter().any(|event| matches!(
            event,
            SimExecutionEvent::Gap {
                owner: "Simulation::next_registry_thread",
            }
        )));
        assert_eq!(snapshot(&program, &ordinary), snapshot(&program, &observed));
    }
}

#[test]
fn resumed_cancellation_errors_are_gaps_not_action_returns() {
    let (program, mut initial, threads) = fixture(0);
    initial.vm.world.actions = Some(Default::default());
    initial.cancellation = Some(sims_sim::cancellation::Execution {
        callback: sims_vm::action::Callback {
            object: 1,
            function: 4,
            stack_object: 2,
        },
        thread: threads[0],
        depth: 1,
        owner: "test".into(),
        bhav: 4096,
        backlog: Default::default(),
    });
    let mut ordinary = initial.clone();
    let mut observed = initial;
    for _ in 0..2 {
        let expected = ordinary.run_current_tick(&program, 1);
        let mut events = Vec::new();
        let actual = observed
            .run_current_tick_with_action_observer(&program, 1, &mut |event| events.push(event));
        assert_eq!(actual, expected);
        assert!(
            actual
                .unwrap_err()
                .contains("cancellation callback result missing")
        );
        assert!(events.iter().any(|event| matches!(
            event,
            SimExecutionEvent::Gap {
                owner: "Simulation::drive_cancellation",
            }
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            SimExecutionEvent::Vm(sims_vm::action_observation::Fact::Returned { .. })
        )));
        assert_eq!(snapshot(&program, &ordinary), snapshot(&program, &observed));
    }
}

#[test]
fn end_pass_cleanup_owner_is_declared_before_the_queue_is_drained() {
    let (program, mut initial, _) = fixture(0);
    initial.removals.push(sims_vm::removal::Pending {
        target: 999,
        suppress_reset: false,
    });
    let mut ordinary = initial.clone();
    let mut observed = initial;
    let expected = ordinary.run_current_tick(&program, 1000);
    let mut events = Vec::new();
    assert_eq!(
        observed
            .run_current_tick_with_action_observer(&program, 1000, &mut |event| events.push(event)),
        expected
    );
    assert!(observed.removals.is_empty());
    assert!(events.iter().any(|event| matches!(
        event,
        SimExecutionEvent::Gap {
            owner: "Simulation::flush_removals",
        }
    )));
    assert_eq!(snapshot(&program, &ordinary), snapshot(&program, &observed));
}

#[test]
fn missing_registry_and_fixture_driver_are_rejected_without_fallback_or_events() {
    let (program, initial, _) = fixture(0);
    let mut cases = vec![initial.clone()];
    cases[0].native_scheduler = None;
    let mut fixture_driver = initial;
    fixture_driver.native_scheduler = None;
    fixture_driver.fixture_driver = Some(Default::default());
    cases.push(fixture_driver);
    for initial in cases {
        let mut ordinary = initial.clone();
        let mut observed = initial.clone();
        let expected = ordinary.run_current_tick(&program, 1);
        let mut events = Vec::new();
        assert_eq!(
            observed.run_current_tick_with_action_observer(&program, 1, &mut |event| events
                .push(event)),
            expected
        );
        assert!(expected.is_err());
        assert!(events.is_empty());
        assert_eq!(observed, initial);
    }
}

#[test]
fn host_poll_rejections_preserve_the_corresponding_entry_error_and_state() {
    let (program, mut initial, _) = fixture(0);
    for fixture_driver in [false, true] {
        initial.native_scheduler = None;
        initial.fixture_driver = fixture_driver.then(Default::default);
        let mut ordinary = initial.clone();
        let mut observed = initial.clone();
        let mut disabled = initial.clone();
        let mut expected_polls = Vec::new();
        let mut actual_polls = Vec::new();
        let mut disabled_polls = Vec::new();
        let mut events = Vec::new();
        let expected = ordinary.run_current_tick_with_host_polls(&program, 1, &mut expected_polls);
        assert_eq!(
            expected.as_ref().unwrap_err(),
            "original household tick requires the native registry scheduler"
        );
        assert_eq!(
            observed.run_current_tick_with_host_polls_and_action_observer(
                &program,
                1,
                Some(&mut actual_polls),
                &mut |event| events.push(event)
            ),
            expected
        );
        assert_eq!(
            disabled.run_current_tick_with_host_polls_and_action_observer(
                &program,
                1,
                Some(&mut disabled_polls),
                &mut |_| {}
            ),
            expected
        );
        assert!(events.is_empty());
        assert!(expected_polls.is_empty() && actual_polls.is_empty() && disabled_polls.is_empty());
        assert_eq!(ordinary, initial);
        assert_eq!(observed, initial);
        assert_eq!(disabled, initial);
    }
}

use sims_formats::{
    animation::{Animation, Motion, Samples},
    avatar::{Bone, Skeleton},
};
use sims_sim::{
    character::{Controls, Resident},
    head_look, pose,
};
use sims_vm::{Definition, program::AnimationAsset};

fn polls(values: &[sims_sim::HostPollTrace]) -> Vec<(usize, sims_vm::Trace)> {
    values
        .iter()
        .map(|poll| (poll.before_instruction, poll.instruction.clone()))
        .collect()
}

fn poll_fixture(flags: u8) -> (Simulation, Program, u64, AnimationAsset) {
    let source = Skeleton {
        name: "test".into(),
        bones: vec![Bone {
            name: "ROOT".into(),
            parent: None,
            translation: [0.; 3],
            rotation: [0., 0., 0., 1.],
            properties: vec![],
            flags: [0; 3],
            wiggle: Default::default(),
        }],
    };
    let clip = Animation {
        name: "test".into(),
        skill: "test".into(),
        duration_ms: 1000.,
        distance: 0.,
        moving: false,
        translation_count: 0,
        rotation_count: 0,
        frames: 30,
        events: vec![],
        motions: vec![Motion {
            bone: "ROOT".into(),
            frames: 30,
            duration_ms: 1000.,
            translation: None,
            rotation: None,
            properties: vec![],
            event_lists: vec![],
        }],
    };
    let asset = AnimationAsset {
        source: "test".into(),
        content_hash: "test".into(),
        samples_source: "test".into(),
        samples_hash: "test".into(),
        clip,
        samples: Samples {
            translations: vec![],
            rotations: vec![],
        },
    };
    let mut program = Program {
        globals: "person".into(),
        ..Program::default()
    };
    program.modules.insert(
        "person".into(),
        Module {
            source: "person".into(),
            object_definition_words: [(1, vec![0; 103])].into(),
            strings: [
                (150, vec!["test".into(); 40]),
                (156, vec!["test".into(); 3]),
            ]
            .into(),
            routines: [(
                4096,
                Routine {
                    name: "animation".into(),
                    body: Behavior {
                        format: 0x8002,
                        kind: 0,
                        args: 1,
                        locals: 2,
                        instructions: vec![Instruction {
                            opcode: 0x2c,
                            on_true: 254,
                            on_false: 0,
                            operands: [2, 0, 1, 0, 3, flags, 2, 0],
                        }],
                    },
                },
            )]
            .into(),
            ..Module::default()
        },
    );
    let mut world = World::default();
    world.objects.insert(
        1,
        Object {
            definition: Some(Definition {
                source: "person".into(),
                object: 1,
                guid: 1,
                slots: 0,
            }),
            person: Some(
                (0..256)
                    .map(|key| (key, if key == 58 { 30 } else { 0 }))
                    .collect(),
            ),
            ..Object::default()
        },
    );
    let mut vm = Vm::new(world, 1);
    let thread = vm
        .spawn(
            &program,
            Start {
                owner: "person",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 1,
                args: vec![2],
                check: false,
            },
        )
        .unwrap();
    vm.run_slice(&program, thread, 1).unwrap();
    let mut simulation = Simulation::from_placements(
        vm,
        Room {
            width: 16,
            height: 16,
            obstacles: vec![],
        },
        BTreeMap::new(),
        [1].into(),
        None,
    )
    .unwrap();
    let local = pose::LocalBone {
        translation: [0.; 3],
        rotation: [0., 0., 0., 1.],
    };
    let matrix = pose::model::Matrix::IDENTITY;
    let mut constructor = pose::construction::State::new(0);
    constructor.carry_selection = Some(-1);
    let character = pose::refresh::Character::new(
        pose::live::Live::new(
            pose::animator::Animator::new(0, 0, 0, false, -1),
            vec![pose::sampling::BoneInput {
                local,
                priority: 0,
                blend_marker: 0,
            }],
        ),
        pose::refresh::Hierarchy {
            parents: vec![None],
            absolute: vec![head_look::rig::AbsoluteBone {
                translation: local.translation,
                rotation: local.rotation,
            }],
            updates: vec![0],
            evaluated_generation: 0,
        },
        head_look::State {
            last_clock: 0,
            basis_heading: 0.,
            target_heading: 0.,
            turn: 0.,
            target_point: Some([0.; 3]),
        },
        head_look::rig::Rig {
            head: None,
            hand: None,
            pelvis: None,
            forward: None,
            turn_bone: None,
        },
        matrix.values,
        constructor.placement.clone(),
        pose::model::Model {
            placement: matrix,
            scale: matrix,
            bounds_transform: matrix,
            bounds_flags: 0,
        },
        pose::refresh::Censor {
            binding: None,
            mask: 0,
        },
        0,
    )
    .unwrap();
    let mut resident = Resident::new(
        character,
        0,
        Controls {
            ordinary: constructor.prefix().unwrap(),
            movement: pose::movement::Movement::new(constructor.movement),
            settings: pose::animator::Settings {
                fade_enabled: true,
                fade_duration: 200,
                sampling_config: 1,
            },
            movement_anchors: [None; 2],
            geometry_generation: 0,
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
    resident.source = Some(source);
    simulation.characters.insert(1, resident);
    simulation.vm.register_object_thread(1, thread).unwrap();
    simulation.vm.world.person = Some(sims_vm::person::WorldState {
        object_order: vec![1],
        person_order: vec![1],
        ..Default::default()
    });
    simulation.vm.world.scheduler_storage = Some(Default::default());
    simulation
        .vm
        .world
        .globals
        .extend([(10, 1), (17, 0), (19, 2)]);
    let mut owner = simulation.vm.world.objects.remove(&1).unwrap();
    owner.data.extend((0..72).map(|field| (field, 0)));
    owner.native_tree_error = Some(0);
    owner.native_flags = Some(0x240);
    simulation
        .vm
        .world
        .register_scheduler_object(1, &mut owner)
        .unwrap();
    owner.idle.as_mut().unwrap().flags = 3;
    simulation.vm.world.objects.insert(1, owner);
    simulation.vm.lot_rng_state = Some(1825128472);
    simulation.vm.crt_rng_state = Some(23);
    simulation.enable_registry_scheduler(0).unwrap();
    (simulation, program, thread, asset)
}

#[test]
fn nonempty_animation_host_polls_remain_in_the_real_registry_trace() {
    let (initial, mut program, thread, asset) = poll_fixture(0);
    program.animations.insert("test".into(), asset);
    let mut ordinary = initial.clone();
    let mut observed = initial;
    let mut expected_polls = Vec::new();
    let mut actual_polls = Vec::new();
    let expected = ordinary.run_current_tick_with_host_polls(&program, 1000, &mut expected_polls);
    let mut events = Vec::new();
    let actual = observed.run_current_tick_with_host_polls_and_action_observer(
        &program,
        1000,
        Some(&mut actual_polls),
        &mut |event| events.push(event),
    );
    assert_eq!(actual, expected);
    assert!(!actual_polls.is_empty());
    assert!(
        actual_polls
            .iter()
            .all(|poll| poll.instruction.thread == thread)
    );
    assert_eq!(polls(&expected_polls), polls(&actual_polls));
    assert_eq!(snapshot(&program, &ordinary), snapshot(&program, &observed));
    assert!(events.iter().any(|event| matches!(
        event,
        SimExecutionEvent::Gap {
            owner: "Simulation::Status::AwaitingAnimation",
        }
    )));
}

fn action_fixture() -> (Program, Simulation, u64) {
    let (mut simulation, mut program, _, _) = poll_fixture(0);
    let module = program.modules.get_mut("person").unwrap();
    for (bhav, opcode, operands) in [
        (4096, 17, [0, 0, 1, 0, 0, 0, 0, 0]),
        (4097, 2, [0, 0, 0, 0, 0, 2, 7, 7]),
        (4098, 0, [0; 8]),
    ] {
        module.routines.insert(
            bhav,
            Routine {
                name: format!("authored action owner {bhav}"),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 4,
                    locals: 0,
                    instructions: vec![Instruction {
                        opcode,
                        operands,
                        on_true: 254,
                        on_false: 255,
                    }],
                },
            },
        );
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
    let mut world = simulation.vm.world.clone();
    world.actions = Some(sims_vm::action::WorldState {
        actors: [(1, sims_vm::action::Actor::default())].into(),
        ..Default::default()
    });
    let mut target = Object {
        definition: Some(Definition {
            source: "person".into(),
            object: 2,
            guid: 2,
            slots: 0,
        }),
        data: (0..80).map(|field| (field, 0)).collect(),
        native_tree_error: Some(0),
        native_flags: Some(0x240),
        ..Default::default()
    };
    world.register_scheduler_object(2, &mut target).unwrap();
    target.idle.as_mut().unwrap().flags = 3;
    world.objects.insert(2, target);
    world.person.as_mut().unwrap().object_order.push(2);
    simulation.vm = Vm::new(world, 1);
    simulation.bodies = [1, 2]
        .map(|object| {
            (
                object,
                sims_sim::Body {
                    position: sims_sim::Point::new(i32::from(object) * 16, 16),
                    level: 1,
                    facing: 1,
                    radius: 1,
                    elevation: 0,
                },
            )
        })
        .into();
    simulation.vm.lot_rng_state = Some(1825128472);
    simulation.vm.crt_rng_state = Some(23);
    let mut actor_thread = None;
    for (object, bhav) in [(1, 4096), (2, 4098)] {
        let thread = simulation
            .vm
            .spawn(
                &program,
                Start {
                    owner: "person",
                    bhav,
                    caller: object,
                    callee: object,
                    stack_object: 2,
                    args: vec![0; 4],
                    check: false,
                },
            )
            .unwrap();
        simulation
            .vm
            .register_object_thread(object, thread)
            .unwrap();
        if object == 1 {
            actor_thread = Some(thread);
        }
    }
    simulation.native_scheduler = None;
    simulation.enable_registry_scheduler(0).unwrap();
    simulation
        .vm
        .enqueue_action(
            &program,
            1,
            sims_vm::action::Action {
                target: 2,
                icon_owner: 2,
                menu_id: 19,
                priority: 100,
                routine: 4097,
                ..Default::default()
            },
        )
        .unwrap();
    (program, simulation, actor_thread.unwrap())
}

#[test]
fn scheduled_action_facts_identify_the_real_actor_target_thread_and_registry_handles() {
    use sims_vm::action_observation::{Evidence, Fact, Unavailable};
    for budget in [1, 2, 1000] {
        let (program, initial, actor_thread) = action_fixture();
        let storage = initial.vm.world.scheduler_storage.as_ref().unwrap();
        let actor_handle = storage.handle(1).unwrap();
        let target_handle = storage.handle(2).unwrap();
        let mut ordinary = initial.clone();
        let mut disabled = initial.clone();
        let mut observed = initial;
        let mut events = Vec::new();
        let mut completed = false;
        for _ in 0..64 {
            let expected = ordinary.run_current_tick(&program, budget);
            assert_eq!(
                disabled.run_current_tick_with_action_observer(&program, budget, &mut |_| {}),
                expected
            );
            let actual =
                observed.run_current_tick_with_action_observer(&program, budget, &mut |event| {
                    events.push(event)
                });
            assert_eq!(actual, expected);
            assert_eq!(snapshot(&program, &ordinary), snapshot(&program, &disabled));
            assert_eq!(snapshot(&program, &ordinary), snapshot(&program, &observed));
            if !matches!(actual.unwrap().stop, StepStop::Budget) {
                completed = true;
                break;
            }
        }
        assert!(completed, "budget {budget}");
        assert!(events.iter().any(|event| matches!(event, SimExecutionEvent::Vm(Fact::FrameBound { frame, action })
            if frame.thread == Some(actor_thread) && frame.frame.location.bhav == 4097
                && action.actor.id == Some(1) && action.actor.handle == Evidence::Known(actor_handle)
                && action.target.id == Some(2) && action.target.handle == Evidence::Known(target_handle)
                && action.routine == 4097)));
        assert!(events.iter().any(|event| matches!(event, SimExecutionEvent::Vm(Fact::Returned { frame, binding, .. })
            if frame.thread == Some(actor_thread) && frame.frame.location.bhav == 4097
                && frame.caller.id == Some(1) && frame.caller.handle == Evidence::Known(actor_handle)
                && frame.callee.id == Some(2) && frame.callee.handle == Evidence::Known(target_handle)
                && *binding == Evidence::Unavailable(Unavailable::FrameHasNoSerialBinding))));
    }
}

#[test]
fn interaction_adapters_are_rejected_at_native_admission_before_instruction_slicing() {
    let (program, mut initial, actor_thread) = action_fixture();
    initial.interactions.insert(
        1,
        sims_sim::interaction::ActorQueue {
            main: Some(actor_thread),
            ..Default::default()
        },
    );
    let mut ordinary = initial.clone();
    let mut observed = initial;
    let mut events = Vec::new();
    let expected = ordinary.run_current_tick(&program, 100);
    let actual = observed
        .run_current_tick_with_action_observer(&program, 100, &mut |event| events.push(event));
    assert_eq!(actual, expected);
    assert!(actual.unwrap_err().contains("not laboratory adapters"));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, SimExecutionEvent::Vm(_)))
    );
    assert_eq!(snapshot(&program, &ordinary), snapshot(&program, &observed));
}

fn checkpoint_action_fixture() -> (Program, Simulation, u64) {
    use sims_sim::{architecture::Architecture, lot_geometry, lot_links::Links, lot_runtime};
    use sims_vm::{autonomy::Placement, person_loading::RoomMembership};

    let (mut program, initial, actor_thread) = action_fixture();
    for definition in [1, 2] {
        let mut words = vec![0; 128];
        words[9] = if definition == 1 { 2 } else { 4 };
        words[14] = definition as i16;
        words[67] = 8;
        program
            .modules
            .get_mut("person")
            .unwrap()
            .object_definition_words
            .insert(definition, words);
    }
    let floor = lot_geometry::FloorSource {
        coverings: vec![0; 36],
        walls: vec![Default::default(); 36],
        object_heads: vec![0; 36],
        flags: vec![0; 36],
    };
    let lot = lot_geometry::Lot::rebuild(
        6,
        vec![[0; 4]; 36],
        vec![0; 36],
        vec![255; 36],
        vec![floor.clone(), floor],
    )
    .unwrap();
    let mut architecture = Architecture::from_original(lot, &program).unwrap();
    architecture.bind_footprints(&program).unwrap();
    let mut vm = initial.vm;
    vm.world.globals.extend([(11, 0), (34, 0)]);
    for object in [1, 2] {
        vm.world.positions.insert(
            object,
            Placement {
                x: 40,
                y: 40,
                level: 1,
            },
        );
        vm.world.objects.get_mut(&object).unwrap().data.extend([
            (3, -1),
            (14, if object == 2 { 0 } else { -1 }),
            (26, if object == 2 { 1 } else { 0 }),
            (29, 0),
        ]);
    }
    architecture.publish_room_context(&mut vm.world);
    let mut heads = vec![vec![0; 36]; 2];
    heads[0][14] = 1;
    let links = Links {
        size: 6,
        heads,
        next: [(1, 2), (2, 0)].into(),
    };
    links.publish_tile_chains(&mut vm.world).unwrap();
    let native = lot_runtime::State {
        placement: sims_sim::lot_placement::State::new_for_world(links, &architecture, &vm.world)
            .unwrap(),
        membership: RoomMembership::constructed(&vm.world),
        indicators: Default::default(),
        observers: vec![],
        diagnostics: sims_sim::lot_acceptance::person::Diagnostics {
            error: 0,
            blocker: None,
        },
        route: None,
        shoo: None,
        movement: None,
        completion: None,
        route_paths: Default::default(),
        snap: None,
        find_location: None,
        removal: None,
        snap_offsets: Default::default(),
        selected_person: None,
        room_effects: vec![],
        routing_policy: Default::default(),
    };
    let mut simulation = Simulation::from_loaded_lot(vm, architecture, native).unwrap();
    simulation.characters = initial.characters;
    simulation.enable_registry_scheduler(0).unwrap();
    (program, simulation, actor_thread)
}

#[test]
fn active_registry_action_restores_preserve_queue_serials_without_rebinding_the_saved_frame() {
    use sims_vm::action_observation::{Evidence, Fact, FrameFingerprint, Unavailable};

    for budget in [1, 2, 1000] {
        let (program, mut initial, actor_thread) = checkpoint_action_fixture();
        let mut entry_events = Vec::new();
        for _ in 0..64 {
            initial
                .run_current_tick_with_action_observer(&program, 1, &mut |event| {
                    entry_events.push(event)
                })
                .unwrap();
            if entry_events.iter().any(|event| {
                matches!(event, SimExecutionEvent::Vm(Fact::FrameBound { frame, .. })
                    if frame.thread == Some(actor_thread) && frame.frame.location.bhav == 4097)
            }) {
                break;
            }
        }
        let bound = entry_events
            .iter()
            .filter_map(|event| match event {
                SimExecutionEvent::Vm(Fact::FrameBound { frame, action })
                    if frame.thread == Some(actor_thread) && frame.frame.location.bhav == 4097 =>
                {
                    Some((frame, action))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(bound.len(), 1);
        let (saved_frame, active_action) = bound[0];
        let active_serial = active_action.serial;
        assert_ne!(active_serial, 0);
        assert_eq!(active_action.actor.id, Some(1));
        assert_eq!(active_action.target.id, Some(2));
        let is_saved_frame = |frame: &FrameFingerprint| {
            frame.thread == saved_frame.thread
                && frame.depth == saved_frame.depth
                && frame.frame.code_owner == saved_frame.frame.code_owner
                && frame.frame.location.bhav == saved_frame.frame.location.bhav
                && frame.caller == saved_frame.caller
                && frame.callee == saved_frame.callee
        };
        let actions = initial.vm.world.actions.as_ref().unwrap();
        assert_eq!(
            actions.actors[&1].active.as_ref().unwrap().id,
            active_serial
        );

        let (queued_serial, accepted) = initial
            .vm
            .enqueue_action(
                &program,
                1,
                sims_vm::action::Action {
                    target: 2,
                    icon_owner: 2,
                    menu_id: 19,
                    priority: 100,
                    routine: 4097,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(accepted);
        assert_ne!(queued_serial, active_serial);
        let saved_actions = initial.vm.world.actions.clone();
        let saved = snapshot(&program, &initial);
        let mut ordinary = initial;
        let mut observed = serde_json::from_slice::<Snapshot>(&saved)
            .unwrap()
            .restore(&program)
            .unwrap();
        assert_eq!(snapshot(&program, &observed), saved);
        assert_eq!(observed.vm.world.actions, saved_actions);

        for _ in 0..3 {
            let expected = ordinary.run_current_tick(&program, 0);
            let mut events = Vec::new();
            assert_eq!(
                observed.run_current_tick_with_action_observer(&program, 0, &mut |event| {
                    events.push(event)
                }),
                expected
            );
            assert!(!events.iter().any(|event| matches!(
                event,
                SimExecutionEvent::Vm(Fact::FrameBound { .. } | Fact::Returned { .. })
            )));
            assert_eq!(observed.vm.world.actions, saved_actions);
            assert_eq!(snapshot(&program, &ordinary), snapshot(&program, &observed));
            restore(&program, &mut observed);
        }

        let mut fresh_events = Vec::new();
        let mut complete = false;
        for _ in 0..64 {
            let mut expected_polls = Vec::new();
            let mut actual_polls = Vec::new();
            let expected =
                ordinary.run_current_tick_with_host_polls(&program, budget, &mut expected_polls);
            let actual = observed.run_current_tick_with_host_polls_and_action_observer(
                &program,
                budget,
                Some(&mut actual_polls),
                &mut |event| fresh_events.push(event),
            );
            assert_eq!(actual, expected);
            assert_eq!(polls(&expected_polls), polls(&actual_polls));
            assert_eq!(snapshot(&program, &ordinary), snapshot(&program, &observed));
            restore(&program, &mut observed);
            assert_eq!(snapshot(&program, &ordinary), snapshot(&program, &observed));
            if actual.unwrap().stop != StepStop::Budget {
                complete = true;
                break;
            }
        }
        assert!(complete, "budget {budget}");
        assert!(
            !fresh_events
                .iter()
                .take_while(|event| !matches!(event,
                    SimExecutionEvent::Vm(Fact::Returned { frame, .. }) if is_saved_frame(frame)))
                .any(|event| matches!(event,
                    SimExecutionEvent::Vm(Fact::FrameBound { frame, .. }) if is_saved_frame(frame)))
        );
        assert!(!fresh_events.iter().any(|event| matches!(event,
            SimExecutionEvent::Vm(Fact::FrameBound { action, .. })
                if action.serial == active_serial)));
        let returns = fresh_events
            .iter()
            .filter_map(|event| match event {
                SimExecutionEvent::Vm(Fact::Returned { frame, binding, .. })
                    if frame.thread == Some(actor_thread) && frame.frame.location.bhav == 4097 =>
                {
                    Some((frame, binding))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(returns.len(), 1);
        assert!(is_saved_frame(returns[0].0));
        assert_eq!(returns[0].0.caller.id, Some(1));
        assert_eq!(returns[0].0.callee.id, Some(2));
        assert_eq!(
            *returns[0].1,
            Evidence::Unavailable(Unavailable::FrameHasNoSerialBinding)
        );
    }
}
