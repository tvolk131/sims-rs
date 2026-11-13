use super::*;
use crate::{Snapshot, Step, StepStop, character_update::Settings, clock::LotClock, pose};

fn fixture() -> (Program, Simulation) {
    let (mut p, mut s) = crate::lot_runtime::tests::fixture();
    let (mut actor, source, _, asset) = crate::character_animation::tests::fixture(32);
    let mut resident = actor.characters.remove(&1).unwrap();
    resident.context.actor = 4;
    // A synthetic, initialized, off-lot person with an empty one-bone model.
    // Zero is an explicit animation mode here, not a replacement saved value.
    resident.controls.ordinary.mode = 0;
    s.characters.insert(4, resident);
    p.animations.insert("test".into(), asset);
    let module = p.modules.get_mut("objects").unwrap();
    module
        .strings
        .extend(source.modules["person"].strings.clone());
    for id in [1, 5, 6] {
        module.named_constants.insert(
            id,
            sims_formats::tuning::Constants {
                version: 1,
                entries: vec![],
            },
        );
    }
    for id in [501, 502, 503, 504] {
        module.strings.insert(id, vec!["(-100;1)(100;1)".into(); 9]);
    }
    let main = &mut module.routines.get_mut(&4096).unwrap().body;
    main.args = 1;
    main.instructions = vec![sims_formats::behavior::Instruction {
        opcode: 0,
        on_true: 254,
        on_false: 0,
        operands: [0; 8],
    }];
    let object = s.vm.world.objects.get_mut(&4).unwrap();
    object.person = actor.vm.world.objects[&1].person.clone();
    object.person.as_mut().unwrap().extend([(32, 1), (85, 150)]);
    object.slots = vec![None; 3];
    object.motives = Some([0; 16]);
    object.data.insert(25, 100);
    s.vm.world.person.as_mut().unwrap().person_order.push(4);
    s.native_lot.as_mut().unwrap().membership =
        sims_vm::person_loading::RoomMembership::constructed(&s.vm.world);
    // A suspended Main must still get native person/animator updates on every
    // lot visit. It must not be re-executed when a tail service resumes.
    let old = s.vm.object_threads.remove(&4).unwrap();
    s.vm.threads.remove(&old);
    s.vm.lifecycles.remove(&4);
    let id = s.vm.start_object_lifecycle(&p, 4).unwrap();
    s.vm.threads.get_mut(&id).unwrap().stack[0].args = vec![100];
    s.native_character_settings = Some(Settings {
        terrain_scale: None,
        movement: movement::Settings {
            short_distance: 3.1,
            turn_rate: 1.,
            small_turn: 0.5,
            medium_turn: 1.5,
            large_turn: 3.,
            minimum_weight: 0.5,
            snap_distance: 0.5,
            rate: ordinary::RateSettings {
                base: 1.2,
                positive: 0.2,
                negative: 0.4,
                change: 0.2,
            },
        },
        root_motion: 0,
        person: person_update::Context {
            only_visible: true,
            display_animation_names: false,
            debug_recording: false,
        },
        floor_vector: [0.; 3],
    });
    s.enable_household(
        &p,
        LotClock {
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
    (p, s)
}

fn restore(p: &Program, s: &Simulation) -> Simulation {
    let snapshot = Snapshot::capture(p, s);
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    serde_json::from_slice::<Snapshot>(&bytes)
        .unwrap()
        .restore(p)
        .unwrap()
}

#[test]
fn house_refresh_stops_native_walk_without_completing_or_cancelling_its_request() {
    use crate::lot_route::{animator::Walk, path::Policy};
    use sims_formats::behavior::Instruction;
    let (mut p, mut initial) = fixture();
    initial.vm.world.actions = Some(sims_vm::action::WorldState {
        actors: [(4, Default::default())].into(),
        ..Default::default()
    });
    p.modules
        .get_mut("objects")
        .unwrap()
        .routines
        .get_mut(&4096)
        .unwrap()
        .body
        .instructions = vec![Instruction {
        opcode: 0x1b,
        on_true: 254,
        on_false: 255,
        operands: [0, 0, 0, 254, 0, 0, 0, 0],
    }];
    let mut placement = crate::lot_placement::live::Execution::new(
        crate::lot_route::movement::commit_operation(4, [72, 72], 1),
    );
    for _ in 0..100 {
        let mut step = Step {
            stop: StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        if initial
            .drive_live_placement(&p, &mut placement, &mut 1000, &mut step)
            .unwrap()
        {
            break;
        }
        service_empty_model(&mut initial, true);
    }
    initial
        .vm
        .world
        .objects
        .get_mut(&4)
        .unwrap()
        .data
        .insert(8, 8);
    let thread = initial.vm.object_threads[&4];
    initial
        .vm
        .threads
        .get_mut(&thread)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .stack_object = 4;
    initial.vm.run_slice(&p, thread, 100).unwrap();
    let sims_vm::Status::AwaitingRoute(request) = initial.vm.threads[&thread].status.clone() else {
        panic!("route expected: {:?}", initial.vm.threads[&thread].status)
    };
    assert!(
        !initial
            .service_original_portals(&p, thread, &request, &mut vec![])
            .unwrap()
    );
    let context = initial.route_control.get_mut(&request.id).unwrap();
    context.controller.stage = 2;
    context.controller.result = 13;
    context.original.as_mut().unwrap().ignored = 7;
    initial.person(4, 72, 99).unwrap();
    initial.native_lot.as_mut().unwrap().routing_policy = Policy::InferredRectangles;
    let points = vec![[72, 72], [72, 80], [72, 88]];
    initial
        .native_lot
        .as_mut()
        .unwrap()
        .route_paths
        .insert(4, points.clone());
    let body = initial.body(4).unwrap().clone();
    let resident = initial.characters.get_mut(&4).unwrap();
    let mut path = pose::movement::path::Path::initialized(
        pose::movement::path::Settings::COMPLETE_COLLECTION,
        [0.2, 0.8],
        [1.; 2],
        [2.; 6],
    );
    path.points = points;
    resident.controls.movement.path = Some(path);
    resident
        .controls
        .movement
        .start_route(
            &mut resident.character.placement,
            &mut resident.controls.ordinary,
            [72, 72],
            0,
        )
        .unwrap();
    resident.native_walk = Some(Walk {
        retained_origin: None,
        thread,
        request: request.id,
        target: 4,
        target_body: body,
        started_tick: initial.vm.tick,
        yielded_tick: initial.vm.tick,
        poll: None,
        physical: None,
        stopping: None,
    });
    // Older named checkpoints have no retained-origin field.
    let walk = resident.native_walk.as_ref().unwrap();
    let mut legacy = serde_json::to_value(walk).unwrap();
    legacy.as_object_mut().unwrap().remove("retained_origin");
    assert_eq!(serde_json::from_value::<Walk>(legacy).unwrap(), *walk);
    initial
        .vm
        .threads
        .get_mut(&thread)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .continuation = 3;
    initial.invalidate_original_route_caches().unwrap();
    let mut s = restore(&p, &initial);
    let world = s.vm.world.clone();
    let rng = s.vm.rng_state;
    let tick = s.vm.tick;
    let mut expected_vm = s.vm.clone();
    expected_vm
        .threads
        .get_mut(&thread)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .continuation = 15;
    let mut step = Step {
        stop: StepStop::Waiting,
        trace: vec![],
        events: vec![],
    };
    assert!(
        !s.service_original_animator_path(&p, thread, &request, &mut 1000, &mut step)
            .unwrap()
    );
    assert_eq!(s.vm.threads[&thread].stack.last().unwrap().continuation, 15);
    assert_eq!(
        s.characters[&4].native_walk.as_ref().unwrap().stopping,
        Some(pose::movement::route::Stop::Refresh)
    );
    assert!(
        s.characters[&4]
            .native_walk
            .as_ref()
            .unwrap()
            .poll
            .is_none()
    );
    assert_eq!(
        s.characters[&4]
            .controls
            .movement
            .path
            .as_ref()
            .unwrap()
            .interrupt,
        1
    );
    assert_eq!(s.vm.world, world);
    assert_eq!(s.vm.rng_state, rng);
    assert_eq!(s.vm.tick, tick);
    assert_eq!(
        s.vm, expected_vm,
        "refresh stop changed VM state beyond the native continuation write"
    );
    s = restore(&p, &s);
    // Explicit animator service reply: braking has finished, as in the native
    // resumption corpus. No fake completed gameplay tick is counted here.
    s.characters
        .get_mut(&4)
        .unwrap()
        .controls
        .movement
        .state
        .status = 2;
    let retained = s.native_lot.as_ref().unwrap().route_paths.clone();
    let control = s.route_control[&request.id].clone();
    assert!(
        s.service_original_animator_path(&p, thread, &request, &mut 1000, &mut step)
            .unwrap()
    );
    assert!(s.characters[&4].native_walk.is_none());
    assert_eq!(s.native_lot.as_ref().unwrap().route_paths, retained);
    assert_eq!(s.route_control[&request.id], control);
    assert!(s.native_lot.as_ref().unwrap().completion.is_none());
    assert!(matches!(
        s.vm.threads[&thread].status,
        sims_vm::Status::AwaitingRoute(_)
    ));
    assert!(
        !s.service_original_portals(&p, thread, &request, &mut step.events)
            .unwrap()
    );
    // Match the original probe's boundary before48235a. The new candidate
    // preparation has its own verified person73 write and begins next.
    assert_eq!(s.vm.world, world);
    assert_eq!(s.vm.rng_state, rng);
    assert_eq!(s.vm.tick, tick);
    assert_eq!(restore(&p, &s), s);
}

#[test]
fn native_look_towards_uses_child_profile_and_restores_each_service_boundary() {
    use sims_formats::behavior::Instruction;
    fn environment(s: &mut Simulation) {
        for (id, request) in s.pending_person_services() {
            let response = match request {
                person_update::Request::Environment { .. } => {
                    person_update::Response::Environment(0f32.to_bits())
                }
                person_update::Request::AllowNegativeOutdoorEnvironment => {
                    person_update::Response::Bool(false)
                }
                _ => continue,
            };
            s.complete_person_service(id, &request, response).unwrap();
        }
    }
    for age in [9, 30] {
        for mode in 1..=3 {
            let (mut p, mut initial) = fixture();
            initial.vm.world.globals.insert(10, 1);
            initial.household.as_mut().unwrap().house_statistics = Some(
                crate::house_statistics::State::from_saved_pairs([[0; 2]; 6]),
            );
            let m = p.modules.get_mut("objects").unwrap();
            m.strings.insert(150, vec![String::new(); 40]);
            m.routines.get_mut(&4096).unwrap().body.instructions = vec![
                Instruction {
                    opcode: 22,
                    on_true: 1,
                    on_false: 255,
                    operands: [mode, 0, 0, 0, 0, 0, 0, 0],
                },
                Instruction {
                    opcode: 0,
                    on_true: 1,
                    on_false: 1,
                    operands: [0; 8],
                },
            ];
            let mut callback = m.routines[&4096].clone();
            callback.body.instructions = vec![
                Instruction {
                    opcode: 2,
                    on_true: 1,
                    on_false: 255,
                    operands: [7, 0, 1, 0, 0, 3, 8, 7],
                },
                Instruction {
                    opcode: 2,
                    on_true: 254,
                    on_false: 255,
                    operands: [7, 0, 1, 0, 0, 3, 8, 7],
                },
            ];
            m.routines.insert(4100, callback);
            m.object_functions.get_mut(&1).unwrap()[9].action = 4100;
            let mut global = m.clone();
            global.source = "turn-globals".into();
            global.strings.extend([
                (150, vec!["fallback".into(); 40]),
                (151, vec!["child".into(); 40]),
                (152, vec!["adult".into(); 40]),
            ]);
            p.globals = global.source.clone();
            p.modules.insert(global.source.clone(), global);
            for name in ["fallback", "child", "adult"] {
                let mut a = p.animations["test"].clone();
                a.clip.name = name.into();
                a.clip.skill = name.into();
                a.clip.duration_ms = 200.;
                p.animations.insert(name.into(), a);
            }
            let person = initial
                .vm
                .world
                .objects
                .get_mut(&4)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            person.insert(58, age);
            let id = initial.vm.object_threads[&4];
            initial.vm.threads.get_mut(&id).unwrap().stack[0].stack_object = 2;
            initial
                .characters
                .get_mut(&4)
                .unwrap()
                .controls
                .movement
                .path = Some(movement::path::Path::initialized(
                movement::path::Settings::COMPLETE_COLLECTION,
                [0.2, 0.8],
                [1., 1.],
                [2.; 6],
            ));
            // Ordinary updates deliberately skip path following off-lot. Place
            // this fixture through the same live setter used by movement.
            let mut placement = crate::lot_placement::live::Execution::new(
                crate::lot_route::movement::commit_operation(4, [72, 72], 1),
            );
            let mut placed = false;
            for _ in 0..100 {
                let mut step = crate::Step {
                    stop: StepStop::Budget,
                    trace: vec![],
                    events: vec![],
                };
                placed = initial
                    .drive_live_placement(&p, &mut placement, &mut 1000, &mut step)
                    .unwrap();
                service_empty_model(&mut initial, true);
                if placed {
                    break;
                }
            }
            assert!(placed);
            let mut outcomes = vec![];
            for budget in [1, 1000] {
                let mut s = initial.clone();
                let mut selected = false;
                let mut done = false;
                let mut captured_pending = false;
                let mut captured_callback = false;
                for _ in 0..1000 {
                    let step = s.run_fixture_tick(&p, budget).unwrap();
                    assert_ne!(step.stop, StepStop::Faulted, "{:?}", s.vm.threads);
                    done |= step
                        .events
                        .iter()
                        .any(|e| e.detail == "native Look Towards completed");
                    if let Some(t) = &s.characters[&4].native_turn {
                        assert!(s.turns.is_empty());
                        assert_eq!(t.path[0], [72, 72]);
                        assert_eq!(t.path[0], t.path[1]);
                        captured_pending = true;
                        let mut bad = Snapshot::capture(&p, &s);
                        bad.state
                            .characters
                            .get_mut(&4)
                            .unwrap()
                            .native_turn
                            .as_mut()
                            .unwrap()
                            .path[2][0] += 1;
                        assert!(bad.restore(&p).is_err());
                        if s.vm.threads[&id].callback.is_some() {
                            captured_callback = true;
                            assert!(s.advance_fixture_tick().is_err());
                            let mut bad = Snapshot::capture(&p, &s);
                            bad.state
                                .characters
                                .get_mut(&4)
                                .unwrap()
                                .native_turn
                                .as_mut()
                                .unwrap()
                                .physical = None;
                            assert!(bad.restore(&p).is_err());
                            let mut bad = sims_vm::Snapshot::capture(&p, &s.vm);
                            let sims_vm::Status::AwaitingTurn(request) = &mut bad
                                .state
                                .threads
                                .get_mut(&id)
                                .unwrap()
                                .callback
                                .as_mut()
                                .unwrap()
                                .resume_status
                            else {
                                panic!()
                            };
                            request.mode = if request.mode == 1 { 2 } else { 1 };
                            assert!(
                                bad.restore(&p).is_err(),
                                "suspended turn must retain its actual operands"
                            );
                        }
                    }
                    let animator = &s.characters[&4].character.skin.animator;
                    if s.characters[&4].controls.ordinary.mode == 4 {
                        let foreground = animator.slot(pose::animator::Slot::Foreground).unwrap();
                        assert_eq!(
                            animator.layer(foreground).unwrap().name,
                            if age < 18 { "child" } else { "adult" }
                        );
                        selected = true;
                    }
                    let mut resumed = restore(&p, &s);
                    assert_eq!(resumed, s);
                    service_empty_model(&mut s, true);
                    service_empty_model(&mut resumed, true);
                    environment(&mut s);
                    environment(&mut resumed);
                    assert_eq!(resumed, s);
                    if step.stop != StepStop::Budget {
                        if done {
                            break;
                        }
                        s.advance_fixture_tick().unwrap();
                    }
                }
                assert!(
                    done && selected && captured_pending,
                    "age={age} mode={mode} budget={budget} done={done} selected={selected} pending={captured_pending} tick={} status={:?} turn={:?} movement={:?} ordinary={:?}",
                    s.vm.tick,
                    s.vm.threads[&id].status,
                    s.characters[&4].native_turn,
                    s.characters[&4].controls.movement,
                    s.characters[&4].controls.ordinary
                );
                assert!(s.characters[&4].native_turn.is_none());
                assert_eq!(captured_callback, budget == 1);
                assert_eq!(
                    s.vm.world.objects[&4].temps[7],
                    initial.vm.world.objects[&4].temps[7] + 2
                );
                assert_eq!(s.vm.world.positions[&4], initial.vm.world.positions[&4]);
                outcomes.push(s);
            }
            assert_eq!(outcomes[0], outcomes[1], "age={age} mode={mode}");
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn snap_offset_publication_matches_native_transforms_without_ticking_animation() {
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../tests/fixtures/native-animator-transform.json"
    ))
    .unwrap();
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let (p, mut s) = fixture();
        let input = &row["input"];
        let expected = &row["original"];
        let size = input["size"].as_u64().unwrap() as usize;
        let corners: [u8; 4] = serde_json::from_value(input["corners"].clone()).unwrap();
        let heights = (0..size)
            .flat_map(|x| {
                (0..size).map(move |y| corners.map(|v| v.wrapping_add((x * 3 + y * 5) as u8)))
            })
            .collect::<Vec<_>>();
        let vm = s.vm.clone();
        let character = &mut s.characters.get_mut(&4).unwrap().character;
        character.placement = pose::transform::Placement {
            base: serde_json::from_value(input["base"].clone()).unwrap(),
            previous: serde_json::from_value(input["previous"].clone()).unwrap(),
            // Make replacement observable instead of starting with the supplied offsets.
            offset: [31., -17., 2.],
            dirty: input["dirty"].as_bool().unwrap(),
        };
        if input["reset"] == true {
            character.placement.invalidate();
        }
        let layers = character.skin.animator.layers().to_vec();
        let generation = character.skin.animator.pose_generation();
        let bones = character.skin.bones.clone();
        let hierarchy = character.hierarchy.clone();
        let clock = character.clock;
        character
            .offset_placement(
                serde_json::from_value(input["offset"].clone()).unwrap(),
                pose::transform::Source {
                    position: serde_json::from_value(input["position"].clone()).unwrap(),
                    facing: input["facing"].as_i64().unwrap() as i16,
                    level: input["level"].as_i64().unwrap() as i32,
                    flags: serde_json::from_value(input["flags"].clone()).unwrap(),
                },
                pose::transform::View {
                    rotation: input["rotation"].as_u64().unwrap() as u8,
                    floor_vector: serde_json::from_value(
                        input["floor_vectors"][input["zoom"].as_u64().unwrap() as usize].clone(),
                    )
                    .unwrap(),
                },
                pose::transform::Terrain {
                    size,
                    corners: &heights,
                },
            )
            .unwrap();
        for (key, actual) in [
            ("base", character.placement.base.as_slice()),
            ("previous", character.placement.previous.as_slice()),
            ("matrix", character.matrix.as_slice()),
        ] {
            for (i, (a, b)) in actual
                .iter()
                .zip(expected[key].as_array().unwrap())
                .enumerate()
            {
                let b = b.as_f64().unwrap() as f32;
                assert!((*a - b).abs() <= 3e-5, "case {case} {key}[{i}]: {a} / {b}");
            }
        }
        assert_eq!(character.skin.animator.layers(), layers);
        assert_eq!(character.skin.bones, bones);
        assert_eq!(character.hierarchy, hierarchy);
        assert_eq!(character.clock, clock);
        assert_eq!(
            character.skin.animator.pose_generation(),
            generation.wrapping_add(1)
        );
        assert_eq!(s.vm, vm);
        assert_eq!(restore(&p, &s), s);
    }
}

#[test]
fn native_automatic_carry_queries_the_first_logical_slot_not_the_head_attachment() {
    // 5903a3 -> 589fce reads a raw occupied word in physical slot 1, which
    // is logical slot 0 here. It does not look up the occupant in the registry.
    // The native carry corpus already validates selector/priority decisions;
    // this regression covers the live host's conversion of physical slot IDs.
    for held in [false, true] {
        for head_attachment in [false, true] {
            let (p, mut s) = fixture();
            s.vm.world.objects.get_mut(&4).unwrap().slots =
                vec![held.then_some(123), head_attachment.then_some(124), None];
            let mut step = crate::Step {
                stop: StepStop::Budget,
                trace: vec![],
                events: vec![],
            };
            let mut budget = 1;
            assert!(
                !s.drive_character_ordinary(&p, 4, true, &mut budget, &mut step)
                    .unwrap()
            );
            assert_eq!(budget, 0);
            assert_eq!(
                s.characters[&4].controls.ordinary.carry.mode,
                i32::from(held)
            );
            if held {
                // One more native request must install the carry animation,
                // regardless of whether the independent head slot is occupied.
                let saved = serde_json::to_vec(&s.characters[&4]).unwrap();
                s.characters
                    .insert(4, serde_json::from_slice(&saved).unwrap());
                assert!(
                    !s.drive_character_ordinary(&p, 4, true, &mut 1, &mut step)
                        .unwrap()
                );
                let r = &s.characters[&4];
                let animator = &r.character.skin.animator;
                let layer = animator
                    .layer(animator.slot(pose::animator::Slot::Carry).unwrap())
                    .unwrap();
                assert_eq!(layer.priority, 10);
                assert_eq!(layer.clock.mode, 0);
            } else {
                assert!(
                    s.characters[&4]
                        .character
                        .skin
                        .animator
                        .slot(pose::animator::Slot::Carry)
                        .is_none()
                );
            }
        }
    }
}

#[test]
fn native_stop_selection_loads_its_asset_and_resumes_the_stop_continuation() {
    use pose::animator::Slot;
    use serde_json::json;
    let (mut p, mut s) = fixture();
    // A suspended native StopSelect, after prepare_stop has published mode 7
    // and selection 19. The component's path/interrupt cases validate reaching
    // this boundary; this test exercises the live scheduler's response to it.
    let entry = ordinary::Entry {
        root_motion: 0,
        movement_input: 1,
        walk_style: 0,
    };
    let resident = s.characters.get_mut(&4).unwrap();
    resident.controls.ordinary.mode = 7;
    let mut animator = serde_json::to_value(&resident.character.skin.animator).unwrap();
    animator["selection_cache"] = json!(19);
    resident.character.skin.animator = serde_json::from_value(animator).unwrap();
    let mut movement = serde_json::to_value(&resident.controls.movement).unwrap();
    movement["pass"] = json!({"tick":0,"now":0,"step":33,"entry":entry,
        "root_motion":0,"stage":"StopSelect","pending":{"StopForeground":{"index":19}},
        "continuation":null});
    resident.controls.movement = serde_json::from_value(movement).unwrap();
    resident.ordinary_update = Some(
        serde_json::from_value(json!({
            "tick":0,"entry":entry,"only_visible":true,"phase":"Movement"
        }))
        .unwrap(),
    );
    let name = s.native_posture_name(&p, 4, 19).unwrap().unwrap();
    let asset = p.animations.remove(&name).unwrap();
    let mut step = crate::Step {
        stop: StepStop::Budget,
        trace: vec![],
        events: vec![],
    };
    let before = s.clone();
    assert!(
        s.drive_character_ordinary(&p, 4, true, &mut 1, &mut step)
            .unwrap_err()
            .contains("asset needs loading")
    );
    assert_eq!(s, before, "an unresolved stop clip must retain its request");
    assert_eq!(
        s.pending_animation_assets(&p).unwrap(),
        [name.clone()].into()
    );
    p.animations.insert(name.clone(), asset);
    let mut resumed = restore(&p, &s);
    for state in [&mut s, &mut resumed] {
        let mut budget = 1;
        assert!(
            !state
                .drive_character_ordinary(&p, 4, true, &mut budget, &mut step)
                .unwrap()
        );
        assert_eq!(budget, 0, "the stop selection must be serviced");
        let r = &state.characters[&4];
        assert_eq!(r.controls.movement.pending_request(), None);
        assert_eq!(r.controls.ordinary.mode, 7);
        let animator = &r.character.skin.animator;
        let layer = animator
            .layer(animator.slot(Slot::Foreground).unwrap())
            .unwrap();
        assert_eq!(layer.name, name);
        assert_eq!(state.vm.tick, 0);
    }
    assert_eq!(s, resumed);
    assert_eq!(s, restore(&p, &s));
}

#[test]
fn native_movement_base_loads_before_mutating_and_restores_its_handoff() {
    use pose::animator::Slot;
    use serde_json::json;
    for posture in [0, 1, 2] {
        for clock_mode in [0, 1] {
            let (mut p, mut s) = fixture();
            let entry = ordinary::Entry {
                root_motion: 0,
                movement_input: 0,
                walk_style: 0,
            };
            s.vm.world
                .objects
                .get_mut(&4)
                .unwrap()
                .person
                .as_mut()
                .unwrap()
                .insert(0, posture);
            let name = s
                .native_posture_name(
                    &p,
                    4,
                    match posture {
                        0 => 3,
                        1 => 1,
                        _ => 2,
                    },
                )
                .unwrap()
                .unwrap();
            let resident = s.characters.get_mut(&4).unwrap();
            resident.controls.ordinary.mode = 1;
            resident.controls.ordinary.clock_mode = clock_mode;
            resident.controls.ordinary.base_posture = -1;
            // A stale layer must not be faded until the new asset is ready.
            resident
                .character
                .skin
                .animator
                .start(
                    Slot::Base,
                    resident.source.as_ref().unwrap(),
                    Some(&p.animations[&name].clip),
                    false,
                    0,
                    resident.controls.settings,
                )
                .unwrap();
            let mut movement = serde_json::to_value(&resident.controls.movement).unwrap();
            movement["pass"] = json!({"tick":0,"now":0,"step":42,"entry":entry,
                "root_motion":0,"stage":"Base","pending":"BasePosture","continuation":null});
            resident.controls.movement = serde_json::from_value(movement).unwrap();
            resident.ordinary_update = Some(
                serde_json::from_value(json!({
                    "tick":0,"entry":entry,"only_visible":true,"phase":"Movement"
                }))
                .unwrap(),
            );
            let asset = p.animations.remove(&name).unwrap();
            let mut step = crate::Step {
                stop: StepStop::Budget,
                trace: vec![],
                events: vec![],
            };
            let before = s.clone();
            assert!(
                s.drive_character_ordinary(&p, 4, true, &mut 1, &mut step)
                    .unwrap_err()
                    .contains("asset needs loading")
            );
            assert_eq!(s, before);
            assert_eq!(
                s.pending_animation_assets(&p).unwrap(),
                [name.clone()].into()
            );
            p.animations.insert(name.clone(), asset);
            let mut resumed = restore(&p, &s);
            for state in [&mut s, &mut resumed] {
                let mut budget = 1;
                assert!(
                    !state
                        .drive_character_ordinary(&p, 4, true, &mut budget, &mut step)
                        .unwrap()
                );
                assert_eq!(budget, 0);
                let r = &state.characters[&4];
                assert_eq!(r.controls.movement.pending_request(), None);
                assert_eq!(r.controls.ordinary.base_posture, i32::from(posture));
                let animator = &r.character.skin.animator;
                let layer = animator.layer(animator.slot(Slot::Base).unwrap()).unwrap();
                assert_eq!(layer.name, name);
                assert_eq!(layer.priority, -10);
                assert_eq!(state.vm.tick, 0);
            }
            assert_eq!(s, resumed);
            assert_eq!(s, restore(&p, &s));
        }
    }
}

#[test]
fn native_route_interruption_uses_person_predicate_and_live_animator_gate() {
    let (_, mut s) = fixture();
    s.vm.world.actions = Some(sims_vm::action::WorldState {
        live_family: true,
        actors: [(4, Default::default())].into(),
        ..Default::default()
    });
    let thread = s.vm.object_threads[&4];
    s.vm.world
        .actions
        .as_mut()
        .unwrap()
        .actors
        .get_mut(&4)
        .unwrap()
        .queue
        .pending
        .push_back(sims_vm::action::Action {
            priority: 50,
            ..Default::default()
        });
    s.vm.world
        .objects
        .get_mut(&4)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .extend([(33, 0), (71, 0)]);
    s.characters.get_mut(&4).unwrap().controls.ordinary.mode = 3;
    assert!(s.route_interrupted(thread, 4).unwrap());
    // A legacy Playback map is empty here; the native mode still blocks the
    // interruption during an animation transition.
    s.characters.get_mut(&4).unwrap().controls.ordinary.mode = 4;
    assert!(!s.route_interrupted(thread, 4).unwrap());
    s.characters.get_mut(&4).unwrap().controls.ordinary.mode = 3;
    s.vm.threads.get_mut(&thread).unwrap().interrupted = true;
    s.vm.world
        .objects
        .get_mut(&4)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(71, 1);
    assert!(
        !s.route_interrupted(thread, 4).unwrap(),
        "legacy interrupt cannot bypass the native predicate"
    );
    s.vm.world
        .objects
        .get_mut(&4)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(71, 0);
    assert!(s.route_interrupted(thread, 4).unwrap());
    s.vm.world.actions.as_mut().unwrap().actors.remove(&4);
    assert!(
        s.route_interrupted(thread, 4).is_err(),
        "missing native queue cannot become an empty default"
    );
}

fn service_empty_model(s: &mut Simulation, visible: bool) -> usize {
    let mut count = 0;
    for (object, request) in s.pending_character_services() {
        // These are explicit boundaries for a fixture with no bound mesh or
        // appearance. Production geometry is tested with actual loaded avatars.
        let response = match request {
            refresh::Request::Viewport => refresh::Response::Viewport([0, 0, 640, 480]),
            refresh::Request::Visibility { .. } => refresh::Response::Visible(visible),
            refresh::Request::ModelGeometry
            | refresh::Request::ModelBounds
            | refresh::Request::Geometry { .. } => refresh::Response::Done,
            _ => panic!("unexpected empty-model service: {request:?}"),
        };
        // Reference the previous copy/acknowledge/commit path at each real
        // service boundary, including both initial and final visibility.
        let mut expected = s.clone();
        let mut character = expected.characters[&object].character.clone();
        character.acknowledge(response.clone()).unwrap();
        expected.characters.get_mut(&object).unwrap().character = character;
        s.complete_character_service(object, &request, response)
            .unwrap();
        assert_eq!(*s, expected, "reply changed more than the old transaction");
        count += 1;
    }
    count
}

#[test]
fn native_view_replies_preserve_rejection_atomicity_and_scheduling() {
    use refresh::{Request, Response};
    for only_visible in [false, true] {
        for visible in [false, true] {
            let (p, mut initial) = fixture();
            initial
                .native_character_settings
                .as_mut()
                .unwrap()
                .person
                .only_visible = only_visible;
            let mut outcomes = vec![];
            for budget in [1, 1000] {
                let mut s = initial.clone();
                let mut views = 0;
                let mut visibility = 0;
                for _ in 0..3 {
                    let mut finished = false;
                    for _ in 0..200 {
                        let step = s.run_fixture_tick(&p, budget).unwrap();
                        if step.stop != StepStop::Budget {
                            assert_eq!(step.stop, StepStop::Waiting);
                            finished = true;
                            break;
                        }
                        for (id, request) in s.pending_character_services() {
                            let (response, wrong_expected) = match request {
                                Request::Viewport => {
                                    views += 1;
                                    (
                                        Response::Viewport([0, 0, 640, 480]),
                                        Request::Visibility { viewport: [0; 4] },
                                    )
                                }
                                Request::Visibility { .. } => {
                                    visibility += 1;
                                    (Response::Visible(visible), Request::Viewport)
                                }
                                _ => continue,
                            };
                            let before = s.clone();
                            for wrong in [
                                Response::Done,
                                Response::Clock(99),
                                Response::PropertySurface(1),
                                Response::CensorBinding(Some(7)),
                                Response::HeadTarget(None),
                                Response::Placement(pose::model::Matrix::IDENTITY),
                            ] {
                                assert!(s.complete_character_service(id, &request, wrong).is_err());
                                assert_eq!(s, before);
                            }
                            assert!(
                                s.complete_character_service(id, &wrong_expected, response.clone())
                                    .is_err()
                            );
                            assert_eq!(s, before);
                            s.vm.tick += 1;
                            let stale = s.clone();
                            assert!(
                                s.complete_character_service(id, &request, response.clone())
                                    .is_err()
                            );
                            assert_eq!(s, stale);
                            s = restore(&p, &before);
                            let mut expected = before.clone();
                            let mut character = expected.characters[&id].character.clone();
                            character.acknowledge(response.clone()).unwrap();
                            expected.characters.get_mut(&id).unwrap().character = character;
                            s.complete_character_service(id, &request, response.clone())
                                .unwrap();
                            assert_eq!(s, expected);
                            assert_eq!(s.vm, before.vm, "reply advanced VM state");
                            assert!(
                                s.complete_character_service(id, &request, response)
                                    .is_err()
                            );
                            assert_eq!(s, expected);
                            s = restore(&p, &s);
                        }
                        service_empty_model(&mut s, visible);
                    }
                    assert!(finished);
                    s.advance_fixture_tick().unwrap();
                }
                assert_eq!(views, 3);
                assert_eq!(visibility, 3 * if only_visible { 2 } else { 1 });
                outcomes.push(s);
            }
            assert_eq!(
                outcomes[0], outcomes[1],
                "reply optimization changed budget scheduling"
            );
        }
    }
}

#[test]
#[ignore = "requires a private retained Mashuga snapshot; prints isolated reply timings"]
fn native_view_replies_measure_real_character_copy_cost() {
    use refresh::{Request, Response};
    use std::{
        hint::black_box,
        time::{Duration, Instant},
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
        "cache/neighborhood-input-experiments/mashuga-live-reset-profile-v75/original-live-character-final.json",
    )).unwrap()).unwrap();
    let snapshot: Snapshot = serde_json::from_value(value["simulation"].clone()).unwrap();
    let mut s = snapshot.state;
    // Retain the real layers, bindings and 29-bone skeletons, but enter a fresh
    // ordinary-tail viewport boundary. This is an isolated service benchmark,
    // not execution/restoration of the captured whole-house continuation.
    for id in [195, 196] {
        let resident = s.characters.get_mut(&id).unwrap();
        resident.refresh_tick = Some(s.vm.tick);
        resident
            .character
            .begin_ordinary_tail(
                resident.character.clock,
                true,
                false,
                resident.character.model.placement,
            )
            .unwrap();
        for expected_request in [
            Request::Viewport,
            Request::Visibility {
                viewport: [0, 0, 640, 480],
            },
        ] {
            let resident = s.characters.get_mut(&id).unwrap();
            let request = resident
                .character
                .next(
                    &Program::default(),
                    &mut Default::default(),
                    resident.context,
                    1,
                )
                .unwrap()
                .unwrap();
            assert_eq!(request, expected_request);
            let response = match request {
                Request::Viewport => Response::Viewport([0, 0, 640, 480]),
                Request::Visibility { .. } => Response::Visible(true),
                _ => unreachable!(),
            };
            let waiting = resident.clone();
            let mut expected = s.clone();
            let mut character = expected.characters[&id].character.clone();
            character.acknowledge(response.clone()).unwrap();
            expected.characters.get_mut(&id).unwrap().character = character;
            let mut measure = |copy: bool| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..5000 {
                    // Recreate the pending request outside the timed region.
                    s.characters.insert(id, waiting.clone());
                    let start = Instant::now();
                    if copy {
                        // Omit the common guards: this conservatively measures
                        // just the old copy/acknowledge/commit cost.
                        let mut c = black_box(&s.characters[&id].character).clone();
                        c.acknowledge(black_box(response.clone())).unwrap();
                        s.characters.get_mut(&id).unwrap().character = c;
                    } else {
                        s.complete_character_service(
                            black_box(id),
                            black_box(&request),
                            black_box(response.clone()),
                        )
                        .unwrap();
                    }
                    elapsed += start.elapsed();
                    black_box(&s.characters[&id]);
                }
                assert_eq!(s, expected);
                elapsed.as_nanos() as f64 / 5000.0
            };
            let (mut old, mut new) = (vec![], vec![]);
            for round in 0..5 {
                if round % 2 == 0 {
                    old.push(measure(true));
                    new.push(measure(false));
                } else {
                    new.push(measure(false));
                    old.push(measure(true));
                }
            }
            old.sort_by(f64::total_cmp);
            new.sort_by(f64::total_cmp);
            println!(
                "person {id}, {} bones, {request:?}: median {:.0} -> {:.0} ns/reply; five alternating 5000-call batches; setup excluded, timer overhead included, full resulting state identical",
                waiting.character.skin.bones.len(),
                old[2],
                new[2]
            );
        }
    }
}

#[test]
fn native_detached_refresh_services_placement_without_external_acknowledgment() {
    let (p, initial) = fixture();
    let mut outcomes = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        let mut finished = false;
        for _ in 0..100 {
            let mut remaining = budget;
            finished = s.drive_detached_character(&p, 4, &mut remaining).unwrap();
            assert!(
                s.pending_character_services()
                    .iter()
                    .all(|(_, r)| !matches!(r, refresh::Request::Placement))
            );
            service_empty_model(&mut s, true);
            s = restore(&p, &s);
            if finished {
                break;
            }
        }
        assert!(finished, "native forced refresh did not finish");
        assert_eq!(s.vm.tick, initial.vm.tick);
        assert_eq!(
            s.vm.threads, initial.vm.threads,
            "refresh executed script work"
        );
        assert!(s.characters[&4].refresh_tick.is_none());
        assert!(!s.characters[&4].character.busy());
        outcomes.push(s);
    }
    assert_eq!(
        outcomes[0], outcomes[1],
        "forced refresh depends on diagnostic budget"
    );
}

#[test]
fn native_deletion_reset_waits_for_its_saved_appearance_owner() {
    let (p, mut s) = fixture();
    s.vm.world.actions = Some(sims_vm::action::WorldState {
        live_family: true,
        actors: [(4, Default::default())].into(),
        ..Default::default()
    });
    s.removals.push(sims_vm::removal::Pending {
        target: 4,
        suppress_reset: true,
    });
    let mut deletion = crate::removal::Deletion::new(4);
    deletion.notified = true;
    s.deletion.push(deletion);
    let mut step = crate::Step {
        stop: StepStop::Waiting,
        trace: vec![],
        events: vec![],
    };
    assert!(
        !s.drive_deletion_person_reset(&p, &mut 1000, &mut step)
            .unwrap()
    );
    assert_eq!(s.pending_deletion_character_services(), vec![4]);
    assert!(s.advance_fixture_tick().is_err());
    let pending = restore(&p, &s);
    assert!(s.complete_deletion_character_service(&p, 4).is_err());
    assert_eq!(s, pending, "unstarted completion changed state");
    assert!(
        !s.drive_deletion_person_reset(&p, &mut 1000, &mut step)
            .unwrap()
    );
    assert_eq!(s, pending, "waiting repeated animator Reset");
    s.begin_deletion_character_service(4).unwrap();
    s = restore(&p, &s);
    assert!(s.begin_deletion_character_service(4).is_err());
    assert!(s.complete_deletion_character_service(&p, 3).is_err());
    let mut corrupt = Snapshot::capture(&p, &s);
    corrupt
        .state
        .deletion
        .last_mut()
        .unwrap()
        .person_reset
        .as_mut()
        .unwrap()
        .pending = None;
    assert!(corrupt.restore(&p).is_err(), "orphan asset token accepted");
    // Empty synthetic model: the real-asset host integration separately checks
    // native record and geometry operations before making this acknowledgment.
    s.complete_deletion_character_service(&p, 4).unwrap();
    assert!(s.pending_deletion_character_services().is_empty());
    assert!(s.complete_deletion_character_service(&p, 4).is_err());
    assert_eq!(s.vm.tick, pending.vm.tick);
    assert!(s.vm.world.objects.contains_key(&4));
    restore(&p, &s);

    // Registry unlink precedes asset destruction, but the saved deletion cursor
    // must keep the lot suspended until its sole asset owner acknowledges it.
    assert!(
        !s.drive_deletion_stack(&p, &mut 1000, &mut step, true)
            .unwrap()
    );
    assert!(!s.vm.world.objects.contains_key(&4));
    assert!(!s.characters.contains_key(&4));
    assert!(s.retired_characters.contains_key(&4));
    assert!(s.deletion.last().unwrap().storage_unlinked);
    // The normal scheduler wrapper publishes spatial caches after deletion;
    // this test calls the deletion driver directly.
    s.sync_spatial_world();
    s = restore(&p, &s);
    let waiting = s.clone();
    assert!(s.advance_fixture_tick().is_err());
    assert_eq!(s.run_fixture_tick(&p, 1000).unwrap().stop, StepStop::Budget);
    assert_eq!(s, waiting, "retired character allowed scheduler work");
    assert!(s.complete_character_retirement(4).is_err());
    assert!(s.begin_character_retirement(3).is_err());
    let mut corrupt = Snapshot::capture(&p, &s);
    corrupt.state.retired_characters.clear();
    assert!(
        corrupt.restore(&p).is_err(),
        "deletion lost its asset owner"
    );
    let mut corrupt = Snapshot::capture(&p, &s);
    corrupt.state.deletion.clear();
    assert!(
        corrupt.restore(&p).is_err(),
        "retired owner lost its deletion"
    );
    s.begin_character_retirement(4).unwrap();
    s = restore(&p, &s);
    assert!(s.begin_character_retirement(4).is_err());
    // The synthetic model owns no external assets. Real record/geometry/layer
    // release and its saved checkpoints are exercised by the live asset host.
    s.complete_character_retirement(4).unwrap();
    assert!(s.retired_characters.is_empty());
    assert!(s.deletion.last().unwrap().assets_released);
    s = restore(&p, &s);
    assert!(s.complete_character_retirement(4).is_err());
    assert!(
        s.drive_deletion_stack(&p, &mut 1000, &mut step, true)
            .unwrap()
    );
    assert!(s.deletion.is_empty());
    assert_eq!(s.vm.tick, pending.vm.tick);
}

#[test]
fn native_person_tick_resumes_tail_without_repeating_object_or_motive_updates() {
    for visible in [false, true] {
        let (p, mut initial) = fixture();
        initial
            .vm
            .world
            .objects
            .get_mut(&4)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .insert(32, 0);
        let mut saved_pairs = [[0; 2]; 6];
        saved_pairs[5] = [41, -3];
        initial.household.as_mut().unwrap().house_statistics = Some(
            crate::house_statistics::State::from_saved_pairs(saved_pairs),
        );
        let mut results = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut restored_services = 0;
            for tick in 0..62 {
                let mut finished = false;
                for _ in 0..200 {
                    let step = s.run_fixture_tick(&p, budget).unwrap();
                    assert_eq!(s.vm.tick, tick);
                    if step.stop != StepStop::Budget {
                        assert_eq!(step.stop, StepStop::Waiting);
                        finished = true;
                        break;
                    }
                    let mut restored = restore(&p, &s);
                    assert_eq!(s, restored);
                    let pending = s.pending_character_services();
                    if !pending.is_empty() {
                        let before = s.clone();
                        assert!(s.advance_fixture_tick().is_err());
                        assert_eq!(s, before);
                    }
                    restored_services += service_empty_model(&mut s, visible);
                    service_empty_model(&mut restored, visible);
                    assert_eq!(s, restored);
                    s = restored;
                }
                assert!(finished, "native person pass did not finish");
                let object = &s.vm.world.objects[&4];
                assert_eq!(object.data[&25], 99 - tick as i16);
                assert_eq!(object.person.as_ref().unwrap()[&27], 1 + tick as i16);
                assert_eq!(
                    s.household
                        .as_ref()
                        .unwrap()
                        .house_statistics
                        .as_ref()
                        .unwrap()
                        .pairs[5],
                    [(u32::MAX - 2).wrapping_add((tick / 10 + 1) as u32), 41]
                );
                let main = s.vm.object_threads[&4];
                assert_eq!(s.vm.threads[&main].stack[0].args, vec![99 - tick as i16]);
                assert_eq!(
                    object.motive_update.as_ref().unwrap().last_decay,
                    if tick < 60 { 0 } else { 60 }
                );
                assert_eq!(s.characters[&4].evaluated_tick, tick);
                assert!(s.characters[&4].ordinary_update.is_none());
                assert!(s.characters[&4].person_update.is_none());
                assert_eq!(
                    s.household.as_ref().unwrap().people[&4].finished,
                    Some(tick)
                );
                // Scale is applied on tick 0 and must not compound on later
                // ticks when feeding the model its placement transform.
                assert_eq!(s.characters[&4].character.model.scale.values[0], 1.5);
                let matrix = s.characters[&4].character.matrix;
                for start in [0, 4, 8] {
                    let length_squared: f32 = matrix[start..start + 3].iter().map(|x| x * x).sum();
                    assert!(
                        (length_squared - 2.25).abs() < 0.00001,
                        "body scale compounded at tick {tick}"
                    );
                }
                assert!(s.animations.is_empty());
                s.advance_fixture_tick().unwrap();
            }
            assert!(restored_services >= 6);
            results.push(s);
        }
        assert_eq!(results[0], results[1], "budget changed person scheduling");
    }
}

#[test]
fn native_person_asset_request_and_rejected_service_remain_resumable() {
    let (mut p, mut s) = fixture();
    let asset = p.animations.remove("test").unwrap();
    assert!(
        s.run_fixture_tick(&p, 1000)
            .unwrap_err()
            .contains("asset needs loading")
    );
    assert_eq!(
        s.pending_animation_assets(&p).unwrap(),
        ["test".into()].into()
    );
    let before = s.clone();
    let animator = person_update::Request::Animator { only_visible: true };
    assert!(
        s.complete_person_service(4, &animator, person_update::Response::Done)
            .is_err()
    );
    assert_eq!(s, before);
    s = restore(&p, &s);
    p.animations.insert("test".into(), asset);
    assert_eq!(s.run_fixture_tick(&p, 1000).unwrap().stop, StepStop::Budget);
    assert_eq!(s.vm.world.objects[&4].data[&25], 99);
    assert_eq!(
        s.pending_character_services(),
        vec![(4, pose::refresh::Request::Viewport)]
    );
}

#[test]
fn native_household_cannot_run_an_uninitialized_person_through_legacy_animation() {
    let (p, mut s) = fixture();
    s.characters.remove(&4);
    let before = s.vm.clone();
    assert!(
        s.run_fixture_tick(&p, 1000)
            .unwrap_err()
            .contains("requires character initialization")
    );
    assert_eq!(s.vm.threads, before.threads);
    assert_eq!(s.vm.world.objects, before.world.objects);
    assert_eq!(s.vm.tick, before.tick);
    assert!(s.animations.is_empty());
}
