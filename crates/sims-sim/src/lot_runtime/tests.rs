use super::*;
use crate::{
    lot_geometry::{FloorSource, Lot},
    lot_links::Links,
};
use sims_vm::{
    Definition, Module, Object, Program, World, autonomy::Placement, person::WorldState,
};
pub(crate) fn fixture() -> (Program, Simulation) {
    let mut m = Module {
        source: "objects".into(),
        ..Default::default()
    };
    let mut words = vec![0; 128];
    words[9] = 4;
    words[14] = 1;
    words[67] = 8;
    m.object_definition_words.insert(1, words);
    m.object_functions.insert(
        1,
        vec![
            sims_formats::execution::ObjectFunction {
                action: 0,
                condition: 0
            };
            27
        ],
    );
    m.object_functions.get_mut(&1).unwrap()[1].action = 4096;
    m.routines.insert(
        4096,
        sims_vm::Routine {
            name: "Main".into(),
            body: sims_formats::behavior::Behavior {
                format: 0x8002,
                kind: 0,
                args: 0,
                locals: 0,
                instructions: vec![sims_formats::behavior::Instruction {
                    opcode: 2,
                    on_true: 254,
                    on_false: 253,
                    operands: [0, 0, 0, 0, 0, 5, 8, 8],
                }],
            },
        },
    );
    let p = Program {
        globals: "objects".into(),
        modules: [("objects".into(), m)].into(),
        ..Default::default()
    };
    let source = FloorSource {
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
        vec![source.clone(), source],
    )
    .unwrap();
    let architecture = Architecture::from_original(lot, &p).unwrap();
    let mut world = World {
        person: Some(WorldState {
            object_order: vec![2, 4, 1, 3],
            ..Default::default()
        }),
        ..Default::default()
    };
    for id in 1..=4 {
        let mut data: std::collections::BTreeMap<_, _> = (0..72).map(|i| (i, 0)).collect();
        data.extend([
            (3, -1),
            (14, if id == 2 || id == 3 { 0 } else { -1 }),
            (
                26,
                match id {
                    2 => 3,
                    3 => 1,
                    _ => 0,
                },
            ),
            (29, if id == 4 { -5 } else { 0 }),
        ]);
        world.objects.insert(
            id,
            Object {
                definition: Some(Definition {
                    source: "objects".into(),
                    object: 1,
                    guid: 1,
                    slots: 0,
                }),
                data,
                ..Default::default()
            },
        );
        world.positions.insert(
            id,
            if id == 4 {
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
    links.publish_tile_chains(&mut world).unwrap();
    let placement = lot_placement::State::new(links, &architecture).unwrap();
    let native = State {
        placement,
        membership: RoomMembership::constructed(&world),
        indicators: Default::default(),
        observers: vec![],
        diagnostics: crate::lot_acceptance::person::Diagnostics {
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
    let mut vm = Vm::new(world, 1);
    vm.start_object_lifecycle(&p, 4).unwrap();
    let state = Simulation::from_loaded_lot(vm, architecture, native).unwrap();
    (p, state)
}

fn terminal_route_fixture() -> (
    Program,
    Simulation,
    sims_vm::ThreadId,
    sims_vm::RouteRequest,
) {
    let (program, mut state) = walking_fixture();
    state.vm.tick = 73;
    state
        .vm
        .world
        .objects
        .get_mut(&1)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(0, 1);
    let thread = state.vm.object_threads[&1];
    state.vm.run_slice(&program, thread, 1000).unwrap();
    let sims_vm::Status::AwaitingRoute(request) = state.vm.threads[&thread].status.clone() else {
        panic!("synthetic native route request absent");
    };
    state.visited.insert(thread);
    state.visit = Some((thread, Some(1)));
    let mut events = vec![];
    assert!(
        !state
            .service_original_portals(&program, thread, &request, &mut events)
            .unwrap()
    );
    (program, state, thread, request)
}

#[test]
fn terminal_observations_native_cleanup_reports_explicit_outcome_without_changing_execution() {
    for (success, native_result) in [(false, 0), (false, 7), (true, 0)] {
        for budget in [1, 1000] {
            let (program, mut observed, thread, request) = terminal_route_fixture();
            observed
                .begin_native_route_completion(thread, &request, success, native_result)
                .unwrap();
            let mut reference = observed.clone();
            observed.enable_terminal_observations(4);
            let expected_result = if success { 0 } else { native_result.max(1) };
            let mut done = false;
            for _ in 0..40 {
                let mut step = crate::Step {
                    stop: crate::StepStop::Waiting,
                    trace: vec![],
                    events: vec![],
                };
                let mut reference_step = step.clone();
                let mut remaining = budget;
                let mut reference_remaining = budget;
                done = observed
                    .drive_native_route_completion(&program, &mut remaining, &mut step)
                    .unwrap();
                assert_eq!(
                    done,
                    reference
                        .drive_native_route_completion(
                            &program,
                            &mut reference_remaining,
                            &mut reference_step
                        )
                        .unwrap()
                );
                assert_eq!(remaining, reference_remaining);
                assert_eq!(step, reference_step);
                assert_eq!(observed, reference);
                assert_eq!(
                    serde_json::to_vec(&observed).unwrap(),
                    serde_json::to_vec(&reference).unwrap()
                );
                let saved = crate::Snapshot::capture(&program, &observed);
                assert!(!saved.state.route_terminal_observations().enabled());
                assert!(!saved.state.vm.terminal_observations().enabled());
                assert!(
                    saved
                        .state
                        .route_terminal_observations()
                        .events()
                        .is_empty()
                );
                assert!(observed.vm.terminal_observations().events().is_empty());
                if done {
                    break;
                }
                assert!(observed.route_terminal_observations().events().is_empty());
            }
            assert!(done);
            assert!(!reference.route_terminal_observations().enabled());
            assert!(reference.drain_route_terminal_observations().is_empty());
            assert_eq!(
                observed.drain_route_terminal_observations(),
                [crate::observations::NativeRouteTerminalEvent {
                    actor: request.caller,
                    target: request.target,
                    request: request.id,
                    thread,
                    action_serial: None,
                    action_invocation: None,
                    tick: 73,
                    success,
                    native_result: expected_result,
                    endpoint: observed.vm.world.positions.get(&request.caller).copied(),
                }]
            );
            assert_eq!(observed.route_terminal_observations().dropped(), 0);
            let mut step = crate::Step {
                stop: crate::StepStop::Waiting,
                trace: vec![],
                events: vec![],
            };
            assert!(
                observed
                    .drive_native_route_completion(&program, &mut 1000, &mut step)
                    .unwrap()
            );
            assert!(observed.drain_route_terminal_observations().is_empty());
            assert_eq!(observed.vm.tick, 73);
        }
    }
}

#[test]
fn terminal_observations_native_cleanup_drops_are_visible_and_clones_are_disabled() {
    let (program, mut state, thread, request) = terminal_route_fixture();
    state
        .begin_native_route_completion(thread, &request, false, 7)
        .unwrap();
    state.enable_terminal_observations(0);
    let mut preview = state.clone();
    let mut step = crate::Step {
        stop: crate::StepStop::Waiting,
        trace: vec![],
        events: vec![],
    };
    assert!(
        preview
            .drive_native_route_completion(&program, &mut 1000, &mut step)
            .unwrap()
    );
    assert!(!preview.route_terminal_observations().enabled());
    assert_eq!(preview.route_terminal_observations().dropped(), 0);
    assert_eq!(state.route_terminal_observations().dropped(), 0);
    assert!(state.native_lot.as_ref().unwrap().completion.is_some());
    assert!(
        state
            .drive_native_route_completion(&program, &mut 1000, &mut step)
            .unwrap()
    );
    assert!(state.drain_route_terminal_observations().is_empty());
    assert_eq!(state.route_terminal_observations().dropped(), 1);
    assert_eq!(state, preview);
    let saved = serde_json::to_vec(&state).unwrap();
    let restored: Simulation = serde_json::from_slice(&saved).unwrap();
    assert_eq!(restored, state);
    assert!(!restored.route_terminal_observations().enabled());
    assert_eq!(restored.route_terminal_observations().dropped(), 0);
    state.disable_terminal_observations();
    assert!(!state.route_terminal_observations().enabled());
    assert!(!state.vm.terminal_observations().enabled());
}

#[test]
fn terminal_observations_native_cleanup_rejects_stale_request_credit() {
    let (program, mut state, thread, mut request) = terminal_route_fixture();
    state.enable_terminal_observations(4);
    request.id += 1;
    state
        .begin_native_route_completion(thread, &request, false, 7)
        .unwrap();
    let mut step = crate::Step {
        stop: crate::StepStop::Waiting,
        trace: vec![],
        events: vec![],
    };
    assert!(
        state
            .drive_native_route_completion(&program, &mut 1000, &mut step)
            .is_err()
    );
    assert!(state.route_terminal_observations().events().is_empty());
    assert_eq!(state.route_terminal_observations().dropped(), 0);
}
#[cfg(feature = "private-fixtures")]
#[test]
fn picker_low_flags_use_original_group_scope_and_invalidation() {
    let data: serde_json::Value = serde_json::from_str(include_str!(
        "../../../sims-vm/tests/fixtures/person-selection.json"
    ))
    .unwrap();
    let mut count = 0;
    for row in data["flag_cases"].as_array().unwrap() {
        let input = &row["input"];
        if input
            .get("mutations")
            .and_then(|v| v.as_array())
            .is_some_and(|v| !v.is_empty())
        {
            continue;
        }
        let (mut p, mut s) = fixture();
        let scope = input["scope"].as_str().unwrap();
        let module = p.modules.get_mut("objects").unwrap();
        module.object_definition_words.get_mut(&1).unwrap()[10] = i16::from(scope != "ordinary");
        module.object_functions.get_mut(&1).unwrap()[8].action =
            if scope == "split" { 4096 } else { 0 };
        for id in 1..=4 {
            let object = s.vm.world.objects.get_mut(&id).unwrap();
            object.native_flags = Some(input["flags"][id as usize - 1].as_u64().unwrap() as u32);
            if scope != "ordinary" {
                object.multitile = Some(sims_vm::multitile::Part {
                    head: (id == 2 || id == 3).then_some(1),
                    next: (id < 3).then_some(id + 1),
                    origin: [0; 3],
                    offset: [0; 3],
                });
            }
        }
        s.vm.world.refresh = Some(sims_vm::refresh::RefreshState {
            visible_level: 1,
            render_clock: 77,
            draw: (1..=4)
                .map(|id| {
                    (
                        id,
                        sims_vm::refresh::DrawState {
                            level: 1,
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            ..Default::default()
        });
        s.update_native_low_flags(
            &p,
            input["target"].as_i64().unwrap() as i16,
            input["requested"].as_u64().unwrap() as u32,
        )
        .unwrap();
        let flags: Vec<_> = (1..=4)
            .map(|id| s.vm.world.objects[&id].native_flags.unwrap())
            .collect();
        assert_eq!(
            serde_json::json!(flags),
            row["original"]["flags"],
            "{input}"
        );
        let invalidated: std::collections::BTreeSet<_> = row["original"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e[0] == "invalidate")
            .map(|e| e[1].as_i64().unwrap() as i16)
            .collect();
        let refresh = s.vm.world.refresh.as_ref().unwrap();
        assert_eq!(refresh.invalidated, invalidated, "{input}");
        for (id, draw) in &refresh.draw {
            assert_eq!(draw.dynamic, invalidated.contains(id));
            if draw.dynamic {
                assert_eq!((draw.expiry, draw.refreshed_at), (i32::MAX, 77));
            }
        }
        assert_eq!(s.vm.tick, 0);
        count += 1;
    }
    assert!(count >= 1000);
    println!("{count} native low-flag cases through the actual simulation adapter");
}

#[cfg(feature = "private-fixtures")]
#[test]
fn creation_rotation_matches_native_arithmetic_observers_and_callback_reads() {
    use serde_json::{Value, json};
    let report: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/creation-rotation.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 906);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let (mut p, mut s) = fixture();
        let initial = c["initial"].as_i64().unwrap() as i16;
        s.vm.world
            .objects
            .get_mut(&4)
            .unwrap()
            .data
            .extend([(1, initial), (23, c["step"].as_i64().unwrap() as i16)]);
        s.vm.world.positions.insert(
            4,
            Placement {
                x: -16,
                y: -16,
                level: 1,
            },
        );
        p.modules
            .get_mut("objects")
            .unwrap()
            .object_functions
            .get_mut(&1)
            .unwrap()[26]
            .action = if c["adjacent"].as_bool().unwrap() {
            4096
        } else {
            0
        };
        s.vm.world.objects.get_mut(&1).unwrap().data.insert(1, 7);
        s.vm.world
            .objects
            .get_mut(&2)
            .unwrap()
            .data
            .insert(1, c["source"].as_i64().unwrap() as i16);
        let creator = s.vm.object_threads[&4];
        s.vm.threads
            .get_mut(&creator)
            .unwrap()
            .stack
            .last_mut()
            .unwrap()
            .stack_object = match c["after_target"].as_i64().unwrap() {
            3 => 4, // The native probe's newly allocated object is #3.
            other => other as i16,
        };
        let mut events = vec![];
        if let Some(rotation) =
            crate::lot_creation::live::copied_rotation(&s.vm, creator, 4).unwrap()
        {
            let lot_placement::VirtualOperation::Base(mut op) = rotation.operation else {
                panic!("ordinary object selected multipart rotation")
            };
            while let Some(event) = op
                .next(
                    &mut s.vm.world,
                    &p,
                    s.architecture.as_ref().unwrap(),
                    &mut s.native_lot.as_mut().unwrap().placement,
                )
                .unwrap()
            {
                let direction = s.vm.world.objects[&4].data[&1];
                events.push(match event {
                    lot_placement::Request::Invalidate {
                        object: 4,
                        children_only: false,
                    } => json!({"kind":"invalidate","direction":direction}),
                    lot_placement::Request::Adjacency(tile) => {
                        json!({"kind":"adjacency","direction":direction,"tile":tile})
                    }
                    lot_placement::Request::Walls(4) => {
                        json!({"kind":"walls","direction":direction})
                    }
                    other => panic!("unexpected rotation event {other:?}"),
                });
                if c["mutate"] == true
                    && matches!(event, lot_placement::Request::Adjacency(_))
                    && events.iter().filter(|e| e["kind"] == "adjacency").count() == 1
                {
                    s.vm.world.objects.get_mut(&4).unwrap().data.insert(1, 5);
                    s.vm.world.positions.insert(
                        4,
                        Placement {
                            x: 40,
                            y: 56,
                            level: 1,
                        },
                    );
                }
                // Every observer is a serializable continuation boundary.
                op = serde_json::from_slice(&serde_json::to_vec(&op).unwrap()).unwrap();
            }
        }
        assert_eq!(json!(events), row["original"]["events"], "{c}");
        assert_eq!(
            json!(s.vm.world.objects[&4].data[&1]),
            row["original"]["direction"],
            "{c}"
        );
        assert_eq!(row["original"]["returned"], 1, "{c}");
        assert_eq!(row["original"]["stack_object"], 3, "{c}");
    }
}

#[test]
fn offlot_creation_rotation_resumes_wall_callback_without_repeating_factory() {
    use sims_formats::behavior::Instruction;
    let (mut p, mut initial) = fixture();
    let m = p.modules.get_mut("objects").unwrap();
    let template = m.routines[&4096].clone();
    let expr = |dest: u8, op: u8, scope: u8, value: u8, next: u8| Instruction {
        opcode: 2,
        on_true: next,
        on_false: 255,
        operands: [dest, 0, value, 0, 0, op, scope, 7],
    };
    for (id, instructions) in [
        (
            4096,
            vec![Instruction {
                opcode: 42,
                on_true: 254,
                on_false: 255,
                operands: [2, 0, 0, 0, 6, 32, 0, 0],
            }],
        ),
        (4097, vec![expr(23, 5, 3, 2, 1), expr(5, 5, 3, 1, 254)]),
        (4098, vec![expr(0, 5, 8, 0, 254)]),
        (4099, vec![expr(34, 3, 6, 1, 1), expr(34, 3, 6, 10, 254)]),
    ] {
        let mut r = template.clone();
        r.body.instructions = instructions;
        m.routines.insert(id, r);
    }
    let mut words = m.object_definition_words[&1].clone();
    words[14] = 2;
    m.object_definition_words.insert(2, words);
    let mut f = m.object_functions[&1].clone();
    f[0].action = 4097;
    f[1].action = 4098;
    f[6].action = 4099;
    m.object_functions.insert(2, f);
    for id in [1, 5, 6] {
        m.named_constants.insert(
            id,
            sims_formats::tuning::Constants {
                version: 1,
                entries: vec![],
            },
        );
    }
    for id in [501, 502, 503, 504] {
        m.strings.insert(id, vec![String::new(); 9]);
    }
    initial
        .architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&p)
        .unwrap();
    initial
        .vm
        .world
        .globals
        .extend([(15, 0), (32, 0), (33, 0), (34, 0), (36, 0)]);
    initial.vm.world.refresh = Some(Default::default());
    initial
        .enable_household(
            &p,
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
    let creator = initial.vm.object_threads[&4];
    initial
        .vm
        .threads
        .get_mut(&creator)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .stack_object = 4;
    initial
        .vm
        .world
        .objects
        .get_mut(&4)
        .unwrap()
        .data
        .insert(1, 6);
    let mut outcomes = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        let mut saved_rotation = false;
        for slice in 0..100 {
            let step = s.run_fixture_tick(&p, budget).unwrap();
            assert_ne!(step.stop, crate::StepStop::Faulted, "{:?}", s.vm.threads);
            if !s.native_creations.is_empty() {
                saved_rotation = true;
                assert!(s.advance_fixture_tick().is_err());
                let mut bad = crate::Snapshot::capture(&p, &s);
                bad.state.native_creations.clear();
                assert!(
                    bad.restore(&p).is_err(),
                    "orphan rotation callback accepted"
                );
            }
            let bytes = serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap();
            s = serde_json::from_slice::<crate::Snapshot>(&bytes)
                .unwrap()
                .restore(&p)
                .unwrap_or_else(|e| panic!("rotation snapshot budget {budget} slice {slice}: {e}"));
            if step.stop != crate::StepStop::Budget {
                break;
            }
            assert!(slice < 99);
        }
        assert_eq!(saved_rotation, budget == 1);
        assert_eq!(s.vm.world.objects[&5].data[&1], 6);
        assert_eq!(s.vm.world.globals[&34], 11);
        assert_eq!(
            s.vm.world.positions[&5],
            Placement {
                x: -16,
                y: -16,
                level: 1
            }
        );
        assert_eq!(
            s.native_lot
                .as_ref()
                .unwrap()
                .observers
                .iter()
                .filter(|r| matches!(
                    r,
                    lot_placement::Request::Signal {
                        event: 0xe2,
                        value: 5
                    }
                ))
                .count(),
            1
        );
        outcomes.push(s);
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[test]
fn multipart_creation_resumes_all_init_main_and_rotation_callbacks() {
    use sims_formats::behavior::Instruction;
    let (mut p, mut initial) = fixture();
    let m = p.modules.get_mut("objects").unwrap();
    let template = m.routines[&4096].clone();
    let expr = |dest: u8, op: u8, scope: u8, value: u8, next: u8| Instruction {
        opcode: 2,
        on_true: next,
        on_false: 255,
        operands: [dest, 0, value, 0, 0, op, scope, 7],
    };
    for (id, instructions) in [
        (
            4096,
            vec![Instruction {
                opcode: 42,
                on_true: 254,
                on_false: 255,
                operands: [2, 0, 0, 0, 6, 32, 0, 0],
            }],
        ),
        (4097, vec![expr(23, 5, 3, 2, 1), expr(34, 3, 6, 1, 254)]),
        (4098, vec![expr(0, 5, 8, 0, 254)]),
        (4099, vec![expr(34, 3, 6, 1, 1), expr(34, 3, 6, 10, 254)]),
    ] {
        let mut r = template.clone();
        r.body.instructions = instructions;
        m.routines.insert(id, r);
    }
    for (number, index) in [(2, -1), (3, 0), (4, 1), (5, 2)] {
        let mut words = m.object_definition_words[&1].clone();
        words[10] = 7;
        words[11] = index;
        words[14] = number;
        words[18] = if index == -1 { 999 } else { 50 };
        words[63] = i16::from(index == 0);
        m.object_definition_words.insert(number as u16, words);
        m.object_definition_order.push(number as u16);
        let mut f = m.object_functions[&1].clone();
        f[0].action = 4097;
        f[1].action = 4098;
        f[10].action = 4099;
        m.object_functions.insert(number as u16, f);
    }
    for id in [1, 5, 6] {
        m.named_constants.insert(
            id,
            sims_formats::tuning::Constants {
                version: 1,
                entries: vec![],
            },
        );
    }
    for id in [501, 502, 503, 504] {
        m.strings.insert(id, vec![String::new(); 9]);
    }
    initial
        .architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&p)
        .unwrap();
    initial
        .vm
        .world
        .globals
        .extend([(15, 0), (32, 0), (33, 0), (34, 0), (36, 0)]);
    initial.vm.world.refresh = Some(Default::default());
    initial
        .enable_household(
            &p,
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
    let creator = initial.vm.object_threads[&4];
    initial
        .vm
        .threads
        .get_mut(&creator)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .stack_object = 4;
    initial
        .vm
        .world
        .objects
        .get_mut(&4)
        .unwrap()
        .data
        .insert(1, 6);
    let mut outcomes = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        let mut saved_rotation = false;
        let mut phases = std::collections::BTreeSet::new();
        for slice in 0..200 {
            let step = s.run_fixture_tick(&p, budget).unwrap();
            assert_ne!(step.stop, crate::StepStop::Faulted, "{:?}", s.vm.threads);
            if !s.native_creations.is_empty() {
                saved_rotation = true;
                for e in s.native_creations.values() {
                    phases.insert(
                        serde_json::to_value(e).unwrap()["phase"]
                            .as_str()
                            .unwrap()
                            .to_owned(),
                    );
                }
                assert!(s.advance_fixture_tick().is_err());
                let mut bad = crate::Snapshot::capture(&p, &s);
                bad.state.native_creations.clear();
                assert!(
                    bad.restore(&p).is_err(),
                    "orphan multipart factory accepted"
                );
            }
            let bytes = serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap();
            s = serde_json::from_slice::<crate::Snapshot>(&bytes)
                .unwrap()
                .restore(&p)
                .unwrap_or_else(|e| panic!("rotation snapshot budget {budget} slice {slice}: {e}"));
            if step.stop != crate::StepStop::Budget {
                break;
            }
            assert!(slice < 199);
        }
        assert_eq!(saved_rotation, budget == 1);
        if budget == 1 {
            assert_eq!(
                phases,
                ["GroupFactory".to_owned(), "GroupRotate".to_owned()].into()
            );
        }
        assert_eq!(s.vm.world.objects[&5].data[&1], 6);
        // Three Init increments, then nine two-instruction detach callbacks.
        assert_eq!(s.vm.world.globals[&34], 102);
        assert_eq!(s.vm.world.objects[&5].data[&41], 999);
        assert_eq!(
            sims_vm::multitile::chain(&s.vm.world, 5).unwrap(),
            vec![5, 6, 7]
        );
        for id in [5, 6, 7] {
            assert_eq!(s.vm.world.objects[&id].data[&1], 6);
            assert!(s.unplaced.contains(&id));
        }
        assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, 1);
        assert_eq!(
            s.vm.world.positions[&5],
            Placement {
                x: -16,
                y: -16,
                level: 1
            }
        );
        assert_eq!(
            s.native_lot
                .as_ref()
                .unwrap()
                .observers
                .iter()
                .filter(|r| matches!(
                    r,
                    lot_placement::Request::Signal {
                        event: 0xe2,
                        value: 5
                    }
                ))
                .count(),
            1
        );
        outcomes.push(s);
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[test]
fn native_removal_detaches_before_queue_and_resumes_child_callbacks() {
    use sims_formats::behavior::Instruction;
    for contained in [false, true] {
        let (mut p, mut initial) = fixture();
        if contained {
            initial.vm.world.objects.get_mut(&1).unwrap().slots = vec![None; 2];
            initial
                .vm
                .world
                .objects
                .get_mut(&1)
                .unwrap()
                .definition
                .as_mut()
                .unwrap()
                .slots = 1;
            let mut step = crate::Step {
                stop: crate::StepStop::Waiting,
                trace: vec![],
                events: vec![],
            };
            for (slot, id) in [2, 3].into_iter().enumerate() {
                let mut operation =
                    lot_placement::live::Execution::new(lot_placement::Operation::set(
                        id,
                        crate::lot_links::Descriptor {
                            on_lot: true,
                            position: [40, 40],
                            container: Some(1),
                            slot: slot as i32,
                            level: 1,
                        },
                        true,
                    ));
                assert!(
                    initial
                        .drive_live_placement(&p, &mut operation, &mut 1000, &mut step)
                        .unwrap()
                );
            }
        }
        let m = p.modules.get_mut("objects").unwrap();
        m.routines.insert(4099, m.routines[&4096].clone());
        m.slots.insert(
            1,
            sims_formats::execution::Slots {
                version: 10,
                entries: vec![
                    sims_formats::execution::Slot {
                        kind: 0,
                        offset: [0.0; 3],
                        standing: 0,
                        sitting: 0,
                        ground: 0,
                        flags: 0,
                        snap_target: -1,
                        proximity: [16; 3],
                        max_size: 0,
                        unknown: 0,
                        gradient: 0.0,
                        height: 0,
                        facing: -2,
                        resolution: 16
                    };
                    2
                ],
            },
        );
        let mut remove = m.routines[&4096].clone();
        remove.body.instructions = vec![
            Instruction {
                opcode: 0x12,
                on_true: 1,
                on_false: 255,
                operands: [1, 0, 0, 0, 0, 0, 0, 0],
            },
            Instruction {
                opcode: 2,
                on_true: 254,
                on_false: 255,
                operands: [7, 0, 1, 0, 0, 3, 8, 7],
            },
        ];
        m.routines.insert(4096, remove);
        for (function, routine, global) in [(10, 4097, 34), (9, 4098, 36)] {
            m.object_functions.get_mut(&1).unwrap()[function].action = routine;
            let mut callback = m.routines[&4096].clone();
            callback.body.instructions = vec![
                Instruction {
                    opcode: 2,
                    on_true: 1,
                    on_false: 255,
                    operands: [global, 0, 1, 0, 0, 3, 6, 7],
                },
                Instruction {
                    opcode: 2,
                    on_true: 254,
                    on_false: 255,
                    operands: [0, 0, 0, 0, 0, 5, 8, 8],
                },
            ];
            m.routines.insert(routine, callback);
        }
        let mut words = m.object_definition_words[&1].clone();
        words[14] = 2;
        m.object_definition_words.insert(2, words);
        let mut functions = m.object_functions[&1].clone();
        functions[1].action = 4099;
        m.object_functions.insert(2, functions);
        for id in 1..=3 {
            let definition = initial
                .vm
                .world
                .objects
                .get_mut(&id)
                .unwrap()
                .definition
                .as_mut()
                .unwrap();
            definition.object = 2;
            definition.guid = 2;
            initial.vm.start_object_lifecycle(&p, id).unwrap();
        }
        initial.architecture = Some(
            Architecture::from_original(
                initial
                    .architecture
                    .as_ref()
                    .unwrap()
                    .original
                    .as_ref()
                    .unwrap()
                    .clone(),
                &p,
            )
            .unwrap(),
        );
        initial.vm.world.globals.extend([(34, 0), (36, 0)]);
        let actor = initial.vm.object_threads[&4];
        initial
            .vm
            .threads
            .get_mut(&actor)
            .unwrap()
            .stack
            .last_mut()
            .unwrap()
            .stack_object = 1;
        let mut outcomes = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut deletion_events = vec![];
            let mut primitive_checkpoints = 0;
            let mut child_checkpoints = 0;
            for checkpoint in 0..100 {
                let step = s.run_fixture_tick(&p, budget).unwrap();
                deletion_events.extend(
                    step.events
                        .iter()
                        .filter(|e| e.detail.starts_with("deleted #"))
                        .map(|e| e.detail.clone()),
                );
                if let Some(e) = s.native_lot.as_ref().unwrap().removal.as_ref() {
                    match e.origin {
                        crate::lot_removal::Origin::Primitive { .. } => {
                            primitive_checkpoints += 1;
                            assert!(s.removals.is_empty());
                        }
                        crate::lot_removal::Origin::Child { .. } => {
                            child_checkpoints += 1;
                            assert_eq!(s.removals[0].target, 1);
                        }
                    }
                    assert!(s.advance_fixture_tick().is_err());
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    bad.state.native_lot.as_mut().unwrap().removal = None;
                    assert!(bad.restore(&p).is_err());
                }
                let snapshot = crate::Snapshot::capture(&p, &s);
                s = serde_json::from_slice::<crate::Snapshot>(
                    &serde_json::to_vec(&snapshot).unwrap(),
                )
                .unwrap()
                .restore(&p)
                .unwrap_or_else(|error| {
                    panic!("removal snapshot {contained}/{budget}/{checkpoint}: {error}")
                });
                if step.stop != crate::StepStop::Budget {
                    break;
                }
                assert!(checkpoint < 99);
            }
            assert_eq!(primitive_checkpoints > 0, budget == 1);
            assert_eq!(child_checkpoints > 0, budget == 1 && contained);
            // Each deletion's base Reset repeats virtual RemoveFromLot,
            // even when the deferred mark already detached the object.
            assert_eq!(s.vm.world.globals[&34], if contained { 6 } else { 2 });
            assert_eq!(s.vm.world.objects[&4].temps[7], 1);
            assert_eq!(
                s.vm.world.objects.keys().copied().collect::<Vec<_>>(),
                if contained { vec![4] } else { vec![2, 3, 4] }
            );
            let order = if contained { vec![2, 3, 1] } else { vec![1] };
            assert_eq!(
                deletion_events,
                order
                    .into_iter()
                    .map(|id| format!("deleted #{id} after lot object updates"))
                    .collect::<Vec<_>>()
            );
            assert!(s.removals.is_empty() && s.deletion.is_empty());
            outcomes.push(s);
        }
        assert_eq!(outcomes[0], outcomes[1]);
    }
}
#[test]
fn scheduler_retains_physical_order_and_exact_offlot_coordinates() {
    let (p, mut s) = fixture();
    let before = s.vm.world.clone();
    s.run_fixture_tick(&p, 1).unwrap();
    assert_eq!(s.vm.world.positions, before.positions);
    assert_eq!(s.vm.world.person, before.person);
    assert_eq!(
        s.vm.world.person.as_ref().unwrap().tile_chains,
        vec![vec![1, 3, 2]]
    );
    assert_eq!(
        s.vm.world.positions[&4],
        Placement {
            x: -32,
            y: -16,
            level: 2
        }
    );
    let restored = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
    assert_eq!(restored, s);
}
#[test]
fn native_self_removal_yields_without_running_true_branch_unless_requested() {
    use sims_formats::behavior::Instruction;
    for placed in [false, true] {
        for flags in 0..=3 {
            let (mut p, mut initial) = fixture();
            if placed {
                let mut step = crate::Step {
                    stop: crate::StepStop::Waiting,
                    trace: vec![],
                    events: vec![],
                };
                let mut operation =
                    lot_placement::live::Execution::new(lot_placement::Operation::set(
                        4,
                        crate::lot_links::Descriptor {
                            on_lot: true,
                            position: [56, 56],
                            container: None,
                            slot: -1,
                            level: 1,
                        },
                        true,
                    ));
                assert!(
                    initial
                        .drive_live_placement(&p, &mut operation, &mut 1000, &mut step)
                        .unwrap()
                );
            }
            let m = p.modules.get_mut("objects").unwrap();
            let mut routine = m.routines[&4096].clone();
            routine.body.instructions = vec![
                Instruction {
                    opcode: 0x12,
                    on_true: 1,
                    on_false: 255,
                    operands: [0, 0, flags, 0, 0, 0, 0, 0],
                },
                Instruction {
                    opcode: 2,
                    on_true: 254,
                    on_false: 255,
                    operands: [34, 0, 1, 0, 0, 3, 6, 7],
                },
            ];
            m.routines.insert(4096, routine.clone());
            routine.body.instructions = vec![
                Instruction {
                    opcode: 2,
                    on_true: 1,
                    on_false: 255,
                    operands: [36, 0, 1, 0, 0, 3, 6, 7],
                },
                Instruction {
                    opcode: 2,
                    on_true: 254,
                    on_false: 255,
                    operands: [0, 0, 0, 0, 0, 5, 8, 8],
                },
            ];
            m.routines.insert(4097, routine);
            m.object_functions.get_mut(&1).unwrap()[10].action = 4097;
            initial.vm.world.globals.extend([(34, 0), (36, 0)]);
            let mut outcomes = vec![];
            for budget in [1, 1000] {
                let mut s = initial.clone();
                let mut checkpoints = 0;
                for i in 0..20 {
                    let step = s.run_fixture_tick(&p, budget).unwrap();
                    if s.native_lot.as_ref().unwrap().removal.is_some() {
                        checkpoints += 1;
                        assert!(s.removals.is_empty());
                    }
                    s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
                    if step.stop != crate::StepStop::Budget {
                        break;
                    }
                    assert!(i < 19);
                }
                assert_eq!(checkpoints > 0, budget == 1);
                assert!(!s.vm.world.objects.contains_key(&4));
                assert_eq!(s.vm.world.globals[&34], i16::from(flags & 1 != 0));
                assert_eq!(s.vm.world.globals[&36], 2);
                assert_eq!(s.vm.tick, initial.vm.tick);
                outcomes.push(s);
            }
            assert_eq!(outcomes[0], outcomes[1]);
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_inactive_head_refresh_retains_gameplay_fields() {
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/ended-pose-placement-refresh.json"
    ))
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let (_, mut s) = walking_fixture();
        let before = s.clone();
        s.refresh_detached_animator(1).unwrap();
        assert_eq!(s.vm, before.vm, "refresh must preserve gameplay state");
        let person = s.vm.world.objects[&1].person.as_ref().unwrap();
        assert_eq!(serde_json::json!(person[&42]), row["original"]["flags"]);
        assert_eq!(serde_json::json!(person[&45]), row["original"]["countdown"]);
        // The native foreground/background/carry pointers survive for
        // both dirty-gate branches. Rust's pose lifetime is exercised by
        // the budget/save deletion regression below.
        assert!(
            row["original"]["retained"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| v.as_u64().unwrap() != 0)
        );
        assert_eq!(s.animations, before.animations);
        assert_eq!(s.backgrounds, before.backgrounds);
        assert_eq!(s.carry, before.carry);
    }
    for field in [30, 41, 42, 43, 44, 45] {
        let (_, mut s) = walking_fixture();
        s.vm.world
            .objects
            .get_mut(&1)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .insert(field, 1);
        let before = s.clone();
        assert!(
            s.refresh_detached_animator(1)
                .unwrap_err()
                .contains("active-layer/head-look")
        );
        assert_eq!(s, before, "unimplemented refresh must not mutate state");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_current_clock_refresh_preserves_active_foreground() {
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/active-placement-refresh.json"
    ))
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 96);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let native = &row["original"];
        let (_, mut s) = walking_fixture();
        s.vm.tick = 99;
        // Dropping an object retains its selector bookkeeping, with a
        // null native carry-layer pointer. It is not an active layer.
        s.carry.insert(1, crate::carry::Carry::default());
        let progress = c["progress"].as_f64().unwrap() as f32;
        let speed = c["speed"].as_f64().unwrap() as f32;
        s.animations.insert(
            1,
            crate::animation::Playback {
                clip: "native-probe".into(),
                thread: s.vm.object_threads[&1],
                interruptible: false,
                pickup: false,
                reverse: speed < 0.,
                started_tick: 0,
                last_tick: 99,
                frame_fifths: (progress * 1000.) as u64,
                speed: 6,
                next_event: usize::from(speed > 0.),
                pending: [7].into(),
                delivered: 0,
                numbered_events: 0,
                substitute_events: 0,
                event_count: 1,
                last_yield_tick: None,
                ended: c["ended"] == 1,
                completed: false,
                looping: c["mode"] == 1,
                hands: [0, 0],
            },
        );
        let before = s.clone();
        for _ in 0..2 {
            s.refresh_detached_animator(1).unwrap();
            assert_eq!(s.vm, before.vm);
            assert_eq!(s.animations, before.animations);
            assert_eq!(s.carry, before.carry);
        }
        let p = &s.animations[&1];
        assert_eq!(native["first"], native["repeated"]);
        let n = &native["first"];
        assert_eq!(n["foreground_retained"], true);
        assert_eq!(n["progress_bits"], progress.to_bits());
        assert_eq!(n["speed_bits"], speed.to_bits());
        assert_eq!(n["ended"], u8::from(p.ended));
        assert_eq!(n["event_cursor"], p.next_event);
        assert_eq!(n["previous_frame"], p.frame_fifths);
        assert_eq!(n["clock"], p.last_tick);
        assert_eq!(n["flags"], 0);
        assert_eq!(n["countdown"], 0);
        assert_eq!(
            native["updates"],
            if c["only_dirty"] == 1 && c["dirty"] == 0 {
                0
            } else {
                2
            }
        );
        s.vm.tick += 1;
        let before = s.clone();
        assert!(s.refresh_detached_animator(1).is_err());
        assert_eq!(s, before, "stale active pose must not silently advance");
        s.vm.tick -= 1;
        s.carry.get_mut(&1).unwrap().layer = Some(crate::carry::Layer {
            clip: "still-active".into(),
            started_tick: 99,
        });
        let before = s.clone();
        assert!(s.refresh_detached_animator(1).is_err());
        assert_eq!(s, before, "active carry refresh still needs its adapter");
    }
}

#[test]
fn native_person_deletion_resets_before_registry_release_and_saves_callback() {
    use sims_formats::behavior::Instruction;
    for (flags, queued, active, use_frame) in (0..=3)
        .flat_map(|flags| [0, 2].map(move |queued| (flags, queued)))
        .flat_map(|(flags, queued)| [false, true].map(move |active| (flags, queued, active)))
        .flat_map(|(flags, queued, active)| {
            [false, true]
                .into_iter()
                .filter(move |used| !used || flags & 2 != 0)
                .map(move |used| (flags, queued, active, used))
        })
    {
        let (mut p, mut initial) = fixture();
        let m = p.modules.get_mut("objects").unwrap();
        let mut words = m.object_definition_words[&1].clone();
        words[9] = 2;
        words[14] = 2;
        words[19] = 100;
        m.object_definition_words.insert(2, words);
        let mut functions = m.object_functions[&1].clone();
        functions[1].action = 4098;
        functions[3].action = 4099;
        functions[10].action = 4100;
        m.object_functions.insert(2, functions);
        m.semiglobal = Some("objects".into());
        let mut profile = vec![String::new(); 15];
        profile[12] = "male".into();
        profile[13] = "30".into();
        profile[14] = "lgt".into();
        m.strings.insert(100, profile);
        m.strings.insert(8192, vec!["".into(), "lgt".into()]);
        let mut routine = m.routines[&4096].clone();
        routine.body.instructions = vec![
            Instruction {
                opcode: 0x12,
                on_true: 1,
                on_false: 255,
                operands: [0, 0, flags, 0, 0, 0, 0, 0],
            },
            Instruction {
                opcode: 2,
                on_true: 254,
                on_false: 255,
                operands: [34, 0, 1, 0, 0, 3, 6, 7],
            },
        ];
        m.routines.insert(4098, routine.clone());
        m.routines.insert(4102, routine.clone());
        if use_frame {
            m.routines.get_mut(&4098).unwrap().body.instructions[0] = Instruction {
                opcode: 30,
                on_true: 1,
                on_false: 255,
                operands: [0; 8],
            };
        }
        m.object_functions.get_mut(&1).unwrap()[4].action = 4101;
        m.object_interactions.insert(
            1,
            vec![
                sims_vm::program::MenuEntry {
                    id: 0,
                    action: 4096,
                    ..Default::default()
                },
                sims_vm::program::MenuEntry {
                    id: 1,
                    action: 4102,
                    ..Default::default()
                },
            ],
        );
        for (bhav, amount) in [(4099, 10), (4100, 1), (4101, 100)] {
            routine.body.instructions = vec![
                Instruction {
                    opcode: 2,
                    on_true: 1,
                    on_false: 255,
                    operands: [36, 0, amount, 0, 0, 3, 6, 7],
                },
                Instruction {
                    opcode: 2,
                    on_true: 254,
                    on_false: 255,
                    operands: [0, 0, 0, 0, 0, 5, 8, 8],
                },
            ];
            m.routines.insert(bhav, routine.clone());
        }
        let o = initial.vm.world.objects.get_mut(&4).unwrap();
        o.definition.as_mut().unwrap().object = 2;
        o.definition.as_mut().unwrap().guid = 2;
        o.person = Some((0..256).map(|i| (i, 0)).collect());
        initial.vm.world.person.as_mut().unwrap().person_order = vec![4];
        initial.vm.world.actions = Some(sims_vm::action::WorldState {
            actors: [(4, Default::default())].into(),
            ..Default::default()
        });
        initial
            .vm
            .world
            .globals
            .extend([(34, 0), (35, 0), (36, 0), (37, 0)]);
        initial
            .vm
            .world
            .actions
            .as_mut()
            .unwrap()
            .actors
            .get_mut(&4)
            .unwrap()
            .queue
            .pending = (0..queued)
            .map(|i| sims_vm::action::Action {
                id: i + 1,
                target: i as i16 + 2,
                icon_owner: i as i16 + 2,
                routine: 4096,
                ..Default::default()
            })
            .collect();
        for id in [1, 2, 3] {
            initial.vm.start_object_lifecycle(&p, id).unwrap();
        }
        initial
            .architecture
            .as_mut()
            .unwrap()
            .bind_footprints(&p)
            .unwrap();
        initial.native_lot.as_mut().unwrap().membership =
            RoomMembership::constructed(&initial.vm.world);
        initial.native_lot.as_mut().unwrap().selected_person = Some(4);
        let person_thread = initial.vm.restart_loaded_main(&p, 4).unwrap();
        if use_frame {
            let t = initial.vm.threads.get_mut(&person_thread).unwrap();
            let mut f = t.stack[0].clone();
            f.location.bhav = 4102;
            f.storage_offset = 8 + f.args.len() + f.locals.len();
            f.action_target = Some(2);
            f.callee = 2;
            t.frame_memory.resize(
                f.storage_offset + 8 + f.args.len() + f.locals.len(),
                Some(0),
            );
            t.stack.push(f);
            t.last_result = false;
            initial
                .vm
                .world
                .objects
                .get_mut(&2)
                .unwrap()
                .data
                .insert(62, 1);
            initial
                .vm
                .world
                .actions
                .as_mut()
                .unwrap()
                .actors
                .get_mut(&4)
                .unwrap()
                .active = Some(sims_vm::action::Action {
                id: 71,
                target: 2,
                icon_owner: 2,
                routine: 4102,
                menu_id: 1,
                priority: 100,
                flags: 12,
                ..Default::default()
            });
        }
        let mut placement = lot_placement::live::Execution::new(lot_placement::Operation::set(
            4,
            crate::lot_links::Descriptor {
                on_lot: true,
                position: [56, 56],
                container: None,
                slot: -1,
                level: 1,
            },
            true,
        ));
        let mut step = crate::Step {
            stop: crate::StepStop::Waiting,
            trace: vec![],
            events: vec![],
        };
        assert!(
            initial
                .drive_live_placement(&p, &mut placement, &mut 1000, &mut step)
                .unwrap()
        );
        p.animations.insert(
            "finished-pose".into(),
            sims_vm::program::AnimationAsset {
                source: "synthetic".into(),
                content_hash: String::new(),
                samples_source: String::new(),
                samples_hash: String::new(),
                clip: sims_formats::animation::Animation {
                    name: "finished-pose".into(),
                    skill: "finished-pose".into(),
                    duration_ms: 33.,
                    distance: 0.,
                    moving: false,
                    translation_count: 0,
                    rotation_count: 0,
                    frames: if active { 10 } else { 1 },
                    motions: vec![],
                    events: vec![],
                },
                samples: sims_formats::animation::Samples {
                    translations: vec![],
                    rotations: vec![],
                },
            },
        );
        initial.vm.tick = 1;
        initial.animations.insert(
            4,
            crate::animation::Playback {
                clip: "finished-pose".into(),
                thread: person_thread,
                interruptible: false,
                pickup: false,
                reverse: false,
                started_tick: 0,
                last_tick: 1,
                frame_fifths: if active { 6 } else { 5 },
                speed: 6,
                next_event: 0,
                pending: Default::default(),
                delivered: 0,
                numbered_events: 0,
                substitute_events: 0,
                event_count: 0,
                last_yield_tick: None,
                ended: !active,
                completed: !active,
                looping: active,
                hands: [0, 0],
            },
        );
        let mut outcomes = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut saved_reset = false;
            let mut saved_detached_pose = false;
            let mut ended = false;
            for _ in 0..100 {
                let step = s.run_fixture_tick(&p, budget).unwrap();
                if s.unplaced.contains(&4) && s.animations.contains_key(&4) {
                    saved_detached_pose = true;
                    assert_eq!(s.animations[&4], initial.animations[&4]);
                    let mut bad = s.clone();
                    bad.removals.clear();
                    bad.deletion.clear();
                    bad.native_lot.as_mut().unwrap().removal = None;
                    assert!(
                        bad.validate_animations(&p).is_ok(),
                        "registered off-lot pose should survive without a destructor"
                    );
                    bad.unplaced.remove(&4);
                    assert!(
                        bad.validate_animations(&p).is_err(),
                        "pose without body, off-lot registration or removal owner accepted"
                    );
                }
                if s.deletion
                    .last()
                    .and_then(|d| d.person_reset.as_ref())
                    .is_some()
                {
                    saved_reset = true;
                    assert!(s.vm.world.objects.contains_key(&4));
                    assert!(s.unplaced.contains(&4));
                    assert_eq!(s.vm.world.person.as_ref().unwrap().person_order, vec![4]);
                    assert!(s.advance_fixture_tick().is_err());
                    if matches!(
                        s.deletion
                            .last()
                            .unwrap()
                            .person_reset
                            .as_ref()
                            .unwrap()
                            .pending,
                        Some(sims_vm::person_reset::Request::Script(_))
                    ) {
                        if use_frame {
                            assert_eq!(s.vm.world.objects[&2].data[&62], 0);
                            let a = &s.vm.world.actions.as_ref().unwrap().actors[&4];
                            assert!(a.active.is_none());
                            assert_eq!(a.previous.as_ref().unwrap().id, 71);
                        }
                        let mut bad = crate::Snapshot::capture(&p, &s);
                        bad.state.deletion.last_mut().unwrap().person_reset = None;
                        assert!(bad.restore(&p).is_err(), "orphan Reset callback accepted");
                    }
                }
                // Round trip through bytes as well as native validation.
                let json = serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap();
                s = serde_json::from_slice::<crate::Snapshot>(&json)
                    .unwrap()
                    .restore(&p)
                    .unwrap();
                if step.stop != crate::StepStop::Budget {
                    ended = true;
                    break;
                }
            }
            assert!(ended);
            assert_eq!(
                saved_reset,
                budget == 1,
                "flags={flags} queued={queued} active={active} used={use_frame} stop={ended} objects={:?}",
                s.vm.world.objects.keys()
            );
            assert_eq!(saved_detached_pose, budget == 1);
            assert!(s.animations.is_empty());
            assert_eq!(s.vm.tick, initial.vm.tick);
            assert_eq!(
                s.vm.world.globals[&34],
                i16::from(flags & 1 != 0) * if use_frame { 2 } else { 1 },
                "replacement Main must not execute"
            );

            assert_eq!(
                s.vm.world.globals[&36],
                11 + 100 * queued as i16,
                "detachment, Reset and queue callbacks execute exactly once"
            );
            assert!(!s.vm.world.objects.contains_key(&4));
            assert!(s.vm.world.person.as_ref().unwrap().person_order.is_empty());
            assert!(s.vm.world.actions.as_ref().unwrap().actors.is_empty());
            assert!(s.native_lot.as_ref().unwrap().membership.0.is_empty());
            assert_eq!(s.native_lot.as_ref().unwrap().selected_person, None);

            assert!(s.removals.is_empty() && s.deletion.is_empty());
            if use_frame {
                assert_eq!(s.vm.world.objects[&2].data[&62], 0);
            }
            outcomes.push(s);
        }
        assert_eq!(outcomes[0], outcomes[1]);
    }
}

#[test]
fn native_route_clearance_owns_other_threads_and_resumes_without_repeating_callbacks() {
    for (permitted, owner, height_mask) in [false, true].into_iter().flat_map(|allowed| {
        [1, 3].into_iter().flat_map(move |owner| {
            [0, 1]
                .into_iter()
                .map(move |height| (allowed, owner, height))
        })
    }) {
        let (p, initial) = clearance_fixture(permitted, owner, height_mask);
        let actor = initial.vm.object_threads[&1];
        let other = initial.vm.object_threads[&3];
        let ordinary = initial.vm.threads[&other].clone();
        let before = initial.vm.world.positions.clone();
        let mut results = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut checkpoint = false;
            let error = loop {
                match s.run_fixture_tick(&p, budget) {
                    Ok(step) => {
                        if step.stop == crate::StepStop::Complete {
                            break None;
                        }
                        assert_eq!(step.stop, crate::StepStop::Budget);
                        if s.vm.threads[&s.vm.object_threads[&owner]]
                            .callback
                            .is_some()
                        {
                            checkpoint = true;
                            let mut bad = crate::Snapshot::capture(&p, &s);
                            bad.state.native_lot.as_mut().unwrap().route = None;
                            assert_eq!(bad.restore(&p).unwrap_err(), "unbound immediate execution");
                        }
                        assert!(s.advance_fixture_tick().is_err());
                        s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
                    }
                    Err(error) => break Some(error),
                }
            };
            let reachable = permitted && height_mask == 1;
            if reachable {
                assert!(
                    error
                        .as_ref()
                        .unwrap()
                        .starts_with("original-lot path construction required")
                );
            } else {
                assert!(error.is_none(), "{error:?}");
                assert!(matches!(
                    s.vm.threads[&actor].status,
                    sims_vm::Status::Returned(false)
                ));
                assert_eq!(
                    s.vm.world.objects[&1].person.as_ref().unwrap()[&62],
                    if permitted { 1 } else { 8 }
                );
                assert!(s.route_control.is_empty());
            }
            assert_eq!(checkpoint, budget == 1);
            assert_eq!(s.vm.tick, 0);
            assert_eq!(s.vm.world.positions, before);
            if reachable {
                assert_eq!(s.vm.threads[&other].stack, ordinary.stack);
                assert_eq!(s.vm.threads[&other].status, ordinary.status);
            }
            assert_eq!(
                s.vm.world.objects[&owner].temps[0],
                if owner == 1 { 3 } else { 1 }
            );
            let expected_calls = if owner == 3 && permitted && height_mask == 1 {
                2
            } else {
                1
            };
            assert_eq!(
                s.vm.world.objects[&owner].temps[1], expected_calls,
                "clearance and obstacle eligibility each call the behavior once"
            );
            let native = s.native_lot.as_ref().unwrap();
            if reachable {
                let set = native.route.as_ref().unwrap().candidates.as_ref().unwrap();
                assert_eq!(set.candidates.len(), 1);
                assert_eq!(set.result, 0);
            } else {
                assert!(native.route.is_none());
            }
            assert_eq!(
                native.diagnostics.error,
                if !permitted {
                    11
                } else if height_mask == 0 {
                    24
                } else {
                    91
                }
            );
            assert_eq!(
                native.diagnostics.blocker,
                if permitted { None } else { Some(3) }
            );
            s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
            if let Some(error) = error {
                let stable = s.clone();
                assert_eq!(s.run_fixture_tick(&p, budget).unwrap_err(), error);
                assert_eq!(s, stable, "finished prefix ran again");
            }
            results.push(s);
        }
        assert_eq!(results[0], results[1]);
    }
}
#[test]
fn native_obstacle_prefix_preserves_callback_order_and_retained_bounds() {
    use sims_formats::behavior::Instruction;
    let (mut p, mut s) = fixture();
    let m = p.modules.get_mut("objects").unwrap();
    let mut words = m.object_definition_words[&1].clone();
    words[9] = 2;
    words[14] = 2;
    m.object_definition_words.insert(2, words);
    let mut functions = m.object_functions[&1].clone();
    functions[1].action = 4098;
    m.object_functions.insert(2, functions);
    m.object_functions.get_mut(&1).unwrap()[5].action = 4099;
    let mut route = m.routines[&4096].clone();
    route.body.instructions = vec![Instruction {
        opcode: 0x2d,
        on_true: 254,
        on_false: 255,
        operands: [0, 0, 1, 0, 1, 0, 0, 0],
    }];
    m.routines.insert(4098, route);
    let mut callback = m.routines[&4096].clone();
    callback.body.instructions = vec![
        Instruction {
            opcode: 2,
            on_true: 1,
            on_false: 255,
            operands: [0, 0, 0, 0, 0, 5, 8, 10],
        },
        // Change the obstacle's cost before returning false. The caller
        // must read data[10] after the callback, not when scanning begins.
        Instruction {
            opcode: 2,
            on_true: 2,
            on_false: 255,
            operands: [10, 0, 253, 255, 0, 5, 3, 7],
        },
        Instruction {
            opcode: 2,
            on_true: 255,
            on_false: 255,
            operands: [1, 0, 1, 0, 0, 3, 8, 7],
        },
    ];
    m.routines.insert(4099, callback);
    let o = s.vm.world.objects.get_mut(&1).unwrap();
    o.definition.as_mut().unwrap().object = 2;
    o.definition.as_mut().unwrap().guid = 2;
    o.person = Some((0..256).map(|n| (n, 0)).collect());
    o.data.insert(8, 8);
    o.data.insert(4, 1);
    o.data.insert(42, 259);
    let registry = s.vm.world.person.as_mut().unwrap();
    registry.person_order = vec![1];
    registry.object_order = vec![4, 1, 3, 2];
    let target = s.vm.world.objects.get_mut(&2).unwrap();
    target.routing_slots = Some(vec![sims_vm::routing::Descriptor([
        0,
        0,
        0,
        u32::MAX,
        0,
        0,
        0,
        0,
        u32::MAX,
        16,
        16,
        16,
        0.1875f32.to_bits(),
        (-2i32) as u32,
        16,
    ])]);
    let lot = s.architecture.as_ref().unwrap().original.as_ref().unwrap();
    s.architecture = Some(
        Architecture::from_original(
            Lot::rebuild(
                lot.size,
                lot.heights.clone(),
                vec![255; 36],
                lot.water.clone(),
                lot.floors.iter().map(|f| f.source.clone()).collect(),
            )
            .unwrap(),
            &p,
        )
        .unwrap(),
    );
    s.architecture
        .as_ref()
        .unwrap()
        .publish_room_context(&mut s.vm.world);
    let n = s.native_lot.as_mut().unwrap();
    n.membership = RoomMembership::constructed(&s.vm.world);
    n.placement.flags = s
        .architecture
        .as_ref()
        .unwrap()
        .original
        .as_ref()
        .unwrap()
        .floors
        .iter()
        .map(|f| f.flags.flags.clone())
        .collect();
    // Retained native rectangles deliberately differ from current positions.
    // Clearance invokes both callbacks before checking rectangle overlap;
    // the obstacle registry then reaches #3 before #2.
    n.placement.bounds.insert(2, [64, 64, 60, 60]);
    n.placement.bounds.insert(3, [74, 84, 70, 80]);
    for id in [1, 2, 3] {
        s.vm.start_object_lifecycle(&p, id).unwrap();
    }
    let actor = s.vm.object_threads[&1];
    s.vm.threads
        .get_mut(&actor)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .stack_object = 2;
    let mut saved = false;
    loop {
        match s.run_fixture_tick(&p, 1) {
            Ok(step) => {
                assert_eq!(step.stop, crate::StepStop::Budget);
                saved |= s.vm.threads[&s.vm.object_threads[&3]].callback.is_some();
                assert!(s.advance_fixture_tick().is_err());
                s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
            }
            Err(error) => {
                assert!(
                    error.starts_with("original-lot path construction required"),
                    "{error}"
                );
                break;
            }
        }
    }
    assert!(saved);
    assert_eq!(s.vm.tick, 0);
    let geometry = s
        .native_lot
        .as_ref()
        .unwrap()
        .route
        .as_ref()
        .unwrap()
        .obstacles
        .as_ref()
        .unwrap()
        .geometry
        .as_ref()
        .unwrap();
    assert_eq!(geometry.object, 3);
    assert_eq!(geometry.kind, 4);
    assert_eq!(
        geometry.rectangle,
        crate::regions::Region([80, 70, 84, 74, 10])
    );
    assert_eq!(s.vm.world.objects[&3].data[&10], -3);
    assert_eq!(&s.vm.world.objects[&3].temps[..2], &[1, 2]);
    assert_eq!(
        s.vm.world.objects[&2].temps[1], 1,
        "callback past protected boundary ran"
    );
    let stable = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
    assert!(s.run_fixture_tick(&p, 1000).is_err());
    assert_eq!(s, stable);
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_obstacle_prefix_matches_original_registry_and_callback_vectors() {
    use sims_formats::behavior::Instruction;
    let report: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/obstacle-prefix.json")).unwrap();
    for case in report["cases"].as_array().unwrap() {
        let (mut p, mut s) = fixture();
        s.vm.world.objects.insert(5, s.vm.world.objects[&3].clone());
        s.vm.world.positions.insert(5, s.vm.world.positions[&3]);
        s.bodies.insert(5, s.bodies[&3].clone());
        s.native_lot
            .as_mut()
            .unwrap()
            .placement
            .links
            .next
            .insert(5, 0);
        let rows = case["input"].as_array().unwrap();
        s.vm.world.person.as_mut().unwrap().object_order = rows
            .iter()
            .map(|v| v["id"].as_i64().unwrap() as i16)
            .collect();
        let ordinary = p.modules["objects"].routines[&4096].clone();
        for row in rows {
            let id = row["id"].as_i64().unwrap() as i16;
            let definition = 10 + id as u16;
            let m = p.modules.get_mut("objects").unwrap();
            let mut words = m.object_definition_words[&1].clone();
            words[14] = definition as i16;
            m.object_definition_words.insert(definition, words);
            let mut functions = m.object_functions[&1].clone();
            functions[5].action = 4100 + id as u16;
            m.object_functions.insert(definition, functions);
            let mut callback = ordinary.clone();
            let after = (row["after_weight"].as_i64().unwrap() as i16).to_le_bytes();
            callback.body.instructions = vec![
                Instruction {
                    opcode: 2,
                    on_true: 1,
                    on_false: 255,
                    operands: [0, 0, 0, 0, 0, 5, 8, 10],
                },
                Instruction {
                    opcode: 2,
                    on_true: if row["allow"].as_bool().unwrap() {
                        254
                    } else {
                        255
                    },
                    on_false: 255,
                    operands: [10, 0, after[0], after[1], 0, 5, 3, 7],
                },
            ];
            m.routines.insert(4100 + id as u16, callback);
            let o = s.vm.world.objects.get_mut(&id).unwrap();
            o.definition.as_mut().unwrap().object = definition;
            o.definition.as_mut().unwrap().guid = u32::from(definition);
            let contained = row["contained"].as_bool().unwrap();
            o.data.extend([
                (8, row["flags"].as_i64().unwrap() as i16),
                (10, row["weight"].as_i64().unwrap() as i16),
                (
                    29,
                    if id == 1 {
                        7
                    } else {
                        row["room"].as_i64().unwrap() as i16
                    },
                ),
                (14, if contained { 1 } else { -1 }),
                (26, if contained { 1 } else { 0 }),
            ]);
            if id == 1 {
                o.person = Some((0..256).map(|n| (n, 0)).collect());
            }
            let bounds: [i32; 4] = row["bounds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap() as i32)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap();
            s.native_lot
                .as_mut()
                .unwrap()
                .placement
                .bounds
                .insert(id, bounds);
            if !s.vm.object_threads.contains_key(&id) {
                s.vm.start_object_lifecycle(&p, id).unwrap();
            }
        }
        let mut prefix = crate::lot_route::obstacle::Prefix::begin(&s, 1).unwrap();
        let mut step = crate::Step {
            stop: crate::StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        let mut budget = 1000;
        assert!(
            s.drive_native_obstacle_prefix(&p, &mut prefix, &mut budget, &mut step)
                .unwrap()
        );
        prefix.validate(&s, &p, 1).unwrap();
        let calls: Vec<_> = step
            .trace
            .chunks_exact(2)
            .map(|pair| {
                let id = pair[0].location.bhav - 4100;
                assert_eq!(s.vm.world.objects[&(id as i16)].temps[0], 1);
                serde_json::json!([id, 5, 1, 0])
            })
            .collect();
        assert_eq!(
            serde_json::to_value(calls).unwrap(),
            case["original"]["callbacks"],
            "{case}"
        );
        assert_eq!(
            serde_json::to_value(prefix.geometry).unwrap(),
            case["original"]["geometry"],
            "{case}"
        );
    }
}
#[test]
fn stale_body_and_reordered_tile_cache_are_rejected_before_execution() {
    let (p, s) = fixture();
    let mut moved = s.clone();
    moved.bodies.get_mut(&1).unwrap().position.x += 1;
    let before = moved.vm.clone();
    assert!(
        moved
            .run_fixture_tick(&p, 1)
            .unwrap_err()
            .contains("bypassed physical placement")
    );
    assert_eq!(moved.vm, before);
    let mut reordered = s.clone();
    reordered.vm.world.person.as_mut().unwrap().tile_chains[0].sort();
    assert!(
        reordered
            .run_fixture_tick(&p, 1)
            .unwrap_err()
            .contains("physical tile order")
    );
    assert!(
        crate::Snapshot::capture(&p, &reordered)
            .restore(&p)
            .is_err()
    );
}

#[test]
fn live_placement_callbacks_preserve_scheduler_and_resume_at_each_instruction() {
    use crate::lot_placement::{Operation, live::Execution};
    let (mut p, initial) = fixture();
    let m = p.modules.get_mut("objects").unwrap();
    let mut callback = m.routines[&4096].clone();
    callback.name = "placement callback".into();
    callback.body.instructions = vec![
        sims_formats::behavior::Instruction {
            opcode: 2,
            on_true: 1,
            on_false: 255,
            operands: [0, 0, 7, 0, 0, 5, 8, 7],
        },
        sims_formats::behavior::Instruction {
            opcode: 2,
            on_true: 254,
            on_false: 255,
            operands: [1, 0, 29, 0, 0, 5, 8, 3],
        },
    ];
    m.routines.insert(4097, callback);
    m.object_functions.get_mut(&1).unwrap()[9].action = 4097;
    let mut results = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        let thread = s.vm.object_threads[&4];
        let ordinary = s.vm.threads[&thread].clone();
        let mut execution = Execution::new(Operation::set(
            4,
            crate::lot_links::Descriptor {
                on_lot: true,
                position: [56, 56],
                level: 1,
                container: None,
                slot: -1,
            },
            true,
        ));
        let mut traces = vec![];
        let mut checkpoints = 0;
        loop {
            let mut step = crate::Step {
                stop: crate::StepStop::Waiting,
                trace: vec![],
                events: vec![],
            };
            let mut remaining = budget;
            let done = s
                .drive_live_placement(&p, &mut execution, &mut remaining, &mut step)
                .unwrap();
            traces.extend(step.trace);
            execution.validate(&s.vm, &p, 4).unwrap();
            s.validate_native_lot().unwrap();
            assert_eq!(s.vm.tick, 0);
            assert_eq!(
                s.vm.world.positions[&4],
                Placement {
                    x: 56,
                    y: 56,
                    level: 1
                }
            );
            assert_eq!(s.vm.world.objects[&4].temps[0], 7);
            assert!(!s.unplaced.contains(&4));
            // The host owns this continuation outside the Simulation in
            // this isolated test. Validate the VM snapshot and round-trip
            // both host halves at the actual instruction suspension.
            s.vm = sims_vm::Snapshot::capture(&p, &s.vm).restore(&p).unwrap();
            (s, execution) = serde_json::from_value(serde_json::json!([s, execution])).unwrap();
            checkpoints += 1;
            assert!(checkpoints < 4);
            if done {
                break;
            }
            assert_eq!(
                s.vm.threads[&thread]
                    .callback
                    .as_ref()
                    .unwrap()
                    .resume_status,
                ordinary.status
            );
            let mut corrupt = execution.clone();
            corrupt.complete = true;
            assert!(corrupt.validate(&s.vm, &p, 4).is_err());
            let mut corrupt = execution.clone();
            corrupt.pending = Some(lot_placement::Request::Function {
                object: 1,
                function: 9,
            });
            assert!(corrupt.validate(&s.vm, &p, 4).is_err());
        }
        assert_eq!(s.vm.threads[&thread].stack, ordinary.stack);
        assert_eq!(s.vm.threads[&thread].status, ordinary.status);
        assert_eq!(
            s.vm.world.objects[&4].temps[1],
            s.vm.world.objects[&4].data[&29]
        );
        assert_eq!(
            s.native_lot
                .as_ref()
                .unwrap()
                .observers
                .iter()
                .filter(|r| matches!(r, lot_placement::Request::ViewPlaced(4)))
                .count(),
            1
        );
        assert_eq!(checkpoints, if budget == 1 { 2 } else { 1 });
        crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
        results.push((s, execution, traces));
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn native_transfer_callbacks_restore_mid_grab_drop_and_drop_onto() {
    use sims_formats::{
        behavior::Instruction,
        execution::{Slot, Slots},
    };
    let (mut p, mut initial) = fixture();
    let m = p.modules.get_mut("objects").unwrap();
    let template = m.routines[&4096].clone();
    m.routines.insert(4097, template.clone());
    let mut main = template.clone();
    main.body.instructions = vec![
        Instruction {
            opcode: 4,
            on_true: 1,
            on_false: 253,
            operands: [255; 8],
        },
        Instruction {
            opcode: 5,
            on_true: 2,
            on_false: 253,
            operands: [255; 8],
        },
        Instruction {
            opcode: 4,
            on_true: 3,
            on_false: 253,
            operands: [255; 8],
        },
        Instruction {
            opcode: 2,
            on_true: 4,
            on_false: 253,
            operands: [0, 0, 3, 0, 0, 5, 10, 7],
        },
        Instruction {
            opcode: 43,
            on_true: 5,
            on_false: 253,
            operands: [0; 8],
        },
        Instruction {
            opcode: 2,
            on_true: 254,
            on_false: 253,
            operands: [7, 0, 1, 0, 0, 3, 8, 7],
        },
    ];
    m.routines.insert(4096, main);
    m.slots.insert(
        1,
        Slots {
            version: 10,
            entries: vec![Slot {
                kind: 0,
                offset: [0.; 3],
                standing: 0,
                sitting: 0,
                ground: 0,
                flags: 0,
                snap_target: -1,
                proximity: [16; 3],
                max_size: 0,
                unknown: 0,
                gradient: 0.,
                height: 1,
                facing: -2,
                resolution: 16,
            }],
        },
    );
    m.object_definition_words.get_mut(&1).unwrap()[20] = 1;
    let mut words = m.object_definition_words[&1].clone();
    words[14] = 2;
    m.object_definition_words.insert(2, words);
    let mut f = m.object_functions[&1].clone();
    f[1].action = 4097;
    m.object_functions.insert(2, f);
    for id in 1..=4 {
        let o = initial.vm.world.objects.get_mut(&id).unwrap();
        o.definition = Some(Definition {
            source: "objects".into(),
            object: if id == 4 { 1 } else { 2 },
            guid: if id == 4 { 1 } else { 2 },
            slots: 1,
        });
        o.slots = vec![None];
        o.data.extend([(4, 1), (42, 3)]);
        if id < 4 {
            initial.vm.start_object_lifecycle(&p, id).unwrap();
        }
    }
    let a = initial.architecture.as_mut().unwrap();
    a.original.as_mut().unwrap().pool.fill(255);
    a.bind_footprints(&p).unwrap();
    a.publish_room_context(&mut initial.vm.world);
    initial.vm.world.refresh = Some(Default::default());
    let mut step = crate::Step {
        stop: crate::StepStop::Waiting,
        trace: vec![],
        events: vec![],
    };
    for (id, to) in [
        (
            3,
            crate::lot_links::Descriptor {
                on_lot: true,
                position: [40, 40],
                container: None,
                slot: 0,
                level: 2,
            },
        ),
        (
            4,
            crate::lot_links::Descriptor {
                on_lot: true,
                position: [56, 56],
                container: None,
                slot: 0,
                level: 1,
            },
        ),
        (
            2,
            crate::lot_links::Descriptor {
                on_lot: true,
                position: [40, 40],
                container: Some(1),
                slot: 0,
                level: 1,
            },
        ),
    ] {
        let mut e =
            lot_placement::live::Execution::new(lot_placement::Operation::set(id, to, true));
        assert!(
            initial
                .drive_live_placement(&p, &mut e, &mut 1000, &mut step)
                .unwrap()
        );
    }
    initial
        .vm
        .world
        .objects
        .get_mut(&4)
        .unwrap()
        .data
        .insert(1, 4);
    initial.bodies.get_mut(&4).unwrap().facing = 16;
    initial.vm.world.globals.extend([(15, 0), (34, 0), (36, 0)]);
    let m = p.modules.get_mut("objects").unwrap();
    for (function, bhav, global) in [(9, 4098, 34), (10, 4099, 36)] {
        let mut r = template.clone();
        r.body.instructions = vec![
            Instruction {
                opcode: 2,
                on_true: 1,
                on_false: 253,
                operands: [global, 0, 1, 0, 0, 3, 6, 7],
            },
            Instruction {
                opcode: 2,
                on_true: 254,
                on_false: 253,
                operands: [0, 0, 0, 0, 0, 5, 8, 7],
            },
        ];
        m.routines.insert(bhav, r);
        for def in [1, 2] {
            m.object_functions.get_mut(&def).unwrap()[function].action = bhav;
        }
    }
    let thread = initial.vm.object_threads[&4];
    initial
        .vm
        .threads
        .get_mut(&thread)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .stack_object = 2;
    let mut results = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        let mut saved = std::collections::BTreeSet::new();
        let mut events = vec![];
        let mut orphan_checks = 0;
        for checkpoint in 0..200 {
            let step = s
                .run_fixture_tick(&p, budget)
                .unwrap_or_else(|e| panic!("transfer {budget}/{checkpoint}: {e}"));
            assert_ne!(step.stop, crate::StepStop::Faulted, "{:?}", s.vm.threads);
            events.extend(
                step.events
                    .iter()
                    .filter(|e| e.detail.starts_with("native transfer #"))
                    .map(|e| e.detail.clone()),
            );
            saved.extend(s.native_transfers.keys().copied());
            if !s.native_transfers.is_empty() {
                assert!(s.advance_fixture_tick().is_err());
            }
            if !s.native_transfers.is_empty()
                && s.vm
                    .threads
                    .values()
                    .any(|t| t.callback.as_ref().is_some_and(|b| b.external))
            {
                let mut bad = crate::Snapshot::capture(&p, &s);
                bad.state.native_transfers.clear();
                assert!(bad.restore(&p).is_err());
                orphan_checks += 1;
            }
            s = serde_json::from_slice::<crate::Snapshot>(
                &serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap(),
            )
            .unwrap()
            .restore(&p)
            .unwrap_or_else(|e| panic!("transfer snapshot {budget}/{checkpoint}: {e}"));
            assert_eq!(s.vm.tick, 0);
            if step.stop != crate::StepStop::Budget {
                break;
            }
            assert!(checkpoint < 199);
        }
        assert_eq!(saved.len(), if budget == 1 { 4 } else { 0 });
        assert_eq!(orphan_checks > 0, budget == 1);
        assert_eq!(s.vm.world.objects[&4].temps[7], 1);
        assert_eq!(s.vm.world.objects[&1].slots[0], None);
        assert_eq!(s.vm.world.objects[&4].slots[0], None);
        assert_eq!(s.vm.world.objects[&3].slots[0], Some(2));
        assert_eq!(s.vm.world.objects[&2].container, Some((3, 0)));
        assert_eq!(s.vm.world.positions[&2], s.vm.world.positions[&3]);
        // SetObject calls function 9 on the object and new parent. It
        // unlinks internally without RemoveFromLot's function-10 callback.
        assert_eq!(s.vm.world.globals[&34], 7);
        assert_eq!(s.vm.world.globals[&36], 0);
        assert_eq!(s.vm.world.positions[&2].level, 2);
        assert_eq!(events.len(), 4);
        assert!(events[1].contains("None slot 0 at [72, 56]/1"));
        assert!(events[3].contains("Some(3) slot 0 at [-16, -16]/1"));
        results.push((s, events));
    }
    assert_eq!(results[0], results[1]);

    // An occupied receiving slot rejects Grab before SetObject. Preserve
    // both containment subtrees and do not run placement callbacks.
    let mut occupied = initial;
    let mut place = lot_placement::live::Execution::new(lot_placement::Operation::set(
        3,
        crate::lot_links::Descriptor {
            on_lot: true,
            position: [56, 56],
            container: Some(4),
            slot: 0,
            level: 1,
        },
        true,
    ));
    assert!(
        occupied
            .drive_live_placement(&p, &mut place, &mut 1000, &mut step)
            .unwrap()
    );
    p.modules
        .get_mut("objects")
        .unwrap()
        .routines
        .get_mut(&4096)
        .unwrap()
        .body
        .instructions = vec![Instruction {
        opcode: 4,
        on_true: 255,
        on_false: 254,
        operands: [0; 8],
    }];
    let before = occupied.clone();
    let outcome = occupied.run_fixture_tick(&p, 1000).unwrap();
    assert_eq!(
        occupied.vm.threads[&thread].status,
        sims_vm::Status::Returned(true)
    );
    assert!(
        outcome
            .events
            .iter()
            .all(|e| !e.detail.starts_with("native transfer #"))
    );
    assert_eq!(occupied.vm.world.positions, before.vm.world.positions);
    assert_eq!(
        occupied.native_lot.as_ref().unwrap().placement,
        before.native_lot.as_ref().unwrap().placement
    );
    for id in 1..=4 {
        assert_eq!(
            occupied.vm.world.objects[&id].slots,
            before.vm.world.objects[&id].slots
        );
        assert_eq!(
            occupied.vm.world.objects[&id].container,
            before.vm.world.objects[&id].container
        );
    }
    assert_eq!(occupied.vm.world.globals[&34], before.vm.world.globals[&34]);
    crate::Snapshot::capture(&p, &occupied).restore(&p).unwrap();
}

#[test]
fn placed_factory_uses_native_links_callbacks_and_immediate_failure_cleanup() {
    use sims_formats::{
        behavior::Instruction,
        execution::{Slot, Slots},
    };
    for (mode, reject, remove_target, copy_direction) in [
        (2, false, false),
        (2, true, false),
        (3, false, false),
        (3, true, false),
        (4, false, false),
        (4, true, false),
        (3, false, true),
        (7, false, true),
        (8, false, true),
        (7, true, false),
        (8, true, false),
    ]
    .into_iter()
    .flat_map(|(mode, reject, remove_target)| {
        [false, true].map(|copy| (mode, reject, remove_target, copy))
    }) {
        let floor = matches!(mode, 3 | 7 | 8);
        let frame_selected = matches!(mode, 7 | 8);
        let parent = if mode == 2 { 4 } else { 1 };
        let (mut p, mut initial) = fixture();
        let m = p.modules.get_mut("objects").unwrap();
        let template = m.routines[&4096].clone();
        let mut make = |id, instructions: Vec<Instruction>| {
            let mut r = template.clone();
            r.body.instructions = instructions;
            m.routines.insert(id, r);
        };
        let expr = |dest: u8, op: u8, scope: u8, value: u8, next: u8| Instruction {
            opcode: 2,
            on_true: next,
            on_false: 255,
            operands: [dest, 0, value, 0, 0, op, scope, 7],
        };
        make(
            4096,
            vec![Instruction {
                opcode: 42,
                on_true: 254,
                on_false: 255,
                operands: [
                    2,
                    0,
                    0,
                    0,
                    mode,
                    (if floor { 17 } else { 1 }) | if copy_direction { 32 } else { 0 },
                    u8::from(frame_selected),
                    0,
                ],
            }],
        );
        make(4097, vec![expr(0, 5, 8, 0, 254)]);
        make(
            4098,
            if floor {
                vec![
                    expr(4, 5, 3, 1, 1),
                    expr(42, 5, 3, 3, 2),
                    expr(34, 3, 6, 1, 254),
                ]
            } else {
                vec![expr(4, 5, 3, 1, 1), expr(34, 3, 6, 1, 254)]
            },
        );
        make(4099, vec![expr(34, 3, 6, 100, 1), expr(0, 5, 8, 0, 254)]);
        make(4100, vec![expr(34, 3, 6, 10, 254)]);
        make(4101, vec![expr(36, 3, 6, 1, 1), expr(0, 5, 8, 0, 254)]);
        let mut wall_count = expr(34, 3, 6, 0, 1);
        wall_count.operands[2..4].copy_from_slice(&1000i16.to_le_bytes());
        make(4102, vec![wall_count, expr(0, 5, 8, 0, 254)]);
        if copy_direction {
            // Init chooses a different facing; the later copy must run
            // virtual Rotate (with saved wall callbacks) before acceptance.
            let init = &mut m.routines.get_mut(&4098).unwrap().body.instructions;
            let previous_last = init.len() - 1;
            init[previous_last].on_true = init.len() as u8;
            init.push(expr(1, 5, 3, 2, init.len() as u8 + 1));
            init.push(expr(23, 5, 3, 2, init.len() as u8 + 1));
            init.push(expr(5, 5, 3, 1, 254));
        }
        if frame_selected {
            let body = &mut m.routines.get_mut(&4096).unwrap().body;
            body.args = 1;
            body.locals = 2;
        }
        let mut words = m.object_definition_words[&1].clone();
        words[14] = 2;
        m.object_definition_words.insert(2, words.clone());
        words[14] = 3;
        words[20] = 1;
        m.object_definition_words.insert(3, words);
        let mut f = m.object_functions[&1].clone();
        f[0].action = 4098;
        f[1].action = 4100;
        f[9].action = 4099;
        f[10].action = 4101;
        f[6].action = if copy_direction { 4102 } else { 0 };
        m.object_functions.insert(2, f);
        let mut f = m.object_functions[&1].clone();
        f[1].action = 4097;
        f[9].action = 4099;
        m.object_functions.insert(3, f);
        if mode == 2 {
            m.object_functions.get_mut(&1).unwrap()[9].action = 4099;
            m.object_definition_words.get_mut(&1).unwrap()[20] = 1;
        }
        m.slots.insert(
            1,
            Slots {
                version: 10,
                entries: vec![Slot {
                    kind: 0,
                    offset: [0.; 3],
                    standing: 0,
                    sitting: 0,
                    ground: 0,
                    flags: 0,
                    snap_target: -1,
                    proximity: [16; 3],
                    max_size: 0,
                    unknown: 0,
                    gradient: 0.,
                    height: 1,
                    facing: -2,
                    resolution: 16,
                }],
            },
        );
        for id in [1, 5, 6] {
            m.named_constants.insert(
                id,
                sims_formats::tuning::Constants {
                    version: 1,
                    entries: vec![],
                },
            );
        }
        for id in [501, 502, 503, 504] {
            m.strings.insert(id, vec![String::new(); 9]);
        }
        for id in 1..=3 {
            let o = initial.vm.world.objects.get_mut(&id).unwrap();
            o.definition = Some(Definition {
                source: "objects".into(),
                object: 3,
                guid: 3,
                slots: 1,
            });
            o.slots = vec![None];
            initial.vm.start_object_lifecycle(&p, id).unwrap();
        }
        initial
            .vm
            .world
            .globals
            .extend([(15, 0), (32, 0), (33, 0), (34, 0), (36, 0)]);
        initial.vm.world.refresh = Some(Default::default());
        let a = initial.architecture.as_mut().unwrap();
        a.original.as_mut().unwrap().pool.fill(255);
        a.bind_footprints(&p).unwrap();
        a.publish_room_context(&mut initial.vm.world);
        initial
            .enable_household(
                &p,
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
        if mode == 2 {
            let o = initial.vm.world.objects.get_mut(&4).unwrap();
            o.definition.as_mut().unwrap().slots = 1;
            o.slots = vec![None];
            let mut place = lot_placement::live::Execution::new(lot_placement::Operation::set(
                4,
                crate::lot_links::Descriptor {
                    on_lot: true,
                    position: [56, 56],
                    container: None,
                    slot: -1,
                    level: 1,
                },
                true,
            ));
            initial
                .drive_live_placement(
                    &p,
                    &mut place,
                    &mut 1000,
                    &mut crate::Step {
                        stop: crate::StepStop::Waiting,
                        trace: vec![],
                        events: vec![],
                    },
                )
                .unwrap();
        }
        if frame_selected {
            let mut e = lot_placement::live::Execution::new(lot_placement::Operation::set(
                1,
                crate::lot_links::Descriptor {
                    on_lot: true,
                    position: [24, 40],
                    container: None,
                    slot: -1,
                    level: 1,
                },
                true,
            ));
            assert!(
                initial
                    .drive_live_placement(
                        &p,
                        &mut e,
                        &mut 1000,
                        &mut crate::Step {
                            stop: crate::StepStop::Waiting,
                            trace: vec![],
                            events: vec![],
                        }
                    )
                    .unwrap()
            );
        }
        if reject {
            let mut e = lot_placement::live::Execution::new(lot_placement::Operation::set(
                2,
                crate::lot_links::Descriptor {
                    on_lot: true,
                    position: match mode {
                        2 => [56, 56],
                        3 | 7 | 8 => [24, 40],
                        _ => [40, 40],
                    },
                    container: (!floor).then_some(parent),
                    slot: if floor { -1 } else { 0 },
                    level: 1,
                },
                true,
            ));
            initial
                .drive_live_placement(
                    &p,
                    &mut e,
                    &mut 1000,
                    &mut crate::Step {
                        stop: crate::StepStop::Waiting,
                        trace: vec![],
                        events: vec![],
                    },
                )
                .unwrap();
        }
        // An unrelated deferred deletion must not be drained by failed
        // factory cleanup; it belongs to the end of this ordinary tick.
        let mut detach =
            lot_placement::live::Execution::new(lot_placement::Operation::remove(3, true));
        initial
            .drive_live_placement(
                &p,
                &mut detach,
                &mut 1000,
                &mut crate::Step {
                    stop: crate::StepStop::Waiting,
                    trace: vec![],
                    events: vec![],
                },
            )
            .unwrap();
        initial.removals.push(sims_vm::removal::Pending {
            target: 3,
            suppress_reset: false,
        });
        initial.native_lot.as_mut().unwrap().observers.clear();
        initial
            .vm
            .world
            .globals
            .extend([(32, 0), (33, 0), (34, 0), (36, 0)]);
        let creator = initial.vm.object_threads[&4];
        if frame_selected {
            let t = initial.vm.threads.get_mut(&creator).unwrap();
            let f = t.stack.last_mut().unwrap();
            f.args = vec![1];
            f.locals = vec![0, 1];
            f.opaque_locals.clear();
            t.frame_memory.resize(12, Some(0));
        }
        initial
            .vm
            .threads
            .get_mut(&creator)
            .unwrap()
            .stack
            .last_mut()
            .unwrap()
            .stack_object = 1;
        let mut results = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut checkpoints = 0;
            let mut deletions = vec![];
            let mut target_removed = false;
            let mut selector_changed = false;
            let mut saved_copy_rotation = false;
            for checkpoint in 0..100 {
                // Supply the same factory-boundary target change at both
                // continuation budgets, then exercise real native physical
                // removal and storage cleanup before restoring the factory.
                let slice = if (remove_target && !target_removed)
                    || (frame_selected && !selector_changed)
                {
                    1
                } else {
                    budget
                };
                let step = s.run_fixture_tick(&p, slice).unwrap_or_else(|e| {
                    panic!("contained {mode}/{reject}/{budget}/{checkpoint}: {e}")
                });
                assert_ne!(
                    step.stop,
                    crate::StepStop::Faulted,
                    "{:?}",
                    s.vm.threads
                        .values()
                        .filter(|t| matches!(t.status, sims_vm::Status::Faulted(_)))
                        .map(|t| &t.status)
                        .collect::<Vec<_>>()
                );
                deletions.extend(
                    step.events
                        .iter()
                        .filter(|e| e.detail.starts_with("deleted #"))
                        .map(|e| e.detail.clone()),
                );
                if !s.native_creations.is_empty() {
                    let phases = serde_json::to_value(&s.native_creations).unwrap();
                    if phases
                        .as_object()
                        .unwrap()
                        .values()
                        .any(|e| e["phase"] == "RotatePlaced")
                    {
                        saved_copy_rotation = true;
                    }
                    checkpoints += 1;
                    assert!(s.advance_fixture_tick().is_err());
                    assert!(s.vm.world.objects.contains_key(&3));
                    assert_eq!(s.removals[0].target, 3);
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    bad.state.native_creations.clear();
                    assert!(bad.restore(&p).is_err(), "orphan native creation accepted");
                    if frame_selected && !selector_changed {
                        let f =
                            s.vm.threads
                                .get_mut(&creator)
                                .unwrap()
                                .stack
                                .last_mut()
                                .unwrap();
                        f.args[0] = -123;
                        f.locals[1] = -123;
                        selector_changed = true;
                    }
                    if remove_target && !target_removed {
                        s.native_removal_tree(&p, 1).unwrap();
                        let mut detach = lot_placement::live::Execution::new(
                            lot_placement::Operation::remove(1, true),
                        );
                        let mut change = crate::Step {
                            stop: crate::StepStop::Waiting,
                            trace: vec![],
                            events: vec![],
                        };
                        assert!(
                            s.drive_live_placement(&p, &mut detach, &mut 1000, &mut change)
                                .unwrap()
                        );
                        s.deletion.push(crate::removal::Deletion::new(1));
                        assert!(
                            s.drive_deletion_stack(&p, &mut 1000, &mut change, false)
                                .unwrap()
                        );
                        deletions.extend(
                            change
                                .events
                                .iter()
                                .filter(|e| e.detail.starts_with("deleted #"))
                                .map(|e| e.detail.clone()),
                        );
                        assert!(!s.vm.world.objects.contains_key(&1));
                        target_removed = true;
                    }
                }
                let snap = serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap();
                s = serde_json::from_slice::<crate::Snapshot>(&snap)
                    .unwrap()
                    .restore(&p)
                    .unwrap_or_else(|e| {
                        panic!("contained snapshot {mode}/{reject}/{budget}/{checkpoint}: {e}")
                    });
                assert_eq!(s.vm.tick, 0);
                if step.stop != crate::StepStop::Budget {
                    break;
                }
                assert!(checkpoint < 99);
            }
            assert_eq!(
                checkpoints > 0,
                budget == 1 || remove_target || frame_selected
            );
            assert_eq!(target_removed, remove_target);
            assert_eq!(selector_changed, frame_selected);
            assert_eq!(
                saved_copy_rotation,
                copy_direction && !remove_target && budget == 1,
                "rotation suspension: mode {mode}, reject {reject}, removed {remove_target}, budget {budget}"
            );
            if copy_direction && !remove_target {
                assert!(
                    s.vm.world.globals[&34] >= 1000,
                    "rotation callbacks were skipped"
                );
            }
            assert_eq!(s.vm.world.globals[&34] % 100, 11);
            assert_eq!(
                s.native_lot
                    .as_ref()
                    .unwrap()
                    .observers
                    .iter()
                    .filter(|r| matches!(
                        r,
                        lot_placement::Request::Signal {
                            event: 0xe2,
                            value: 5
                        }
                    ))
                    .count(),
                1
            );
            assert!(
                s.native_creations.is_empty() && s.deletion.is_empty() && s.removals.is_empty()
            );
            if reject {
                assert!(!s.vm.world.objects.contains_key(&5));
                if !floor {
                    assert_eq!(s.vm.world.objects[&parent].slots[0], Some(2));
                }
                assert_eq!(
                    deletions,
                    [
                        "deleted #5 during synchronous factory cleanup",
                        "deleted #3 after lot object updates"
                    ]
                );
                assert_eq!(s.vm.world.globals[&36], 2);
            } else {
                assert!(
                    s.vm.world.objects.contains_key(&5),
                    "unexpected rejection: {:?}",
                    s.native_lot.as_ref().unwrap().diagnostics
                );
                if floor {
                    assert_eq!(s.vm.world.objects[&5].container, None);
                    assert_eq!(
                        s.vm.world.positions[&5],
                        Placement {
                            x: 24,
                            y: 40,
                            level: 1
                        }
                    );
                    assert!(s.vm.world.globals[&34] >= 111);
                } else {
                    assert_eq!(s.vm.world.objects[&5].container, Some((parent, 0)));
                    assert_eq!(s.vm.world.objects[&parent].slots[0], Some(5));
                    assert_eq!(s.vm.world.positions[&5], s.vm.world.positions[&parent]);
                    assert!(s.vm.world.globals[&34] >= 211);
                }
                let expected = if remove_target {
                    vec![
                        "deleted #1 during synchronous factory cleanup",
                        "deleted #3 after lot object updates",
                    ]
                } else {
                    vec!["deleted #3 after lot object updates"]
                };
                assert_eq!(deletions, expected);
            }
            results.push(s);
        }
        assert_eq!(results[0], results[1]);
    }
}

#[test]
fn offlot_factory_preserves_native_links_across_every_budget_checkpoint() {
    let (mut p, mut initial) = fixture();
    let m = p.modules.get_mut("objects").unwrap();
    let main = m.routines[&4096].clone();
    m.routines.insert(4097, main);
    let mut words = m.object_definition_words[&1].clone();
    words[14] = 2;
    m.object_definition_words.insert(2, words);
    let mut functions = m.object_functions[&1].clone();
    functions[1].action = 4097;
    m.object_functions.insert(2, functions);
    m.routines.get_mut(&4096).unwrap().body.instructions[0] = sims_formats::behavior::Instruction {
        opcode: 42,
        on_true: 254,
        on_false: 255,
        operands: [2, 0, 0, 0, 6, 0, 0, 0],
    };
    for id in [1, 5, 6] {
        m.named_constants.insert(
            id,
            sims_formats::tuning::Constants {
                version: 1,
                entries: vec![],
            },
        );
    }
    for id in [501, 502, 503, 504] {
        m.strings.insert(id, vec![String::new(); 9]);
    }
    initial.vm.world.refresh = Some(Default::default());
    let a = initial.architecture.as_mut().unwrap();
    a.bind_footprints(&p).unwrap();
    a.publish_room_context(&mut initial.vm.world);
    initial
        .enable_household(
            &p,
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
    let heads = initial
        .native_lot
        .as_ref()
        .unwrap()
        .placement
        .links
        .heads
        .clone();
    let mut results = vec![];
    // Flag 1 queries the off-lot sentinel tile, not all registered objects.
    // The original 44e0c8 probe still allocates without placement there.
    for (flags, budget) in [(0, 1), (0, 1000), (1, 1), (1, 1000)] {
        p.modules
            .get_mut("objects")
            .unwrap()
            .routines
            .get_mut(&4096)
            .unwrap()
            .body
            .instructions[0]
            .operands[5] = flags;
        let mut s = initial.clone();
        let mut checkpoints = 0;
        loop {
            let step = s.run_fixture_tick(&p, budget).unwrap();
            s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
            assert_eq!(s.vm.tick, 0);
            assert_eq!(s.native_lot.as_ref().unwrap().placement.links.heads, heads);
            checkpoints += 1;
            assert!(checkpoints < 20);
            if step.stop != crate::StepStop::Budget {
                break;
            }
        }
        assert_eq!(s.native_lot.as_ref().unwrap().placement.links.next[&5], 0);
        assert_eq!(
            s.vm.world.positions[&5],
            Placement {
                x: -16,
                y: -16,
                level: 1
            }
        );
        assert!(s.unplaced.contains(&5));
        assert_eq!(
            s.native_lot.as_ref().unwrap().observers,
            [lot_placement::Request::Signal {
                event: 0xe2,
                value: 5
            }]
        );
        if budget == 1 {
            assert!(checkpoints > 1);
        }
        results.push(s);
    }
    for state in &results[1..] {
        assert_eq!(&results[0], state);
    }
}
#[test]
fn native_find_location_resumes_character_refresh_without_repeating_detach() {
    use crate::{Snapshot, head_look, pose};
    use head_look::rig::{AbsoluteBone, Anchor, Kind, Rig};
    use pose::{
        animator::Animator,
        live::Live,
        model::{Matrix, Model},
        refresh::{Censor, Character, Hierarchy, Request, Response},
        sampling::BoneInput,
    };
    use sims_formats::behavior::Instruction;
    let (mut p, mut initial) = walking_fixture();
    let m = p.modules.get_mut("objects").unwrap();
    let main = &mut m.routines.get_mut(&4098).unwrap().body;
    main.locals = 1;
    main.instructions = vec![Instruction {
        opcode: 0x10,
        on_true: 254,
        on_false: 255,
        operands: [1, 0, 2, 0, 0, 0, 0, 0],
    }];
    let mut callback = m.routines[&4098].clone();
    callback.body.instructions = vec![Instruction {
        opcode: 2,
        on_true: 254,
        on_false: 255,
        operands: [5, 0, 1, 0, 0, 3, 8, 7],
    }];
    m.routines.insert(4100, callback);
    m.object_functions.get_mut(&2).unwrap()[10].action = 4100;
    let thread = initial.vm.object_threads[&1];
    let frame = initial
        .vm
        .threads
        .get_mut(&thread)
        .unwrap()
        .stack
        .last_mut()
        .unwrap();
    frame.locals = vec![1];
    frame.stack_object = 1;
    let person = initial
        .vm
        .world
        .objects
        .get_mut(&1)
        .unwrap()
        .person
        .as_mut()
        .unwrap();
    person.extend([(41, 2), (42, 1), (43, 0), (44, 0), (45, 60)]);
    // Explicit synthetic rig/host state exercises the real scheduler
    // connection. Imported rig construction is not inferred by this test.
    let local = [
        pose::LocalBone {
            translation: [0.; 3],
            rotation: [0., 0., 0., 1.],
        },
        pose::LocalBone {
            translation: [0., 1., 0.],
            rotation: [0., 0., 0., 1.],
        },
    ];
    let mut character = Character::new(
        Live::new(
            Animator::new(0, 0, 0, true, -1),
            local
                .into_iter()
                .map(|local| BoneInput {
                    local,
                    priority: 0,
                    blend_marker: 1,
                })
                .collect(),
        ),
        Hierarchy {
            parents: vec![None, Some(0)],
            absolute: local
                .into_iter()
                .map(|b| AbsoluteBone {
                    translation: b.translation,
                    rotation: b.rotation,
                })
                .collect(),
            updates: vec![0; 2],
            evaluated_generation: 0,
        },
        head_look::State {
            last_clock: u32::MAX,
            basis_heading: 1.,
            target_heading: 1.,
            turn: 0.25,
            target_point: Some([0.; 3]),
        },
        Rig {
            head: Some(Anchor {
                bone: 1,
                kind: Kind::Top,
            }),
            hand: None,
            pelvis: None,
            forward: Some(Anchor {
                bone: 1,
                kind: Kind::HeadForward,
            }),
            turn_bone: Some(1),
        },
        Matrix::IDENTITY.values,
        pose::construction::State::new(0).placement,
        Model {
            placement: Matrix::IDENTITY,
            scale: Matrix::IDENTITY,
            bounds_transform: Matrix::IDENTITY,
            bounds_flags: 0,
        },
        Censor {
            binding: None,
            mask: 0,
        },
        0,
    )
    .unwrap();
    character.visible = true;
    initial.characters.insert(
        1,
        crate::character::Resident::new(
            character,
            initial.vm.tick,
            crate::character::Controls {
                ordinary: pose::ordinary::State {
                    clock_mode: 0,
                    base_posture: -1,
                    pickup: 0,
                    carry: sims_vm::animation::CarrySelector {
                        mode: 0,
                        selection: -1,
                    },
                    mode: 3,
                    rate: 1.,
                    target_rate: 1.,
                },
                movement: pose::movement::Movement::new(pose::construction::State::new(0).movement),
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
                sound_visible: true,
                selected: true,
                debug_sound: false,
                position_x: initial.vm.world.positions[&1].x as f32,
                appearance_mode: 0,
                snow: false,
                lot_kind: 1,
            },
        ),
    );
    // 45cc8c runs ObjectUpdate (and its detachment) before the ordinary
    // animator. The preceding tick's completed character is valid here.
    for elapsed in [0, 1] {
        let mut initial = initial.clone();
        initial.vm.tick += elapsed;
        let mut results = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut services = vec![];
            for _ in 0..100 {
                let step = s.run_fixture_tick(&p, budget).unwrap();
                for (id, request) in s.pending_character_services() {
                    assert_eq!(id, 1);
                    assert_eq!(
                        [s.vm.world.positions[&1].x, s.vm.world.positions[&1].y],
                        [-16, -16]
                    );
                    let before = s.clone();
                    assert!(s.advance_fixture_tick().is_err());
                    assert_eq!(s, before);
                    s.run_fixture_tick(&p, budget).unwrap();
                    assert_eq!(s, before, "unanswered service must remain suspended");
                    s = serde_json::from_slice::<Snapshot>(
                        &serde_json::to_vec(&Snapshot::capture(&p, &s)).unwrap(),
                    )
                    .unwrap()
                    .restore(&p)
                    .unwrap();
                    assert!(
                        s.complete_character_service(
                            id,
                            &Request::LoadedAppearance,
                            Response::Done
                        )
                        .is_err()
                    );
                    assert_eq!(s, before);
                    match &request {
                        Request::Placement => s.place_character(id, [0.; 3]).unwrap(),
                        Request::Geometry { .. } => {
                            // This explicit fixture has no appearance bindings;
                            // the original geometry binding traversal is empty.
                            s.complete_character_service(id, &request, Response::Done)
                                .unwrap();
                        }
                        other => panic!("unexpected character service: {other:?}"),
                    }
                    services.push(request);
                }
                s = Snapshot::capture(&p, &s).restore(&p).unwrap();
                if step.stop != crate::StepStop::Budget {
                    break;
                }
            }
            assert_eq!(
                s.vm.threads[&thread].status,
                sims_vm::Status::Returned(true)
            );
            assert_eq!(
                s.vm.world.objects[&1].temps[5],
                initial.vm.world.objects[&1].temps[5] + 1,
                "detach callback repeated"
            );
            assert_eq!(
                s.vm.world.objects[&1].person.as_ref().unwrap()[&45],
                59,
                "head clock advanced twice"
            );
            assert_eq!(services.len(), 4);
            assert_eq!(
                services
                    .iter()
                    .filter(|r| matches!(r, Request::Geometry { .. }))
                    .count(),
                3
            );
            let character = &s.characters[&1].character;
            assert_eq!(
                character.head.target_point.unwrap(),
                head_look::ordinary_target_point([
                    s.vm.world.positions[&2].x,
                    s.vm.world.positions[&2].y
                ])
            );
            assert_eq!(
                character.skin.bones[1].local.rotation,
                [0., 0., 0., 1.],
                "temporary head pose was not restored"
            );
            assert_eq!(character.model.placement.values[12..15], [-3., 0., -3.]);
            assert!(s.characters[&1].refresh_tick.is_none());
            assert_eq!(
                s.native_lot
                    .as_ref()
                    .unwrap()
                    .observers
                    .iter()
                    .filter(|r| **r == lot_placement::Request::RefreshAnimator(1))
                    .count(),
                1
            );
            let mut bad = Snapshot::capture(&p, &s);
            bad.state
                .characters
                .get_mut(&1)
                .unwrap()
                .character
                .rig
                .turn_bone = Some(999);
            assert!(bad.restore(&p).is_err());
            let mut stale = s.clone();
            stale.vm.tick += 2;
            let before = stale.clone();
            assert!(
                stale
                    .run_fixture_tick(&p, 1000)
                    .unwrap_err()
                    .contains("ordinary person update")
            );
            assert_eq!(stale, before);
            let mut reset = s.clone();
            let owner = reset.characters.get_mut(&1).unwrap();
            owner.controls.engaged = 23;
            owner.controls.animation_events = 7;
            owner.controls.geometry_generation = u32::MAX;
            owner.controls.movement_anchors = [Some(0), Some(1)];
            owner.controls.movement.state.status = 2;
            owner.character.placement.offset = [1., 2., 3.];
            let head = owner.character.head.clone();
            let vm = reset.vm.clone();
            reset.reset_character(1).unwrap();
            let owner = &reset.characters[&1];
            assert_eq!(owner.character.head, head);
            assert_eq!(owner.character.placement.base, [-999.; 3]);
            assert_eq!(owner.character.placement.offset, [0.; 3]);
            assert_eq!(owner.controls.engaged, 0);
            assert_eq!(owner.controls.animation_events, 7);
            assert_eq!(owner.controls.geometry_generation, 0);
            assert_eq!(owner.controls.movement_anchors, [None; 2]);
            assert_eq!(reset.native_lot.as_ref().unwrap().diagnostics.error, 7);
            assert_eq!(
                reset.vm, vm,
                "animator reset must not advance or replace VM storage"
            );
            assert_eq!(Snapshot::capture(&p, &reset).restore(&p).unwrap(), reset);
            // Successful Find Location uses view +0x24, which invalidates
            // placement only. It must not accidentally perform Reset's
            // layer/movement/compensation changes tested above.
            let mut appearance = s.clone();
            appearance
                .characters
                .get_mut(&1)
                .unwrap()
                .character
                .placement
                .offset = [1., 2., 3.];
            appearance
                .vm
                .world
                .objects
                .get_mut(&1)
                .unwrap()
                .person
                .as_mut()
                .unwrap()
                .insert(64, 7);
            let mut expected_owner = appearance.characters[&1].clone();
            expected_owner.character.placement.base = [-999.; 3];
            expected_owner.character.placement.dirty = true;
            appearance.refresh_find_location_appearance(1).unwrap();
            assert_eq!(appearance.characters[&1], expected_owner);
            assert_eq!(
                appearance.vm.world.objects[&1].person.as_ref().unwrap()[&64],
                0
            );
            assert_eq!(
                Snapshot::capture(&p, &appearance).restore(&p).unwrap(),
                appearance
            );
            results.push(s);
        }
        assert_eq!(results[0], results[1]);
    }
}

fn clearance_fixture(
    permitted: bool,
    owner: sims_vm::ObjectId,
    height_mask: i16,
) -> (Program, Simulation) {
    use sims_formats::behavior::Instruction;
    let (mut program, mut initial) = fixture();
    let module = program.modules.get_mut("objects").unwrap();
    let mut words = module.object_definition_words[&1].clone();
    words[9] = 2;
    words[14] = 2;
    module.object_definition_words.insert(2, words);
    let mut functions = module.object_functions[&1].clone();
    functions[1].action = 4098;
    module.object_functions.insert(2, functions);
    module
        .object_functions
        .get_mut(&(if owner == 1 { 2 } else { 1 }))
        .unwrap()[5]
        .action = 4099;
    let mut route = module.routines[&4096].clone();
    route.body.instructions = vec![Instruction {
        opcode: 0x2d,
        on_true: 254,
        on_false: 255,
        operands: [0, 0, 1, 0, 1, 0, 0, 0],
    }];
    module.routines.insert(4098, route);
    let mut callback = module.routines[&4096].clone();
    callback.body.instructions = vec![
        Instruction {
            opcode: 2,
            on_true: 1,
            on_false: 255,
            operands: [0, 0, 0, 0, 0, 5, 8, 10],
        },
        Instruction {
            opcode: 2,
            on_true: if permitted { 254 } else { 255 },
            on_false: 255,
            operands: [1, 0, 1, 0, 0, 3, 8, 7],
        },
    ];
    module.routines.insert(4099, callback);
    let object = initial.vm.world.objects.get_mut(&1).unwrap();
    object.definition.as_mut().unwrap().object = 2;
    object.definition.as_mut().unwrap().guid = 2;
    object.person = Some((0..256).map(|index| (index, 0)).collect());
    object.data.insert(8, 8);
    object.data.insert(4, height_mask);
    object.data.insert(42, 259);
    initial.vm.world.person.as_mut().unwrap().person_order = vec![1];
    initial
        .vm
        .world
        .objects
        .get_mut(&2)
        .unwrap()
        .data
        .insert(8, 16);
    initial.vm.world.objects.get_mut(&2).unwrap().routing_slots =
        Some(vec![sims_vm::routing::Descriptor([
            0,
            0,
            0,
            u32::MAX,
            0,
            0,
            0,
            0,
            u32::MAX,
            16,
            16,
            16,
            0.1875f32.to_bits(),
            (-2i32) as u32,
            16,
        ])]);
    let architecture = initial.architecture.as_ref().unwrap();
    let lot = architecture.original.as_ref().unwrap();
    let rebuilt = Lot::rebuild(
        lot.size,
        lot.heights.clone(),
        vec![255; 36],
        lot.water.clone(),
        lot.floors
            .iter()
            .map(|floor| floor.source.clone())
            .collect(),
    )
    .unwrap();
    initial.architecture = Some(Architecture::from_original(rebuilt, &program).unwrap());
    initial
        .architecture
        .as_ref()
        .unwrap()
        .publish_room_context(&mut initial.vm.world);
    let native = initial.native_lot.as_mut().unwrap();
    native.membership = RoomMembership::constructed(&initial.vm.world);
    native.placement.flags = initial
        .architecture
        .as_ref()
        .unwrap()
        .original
        .as_ref()
        .unwrap()
        .floors
        .iter()
        .map(|floor| floor.flags.flags.clone())
        .collect();
    native.placement.bounds.insert(3, [44, 44, 36, 36]);
    native.diagnostics = crate::lot_acceptance::person::Diagnostics {
        error: 91,
        blocker: Some(2),
    };
    initial.vm.start_object_lifecycle(&program, 1).unwrap();
    initial.vm.start_object_lifecycle(&program, 3).unwrap();
    let actor = initial.vm.object_threads[&1];
    initial
        .vm
        .threads
        .get_mut(&actor)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .stack_object = 2;

    (program, initial)
}

fn walking_fixture() -> (Program, Simulation) {
    use sims_formats::behavior::Instruction;
    let (mut p, mut initial) = clearance_fixture(true, 3, 1);

    initial
        .vm
        .world
        .objects
        .get_mut(&2)
        .unwrap()
        .routing_slots
        .as_mut()
        .unwrap()[0]
        .0[0] = 16f32.to_bits();
    initial.native_lot.as_mut().unwrap().routing_policy =
        crate::lot_route::path::Policy::InferredRectangles;
    let idle = &mut p
        .modules
        .get_mut("objects")
        .unwrap()
        .routines
        .get_mut(&4096)
        .unwrap()
        .body;
    idle.args = 1;
    idle.instructions = vec![
        Instruction {
            opcode: 2,
            on_true: 1,
            on_false: 255,
            operands: [0, 0, 232, 3, 0, 5, 9, 7],
        },
        Instruction {
            opcode: 0,
            on_true: 0,
            on_false: 0,
            operands: [0; 8],
        },
    ];
    for object in [3, 4] {
        let id = initial.vm.object_threads.remove(&object).unwrap();
        initial.vm.threads.remove(&id);
        initial.vm.lifecycles.remove(&object);
        initial.vm.start_object_lifecycle(&p, object).unwrap();
    }

    (p, initial)
}
// The route wrapper reads the retained room, and snapshot validation also
// checks its clock/refresh owners. Build these from the same synthetic lot;
// do not substitute a guessed outdoor flag or relax production validation.
fn retain_fixture_environment(p: &Program, s: &mut Simulation) {
    use crate::lot_environment::{
        Tuning,
        live::Live,
        manager::{Manager, Settings},
    };
    s.vm.world.refresh.get_or_insert_with(Default::default);
    let a = s.architecture.as_ref().unwrap();
    a.publish_room_context(&mut s.vm.world);
    s.room_environment = Some(Live {
        manager: Manager::new(
            a.original.as_ref().unwrap(),
            Tuning::from_program(p).unwrap(),
            Settings {
                daylight_multiplier: 1.,
                light_range: 10.,
                rooms_enabled: true,
                models: [None; 6],
            },
        )
        .unwrap(),
        room_effects_consumed: 0,
        clock_events_consumed: s.household.as_ref().unwrap().clock_events.len(),
    });
}

fn retain_fixture_household_person(simulation: &mut Simulation, identity: sims_vm::ObjectId) {
    let object = simulation.vm.world.objects.get_mut(&identity).unwrap();
    object.motives = Some([0; 16]);
    object.motive_update = Some(sims_vm::motives::UpdateState {
        previous: [0; 16],
        last_decay: simulation.vm.tick as i32,
    });
    simulation.household.as_mut().unwrap().people.insert(
        identity,
        crate::household::PersonUpdate {
            started: None,
            finished: None,
        },
    );
}

pub(crate) fn routed_walking_fixture() -> (Program, Simulation) {
    let (mut p, mut s) = walking_fixture();
    let m = p.modules.get_mut("objects").unwrap();
    for id in [1, 5, 6] {
        m.named_constants
            .entry(id)
            .or_insert_with(|| sims_formats::tuning::Constants {
                version: 1,
                entries: vec![],
            });
    }
    for id in [501, 502, 503, 504] {
        m.strings
            .entry(id)
            .or_insert_with(|| vec!["(-100;1)(100;1)".into(); 9]);
    }
    s.vm.world.objects.get_mut(&1).unwrap().motives = Some([0; 16]);
    s.enable_household(
        &p,
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
    retain_fixture_environment(&p, &mut s);
    (p, s)
}

#[test]
fn native_shoo_factory_places_queues_and_destroys_rejected_move_with_saved_callbacks() {
    check_native_shoo_departure_queries(0);
}

#[test]
fn native_shoo_mid_stand_lateral_query_saves_without_second_stand() {
    check_native_shoo_departure_queries(1);
}

#[test]
fn native_shoo_contained_lateral_departure_saves_before_move_enqueue() {
    check_native_shoo_departure_queries(2);
}

fn install_shoo_program_resources(program: &mut Program) {
    use sims_formats::behavior::Instruction;
    let module = program.modules.get_mut("objects").unwrap();
    let mut words = module.object_definition_words[&1].clone();
    words[14] = 0x7c4;
    module.object_definition_words.insert(3, words);
    let mut person_words = module.object_definition_words[&2].clone();
    person_words[14] = 4;
    module.object_definition_words.insert(4, person_words);
    module
        .object_functions
        .insert(4, module.object_functions[&1].clone());
    let mut functions = module.object_functions[&1].clone();
    for function in [0, 9, 10] {
        functions[function].action = 4100;
    }
    module.object_functions.insert(3, functions);
    let mut callback = module.routines[&4099].clone();
    callback.body.instructions = vec![
        Instruction {
            opcode: 2,
            on_true: 1,
            on_false: 255,
            operands: [8, 0, 16, 0, 0, 5, 3, 7],
        },
        Instruction {
            opcode: 2,
            on_true: 254,
            on_false: 255,
            operands: [1, 0, 1, 0, 0, 3, 8, 7],
        },
    ];
    module.routines.insert(4100, callback);
    module.object_interactions.insert(
        3,
        vec![sims_vm::program::MenuEntry {
            id: 3,
            action: 4096,
            ..Default::default()
        }],
    );
    module.object_interactions.insert(
        1,
        vec![sims_vm::program::MenuEntry {
            id: 0,
            action: 4096,
            ..Default::default()
        }],
    );
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
        module.strings.insert(id, vec![String::new(); 9]);
    }
}

fn check_native_shoo_departure_queries(departure: u8) {
    let (mut p, mut initial) = walking_fixture();
    install_shoo_program_resources(&mut p);
    initial.vm.world.refresh = Some(Default::default());
    initial
        .architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&p)
        .unwrap();
    initial.native_lot.as_mut().unwrap().routing_policy = crate::lot_route::path::Policy::Strict;
    assert!(
        initial
            .run_fixture_tick(&p, 1000)
            .unwrap_err()
            .starts_with("original-lot path construction required")
    );
    let old = initial.native_lot.as_mut().unwrap().route.take().unwrap();
    let r = crate::contention::pending_route(&initial.vm.threads[&old.thread], old.request)
        .unwrap()
        .clone();
    initial.native_lot.as_mut().unwrap().routing_policy =
        crate::lot_route::path::Policy::InferredRectangles;
    let blocker = initial.vm.world.objects.get_mut(&3).unwrap();
    blocker.person = Some((0..256).map(|n| (n, 0)).collect());
    blocker.definition.as_mut().unwrap().object = 4;
    blocker.definition.as_mut().unwrap().guid = 4;
    blocker.data.insert(8, 8);
    blocker.data.insert(4, 1);
    blocker.data.insert(42, 259);
    if departure != 0 {
        blocker.person.as_mut().unwrap().extend([(0, 1), (37, 4)]);
    }
    // Keep its already sleeping Main; this synchronous operation must not run it.
    initial
        .vm
        .world
        .person
        .as_mut()
        .unwrap()
        .person_order
        .push(3);
    initial.vm.lifecycles.get_mut(&3).unwrap().definition = 4;
    initial.vm.start_object_lifecycle(&p, 2).unwrap();
    initial.native_lot.as_mut().unwrap().membership =
        RoomMembership::constructed(&initial.vm.world);
    initial.vm.world.actions = Some(sims_vm::action::WorldState {
        actors: [(1, Default::default()), (3, Default::default())].into(),
        ..Default::default()
    });
    for id in [1, 3] {
        initial.vm.world.objects.get_mut(&id).unwrap().motives = Some([0; 16]);
    }
    initial
        .enable_household(
            &p,
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
    retain_fixture_environment(&p, &mut initial);
    initial.route_control.get_mut(&r.id).unwrap().deciding = true;
    p.modules
        .get_mut("objects")
        .unwrap()
        .object_functions
        .get_mut(&1)
        .unwrap()[4]
        .action = 4100;
    // Imported people retain native selection flags used by synchronous deletion.
    for id in [1, 3] {
        initial.vm.world.objects.get_mut(&id).unwrap().native_flags = Some(0);
    }
    if departure == 2 {
        contain_potential_actor(&mut p, &mut initial, 3, 2, 0x55);
    }
    let before = initial.vm.world.positions[&3];
    for (accepted, cancelled) in [(true, false), (true, true), (false, false)] {
        let mut results = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            if !accepted || cancelled {
                let actions = s.vm.world.actions.as_mut().unwrap();
                actions.serial = 8;
                actions.actors.get_mut(&3).unwrap().queue.pending = (1..=8)
                    .map(|id| sims_vm::action::Action {
                        id,
                        target: 2,
                        icon_owner: 2,
                        priority: if cancelled { -1 } else { 100 },
                        menu_id: 0,
                        routine: 4096,
                        ..Default::default()
                    })
                    .collect();
            }
            // Supply only the outer potential path, as the original native
            // descriptor oracle does; factory/placement/lateral query run here.
            let mut selection = crate::lot_route::shoo::Selection::new(vec![3]);
            selection.next();
            selection
                .path_result(Some(vec![[16, 40], [64, 40]]), |_| unreachable!())
                .unwrap();
            s.native_lot.as_mut().unwrap().shoo = Some(crate::lot_route::shoo::Execution {
                thread: old.thread,
                route: r.clone(),
                selection,
                ignored_word: 0,
                blocked_object: 0,
                retained_result: 0,
                candidate_limit: Some(if departure == 2 { 8 } else { 48 }),
                query: None,
                attempt: None,
            });
            if accepted && !cancelled && budget == 1 {
                //This fully owned authored continuation is save-valid before
                //the child runs. Historical unknown/forged cap is denied by
                //the complete Snapshot restore path, not repaired as fresh48.
                let restored = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
                assert_eq!(restored, s);
                for invalid in [None, Some(0), Some(47)] {
                    let mut forged = s.clone();
                    forged
                        .native_lot
                        .as_mut()
                        .unwrap()
                        .shoo
                        .as_mut()
                        .unwrap()
                        .candidate_limit = invalid;
                    let error = crate::Snapshot::capture(&p, &forged)
                        .restore(&p)
                        .unwrap_err();
                    assert!(
                        error.contains(if invalid.is_none() {
                            "shoo retained candidate limit absent"
                        } else {
                            "invalid shoo retained candidate limit"
                        }),
                        "{error}"
                    );
                }
            }
            let mut saved_callback = false;
            let mut checked_lateral_context = false;
            let mut saved_lateral_departure = false;
            for n in 0..5000 {
                assert!(n < 4999, "shoo did not terminate");
                let mut remaining = budget;
                let mut step = crate::Step {
                    stop: crate::StepStop::Budget,
                    trace: vec![],
                    events: vec![],
                };
                let done = s.drive_native_shoo(&p, &mut remaining, &mut step).unwrap();
                saved_callback |= s.vm.threads.values().any(|t| t.callback.is_some());
                if departure == 2 {
                    if let Some(shoo) = &s.native_lot.as_ref().unwrap().shoo {
                        assert_eq!(shoo.candidate_limit, Some(8));
                        let saved = serde_json::to_value(shoo).unwrap();
                        if !saved["attempt"]["query"].is_null() {
                            assert_eq!(saved["attempt"]["query"]["candidate_limit"], 48);
                            saved_lateral_departure |=
                                !saved["attempt"]["query"]["departure"]["floor"].is_null()
                                    && s.vm
                                        .threads
                                        .values()
                                        .any(|thread| thread.callback.is_some());
                            if !checked_lateral_context {
                                for (path, value) in [
                                    (
                                        vec!["attempt", "query", "potential", "ignored_word"],
                                        serde_json::json!(19),
                                    ),
                                    (vec!["attempt", "query", "thread"], serde_json::json!(67)),
                                    (
                                        vec!["attempt", "query", "candidate_limit"],
                                        serde_json::json!(9),
                                    ),
                                    (
                                        vec!["attempt", "query", "potential_request", "target"],
                                        serde_json::json!(300),
                                    ),
                                    (vec!["attempt", "phase"], serde_json::json!("Enqueue")),
                                ] {
                                    let mut forged_owner = saved.clone();
                                    let mut field = &mut forged_owner;
                                    for component in path {
                                        field = &mut field[component];
                                    }
                                    *field = value;
                                    let mut forged = s.clone();
                                    forged.native_lot.as_mut().unwrap().shoo =
                                        Some(serde_json::from_value(forged_owner).unwrap());
                                    assert!(
                                        crate::Snapshot::capture(&p, &forged).restore(&p).is_err()
                                    );
                                    let before = forged.clone();
                                    let mut untouched_budget = 1000;
                                    let mut untouched_output = crate::Step {
                                        stop: crate::StepStop::Budget,
                                        trace: vec![],
                                        events: vec![],
                                    };
                                    assert!(
                                        forged
                                            .drive_native_shoo(
                                                &p,
                                                &mut untouched_budget,
                                                &mut untouched_output
                                            )
                                            .is_err()
                                    );
                                    assert_eq!(forged, before);
                                    assert_eq!(untouched_budget, 1000);
                                    assert!(
                                        untouched_output.trace.is_empty()
                                            && untouched_output.events.is_empty()
                                    );
                                }
                                checked_lateral_context = true;
                            }
                        }
                    }
                }
                if s.cancellation.is_some() {
                    assert!(
                        s.route_control[&r.id].link.is_none(),
                        "link published before cancelled targets' callbacks"
                    );
                }
                s = serde_json::from_slice::<crate::Snapshot>(
                    &serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap(),
                )
                .unwrap()
                .restore(&p)
                .unwrap();
                if done {
                    break;
                }
            }
            assert_eq!(saved_callback, budget == 1);
            if departure == 2 {
                assert_eq!(checked_lateral_context, budget == 1);
                assert_eq!(saved_lateral_departure, budget == 1);
            }
            assert_eq!(s.vm.world.positions[&3], before);
            assert_eq!(
                s.vm.world.objects[&3].person.as_ref().unwrap()[&0],
                i16::from(departure != 0)
            );
            assert_eq!(
                s.vm.world.objects[&3].person.as_ref().unwrap()[&37],
                if departure == 0 { 0 } else { 4 }
            );
            assert_eq!(s.route_control[&r.id].shoo_result, Some(accepted));
            if accepted {
                let link = s.route_control[&r.id].link.as_ref().unwrap();
                assert_eq!(
                    (link.blocker, link.object, link.action),
                    (3, 5, if cancelled { 9 } else { 1 })
                );
                assert_eq!(
                    s.vm.world.objects[&2].temps[1],
                    if cancelled { 8 } else { 0 }
                );
                assert_eq!(s.vm.world.positions[&5], before);
                assert!(!s.unplaced.contains(&5));
                assert!(s.vm.world.objects[&5].temps[1] >= 2);
                let descriptor = &s.vm.world.objects[&5].routing_slots.as_ref().unwrap()[0];
                assert!(
                    [true, false]
                        .into_iter()
                        .any(|side| descriptor == &crate::contention::shoo_descriptor(4, side))
                );
                assert_eq!(
                    s.vm.world.actions.as_ref().unwrap().actors[&3]
                        .queue
                        .pending
                        .len(),
                    1
                );
            } else {
                assert!(!s.vm.world.objects.contains_key(&5));
                assert!(
                    !s.native_lot
                        .as_ref()
                        .unwrap()
                        .placement
                        .links
                        .next
                        .contains_key(&5)
                );
                assert!(s.deletion.is_empty());
            }
            results.push(s);
        }
        assert_eq!(results[0], results[1]);
    }
}
#[test]
fn native_potential_queries_save_callbacks_without_publishing_a_walk() {
    check_native_potential_queries_save_callbacks(false);
}

#[test]
fn native_seated_potential_queries_save_callbacks_without_dispatching_stand() {
    check_native_potential_queries_save_callbacks(true);
}

#[test]
fn native_shoo_outer_seated_departure_snapshots_preserve_cap_and_callback_owner() {
    let (mut program, mut initial) = routed_walking_fixture();
    initial.native_lot.as_mut().unwrap().routing_policy = crate::lot_route::path::Policy::Strict;
    assert!(
        initial
            .run_fixture_tick(&program, 1000)
            .unwrap_err()
            .starts_with("original-lot path construction required")
    );
    let previous = initial.native_lot.as_mut().unwrap().route.take().unwrap();
    let request =
        crate::contention::pending_route(&initial.vm.threads[&previous.thread], previous.request)
            .unwrap()
            .clone();
    initial.native_lot.as_mut().unwrap().routing_policy =
        crate::lot_route::path::Policy::InferredRectangles;
    let module = program.modules.get_mut("objects").unwrap();
    let mut person_words = module.object_definition_words[&2].clone();
    person_words[14] = 4;
    module.object_definition_words.insert(4, person_words);
    module
        .object_functions
        .insert(4, module.object_functions[&1].clone());
    let blocker = initial.vm.world.objects.get_mut(&3).unwrap();
    blocker.person = Some((0..256).map(|field| (field, 0)).collect());
    blocker.definition.as_mut().unwrap().object = 4;
    blocker.definition.as_mut().unwrap().guid = 4;
    blocker.data.extend([(8, 8), (4, 1), (42, 259)]);
    initial.vm.lifecycles.get_mut(&3).unwrap().definition = 4;
    initial
        .architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&program)
        .unwrap();
    initial
        .vm
        .world
        .person
        .as_mut()
        .unwrap()
        .person_order
        .push(3);
    contain_potential_actor(
        &mut program,
        &mut initial,
        request.caller,
        request.target,
        0x55,
    );
    initial.native_lot.as_mut().unwrap().membership =
        RoomMembership::constructed(&initial.vm.world);
    retain_fixture_household_person(&mut initial, 3);
    initial.route_control.get_mut(&request.id).unwrap().deciding = true;
    initial.native_lot.as_mut().unwrap().shoo = Some(crate::lot_route::shoo::Execution {
        thread: previous.thread,
        route: request.clone(),
        selection: crate::lot_route::shoo::Selection::new(vec![3]),
        ignored_word: 0,
        blocked_object: 219,
        retained_result: 3,
        candidate_limit: Some(8),
        query: None,
        attempt: None,
    });
    let mut outcomes = vec![];
    for budget in [1, 1000] {
        let mut simulation = initial.clone();
        let mut saved_callback = false;
        let mut checked_callback = false;
        let mut saved_departure = false;
        for turn in 0..1000 {
            assert!(turn < 999);
            let mut remaining = budget;
            let mut output = crate::Step {
                stop: crate::StepStop::Budget,
                trace: vec![],
                events: vec![],
            };
            let done = simulation
                .drive_native_shoo(&program, &mut remaining, &mut output)
                .unwrap();
            if let Some(shoo) = &simulation.native_lot.as_ref().unwrap().shoo {
                assert_eq!(shoo.candidate_limit, Some(8));
            }
            let callback = simulation
                .vm
                .threads
                .iter()
                .find_map(|(&thread, state)| state.callback.is_some().then_some(thread));
            if let Some(shoo) = &simulation.native_lot.as_ref().unwrap().shoo {
                let saved = serde_json::to_value(shoo).unwrap();
                saved_departure |=
                    !saved["query"]["departure"]["floor"].is_null() && callback.is_some();
            }
            saved_callback |= callback.is_some();
            if !checked_callback {
                if let Some(thread) = callback {
                    let mut forged = simulation.clone();
                    forged
                        .vm
                        .threads
                        .get_mut(&thread)
                        .unwrap()
                        .stack
                        .last_mut()
                        .unwrap()
                        .caller = 300;
                    assert!(
                        crate::Snapshot::capture(&program, &forged)
                            .restore(&program)
                            .is_err()
                    );
                    let before = forged.clone();
                    let mut untouched_budget = 1000;
                    let mut untouched_output = crate::Step {
                        stop: crate::StepStop::Budget,
                        trace: vec![],
                        events: vec![],
                    };
                    assert!(
                        forged
                            .drive_native_shoo(
                                &program,
                                &mut untouched_budget,
                                &mut untouched_output,
                            )
                            .is_err()
                    );
                    assert_eq!(forged, before);
                    assert_eq!(untouched_budget, 1000);
                    assert!(
                        untouched_output.trace.is_empty() && untouched_output.events.is_empty()
                    );
                    checked_callback = true;
                }
            }
            simulation = serde_json::from_slice::<crate::Snapshot>(
                &serde_json::to_vec(&crate::Snapshot::capture(&program, &simulation)).unwrap(),
            )
            .unwrap()
            .restore(&program)
            .unwrap();
            if done {
                break;
            }
        }
        assert_eq!(saved_callback, budget == 1);
        assert_eq!(checked_callback, budget == 1);
        assert_eq!(saved_departure, budget == 1);
        assert!(simulation.native_lot.as_ref().unwrap().shoo.is_none());
        assert_eq!(
            simulation.vm.world.objects[&request.caller]
                .person
                .as_ref()
                .unwrap()[&0],
            1
        );
        outcomes.push(simulation);
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

pub(crate) fn contain_potential_actor(
    program: &mut Program,
    simulation: &mut Simulation,
    caller: sims_vm::ObjectId,
    seat: sims_vm::ObjectId,
    mask: i16,
) {
    let definition = simulation.vm.world.objects[&seat]
        .definition
        .as_ref()
        .unwrap()
        .clone();
    let module = program.modules.get_mut(&definition.source).unwrap();
    module.object_functions.get_mut(&definition.object).unwrap()[26].action = 4096;
    module
        .object_definition_words
        .get_mut(&definition.object)
        .unwrap()[0x84 / 2] = mask;
    let container = simulation.vm.world.objects.get_mut(&seat).unwrap();
    container.slots = vec![None];
    container.native_slot_geometry = Some(vec![sims_vm::slot_geometry::Slot {
        offset: [0; 3],
        height: 1,
        anchor: None,
    }]);
    let position = simulation.vm.world.positions[&seat];
    simulation
        .native_lot
        .as_mut()
        .unwrap()
        .placement
        .links
        .apply(
            &mut simulation.vm.world,
            caller,
            crate::lot_links::Descriptor {
                on_lot: false,
                position: [-16, -16],
                level: position.level,
                container: None,
                slot: -1,
            },
        )
        .unwrap();
    simulation
        .native_lot
        .as_mut()
        .unwrap()
        .placement
        .links
        .apply(
            &mut simulation.vm.world,
            caller,
            crate::lot_links::Descriptor {
                on_lot: true,
                position: [position.x, position.y],
                level: position.level,
                container: Some(seat),
                slot: 0,
            },
        )
        .unwrap();
    simulation
        .native_lot
        .as_ref()
        .unwrap()
        .placement
        .links
        .publish_tile_chains(&mut simulation.vm.world)
        .unwrap();
    let actor = simulation.vm.world.objects.get_mut(&caller).unwrap();
    actor.container = Some((seat, 0));
    actor.person.as_mut().unwrap().insert(0, 1);
}

fn check_native_potential_queries_save_callbacks(seated: bool) {
    let (mut p, mut initial) = routed_walking_fixture();
    initial.native_lot.as_mut().unwrap().routing_policy = crate::lot_route::path::Policy::Strict;
    let error = loop {
        match initial.run_fixture_tick(&p, 1000) {
            Err(e) => break e,
            Ok(_) => panic!("fixture must stop at protected route boundary"),
        }
    };
    assert!(
        error.starts_with("original-lot path construction required"),
        "{error}"
    );
    let old = initial.native_lot.as_mut().unwrap().route.take().unwrap();
    let r = crate::contention::pending_route(&initial.vm.threads[&old.thread], old.request)
        .unwrap()
        .clone();
    let module = p.modules.get_mut("objects").unwrap();
    let mut person_words = module.object_definition_words[&2].clone();
    person_words[14] = 4;
    module.object_definition_words.insert(4, person_words);
    module
        .object_functions
        .insert(4, module.object_functions[&1].clone());
    let blocker = initial.vm.world.objects.get_mut(&3).unwrap();
    blocker.person = Some((0..256).map(|field| (field, 0)).collect());
    blocker.definition.as_mut().unwrap().object = 4;
    blocker.definition.as_mut().unwrap().guid = 4;
    blocker.data.extend([(8, 8), (4, 1), (42, 259)]);
    initial.vm.lifecycles.get_mut(&3).unwrap().definition = 4;
    initial
        .architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&p)
        .unwrap();
    initial
        .vm
        .world
        .person
        .as_mut()
        .unwrap()
        .person_order
        .push(3);
    initial.native_lot.as_mut().unwrap().membership =
        RoomMembership::constructed(&initial.vm.world);
    retain_fixture_household_person(&mut initial, 3);
    initial.route_control.get_mut(&r.id).unwrap().deciding = true;
    if seated {
        contain_potential_actor(&mut p, &mut initial, r.caller, r.target, 0x55);
    }
    initial.native_lot.as_mut().unwrap().routing_policy =
        crate::lot_route::path::Policy::InferredRectangles;
    initial
        .native_lot
        .as_mut()
        .unwrap()
        .route_paths
        .insert(r.caller, vec![[13, 17], [19, 23]]);
    initial
        .vm
        .world
        .objects
        .get_mut(&r.caller)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(37, 123);
    let before_positions = initial.vm.world.positions.clone();
    let before_bodies = initial.bodies.clone();
    let before_controller = initial.route_control.clone();
    let before_paths = initial.native_lot.as_ref().unwrap().route_paths.clone();
    let mut results = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        let q = crate::lot_route::live::Execution::begin_potential(
            &mut s,
            &p,
            old.thread,
            &r,
            crate::lot_route::live::Potential {
                ignored_person: Some(3),
                ignore_people: false,
                ignored_word: 0,
                blocked_object: 0,
                retained_result: 0,
            },
        )
        .unwrap();
        let mut selection = crate::lot_route::shoo::Selection::new(vec![3]);
        selection.next().unwrap();
        s.native_lot.as_mut().unwrap().shoo = Some(crate::lot_route::shoo::Execution {
            thread: old.thread,
            route: r.clone(),
            selection,
            ignored_word: 0,
            blocked_object: 0,
            retained_result: 0,
            candidate_limit: Some(q.potential_candidate_limit()),
            query: Some(q),
            attempt: None,
        });
        let mut saved_callback = false;
        for n in 0..1000 {
            assert!(n < 999, "potential query failed to terminate");
            let mut q = s
                .native_lot
                .as_mut()
                .unwrap()
                .shoo
                .as_mut()
                .unwrap()
                .query
                .take()
                .unwrap();
            let mut remaining = budget;
            let mut step = crate::Step {
                stop: crate::StepStop::Budget,
                trace: vec![],
                events: vec![],
            };
            let complete = s
                .drive_potential_preparation(&p, &mut q, &r, &mut remaining, &mut step)
                .unwrap()
                && {
                    q.mark_potential_prepared();
                    s.drive_native_path_prefix(&p, &mut q, &mut remaining, &mut step)
                        .unwrap()
                };
            saved_callback |= s.vm.threads.values().any(|t| t.callback.is_some());
            let owner = s.native_lot.as_mut().unwrap().shoo.as_mut().unwrap();
            owner.candidate_limit = Some(q.potential_candidate_limit());
            owner.query = Some(q);
            s = serde_json::from_slice::<crate::Snapshot>(
                &serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap(),
            )
            .unwrap()
            .restore(&p)
            .unwrap();
            if complete {
                break;
            }
        }
        assert_eq!(saved_callback, budget == 1);
        let q = s
            .native_lot
            .as_ref()
            .unwrap()
            .shoo
            .as_ref()
            .unwrap()
            .query
            .as_ref()
            .unwrap();
        assert!(q.path.as_ref().unwrap().points.as_ref().unwrap().len() >= 2);
        assert!(q.goal.is_none() && q.selected_seat.is_none() && s.routes.is_empty());
        assert_eq!(s.vm.world.positions, before_positions);
        assert_eq!(s.bodies, before_bodies);
        assert_eq!(s.route_control, before_controller);
        assert_eq!(s.native_lot.as_ref().unwrap().route_paths, before_paths);
        assert_eq!(
            s.vm.world.objects[&r.caller].person.as_ref().unwrap()[&37],
            123
        );
        assert_eq!(
            s.vm.world.objects[&r.caller].person.as_ref().unwrap()[&73],
            0
        );
        results.push(s);
    }
    assert_eq!(results[0], results[1]);
}
#[test]
fn native_posture_clearance_resumes_between_candidates_and_callbacks() {
    for sitting in [0, 10] {
        check_native_posture_clearance_resume(sitting);
    }
}
fn check_native_posture_clearance_resume(sitting: i32) {
    let (p, mut initial) = walking_fixture();
    initial
        .vm
        .world
        .objects
        .get_mut(&1)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(73, 123);
    let words = &mut initial
        .vm
        .world
        .objects
        .get_mut(&2)
        .unwrap()
        .routing_slots
        .as_mut()
        .unwrap()[0]
        .0;
    words[0] = 0;
    words[4] = 1;
    words[5] = sitting as u32;
    words[7] = 0x2055;
    words[9..12].copy_from_slice(&[1, 16, 4]);
    words[14] = 4;
    initial.native_lot.as_mut().unwrap().routing_policy = crate::lot_route::path::Policy::Strict;
    let mut outcomes = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        let mut saved = false;
        let mut steps = 0;
        loop {
            steps += 1;
            assert!(steps < 1000);
            match s.run_fixture_tick(&p, budget) {
                Ok(step) => {
                    assert_eq!(step.stop, crate::StepStop::Budget);
                    if s.native_lot
                        .as_ref()
                        .unwrap()
                        .route
                        .as_ref()
                        .is_some_and(|r| {
                            r.standing.as_ref().is_some_and(|s| s.pending().is_some())
                                || r.posture.as_ref().is_some_and(|s| s.pending().is_some())
                        })
                        && s.vm.threads.values().any(|t| t.callback.is_some())
                    {
                        saved = true;
                        let snapshot = crate::Snapshot::capture(&p, &s);
                        s = serde_json::from_slice::<crate::Snapshot>(
                            &serde_json::to_vec(&snapshot).unwrap(),
                        )
                        .unwrap()
                        .restore(&p)
                        .unwrap();
                    } else {
                        s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
                    }
                }
                Err(error) => {
                    assert!(
                        error.starts_with("original-lot path construction required"),
                        "{error}"
                    );
                    break;
                }
            }
        }
        assert_eq!(saved, budget == 1);
        assert_eq!(
            s.vm.world.objects[&1].data[&1],
            initial.vm.world.objects[&1].data[&1]
        );
        assert_eq!(s.vm.world.objects[&1].person.as_ref().unwrap()[&73], 0);
        let route = s.native_lot.as_ref().unwrap().route.as_ref().unwrap();
        assert!(route.candidates.as_ref().unwrap().candidates.len() > 1);
        assert!(
            s.vm.world.objects[&3].temps[1] > 1,
            "candidate callbacks did not run repeatedly"
        );
        let restored = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
        assert_eq!(restored, s);
        outcomes.push(s);
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[test]
fn native_mixed_posture_standing_path_roundtrips() {
    let (p, mut s) = routed_walking_fixture();
    let words = &mut s
        .vm
        .world
        .objects
        .get_mut(&2)
        .unwrap()
        .routing_slots
        .as_mut()
        .unwrap()[0]
        .0;
    words[0] = 0;
    words[4] = 1;
    words[5] = 10;
    words[7] = 0x55;
    words[9..12].copy_from_slice(&[16, 32, 16]);
    let mut walking = false;
    let mut completed = false;
    for _ in 0..5000 {
        let step = s.run_fixture_tick(&p, 1).unwrap();
        walking |= !s.routes.is_empty();
        completed |= step
            .events
            .iter()
            .any(|e| e.detail == "original-lot native route cleanup completed: true");
        s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
        if completed {
            break;
        }
        if step.stop != crate::StepStop::Budget {
            s.advance_fixture_tick().unwrap();
        }
    }
    assert!(walking && completed);
}

#[test]
fn original_relative_destination_walks_and_restores_selected_facing() {
    use sims_formats::behavior::Instruction;
    for (side, facing) in [0u8, 2, 4, 6, 255].into_iter().flat_map(|side| {
        [254u8, 255, 2]
            .into_iter()
            .map(move |facing| (side, facing))
    }) {
        let (mut p, initial) = routed_walking_fixture();
        p.modules
            .get_mut("objects")
            .unwrap()
            .routines
            .get_mut(&4098)
            .unwrap()
            .body
            .instructions = vec![Instruction {
            opcode: 0x1b,
            on_true: 254,
            on_false: 255,
            operands: [0, 0, side, facing, 0, 0, 2, 0],
        }];
        let mut outcomes = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut goal = None;
            let mut finished = false;
            for _ in 0..5000 {
                let step = s.run_fixture_tick(&p, budget).unwrap();
                if let Some(route) = s.routes.values().next() {
                    assert_eq!(
                        s.vm.world.objects[&1].person.as_ref().unwrap()[&37],
                        route.goal.entry_flags
                    );
                    goal = Some(route.goal.clone());
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    let forged = bad.state.routes.values_mut().next().unwrap();
                    forged.goal.entry_flags = 0;
                    assert!(bad.restore(&p).is_err());
                }
                s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
                finished |= step
                    .events
                    .iter()
                    .any(|e| e.detail == "original-lot native route cleanup completed: true");
                if step.stop != crate::StepStop::Budget {
                    if finished {
                        break;
                    }
                    s.advance_fixture_tick().unwrap();
                }
            }
            assert!(finished);
            let goal = goal.unwrap();
            assert_ne!(goal.entry_flags, 0);
            assert_eq!(s.bodies[&1].position, goal.position);
            assert_eq!(s.bodies[&1].facing, goal.facing);
            outcomes.push(s);
        }
        assert_eq!(outcomes[0], outcomes[1], "side {side}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_route_entry_and_portal_interruptions_match_original_and_restore() {
    use serde_json::Value;
    use sims_formats::behavior::Instruction;
    let report: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/route-portal-interrupt.json"
    ))
    .unwrap();
    let mut compared = 0;
    let mut restored = 0;
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let expected = &row["original"];
        // Portal selection itself is covered by the dispatch/room-gate
        // corpora. Here the connected adapter runs through cancellation or
        // the boundary immediately before same-room destination search.
        if expected["outcome"] == "portal" {
            continue;
        }
        let (mut p, mut s) = walking_fixture();
        let m = p.modules.get_mut("objects").unwrap();
        let mut script = m.routines[&4098].clone();
        script.body.instructions = vec![Instruction {
            opcode: 2,
            on_true: if c["success"].as_bool().unwrap() {
                254
            } else {
                255
            },
            on_false: 255,
            operands: [0, 0, 0, 0, 0, 5, 8, 7],
        }];
        m.routines.insert(4100, script);
        m.object_functions.get_mut(&1).unwrap()[15].action = 4100;
        m.object_definition_words.get_mut(&1).unwrap()[10] = 1;
        for (id, head, next) in [(2, None, Some(3)), (3, Some(2), None)] {
            s.vm.world.objects.get_mut(&id).unwrap().multitile = Some(sims_vm::multitile::Part {
                head,
                next,
                origin: [0; 3],
                offset: [0; 3],
            });
        }
        let lot = s
            .architecture
            .as_ref()
            .unwrap()
            .original
            .as_ref()
            .unwrap()
            .clone();
        s.architecture = Some(Architecture::from_original(lot, &p).unwrap());
        s.vm.world.actions = Some(sims_vm::action::WorldState {
            actors: [(1, Default::default())].into(),
            ..Default::default()
        });
        let thread = s.vm.object_threads[&1];
        s.vm.run_slice(&p, thread, 1).unwrap();
        let sims_vm::Status::AwaitingRoute(request) = s.vm.threads[&thread].status.clone() else {
            panic!("route absent")
        };
        assert!(
            !s.service_original_portals(&p, thread, &request, &mut vec![])
                .unwrap()
        );
        s.visited.insert(thread);
        s.visit = Some((thread, Some(1)));
        let crossing = c["state"] == 2;
        let previous = c["previous"].as_bool().unwrap();
        let control = s.route_control.get_mut(&request.id).unwrap();
        control.original.as_mut().unwrap().crossing = crossing;
        control.original.as_mut().unwrap().ignored = 7;
        control.last_portal = previous.then_some(2);
        s.portal_cache.entry(1).or_default().costs.insert(2, 1001.);
        let different = c["different"].as_bool().unwrap();
        // Supply rooms independently of geometry, as in the native probe.
        s.vm.world
            .objects
            .get_mut(&2)
            .unwrap()
            .data
            .insert(29, if different { 1 } else { 0 });
        if crossing && previous {
            s.vm.push_native_portal_service(&p, thread, "objects".into(), 4100, 2)
                .unwrap();
            s.vm.run_slice(&p, thread, 1000).unwrap();
            assert_eq!(
                s.vm.threads[&thread].service.as_ref().unwrap().result,
                Some(c["success"].as_bool().unwrap())
            );
        }
        // The native probe enters after the target-tree has returned. Its
        // supplied fields must follow, not precede, that frame's own cleanup.
        let actor = s.vm.world.objects.get_mut(&1).unwrap();
        actor.data.insert(9, 7);
        actor.person.as_mut().unwrap().extend([
            (39, 0),
            (62, 9),
            (68, c["person68"].as_i64().unwrap() as i16),
            (72, 99),
            (73, 23),
        ]);
        // This fixture supplies the queue query and the independently native-
        // checked animator gate; it does not fabricate a loaded character.
        s.vm.threads.get_mut(&thread).unwrap().interrupted = c["requested"].as_bool().unwrap()
            && crate::contention::animator_can_interrupt(
                c["mode"].as_u64().unwrap() as u32,
                c["clip"].as_i64().map(|n| n as i32),
            );
        let mut step = crate::Step {
            stop: crate::StepStop::Waiting,
            trace: vec![],
            events: vec![],
        };
        let completed = expected["outcome"] == "completed";
        assert_eq!(
            s.service_original_portals(&p, thread, &request, &mut step.events)
                .unwrap(),
            completed,
            "{c}"
        );
        assert!(
            s.vm.threads[&thread].service.is_none(),
            "returned portal must be popped"
        );
        if completed {
            // Only the synthetic cross-room inputs differ from the actual
            // fixture's architecture. The real same-room cases serialize
            // the pending cleanup and resume before it changes any fields.
            if !different {
                s.sync_spatial_world();
                s = serde_json::from_slice::<crate::Snapshot>(
                    &serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap(),
                )
                .unwrap()
                .restore(&p)
                .unwrap_or_else(|error| panic!("{c}: {error}"));
                restored += 1;
            }
            assert_eq!(s.vm.world.objects[&1].person.as_ref().unwrap()[&62], 9);
            assert!(
                s.drive_native_route_completion(&p, &mut 1, &mut step)
                    .unwrap()
            );
            assert!(s.route_control.is_empty());
            assert!(!s.vm.threads[&thread].last_result);
        }
        let actor = &s.vm.world.objects[&1];
        for index in [39, 62, 72, 73] {
            assert_eq!(
                i64::from(actor.person.as_ref().unwrap()[&index]),
                expected[format!("person{index}")].as_i64().unwrap(),
                "{c}"
            );
        }
        assert_eq!(
            i64::from(actor.data[&9]),
            expected["data9"].as_i64().unwrap(),
            "{c}"
        );
        assert_eq!(
            s.portal_cache[&1].costs[&2], 1001.,
            "no failed-portal penalty: {c}"
        );
        assert!(s.route_feedback.is_empty());
        if completed && !different {
            s.sync_spatial_world();
            assert_eq!(crate::Snapshot::capture(&p, &s).restore(&p).unwrap(), s);
        }
        compared += 1;
    }
    assert_eq!(compared, 144);
    assert_eq!(restored, 32);
}

#[test]
fn native_portal_returns_after_nested_route_and_resumes_from_snapshots() {
    use sims_formats::behavior::Instruction;
    let (mut p, mut initial) = routed_walking_fixture();
    let m = p.modules.get_mut("objects").unwrap();
    let mut script = m.routines[&4098].clone();
    script.body.instructions = vec![
        Instruction {
            opcode: 0x1b,
            on_true: 1,
            on_false: 255,
            operands: [0, 0, 0, 254, 0, 0, 2, 0],
        },
        Instruction {
            opcode: 2,
            on_true: 255,
            on_false: 255,
            operands: [72, 0, 99, 0, 0, 5, 18, 7],
        },
    ];
    m.routines.insert(4100, script);
    m.object_functions.get_mut(&1).unwrap()[15].action = 4100;
    m.object_definition_words.get_mut(&1).unwrap()[10] = 1;
    for (id, head, next) in [(2, None, Some(3)), (3, Some(2), None)] {
        initial.vm.world.objects.get_mut(&id).unwrap().multitile = Some(sims_vm::multitile::Part {
            head,
            next,
            origin: [0; 3],
            offset: [0; 3],
        });
    }
    let lot = initial
        .architecture
        .as_ref()
        .unwrap()
        .original
        .as_ref()
        .unwrap()
        .clone();
    initial.architecture = Some(Architecture::from_original(lot, &p).unwrap());
    let thread = initial.vm.object_threads[&1];
    initial
        .vm
        .world
        .actions
        .get_or_insert_with(Default::default)
        .actors
        .insert(1, Default::default());
    initial
        .vm
        .world
        .objects
        .get_mut(&1)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(72, 7);
    initial.vm.run_slice(&p, thread, 1).unwrap();
    let sims_vm::Status::AwaitingRoute(request) = initial.vm.threads[&thread].status.clone() else {
        panic!("route expected")
    };
    assert!(
        !initial
            .service_original_portals(&p, thread, &request, &mut vec![])
            .unwrap()
    );
    // Enter at the native script-yield boundary. Both objects already have
    // room 0, deliberately covering a false callback after room arrival.
    let control = initial.route_control.get_mut(&request.id).unwrap();
    control.last_portal = Some(2);
    control.original.as_mut().unwrap().crossing = true;
    initial
        .portal_cache
        .entry(1)
        .or_default()
        .costs
        .extend([(2, 1001.), (3, 1001.)]);
    initial.person(1, 72, 0).unwrap();
    initial
        .vm
        .push_native_portal_service(&p, thread, "objects".into(), 4100, 2)
        .unwrap();
    assert_eq!(initial.vm.world.objects[&2].data[&62], 1);
    let mut outcomes = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        let mut saved_nested = false;
        let mut returned = false;
        let mut finished = false;
        for _ in 0..5000 {
            let step = s.run_fixture_tick(&p, budget).unwrap();
            saved_nested |= s.route_control.len() == 2;
            assert_ne!(
                step.stop,
                crate::StepStop::Faulted,
                "{:?}",
                s.vm.threads
                    .iter()
                    .map(|(id, t)| (id, &t.status))
                    .collect::<Vec<_>>()
            );
            s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap_or_else(|error| {
                let t = &s.vm.threads[&thread];
                panic!("{error}; portal frames {:?}; status {:?}; service {:?}",
                    t.stack.iter().map(|f| (&f.location,f.continuation)).collect::<Vec<_>>(),
                    t.status,t.service);
            });
            returned |= step.events.iter().any(|e| {
                e.detail == "original portal #2 returned false; destination room reached: true"
            });
            finished |= returned && s.route_control.is_empty();
            if step.stop != crate::StepStop::Budget {
                if finished {
                    break;
                }
                s.advance_fixture_tick().unwrap();
            }
        }
        assert!(saved_nested && returned && finished);
        assert_eq!(s.portal_cache[&1].costs[&2], 1001.);
        assert_eq!(s.vm.world.objects[&2].data[&62], 0);
        assert_eq!(s.vm.world.objects[&1].person.as_ref().unwrap()[&72], 7);
        outcomes.push(s);
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[test]
fn house_refresh_retains_portal_callback_and_consumes_stale_costs_after_restore() {
    use sims_formats::behavior::Instruction;
    for invalidated in [false, true] {
        let (mut p, mut s) = walking_fixture();
        let m = p.modules.get_mut("objects").unwrap();
        let mut script = m.routines[&4098].clone();
        script.body.instructions = vec![Instruction {
            opcode: 2,
            on_true: 255,
            on_false: 255,
            operands: [0, 0, 0, 0, 0, 5, 8, 7],
        }];
        m.routines.insert(4100, script);
        m.object_functions.get_mut(&1).unwrap()[15].action = 4100;
        m.object_definition_words.get_mut(&1).unwrap()[10] = 1;
        for (id, head, next) in [(2, None, Some(3)), (3, Some(2), None)] {
            s.vm.world.objects.get_mut(&id).unwrap().multitile = Some(sims_vm::multitile::Part {
                head,
                next,
                origin: [0; 3],
                offset: [0; 3],
            });
        }
        let lot = s
            .architecture
            .as_ref()
            .unwrap()
            .original
            .as_ref()
            .unwrap()
            .clone();
        s.architecture = Some(Architecture::from_original(lot, &p).unwrap());
        s.vm.world.actions = Some(sims_vm::action::WorldState {
            actors: [(1, Default::default())].into(),
            ..Default::default()
        });
        let thread = s.vm.object_threads[&1];
        s.vm.run_slice(&p, thread, 1).unwrap();
        let sims_vm::Status::AwaitingRoute(request) = s.vm.threads[&thread].status.clone() else {
            panic!()
        };
        assert!(
            !s.service_original_portals(&p, thread, &request, &mut vec![])
                .unwrap()
        );
        let control = s.route_control.get_mut(&request.id).unwrap();
        control.original.as_mut().unwrap().crossing = true;
        control.original.as_mut().unwrap().ignored = 7;
        control.last_portal = Some(2);
        s.portal_cache
            .entry(1)
            .or_default()
            .costs
            .extend([(2, 1001.), (3, 1101.)]);
        s.vm.push_native_portal_service(&p, thread, "objects".into(), 4100, 2)
            .unwrap();
        if invalidated {
            let before = s.clone();
            s.invalidate_original_route_caches().unwrap();
            let mut expected = before;
            expected
                .route_control
                .get_mut(&request.id)
                .unwrap()
                .original
                .as_mut()
                .unwrap()
                .cache_valid = false;
            for value in expected
                .portal_cache
                .get_mut(&1)
                .unwrap()
                .costs
                .values_mut()
            {
                *value = 10001.;
            }
            assert_eq!(
                s, expected,
                "refresh must retain frames, queues, RNG, paths and world state"
            );
        }
        s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
        assert_eq!(
            s.route_control[&request.id]
                .original
                .as_ref()
                .unwrap()
                .cache_valid,
            !invalidated
        );
        s.vm.run_slice(&p, thread, 1000).unwrap();
        s.person(1, 72, 99).unwrap();
        s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
        let rng = s.vm.rng_state;
        assert!(
            !s.service_original_portals(&p, thread, &request, &mut vec![])
                .unwrap()
        );
        assert!(s.vm.threads[&thread].service.is_none());
        assert_eq!(
            s.route_control[&request.id].last_portal,
            (!invalidated).then_some(2)
        );
        assert!(
            s.route_control[&request.id]
                .original
                .as_ref()
                .unwrap()
                .cache_valid
        );
        assert_eq!(s.vm.world.objects[&1].person.as_ref().unwrap()[&72], 7);
        assert_eq!(s.vm.rng_state, rng);
        assert_eq!(
            s.portal_cache[&1].costs[&2],
            if invalidated { 10001. } else { 1001. }
        );
        assert_eq!(
            s.portal_cache[&1].costs[&3],
            if invalidated { 10001. } else { 1101. }
        );
        assert_eq!(crate::Snapshot::capture(&p, &s).restore(&p).unwrap(), s);
    }
}

#[test]
fn native_empty_route_retries_and_restores_original_failure_feedback() {
    use sims_formats::behavior::Instruction;
    let (mut p, mut initial) = walking_fixture();
    initial.vm.world.actions = Some(sims_vm::action::WorldState {
        actors: [(1, Default::default())].into(),
        ..Default::default()
    });
    initial
        .vm
        .world
        .objects
        .get_mut(&2)
        .unwrap()
        .routing_slots
        .as_mut()
        .unwrap()[0]
        .0[0] = 0;
    let m = p.modules.get_mut("objects").unwrap();
    m.routines.get_mut(&4099).unwrap().body.instructions[1].on_true = 255;
    m.routines.get_mut(&4098).unwrap().body.instructions[0].operands[4] = 0;
    let mut feedback = m.routines[&4096].clone();
    feedback.name = "Route failure feedback".into();
    feedback.body.args = 4;
    feedback.body.instructions = vec![
        Instruction {
            opcode: 2,
            on_true: 1,
            on_false: 253,
            operands: [4, 0, 0, 0, 0, 5, 8, 9],
        },
        Instruction {
            opcode: 2,
            on_true: 2,
            on_false: 253,
            operands: [5, 0, 1, 0, 0, 5, 8, 9],
        },
        Instruction {
            opcode: 2,
            on_true: 3,
            on_false: 253,
            operands: [6, 0, 1, 0, 0, 3, 8, 7],
        },
        Instruction {
            opcode: 2,
            on_true: 255,
            on_false: 253,
            operands: [62, 0, 99, 0, 0, 5, 18, 7],
        },
    ];
    m.routines.insert(398, feedback);
    let mut outcomes = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        let mut saw_retry = false;
        let mut saved_feedback = false;
        let mut done = false;
        for _ in 0..1000 {
            let step = s.run_fixture_tick(&p, budget).unwrap();
            saw_retry |= step.events.iter().any(|e| {
                e.detail
                    .starts_with("original-lot route enters native retry")
            });
            assert!(
                !matches!(step.stop, crate::StepStop::Faulted),
                "{:?} {:?}",
                step,
                s.vm.threads
            );
            saved_feedback |= !s.route_feedback.is_empty();
            let snap = crate::Snapshot::capture(&p, &s);
            s = serde_json::from_slice::<crate::Snapshot>(&serde_json::to_vec(&snap).unwrap())
                .unwrap()
                .restore(&p)
                .unwrap();
            if s.vm.world.objects[&1].temps[6] == 1 && s.route_control.is_empty() {
                done = true;
            }
            if step.stop != crate::StepStop::Budget {
                if done {
                    break;
                }
                s.advance_fixture_tick().unwrap();
            }
        }
        assert!(done && saw_retry);
        assert_eq!(saved_feedback, budget == 1);
        assert_eq!(&s.vm.world.objects[&1].temps[4..7], &[8, 3, 1]);
        assert_eq!(s.vm.world.objects[&1].person.as_ref().unwrap()[&62], 8);
        assert!(s.native_lot.as_ref().unwrap().route.is_none());
        assert_eq!(s.vm.world.positions[&1], initial.vm.world.positions[&1]);
        outcomes.push(s);
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[test]
fn native_coincident_route_preserves_endpoint_points_across_resume() {
    for facing in [-3i32, -2, -1, 0, 2] {
        let (p, mut initial) = routed_walking_fixture();
        let words = &mut initial
            .vm
            .world
            .objects
            .get_mut(&2)
            .unwrap()
            .routing_slots
            .as_mut()
            .unwrap()[0]
            .0;
        words[0] = 0;
        words[13] = facing as u32;
        let mut outcomes = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut finished = false;
            let mut saved_route = false;
            for _ in 0..1000 {
                let step = s.run_fixture_tick(&p, budget).unwrap();
                if let Some(route) = s.routes.values().next() {
                    saved_route = true;
                    assert_eq!(route.path, vec![Point::new(40, 40)]);
                    assert_eq!(route.original_path, Some(vec![Point::new(40, 40); 2]));
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    bad.state
                        .routes
                        .values_mut()
                        .next()
                        .unwrap()
                        .original_path
                        .as_mut()
                        .unwrap()
                        .pop();
                    assert!(bad.restore(&p).is_err());
                }
                let snapshot = crate::Snapshot::capture(&p, &s);
                s = serde_json::from_slice::<crate::Snapshot>(
                    &serde_json::to_vec(&snapshot).unwrap(),
                )
                .unwrap()
                .restore(&p)
                .unwrap();
                finished |= step
                    .events
                    .iter()
                    .any(|e| e.detail == "original-lot native route cleanup completed: true");
                if step.stop != crate::StepStop::Budget {
                    if finished {
                        break;
                    }
                    s.advance_fixture_tick().unwrap();
                }
            }
            assert!(finished, "facing {facing}");
            assert_eq!(saved_route, budget == 1 || matches!(facing, -1 | 2));
            assert_eq!(s.vm.world.positions[&1], initial.vm.world.positions[&1]);
            outcomes.push(s);
        }
        assert_eq!(outcomes[0], outcomes[1]);
    }
}

#[test]
fn native_walk_commits_resume_inside_callbacks_without_replaying_steps() {
    let (p, initial) = routed_walking_fixture();
    let mut outcomes = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        let mut saved_callback = false;
        let mut finished = false;
        for _ in 0..5000 {
            let step = s.run_fixture_tick(&p, budget).unwrap();
            let n = s.native_lot.as_ref().unwrap();
            if n.movement.is_some() && s.vm.threads.values().any(|t| t.callback.is_some()) {
                saved_callback = true;
                assert!(s.advance_fixture_tick().is_err());
                let mut bad = crate::Snapshot::capture(&p, &s);
                bad.state.native_lot.as_mut().unwrap().movement = None;
                assert!(bad.restore(&p).is_err());
            }
            s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
            if step
                .events
                .iter()
                .any(|e| e.detail == "original-lot native route cleanup completed: true")
            {
                finished = true;
            }
            if step.stop != crate::StepStop::Budget {
                if finished {
                    break;
                }
                s.advance_fixture_tick().unwrap();
            }
        }
        assert!(finished);
        assert_eq!(saved_callback, budget == 1);
        assert_eq!(
            s.vm.world.positions[&1],
            Placement {
                x: 40,
                y: 56,
                level: 1
            }
        );
        assert_eq!(s.bodies[&1].position, Point::new(56, 40));
        assert_eq!(
            s.native_lot.as_ref().unwrap().membership.0[&1],
            s.vm.world.objects[&1].data[&29]
        );
        assert!(s.routes.is_empty());
        outcomes.push(s);
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[test]
fn native_rejected_walk_saves_callback_and_preserves_last_physical_position() {
    use sims_formats::behavior::Instruction;
    let (mut p, mut initial) = routed_walking_fixture();
    let instructions = &mut p
        .modules
        .get_mut("objects")
        .unwrap()
        .routines
        .get_mut(&4099)
        .unwrap()
        .body
        .instructions;
    instructions[1].on_true = 2;
    instructions.push(Instruction {
        opcode: 2,
        on_true: 254,
        on_false: 255,
        operands: [33, 0, 0, 0, 0, 2, 6, 7],
    });
    initial.vm.world.globals.insert(33, 0);
    let actor = initial.vm.object_threads[&1];
    for i in 0..1000 {
        let step = initial.run_fixture_tick(&p, 1).unwrap();
        if step.stop != crate::StepStop::Budget {
            assert!(initial.routes.contains_key(&actor));
            initial.advance_fixture_tick().unwrap();
            break;
        }
        assert!(i < 999);
    }
    let request = initial.routes[&actor].request;
    // Dynamic obstacle permission changes after path selection. The check
    // callback now rejects the next step, including a zero-distance turn.
    initial.vm.world.globals.insert(33, 1);
    let control = initial.route_control.get_mut(&request).unwrap();
    control.controller.result = 13;
    control.controller.stage = 2;
    control.original.as_mut().unwrap().blocked_object = 2;
    let before_position = initial.vm.world.positions[&1];
    let before_path = initial.native_lot.as_ref().unwrap().route_paths[&1].clone();
    let before_calls = initial.vm.world.objects[&3].temps[1];
    for i in 0..1000 {
        let step = initial.run_fixture_tick(&p, 1).unwrap();
        assert_eq!(step.stop, crate::StepStop::Budget);
        if initial.native_lot.as_ref().unwrap().movement.is_some()
            && initial.vm.threads[&initial.vm.object_threads[&3]]
                .callback
                .is_some()
        {
            break;
        }
        assert!(i < 999);
    }
    let snapshot = crate::Snapshot::capture(&p, &initial);
    let mut bad = snapshot.clone();
    bad.state.native_lot.as_mut().unwrap().movement = None;
    assert!(bad.restore(&p).is_err());
    initial = serde_json::from_slice::<crate::Snapshot>(&serde_json::to_vec(&snapshot).unwrap())
        .unwrap()
        .restore(&p)
        .unwrap();
    let mut outcomes = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        let mut rejected = 0;
        for i in 0..100 {
            assert!(s.advance_fixture_tick().is_err());
            let mut step = crate::Step {
                stop: crate::StepStop::Waiting,
                trace: vec![],
                events: vec![],
            };
            let done = s
                .drive_pending_native_movement(&p, &mut { budget }, &mut step)
                .unwrap();
            rejected += step.events.iter().filter(|e| e.detail ==
                "original-lot rejected walking step: placement 11; native return state 6, route result 13").count();
            s = serde_json::from_slice::<crate::Snapshot>(
                &serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap(),
            )
            .unwrap()
            .restore(&p)
            .unwrap();
            if done {
                break;
            }
            assert!(i < 99);
        }
        assert_eq!(rejected, 1);
        assert_eq!(s.vm.world.positions[&1], before_position);
        assert_eq!(s.native_lot.as_ref().unwrap().route_paths[&1], before_path);
        assert!(s.native_lot.as_ref().unwrap().movement.is_none());
        assert!(!s.routes.contains_key(&actor));
        let control = &s.route_control[&request];
        assert!(control.deciding);
        assert_eq!(control.controller.phase, crate::contention::Phase::Retry);
        assert_eq!(control.controller.result, 13);
        assert_eq!(control.controller.stage, 2);
        assert_eq!(control.original.as_ref().unwrap().blocked_object, 2);
        assert_eq!(s.vm.world.objects[&1].person.as_ref().unwrap()[&72], 0);
        assert_eq!(s.vm.world.objects[&3].temps[1], before_calls + 1);
        assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, 11);
        assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.blocker, Some(3));
        assert_eq!(s.vm.tick, initial.vm.tick);
        // Resume the normal scheduler from exactly this host-return state.
        // Both budgeting histories must produce the same subsequent work.
        let after = s.run_fixture_tick(&p, 1).unwrap();
        outcomes.push((s, after));
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[test]
fn native_floor_snap_saves_acceptance_and_commits_once_for_people_and_objects() {
    for person in [true, false] {
        for accepted in [true, false] {
            let (mut p, mut initial) = walking_fixture();
            let m = p.modules.get_mut("objects").unwrap();
            let ins = &mut m.routines.get_mut(&4098).unwrap().body.instructions[0];
            ins.opcode = 0x2e;
            ins.operands = [0, 0, 3, 0, 0, 0, 0, 0];
            if !accepted {
                m.routines.get_mut(&4099).unwrap().body.instructions[1].on_true = 255;
                initial
                    .vm
                    .world
                    .objects
                    .get_mut(&2)
                    .unwrap()
                    .routing_slots
                    .as_mut()
                    .unwrap()[0]
                    .0[0] = 0;
            }
            if !person {
                m.object_definition_words.get_mut(&2).unwrap()[9] = 4;
                initial.vm.world.objects.get_mut(&1).unwrap().person = None;
                initial
                    .vm
                    .world
                    .person
                    .as_mut()
                    .unwrap()
                    .person_order
                    .clear();
                initial.native_lot.as_mut().unwrap().membership =
                    RoomMembership::constructed(&initial.vm.world);
                initial.architecture.as_mut().unwrap().footprints.remove(&2);
                initial
                    .architecture
                    .as_mut()
                    .unwrap()
                    .bind_footprints(&p)
                    .unwrap();
            }
            let actor = initial.vm.object_threads[&1];
            initial.vm.start_object_lifecycle(&p, 2).unwrap();
            let mut outcomes = vec![];
            for budget in [1, 1000] {
                let mut s = initial.clone();
                let mut saved = false;
                let mut finished = false;
                for _ in 0..1000 {
                    let step = s.run_fixture_tick(&p, budget).unwrap();
                    if s.native_lot.as_ref().unwrap().snap.is_some()
                        && s.vm.threads.values().any(|t| t.callback.is_some())
                    {
                        assert!(s.advance_fixture_tick().is_err());
                        saved = true;
                        let mut bad = crate::Snapshot::capture(&p, &s);
                        bad.state.native_lot.as_mut().unwrap().snap = None;
                        assert!(bad.restore(&p).is_err());
                        let mut bad = crate::Snapshot::capture(&p, &s);
                        bad.state
                            .native_lot
                            .as_mut()
                            .unwrap()
                            .snap
                            .as_mut()
                            .unwrap()
                            .acceptance = None;
                        assert!(bad.restore(&p).is_err());
                    }
                    s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
                    finished |= step
                        .events
                        .iter()
                        .any(|e| e.detail.starts_with("native Snap #1"));
                    if step.stop != crate::StepStop::Budget {
                        break;
                    }
                }
                assert!(finished, "person={person}, accepted={accepted}");
                assert_eq!(saved, budget == 1);
                assert_eq!(
                    s.vm.threads[&actor].status,
                    sims_vm::Status::Returned(accepted)
                );
                assert_eq!(s.vm.tick, initial.vm.tick);
                assert_eq!(
                    s.vm.world.positions[&1],
                    Placement {
                        x: 40,
                        y: if accepted { 56 } else { 40 },
                        level: 1
                    }
                );
                assert_eq!(
                    s.native_lot.as_ref().unwrap().snap_offsets.contains_key(&1),
                    accepted && person
                );
                if !accepted {
                    assert_eq!(s.vm.world.objects[&1].data[&54], 3);
                }
                outcomes.push(s);
            }
            assert_eq!(outcomes[0], outcomes[1]);
        }
    }
}

fn snap_shoo_fixture() -> (Program, Simulation) {
    let (mut p, mut initial) = walking_fixture();
    install_shoo_program_resources(&mut p);
    initial.vm.world.refresh = Some(Default::default());
    initial
        .architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&p)
        .unwrap();
    let blocker = initial.vm.world.objects.get_mut(&3).unwrap();
    blocker.person = Some((0..256).map(|n| (n, 0)).collect());
    blocker.definition.as_mut().unwrap().object = 4;
    blocker.definition.as_mut().unwrap().guid = 4;
    blocker.data.insert(8, 8);
    blocker.data.insert(4, 1);
    blocker.data.insert(42, 259);
    // Keep its already sleeping Main; this synchronous operation must not run it.
    initial
        .vm
        .world
        .person
        .as_mut()
        .unwrap()
        .person_order
        .push(3);
    initial.vm.lifecycles.get_mut(&3).unwrap().definition = 4;
    initial.vm.start_object_lifecycle(&p, 2).unwrap();
    initial.native_lot.as_mut().unwrap().membership =
        RoomMembership::constructed(&initial.vm.world);
    initial.vm.world.actions = Some(sims_vm::action::WorldState {
        actors: [(1, Default::default()), (3, Default::default())].into(),
        ..Default::default()
    });
    for id in [1, 3] {
        initial.vm.world.objects.get_mut(&id).unwrap().motives = Some([0; 16]);
    }
    initial
        .enable_household(
            &p,
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
    p.modules
        .get_mut("objects")
        .unwrap()
        .object_functions
        .get_mut(&1)
        .unwrap()[4]
        .action = 4100;

    let ins = &mut p
        .modules
        .get_mut("objects")
        .unwrap()
        .routines
        .get_mut(&4098)
        .unwrap()
        .body
        .instructions[0];
    ins.opcode = 0x2e;
    ins.operands = [0, 0, 3, 0, 2, 0, 0, 0];
    p.modules
        .get_mut("objects")
        .unwrap()
        .routines
        .get_mut(&4099)
        .unwrap()
        .body
        .instructions[1]
        .on_true = 255;
    initial
        .vm
        .world
        .objects
        .get_mut(&2)
        .unwrap()
        .routing_slots
        .as_mut()
        .unwrap()[0]
        .0[0] = 0;
    let priority = 70_003; // Native action priority is not its i16 data33 mirror.
    initial
        .vm
        .world
        .actions
        .as_mut()
        .unwrap()
        .actors
        .get_mut(&1)
        .unwrap()
        .active = Some(sims_vm::action::Action {
        id: 100,
        target: 2,
        icon_owner: 2,
        routine: 4096,
        menu_id: 0,
        priority,
        ..Default::default()
    });
    for id in [1, 3] {
        initial.vm.world.objects.get_mut(&id).unwrap().native_flags = Some(0);
    }
    (p, initial)
}

#[test]
fn native_snap_shoo_keeps_failed_snap_and_saves_factory_queue_and_rejection() {
    let (p, initial) = snap_shoo_fixture();
    let caller = initial.vm.object_threads[&1];
    let before = initial.vm.world.positions[&3];
    let priority = 70_003;
    for (accepted, cancelled) in [(true, false), (true, true), (false, false)] {
        let mut results = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            if !accepted || cancelled {
                let actions = s.vm.world.actions.as_mut().unwrap();
                actions.serial = 8;
                actions.actors.get_mut(&3).unwrap().queue.pending = (1..=8)
                    .map(|id| sims_vm::action::Action {
                        id,
                        target: 2,
                        icon_owner: 2,
                        priority: if cancelled { -1 } else { priority + 1 },
                        menu_id: 0,
                        routine: 4096,
                        ..Default::default()
                    })
                    .collect();
            }
            let mut saved_callback = false;
            let mut finished = false;
            for n in 0..5000 {
                assert!(n < 4999, "Snap shoo did not terminate");
                let step = s.run_fixture_tick(&p, budget).unwrap();
                saved_callback |= s.native_lot.as_ref().unwrap().snap.is_some()
                    && s.vm.threads.values().any(|t| t.callback.is_some());
                if let Some(e) = &s.native_lot.as_ref().unwrap().snap {
                    let mut value = serde_json::to_value(e).unwrap();
                    if value["failure"]["Shoo"].is_object() {
                        value["failure"]["Shoo"]["object"] = serde_json::json!(2);
                        let mut bad = crate::Snapshot::capture(&p, &s);
                        bad.state.native_lot.as_mut().unwrap().snap =
                            Some(serde_json::from_value(value).unwrap());
                        assert!(bad.restore(&p).is_err(), "rebound shoo helper accepted");
                    }
                }
                s = serde_json::from_slice::<crate::Snapshot>(
                    &serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap(),
                )
                .unwrap()
                .restore(&p)
                .unwrap();
                finished |= step
                    .events
                    .iter()
                    .any(|e| e.detail.starts_with("native Snap #1"));
                if step.stop != crate::StepStop::Budget {
                    break;
                }
            }
            assert!(finished);
            assert_eq!(saved_callback, budget == 1);
            assert_eq!(s.vm.tick, initial.vm.tick);
            assert_eq!(
                s.vm.threads[&caller].status,
                sims_vm::Status::Returned(false)
            );
            assert_eq!(s.vm.world.positions[&1], before);
            assert_eq!(s.vm.world.positions[&3], before);
            assert_eq!(s.vm.world.objects[&1].data[&52], i16::from(accepted));
            assert!(
                s.route_control.is_empty(),
                "standalone shoo acquired a route link"
            );
            assert!(s.native_lot.as_ref().unwrap().snap.is_none());
            assert!(s.deletion.is_empty());
            if accepted {
                let action = &s.vm.world.actions.as_ref().unwrap().actors[&3]
                    .queue
                    .pending[0];
                assert_eq!(
                    (action.target, action.menu_id, action.priority),
                    (5, 3, priority)
                );
                assert_eq!(s.vm.world.positions[&5], before);
                assert!(!s.unplaced.contains(&5));
                assert_eq!(
                    s.vm.world.objects[&5].routing_slots,
                    Some(vec![crate::contention::placement_shoo_descriptor()])
                );
                assert_eq!(
                    s.vm.world.objects[&2].temps[1],
                    if cancelled { 8 } else { 0 }
                );
            } else {
                assert!(!s.vm.world.objects.contains_key(&5));
                assert!(
                    !s.native_lot
                        .as_ref()
                        .unwrap()
                        .placement
                        .links
                        .next
                        .contains_key(&5)
                );
            }
            results.push(s);
        }
        assert_eq!(results[0], results[1]);
    }
}

#[test]
fn native_snap_shoo_rechecks_live_globals_after_saved_callbacks() {
    // Supplied callback effects: the native wrapper reads these globals and
    // field52 only after 45bd5e returns. Acceptance must not be re-entered.
    for (accepted, field52, expected) in [(true, 7, 1), (false, 7, 7), (false, 0, 2)] {
        let (p, mut s) = snap_shoo_fixture();
        let actions = s.vm.world.actions.as_mut().unwrap();
        actions.serial = 8;
        actions.actors.get_mut(&3).unwrap().queue.pending = (1..=8)
            .map(|id| sims_vm::action::Action {
                id,
                target: 2,
                icon_owner: 2,
                priority: if accepted { -1 } else { 80_000 },
                menu_id: 0,
                routine: 4096,
                ..Default::default()
            })
            .collect();
        let mut injected = false;
        let mut finished = false;
        for _ in 0..5000 {
            let step = s.run_fixture_tick(&p, 1).unwrap();
            if !injected && let Some(e) = &s.native_lot.as_ref().unwrap().snap {
                let value = serde_json::to_value(e).unwrap();
                if matches!(
                    value["failure"]["Shoo"]["phase"].as_str(),
                    Some("Publish" | "Detach" | "Destroy")
                ) {
                    s.native_lot.as_mut().unwrap().diagnostics =
                        crate::lot_acceptance::person::Diagnostics {
                            error: 1,
                            blocker: Some(2),
                        };
                    s.vm.world
                        .objects
                        .get_mut(&1)
                        .unwrap()
                        .data
                        .insert(52, field52);
                    injected = true;
                }
            }
            s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
            if step
                .events
                .iter()
                .any(|e| e.detail.starts_with("native Snap #1"))
            {
                finished = true;
                break;
            }
        }
        assert!(injected && finished);
        assert_eq!(s.vm.world.objects[&1].data[&52], expected);
        assert_eq!(s.vm.world.objects[&1].data[&54], 2);
        assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, 1);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_snap_shoo_callback_results_match_original_instructions() {
    let doc: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/snap-shoo-callbacks.json"
    ))
    .unwrap();
    let mut checked = 0;
    for row in doc["failures"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        assert!(n["error"].is_null());
        assert_eq!(n["before_shoo_result"], 0);
        assert_eq!(n["shoo"][1], c["priority"]);
        if c["priority"] != 70003 {
            continue;
        }
        let accepted = c["shoo_accepted"].as_bool().unwrap();
        let (p, mut s) = snap_shoo_fixture();
        let actions = s.vm.world.actions.as_mut().unwrap();
        actions.serial = 8;
        actions.actors.get_mut(&3).unwrap().queue.pending = (1..=8)
            .map(|id| sims_vm::action::Action {
                id,
                target: 2,
                icon_owner: 2,
                priority: if accepted { -1 } else { 80_000 },
                menu_id: 0,
                routine: 4096,
                ..Default::default()
            })
            .collect();
        let mut injected = false;
        let mut finished = false;
        for _ in 0..5000 {
            let step = s.run_fixture_tick(&p, 1).unwrap();
            if !injected && let Some(e) = &s.native_lot.as_ref().unwrap().snap {
                let value = serde_json::to_value(e).unwrap();
                if matches!(
                    value["failure"]["Shoo"]["phase"].as_str(),
                    Some("Publish" | "Detach" | "Destroy")
                ) {
                    let blocker = c["callback_blocker"].as_i64().unwrap() as i16;
                    s.native_lot.as_mut().unwrap().diagnostics =
                        crate::lot_acceptance::person::Diagnostics {
                            error: c["callback_error"].as_i64().unwrap() as i32,
                            blocker: (blocker != 0).then_some(blocker),
                        };
                    s.vm.world
                        .objects
                        .get_mut(&1)
                        .unwrap()
                        .data
                        .insert(52, c["callback_result"].as_i64().unwrap() as i16);
                    injected = true;
                }
            }
            s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
            if step
                .events
                .iter()
                .any(|e| e.detail.starts_with("native Snap #1"))
            {
                finished = true;
                break;
            }
        }
        assert!(injected && finished, "{row}");
        assert_eq!(
            s.native_lot.as_ref().unwrap().diagnostics.error,
            n["diagnostic_error"].as_i64().unwrap() as i32,
            "{row}"
        );
        assert_eq!(
            s.vm.world.objects[&1].data[&52],
            n["result"].as_i64().unwrap() as i16,
            "{row}"
        );
        assert_eq!(
            s.vm.world.objects[&1].data[&54],
            n["blocker"].as_i64().unwrap() as i16,
            "{row}"
        );
        assert_eq!(
            s.vm.threads[&s.vm.object_threads[&1]].status,
            sims_vm::Status::Returned(n["returned"] == 1),
            "{row}"
        );
        checked += 1;
    }
    assert_eq!(checked, 54);
}

#[test]
fn native_snap_shoo_missing_resource_is_an_explicit_saved_host_reply() {
    let (mut p, mut s) = snap_shoo_fixture();
    p.modules
        .get_mut("objects")
        .unwrap()
        .object_definition_words
        .remove(&3);
    s.architecture.as_mut().unwrap().footprints.remove(&0x7c4);
    let error = s.run_fixture_tick(&p, 1000).unwrap_err();
    assert_eq!(
        error,
        "native placement shoo GoHere GUID 000007C4 is not loaded"
    );
    assert!(s.pending_snap_shoo_resource(&p).unwrap());
    s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
    assert!(s.pending_snap_shoo_resource(&p).unwrap());
    s.complete_missing_snap_shoo_resource(&p).unwrap();
    assert!(!s.pending_snap_shoo_resource(&p).unwrap());
    s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
    let step = s.run_fixture_tick(&p, 1000).unwrap();
    assert!(
        step.events
            .iter()
            .any(|e| e.detail.starts_with("native Snap #1"))
    );
    assert_eq!(s.vm.world.objects[&1].data[&52], 0);
    assert_eq!(s.vm.world.objects[&1].data[&54], 3);
    assert_eq!(s.vm.world.objects.len(), 4);
    assert!(s.complete_missing_snap_shoo_resource(&p).is_err());
}

#[test]
fn native_snap_shoo_skips_existing_active_or_pending_move() {
    for active in [true, false] {
        let (p, mut s) = snap_shoo_fixture();
        let object = s.vm.world.objects.get_mut(&4).unwrap();
        object.definition.as_mut().unwrap().object = 3;
        object.definition.as_mut().unwrap().guid = 0x7c4;
        s.vm.lifecycles.get_mut(&4).unwrap().definition = 3;
        s.vm.lifecycles.get_mut(&4).unwrap().init = 4100;
        let action = sims_vm::action::Action {
            id: 1,
            target: 4,
            icon_owner: 4,
            menu_id: 3,
            routine: 4096,
            ..Default::default()
        };
        let actions = s.vm.world.actions.as_mut().unwrap();
        actions.serial = 1;
        let owner = actions.actors.get_mut(&3).unwrap();
        if active {
            owner.active = Some(action);
        } else {
            owner.queue.pending.push_back(action);
        }
        let original_actions = actions.clone();
        let mut finished = false;
        for _ in 0..5000 {
            let step = s.run_fixture_tick(&p, 1).unwrap();
            s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
            if step
                .events
                .iter()
                .any(|e| e.detail.starts_with("native Snap #1"))
            {
                finished = true;
                break;
            }
        }
        assert!(finished);
        assert_eq!(s.vm.world.objects.len(), 4);
        assert_eq!(s.vm.world.objects[&1].data[&52], 0);
        assert_eq!(s.vm.world.objects[&1].data[&54], 3);
        assert_eq!(s.vm.world.actions.as_ref().unwrap(), &original_actions);
    }
}

#[test]
fn native_multipart_snap_resumes_selected_member_and_publishes_facing_after_callbacks() {
    use sims_formats::behavior::Instruction;
    for selected in [4, 5] {
        for accepted in [false, true] {
            let (mut p, mut initial) = walking_fixture();
            let m = p.modules.get_mut("objects").unwrap();
            m.object_functions.get_mut(&2).unwrap()[1].action = 4096;
            m.routines.get_mut(&4098).unwrap().body.instructions = vec![Instruction {
                opcode: 0x2e,
                on_true: 254,
                on_false: 255,
                operands: [0, 0, 3, 0, 2, 0, 0, 0],
            }];
            let mut callback = m.routines[&4099].clone();
            callback.body.instructions = vec![
                Instruction {
                    opcode: 2,
                    on_true: 1,
                    on_false: 255,
                    operands: [0, 0, 1, 0, 0, 5, 8, 3],
                }, // Temp0 = current facing.
                Instruction {
                    opcode: 2,
                    on_true: 254,
                    on_false: 255,
                    operands: [2, 0, 1, 0, 0, 3, 8, 7],
                },
            ];
            m.routines.insert(4100, callback);
            for (definition, object) in [(3, 4), (4, 5)] {
                let mut words = m.object_definition_words[&1].clone();
                words[14] = definition as i16;
                words[10] = 7;
                words[11] = (object == 5) as i16;
                words[63] = (object == 4) as i16;
                m.object_definition_words.insert(definition, words);
                m.object_definition_order.push(definition);
                let mut functions = m.object_functions[&1].clone();
                functions[1].action = if object == selected { 4098 } else { 4096 };
                functions[9].action = 4100;
                m.object_functions.insert(definition, functions);
            }
            if !accepted {
                m.routines.get_mut(&4099).unwrap().body.instructions[1].on_true = 255;
            }
            let slot = &mut initial
                .vm
                .world
                .objects
                .get_mut(&2)
                .unwrap()
                .routing_slots
                .as_mut()
                .unwrap()[0]
                .0;
            slot[0] = if accepted { 16f32.to_bits() } else { 0 };
            slot[7] = 0x800 | 0x1000 | (1 << 6);
            let root = initial.vm.world.objects.get_mut(&4).unwrap();
            root.definition.as_mut().unwrap().object = 3;
            root.definition.as_mut().unwrap().guid = 3;
            root.data.extend([(4, 1), (8, 16), (42, 259)]);
            root.multitile = Some(sims_vm::multitile::Part {
                head: None,
                next: Some(5),
                origin: [0; 3],
                offset: [0; 3],
            });
            let mut sibling = root.clone();
            sibling.definition.as_mut().unwrap().object = 4;
            sibling.definition.as_mut().unwrap().guid = 4;
            sibling.data.insert(11, 5);
            sibling.multitile = Some(sims_vm::multitile::Part {
                head: Some(4),
                next: None,
                origin: [0, 1, 0],
                offset: [0, 1, 0],
            });
            initial.vm.world.objects.insert(5, sibling);
            initial
                .vm
                .world
                .positions
                .insert(5, initial.vm.world.positions[&4]);
            initial
                .vm
                .world
                .person
                .as_mut()
                .unwrap()
                .object_order
                .insert(0, 5);
            let native = initial.native_lot.as_mut().unwrap();
            native.placement.links.next.insert(5, 0);
            native.placement.bounds.insert(5, [0; 4]);
            initial.unplaced.insert(5);
            initial
                .architecture
                .as_mut()
                .unwrap()
                .bind_footprints(&p)
                .unwrap();
            initial.vm = Vm::new(initial.vm.world.clone(), 1);
            for id in [1, 2, 3, 4, 5] {
                initial.vm.start_object_lifecycle(&p, id).unwrap();
            }
            let thread = initial.vm.object_threads[&selected];
            initial
                .vm
                .threads
                .get_mut(&thread)
                .unwrap()
                .stack
                .last_mut()
                .unwrap()
                .stack_object = 2;
            let mut results = vec![];
            for budget in [1, 1000] {
                let mut s = initial.clone();
                let mut saved = false;
                let mut finished = false;
                for _ in 0..1000 {
                    let step = s.run_fixture_tick(&p, budget).unwrap();
                    if let Some(snap) = &s.native_lot.as_ref().unwrap().snap {
                        if snap.placement.is_some()
                            && s.vm.threads.values().any(|t| t.callback.is_some())
                        {
                            saved = true;
                            assert!(s.advance_fixture_tick().is_err());
                            let mut bad = crate::Snapshot::capture(&p, &s);
                            bad.state
                                .native_lot
                                .as_mut()
                                .unwrap()
                                .snap
                                .as_mut()
                                .unwrap()
                                .placement = None;
                            assert!(
                                bad.restore(&p).is_err(),
                                "lost group callback owner accepted"
                            );
                        }
                    }
                    s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
                    finished |= step
                        .events
                        .iter()
                        .any(|e| e.detail.starts_with(&format!("native Snap #{selected}")));
                    if step.stop != crate::StepStop::Budget {
                        break;
                    }
                }
                assert!(finished, "selected={selected}, accepted={accepted}");
                assert_eq!(
                    s.vm.threads[&thread].status,
                    sims_vm::Status::Returned(accepted)
                );
                assert_eq!(s.vm.tick, initial.vm.tick);
                assert_eq!(saved, accepted && budget == 1);
                if accepted {
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    bad.state
                        .vm
                        .world
                        .objects
                        .get_mut(&5)
                        .unwrap()
                        .multitile
                        .as_mut()
                        .unwrap()
                        .offset = [37, 0, 0];
                    assert!(
                        bad.restore(&p).is_err(),
                        "inconsistent offset cache accepted"
                    );
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    bad.state.bodies.get_mut(&4).unwrap().position.x += 1;
                    assert!(bad.restore(&p).is_err(), "body bypassed native position");
                }
                for id in [4, 5] {
                    if accepted {
                        assert_eq!(
                            s.vm.world.positions[&id],
                            Placement {
                                x: 40
                                    + if selected == 5 { -16 } else { 0 }
                                    + if id == 5 { 16 } else { 0 },
                                y: 56,
                                level: 1,
                            }
                        );
                        assert!(s.vm.world.objects[&id].temps[2] > 0);
                        assert_eq!(
                            s.vm.world.objects[&id].temps[0], 0,
                            "callback saw new facing too early"
                        );
                        assert_eq!(
                            s.vm.world.objects[&id].data[&1],
                            if id == selected { 6 } else { 0 }
                        );
                    } else {
                        assert_eq!(s.vm.world.positions[&id], initial.vm.world.positions[&id]);
                    }
                }
                results.push(s);
            }
            assert_eq!(results[0], results[1]);
        }
    }
}

#[test]
fn native_contained_snap_saves_placement_callbacks_without_repeating_attachment() {
    use sims_formats::execution::{Slot, Slots};
    for accepted in [false, true] {
        let (mut p, mut initial) = walking_fixture();
        let m = p.modules.get_mut("objects").unwrap();
        let ins = &mut m.routines.get_mut(&4098).unwrap().body.instructions[0];
        ins.opcode = 0x2e;
        ins.operands = [0, 0, 1, 0, 4, 0, 0, 0];
        m.object_functions.get_mut(&1).unwrap()[9].action = 4099;
        m.object_functions.get_mut(&2).unwrap()[9].action = 4099;
        m.slots.insert(
            1,
            Slots {
                version: 10,
                entries: vec![Slot {
                    kind: 0,
                    offset: [1.75, -2.5, 0.],
                    standing: 0,
                    sitting: 0,
                    ground: 0,
                    flags: 0,
                    snap_target: -1,
                    proximity: [16; 3],
                    max_size: 0,
                    unknown: 0,
                    gradient: 0.,
                    height: 1,
                    facing: -2,
                    resolution: 16,
                }],
            },
        );
        initial.vm.world.globals.insert(15, 0);
        initial
            .vm
            .world
            .objects
            .get_mut(&1)
            .unwrap()
            .data
            .insert(4, i16::from(accepted));
        let parent = initial.vm.world.objects.get_mut(&2).unwrap();
        parent.definition.as_mut().unwrap().slots = 1;
        parent.slots = vec![None];
        initial.vm.start_object_lifecycle(&p, 2).unwrap();
        let actor = initial.vm.object_threads[&1];
        let mut outcomes = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut saved = false;
            let mut completed = 0;
            for _ in 0..1000 {
                let step = s.run_fixture_tick(&p, budget).unwrap();
                if s.native_lot
                    .as_ref()
                    .unwrap()
                    .snap
                    .as_ref()
                    .is_some_and(|e| e.placement.is_some())
                    && s.vm.threads.values().any(|t| t.callback.is_some())
                {
                    saved = true;
                    assert!(s.advance_fixture_tick().is_err());
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    bad.state
                        .native_lot
                        .as_mut()
                        .unwrap()
                        .snap
                        .as_mut()
                        .unwrap()
                        .plan
                        .container = None;
                    assert!(bad.restore(&p).is_err());
                }
                s = serde_json::from_slice::<crate::Snapshot>(
                    &serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap(),
                )
                .unwrap()
                .restore(&p)
                .unwrap();
                completed += step
                    .events
                    .iter()
                    .filter(|e| e.detail.starts_with("native Snap #1"))
                    .count();
                if step.stop != crate::StepStop::Budget {
                    break;
                }
            }
            assert_eq!(completed, 1);
            assert_eq!(saved, accepted && budget == 1);
            assert_eq!(
                s.vm.threads[&actor].status,
                sims_vm::Status::Returned(accepted)
            );
            assert_eq!(s.vm.tick, initial.vm.tick);
            assert_eq!(s.vm.world.objects[&2].slots, vec![accepted.then_some(1)]);
            assert_eq!(s.vm.world.objects[&1].container, accepted.then_some((2, 0)));
            if accepted {
                assert_eq!(s.vm.world.objects[&1].temps[1], 1);
                assert_eq!(s.vm.world.objects[&2].temps[1], 1);
                assert_eq!(
                    s.native_lot.as_ref().unwrap().snap_offsets[&1].map(f32::from_bits),
                    [1., -2., 0.]
                );
            }
            outcomes.push(s);
        }
        assert_eq!(outcomes[0], outcomes[1]);
    }
}

#[test]
fn native_route_completion_restores_acceptance_and_detachment_callbacks() {
    use sims_formats::execution::{Slot, Slots};
    for accepted in [false, true] {
        for posture in [0, 1] {
            let (mut p, mut initial) = walking_fixture();
            p.modules.get_mut("objects").unwrap().slots.insert(
                1,
                Slots {
                    version: 10,
                    entries: vec![Slot {
                        kind: 0,
                        offset: [0.; 3],
                        standing: 0,
                        sitting: 0,
                        ground: 0,
                        flags: 0,
                        snap_target: -1,
                        proximity: [16; 3],
                        max_size: 0,
                        unknown: 0,
                        gradient: 0.,
                        height: 1,
                        facing: -2,
                        resolution: 16,
                    }],
                },
            );
            let parent = initial.vm.world.objects.get_mut(&2).unwrap();
            parent.definition.as_mut().unwrap().slots = 1;
            parent.slots = vec![None];
            initial.vm.start_object_lifecycle(&p, 2).unwrap();
            let mut step = crate::Step {
                stop: crate::StepStop::Waiting,
                trace: vec![],
                events: vec![],
            };
            let mut attach = lot_placement::live::Execution::new(lot_placement::Operation::set(
                1,
                crate::lot_links::Descriptor {
                    on_lot: true,
                    position: [40, 40],
                    level: 1,
                    container: Some(2),
                    slot: 0,
                },
                true,
            ));
            assert!(
                initial
                    .drive_live_placement(&p, &mut attach, &mut 1000, &mut step)
                    .unwrap()
            );
            initial.update_original_person_room(1).unwrap();
            let m = p.modules.get_mut("objects").unwrap();
            // Different callbacks let the test distinguish acceptance from detach.
            let mut cb = m.routines[&4099].clone();
            cb.body.instructions[1].operands[0] = 2;
            m.routines.insert(4101, cb);
            m.object_functions.get_mut(&1).unwrap()[9].action = 4101;
            m.object_functions.get_mut(&2).unwrap()[9].action = 4101;
            m.routines.get_mut(&4099).unwrap().body.instructions[1].on_true =
                if accepted { 254 } else { 255 };
            let person = initial.vm.world.objects.get_mut(&1).unwrap();
            person
                .person
                .as_mut()
                .unwrap()
                .extend([(0, posture), (72, 7), (73, 23)]);
            person.data.insert(9, 1);
            let thread = initial.vm.object_threads[&1];
            initial.vm.run_slice(&p, thread, 1000).unwrap();
            let sims_vm::Status::AwaitingRoute(request) =
                initial.vm.threads[&thread].status.clone()
            else {
                panic!("route request absent")
            };
            initial.visited.insert(thread);
            initial.visit = Some((thread, Some(1)));
            assert!(
                !initial
                    .service_original_portals(&p, thread, &request, &mut step.events)
                    .unwrap()
            );
            initial
                .begin_native_route_completion(thread, &request, true, 0)
                .unwrap();
            let mut results = vec![];
            for budget in [1, 1000] {
                let mut s = initial.clone();
                let mut splits = 0;
                loop {
                    let mut remaining = budget;
                    let done = s
                        .drive_native_route_completion(&p, &mut remaining, &mut step)
                        .unwrap();
                    s.sync_spatial_world();
                    if !done {
                        splits += 1;
                        assert!(splits < 30);
                        assert!(s.advance_fixture_tick().is_err());
                        let mut bad = crate::Snapshot::capture(&p, &s);
                        bad.state.native_lot.as_mut().unwrap().completion = None;
                        assert!(bad.restore(&p).is_err());
                    }
                    s = serde_json::from_slice::<crate::Snapshot>(
                        &serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap(),
                    )
                    .unwrap()
                    .restore(&p)
                    .unwrap();
                    if done {
                        break;
                    }
                }
                assert_eq!(splits > 0, budget == 1);
                assert_eq!(s.vm.tick, initial.vm.tick);
                assert_eq!(
                    s.vm.world.objects[&1].container,
                    if accepted { None } else { Some((2, 0)) }
                );
                assert_eq!(
                    s.vm.world.objects[&3].temps[1], 1,
                    "acceptance must run once"
                );
                assert_eq!(s.vm.world.objects[&1].temps[2], i16::from(accepted));
                // SetObject notifies the destination container only; floor placement
                // does not invoke the old chair's function 9.
                assert_eq!(s.vm.world.objects[&2].temps[2], 0);
                let person = s.vm.world.objects[&1].person.as_ref().unwrap();
                assert_eq!(person[&73], if accepted { 7 } else { 0 });
                assert_eq!(person[&62], i16::from(!accepted && posture == 0));
                assert_eq!(person[&39], 1);
                assert!(s.route_control.is_empty());
                results.push(s);
            }
            assert_eq!(results[0], results[1]);
        }
    }
}

#[test]
fn native_path_callback_resume_does_not_repeat_stand_decision() {
    let (p, mut initial) = routed_walking_fixture();
    let mut paused = false;
    for _ in 0..1000 {
        let step = initial.run_fixture_tick(&p, 1).unwrap();
        if initial
            .native_lot
            .as_ref()
            .and_then(|n| n.route.as_ref())
            .is_some_and(|r| r.obstacles.is_some())
        {
            paused = true;
            break;
        }
        assert_eq!(step.stop, crate::StepStop::Budget);
    }
    assert!(paused, "path callback checkpoint was not reached");
    // Model a later synchronous callback changing the person's posture.
    // State 1 already chose the non-seated path. It must not return to that
    // decision when the host resumes the path-construction continuation.
    initial
        .vm
        .world
        .objects
        .get_mut(&1)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(0, 1);
    let mut outcomes = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        let mut done = false;
        for _ in 0..2000 {
            s = serde_json::from_slice::<crate::Snapshot>(
                &serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap(),
            )
            .unwrap()
            .restore(&p)
            .unwrap();
            let step = s.run_fixture_tick(&p, budget).unwrap();
            assert_ne!(step.stop, crate::StepStop::Faulted);
            done |= step
                .events
                .iter()
                .any(|e| e.detail == "original-lot native route cleanup completed: true");
            if step.stop != crate::StepStop::Budget {
                if done {
                    break;
                }
                s.advance_fixture_tick().unwrap();
            }
        }
        assert!(done, "resumption repeated the Stand guard");
        assert!(s.route_control.is_empty());
        outcomes.push(s);
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[test]
fn native_stand_exit_saves_during_script_and_replans_after_detachment() {
    use sims_formats::{
        behavior::Instruction,
        execution::{Slot, Slots},
    };
    for success in [false, true] {
        let (mut p, mut initial) = routed_walking_fixture();
        let m = p.modules.get_mut("objects").unwrap();
        m.slots.insert(
            1,
            Slots {
                version: 10,
                entries: vec![Slot {
                    kind: 0,
                    offset: [0.; 3],
                    standing: 0,
                    sitting: 0,
                    ground: 0,
                    flags: 0,
                    snap_target: -1,
                    proximity: [16; 3],
                    max_size: 0,
                    unknown: 0,
                    gradient: 0.,
                    height: 1,
                    facing: -2,
                    resolution: 16,
                }],
            },
        );
        m.object_definition_words.get_mut(&1).unwrap()[0x84 / 2] = 1;
        let functions = m.object_functions.get_mut(&1).unwrap();
        functions.resize(
            28,
            sims_formats::execution::ObjectFunction {
                action: 0,
                condition: 0,
            },
        );
        functions[26].action = 4096;
        functions[27].action = 4100;
        let mut stand = m.routines[&4096].clone();
        stand.body.args = 4;
        stand.body.instructions = vec![
            Instruction {
                opcode: 2,
                on_true: 1,
                on_false: 253,
                operands: [5, 0, 1, 0, 0, 3, 8, 7],
            },
            Instruction {
                opcode: 2,
                on_true: 2,
                on_false: 253,
                operands: [6, 0, 0, 0, 0, 5, 8, 9],
            },
            Instruction {
                opcode: 2,
                on_true: 3,
                on_false: 253,
                operands: [72, 0, 99, 0, 0, 5, 18, 7],
            },
            Instruction {
                opcode: 2,
                on_true: 4,
                on_false: 253,
                operands: [1, 0, 2, 0, 0, 5, 9, 7],
            },
            Instruction {
                opcode: 0,
                on_true: if success { 5 } else { 255 },
                on_false: 253,
                operands: [1, 0, 0, 0, 0, 0, 0, 0],
            },
            Instruction {
                opcode: 0x2e,
                on_true: 6,
                on_false: 253,
                operands: [0, 0, 2, 0, 0, 0, 0, 0],
            },
            Instruction {
                opcode: 2,
                on_true: 254,
                on_false: 253,
                operands: [0, 0, 0, 0, 0, 5, 18, 7],
            },
        ];
        m.routines.insert(4100, stand);
        initial.native_lot.as_mut().unwrap().routing_policy =
            crate::lot_route::path::Policy::InferredRectangles;
        let seat = initial.vm.world.objects.get_mut(&2).unwrap();
        seat.definition.as_mut().unwrap().slots = 1;
        seat.slots = vec![None];
        initial.vm.start_object_lifecycle(&p, 2).unwrap();
        let mut step = crate::Step {
            stop: crate::StepStop::Waiting,
            trace: vec![],
            events: vec![],
        };
        let mut attach = lot_placement::live::Execution::new(lot_placement::Operation::set(
            1,
            crate::lot_links::Descriptor {
                on_lot: true,
                position: [40, 40],
                level: 1,
                container: Some(2),
                slot: 0,
            },
            true,
        ));
        assert!(
            initial
                .drive_live_placement(&p, &mut attach, &mut 1000, &mut step)
                .unwrap()
        );
        initial.update_original_person_room(1).unwrap();
        initial.person(1, 0, 1).unwrap();
        initial.person(1, 72, 7).unwrap();
        let thread = initial.vm.object_threads[&1];
        let mut outcomes = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut seen_stand = false;
            let mut seen_snap = false;
            let mut seen_return = false;
            for turn in 0..5000 {
                let step = s.run_fixture_tick(&p, budget).unwrap();
                assert_ne!(
                    step.stop,
                    crate::StepStop::Faulted,
                    "{success}/{budget}/{turn}: {:?}",
                    step.trace
                );
                seen_stand |= s
                    .route_control
                    .values()
                    .any(|c| c.original.as_ref().is_some_and(|o| o.standing == Some(2)));
                seen_snap |= s.native_lot.as_ref().unwrap().snap.is_some();
                seen_return |= step
                    .events
                    .iter()
                    .any(|e| e.detail.starts_with("original Stand #2 returned"));
                if s.route_control
                    .values()
                    .any(|c| c.original.as_ref().is_some_and(|o| o.standing.is_some()))
                {
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    for c in bad.state.route_control.values_mut() {
                        c.original.as_mut().unwrap().standing = None;
                    }
                    assert!(bad.restore(&p).is_err());
                }
                s = serde_json::from_slice::<crate::Snapshot>(
                    &serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap(),
                )
                .unwrap()
                .restore(&p)
                .unwrap_or_else(|e| panic!("{success}/{budget}/{turn}: {e}"));
                if step.stop != crate::StepStop::Budget {
                    if seen_return && s.route_control.is_empty() {
                        break;
                    }
                    s.advance_fixture_tick().unwrap();
                }
            }
            assert!(
                seen_stand && seen_return,
                "Stand not observed {success}/{budget}"
            );
            assert!(s.route_control.is_empty());
            assert_eq!(s.vm.world.objects[&1].temps[5], 1, "Stand repeated");
            assert_eq!(
                s.vm.world.objects[&1].temps[6], 1,
                "departure argument missing"
            );
            assert_eq!(
                s.vm.world.objects[&1].person.as_ref().unwrap()[&37],
                if success { 0 } else { 1 }
            );
            assert_eq!(s.vm.world.objects[&1].person.as_ref().unwrap()[&72], 7);
            assert_eq!(
                s.vm.world.objects[&1].person.as_ref().unwrap()[&62],
                if success { 0 } else { 6 }
            );
            assert_eq!(
                s.vm.world.objects[&1].container,
                if success { None } else { Some((2, 0)) }
            );
            if success && budget == 1 {
                assert!(seen_snap);
            }
            assert_eq!(s.vm.threads[&thread].last_result, success);
            outcomes.push(s);
        }
        assert_eq!(outcomes[0], outcomes[1]);
    }
}

#[test]
fn native_floor_snap_bypassed_adjacency_still_rejects_off_lot() {
    let (mut p, mut s) = walking_fixture();
    let i = &mut p
        .modules
        .get_mut("objects")
        .unwrap()
        .routines
        .get_mut(&4098)
        .unwrap()
        .body
        .instructions[0];
    i.opcode = 0x2e;
    i.operands = [0, 0, 3, 0, 0, 0, 0, 0];
    let d = &mut s
        .vm
        .world
        .objects
        .get_mut(&2)
        .unwrap()
        .routing_slots
        .as_mut()
        .unwrap()[0];
    d.0[0] = 100f32.to_bits();
    d.0[7] = 0x800;
    let before = s.vm.world.positions[&1];
    let tick = s.vm.tick;
    let step = s.run_fixture_tick(&p, 1000).unwrap();
    assert!(
        step.events
            .iter()
            .any(|e| e.detail.starts_with("native Snap #1") && e.detail.ends_with(": false"))
    );
    assert_eq!(s.vm.world.positions[&1], before);
    assert_eq!(s.vm.tick, tick);
    assert_eq!(s.vm.world.objects[&1].data[&52], 2);
    assert_eq!(s.vm.world.objects[&1].data[&54], 0);
    assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, 1);
    assert_eq!(crate::Snapshot::capture(&p, &s).restore(&p).unwrap(), s);
}

#[test]
fn native_floor_snap_reports_off_lot_beyond_the_active_world_extent() {
    for (global23, map, off_lot) in [
        (Some(4), None, true),
        (None, Some(4), true),
        (Some(4), Some(6), false),
        (Some(0), None, false),
        (Some(3), None, false),
        (None, None, false),
    ] {
        let case = (global23, map);
        let (mut program, mut simulation) = walking_fixture();
        let instruction = &mut program
            .modules
            .get_mut("objects")
            .unwrap()
            .routines
            .get_mut(&4098)
            .unwrap()
            .body
            .instructions[0];
        instruction.opcode = 0x2e;
        instruction.operands = [0, 0, 3, 0, 0, 0, 0, 0];
        simulation
            .vm
            .world
            .objects
            .get_mut(&2)
            .unwrap()
            .routing_slots
            .as_mut()
            .unwrap()[0]
            .0[0] = 16f32.to_bits();
        match global23 {
            Some(value) => simulation.vm.world.globals.insert(23, value),
            None => simulation.vm.world.globals.remove(&23),
        };
        simulation.vm.world.map_size =
            map.map(|active_size| sims_vm::world_map::State { active_size });
        let lot = simulation
            .architecture
            .as_ref()
            .unwrap()
            .original
            .as_ref()
            .unwrap();
        assert_eq!(
            crate::lot_snap::active_extent(&simulation.vm.world, lot).unwrap(),
            if off_lot { 4 } else { 6 },
            "{case:?}"
        );
        let before = simulation.vm.world.positions[&1];
        let step = simulation.run_fixture_tick(&program, 1000).unwrap();
        assert!(
            step.events
                .iter()
                .any(|event| event.detail.starts_with("native Snap #1")),
            "{case:?}"
        );
        let object = &simulation.vm.world.objects[&1];
        assert_eq!(object.data[&52] == 2, off_lot, "{case:?}");
        assert_eq!(
            simulation.native_lot.as_ref().unwrap().diagnostics.error == 1,
            off_lot,
            "{case:?}"
        );
        if off_lot {
            assert_eq!(object.data[&54], 0, "{case:?}");
            assert_eq!(simulation.vm.world.positions[&1], before, "{case:?}");
        }
        assert_eq!(
            crate::Snapshot::capture(&program, &simulation)
                .restore(&program)
                .unwrap(),
            simulation
        );
    }
}

#[test]
fn native_find_location_group_detachment_saves_before_each_member_is_removed() {
    use crate::lot_find_location;
    use sims_formats::behavior::Instruction;
    for (selected, caller_id) in [(1, 4), (2, 4), (1, 1), (2, 2)] {
        let (mut p, mut initial) = fixture();
        let m = p.modules.get_mut("objects").unwrap();
        let mut words = m.object_definition_words[&1].clone();
        words[14] = 2;
        words[10] = if caller_id == 4 { 0 } else { 7 };
        m.object_definition_words.insert(2, words);
        m.object_definition_words.get_mut(&1).unwrap()[10] = 7;
        let mut functions = m.object_functions[&1].clone();
        functions[1].action = 4098;
        functions[10].action = 4100;
        m.object_functions.insert(2, functions);
        m.object_functions.get_mut(&1).unwrap()[10].action = 4100;
        let mut main = m.routines[&4096].clone();
        main.body.instructions = vec![Instruction {
            opcode: 0x10,
            on_true: 254,
            on_false: 255,
            operands: [1, 0, 2, 0, 0, 0, 0, 0],
        }];
        m.routines.insert(4098, main);
        let mut callback = m.routines[&4096].clone();
        callback.body.instructions = vec![
            Instruction {
                opcode: 2,
                on_true: 1,
                on_false: 255,
                operands: [1, 0, 1, 0, 0, 3, 8, 7],
            },
            Instruction {
                opcode: 2,
                on_true: 255,
                on_false: 255,
                operands: [0, 0, 0, 0, 0, 5, 8, 8],
            },
        ];
        m.routines.insert(4100, callback);
        for (id, head, next) in [
            (1, None, Some(3)),
            (3, Some(1), Some(2)),
            (2, Some(1), None),
        ] {
            initial.vm.world.objects.get_mut(&id).unwrap().multitile =
                Some(sims_vm::multitile::Part {
                    head,
                    next,
                    origin: [0; 3],
                    offset: [0; 3],
                });
        }
        let caller = initial
            .vm
            .world
            .objects
            .get_mut(&caller_id)
            .unwrap()
            .definition
            .as_mut()
            .unwrap();
        caller.object = 2;
        caller.guid = 2;
        initial
            .architecture
            .as_mut()
            .unwrap()
            .bind_footprints(&p)
            .unwrap();
        initial.vm = Vm::new(initial.vm.world.clone(), 1);
        for id in 1..=4 {
            initial.vm.start_object_lifecycle(&p, id).unwrap();
        }
        let thread = initial.vm.object_threads[&caller_id];
        initial
            .vm
            .threads
            .get_mut(&thread)
            .unwrap()
            .stack
            .last_mut()
            .unwrap()
            .stack_object = selected;
        let initial_objects = initial.vm.world.objects.len();
        let mut results = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut saved = 0;
            let mut completed = 0;
            for _ in 0..100 {
                let step = s.run_fixture_tick(&p, budget).unwrap();
                if let Some(lot_find_location::Execution::Detach { placement, .. }) =
                    s.native_lot.as_ref().unwrap().find_location.as_ref()
                {
                    assert!(matches!(
                        placement.operation,
                        lot_placement::VirtualOperation::Group(_)
                    ));
                    saved += 1;
                    assert!(s.advance_fixture_tick().is_err());
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    let lot_find_location::Execution::Detach { placement, .. } = bad
                        .state
                        .native_lot
                        .as_mut()
                        .unwrap()
                        .find_location
                        .as_mut()
                        .unwrap()
                    else {
                        unreachable!()
                    };
                    *placement = lot_placement::live::Execution::new(
                        lot_placement::Operation::remove(selected, true),
                    );
                    assert!(
                        bad.restore(&p).is_err(),
                        "group continuation cannot become base removal"
                    );
                }
                // All siblings receive function 10 before the first physical
                // removal. A secondary selected member cannot change that order.
                if [1, 2, 3].iter().any(|id| s.vm.world.positions[id].x < 0) {
                    assert!(
                        [1, 2, 3]
                            .iter()
                            .all(|id| s.vm.world.objects[id].temps[1] == 1)
                    );
                }
                s = serde_json::from_slice::<crate::Snapshot>(
                    &serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap(),
                )
                .unwrap()
                .restore(&p)
                .unwrap();
                completed += step
                    .events
                    .iter()
                    .filter(|e| {
                        e.detail == format!("native Find Location detached #{selected} from lot")
                    })
                    .count();
                if step.stop != crate::StepStop::Budget {
                    break;
                }
            }
            assert_eq!(completed, 1);
            assert_eq!(saved > 0, budget == 1);
            assert_eq!(s.vm.tick, initial.vm.tick);
            assert_eq!(s.vm.world.objects.len(), initial_objects);
            assert!(s.removals.is_empty() && s.deletion.is_empty());
            for id in [1, 2, 3] {
                assert_eq!(s.vm.world.objects[&id].temps[1], 1);
                assert_eq!(
                    s.vm.world.positions[&id],
                    Placement {
                        x: -16,
                        y: -16,
                        level: 1
                    }
                );
                assert!(s.unplaced.contains(&id) && !s.bodies.contains_key(&id));
            }
            assert_eq!(
                sims_vm::multitile::chain(&s.vm.world, selected).unwrap(),
                vec![1, 3, 2]
            );
            assert_eq!(
                s.vm.threads[&thread].status,
                sims_vm::Status::Returned(true)
            );
            results.push(s);
        }
        assert_eq!(results[0], results[1]);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_find_location_detaches_without_destroying_objects_or_children() {
    use sims_formats::behavior::Instruction;
    for contained in [false, true] {
        let (mut p, mut initial) = fixture();
        if contained {
            let mut step = crate::Step {
                stop: crate::StepStop::Waiting,
                trace: vec![],
                events: vec![],
            };
            for (parent, child) in [(1, 2), (2, 3)] {
                let o = initial.vm.world.objects.get_mut(&parent).unwrap();
                o.slots = vec![None];
                o.definition.as_mut().unwrap().slots = 1;
                let mut e = lot_placement::live::Execution::new(lot_placement::Operation::set(
                    child,
                    crate::lot_links::Descriptor {
                        on_lot: true,
                        position: [40, 40],
                        container: Some(parent),
                        slot: 0,
                        level: 1,
                    },
                    true,
                ));
                initial
                    .drive_live_placement(&p, &mut e, &mut 1000, &mut step)
                    .unwrap();
            }
        }
        let m = p.modules.get_mut("objects").unwrap();
        m.slots.insert(
            1,
            sims_formats::execution::Slots {
                version: 10,
                entries: vec![sims_formats::execution::Slot {
                    kind: 0,
                    offset: [0.; 3],
                    standing: 0,
                    sitting: 0,
                    ground: 0,
                    flags: 0,
                    snap_target: -1,
                    proximity: [16; 3],
                    max_size: 0,
                    unknown: 0,
                    gradient: 0.,
                    height: 0,
                    facing: -2,
                    resolution: 16,
                }],
            },
        );
        let idle = m.routines[&4096].clone();
        m.routines.insert(4099, idle.clone());
        let main = &mut m.routines.get_mut(&4096).unwrap().body;
        main.locals = 1;
        main.instructions = vec![Instruction {
            opcode: 0x10,
            on_true: 254,
            on_false: 255,
            operands: [1, 0, 2, 0, 0, 0, 0, 0],
        }];
        for (function, bhav, temp) in [(10, 4097, 1), (9, 4098, 2)] {
            m.object_functions.get_mut(&1).unwrap()[function].action = bhav;
            let mut routine = idle.clone();
            routine.body.instructions = vec![
                Instruction {
                    opcode: 2,
                    on_true: 1,
                    on_false: 253,
                    operands: [temp, 0, 1, 0, 0, 3, 8, 7],
                },
                Instruction {
                    opcode: 2,
                    on_true: 255,
                    on_false: 255,
                    operands: [0, 0, 0, 0, 0, 5, 8, 8],
                },
            ];
            m.routines.insert(bhav, routine);
        }
        let mut words = m.object_definition_words[&1].clone();
        words[14] = 2;
        m.object_definition_words.insert(2, words);
        let mut functions = m.object_functions[&1].clone();
        functions[1].action = 4099;
        m.object_functions.insert(2, functions);
        initial.vm = Vm::new(initial.vm.world.clone(), 1);
        for id in 1..=3 {
            let d = initial
                .vm
                .world
                .objects
                .get_mut(&id)
                .unwrap()
                .definition
                .as_mut()
                .unwrap();
            d.object = 2;
            d.guid = 2;
            initial.vm.start_object_lifecycle(&p, id).unwrap();
        }
        initial
            .architecture
            .as_mut()
            .unwrap()
            .bind_footprints(&p)
            .unwrap();
        initial.native_lot.as_mut().unwrap().observers.clear();
        initial.native_lot.as_mut().unwrap().diagnostics =
            crate::lot_acceptance::person::Diagnostics {
                error: 91,
                blocker: Some(2),
            };
        initial.vm.start_object_lifecycle(&p, 4).unwrap();
        let target = if contained { 2 } else { 1 };
        let thread = initial.vm.object_threads[&4];
        let frame = initial
            .vm
            .threads
            .get_mut(&thread)
            .unwrap()
            .stack
            .last_mut()
            .unwrap();
        frame.stack_object = target;
        frame.locals = vec![1];
        let mut results = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut saved = false;
            let mut completed = 0;
            for _ in 0..100 {
                let step = s.run_fixture_tick(&p, budget).unwrap();
                if s.native_lot.as_ref().unwrap().find_location.is_some() {
                    saved = true;
                    assert!(s.advance_fixture_tick().is_err());
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    bad.state.native_lot.as_mut().unwrap().find_location = None;
                    assert!(bad.restore(&p).is_err());
                }
                s = serde_json::from_slice::<crate::Snapshot>(
                    &serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap(),
                )
                .unwrap()
                .restore(&p)
                .unwrap();
                completed += step
                    .events
                    .iter()
                    .filter(|e| {
                        e.detail == format!("native Find Location detached #{target} from lot")
                    })
                    .count();
                if step.stop != crate::StepStop::Budget {
                    break;
                }
            }
            assert_eq!(completed, 1);
            assert_eq!(saved, budget == 1);
            assert_eq!(
                s.vm.threads[&thread].status,
                sims_vm::Status::Returned(true)
            );
            assert_eq!(s.vm.tick, initial.vm.tick);
            let native: serde_json::Value = serde_json::from_str(include_str!(
                "../../../sims-vm/tests/fixtures/find-location-detach.json"
            ))
            .unwrap();
            assert_eq!(
                serde_json::json!(s.native_lot.as_ref().unwrap().diagnostics.error),
                native["cases"][0]["original"]["placement_error"]
            );
            assert_eq!(
                serde_json::json!(s.native_lot.as_ref().unwrap().diagnostics.blocker),
                native["cases"][0]["original"]["placement_blocker"]
            );
            assert_eq!(s.vm.world.objects.len(), 4);
            assert!(s.removals.is_empty() && s.deletion.is_empty());
            assert_eq!(s.vm.world.objects[&target].temps[1], 1);
            assert_eq!(s.vm.world.objects[&target].container, None);
            assert_eq!(
                s.vm.world.positions[&target],
                Placement {
                    x: -16,
                    y: -16,
                    level: 1
                }
            );
            assert!(s.unplaced.contains(&target));
            assert!(!s.bodies.contains_key(&target));
            assert_eq!(
                s.native_lot
                    .as_ref()
                    .unwrap()
                    .observers
                    .iter()
                    .filter(|e| matches!(e, lot_placement::Request::RefreshAppearance(_)))
                    .count(),
                0
            );
            if contained {
                assert_eq!(s.vm.world.objects[&1].slots, vec![None]);
                assert_eq!(s.vm.world.objects[&1].temps[2], 1);
                assert_eq!(s.vm.world.objects[&2].slots, vec![Some(3)]);
                assert_eq!(s.vm.world.objects[&3].container, Some((2, 0)));
                assert_eq!(s.vm.world.objects[&3].temps[1], 0);
            }
            results.push(s);
        }
        assert_eq!(results[0], results[1]);
    }
}

#[test]
fn ground_input_rechecks_live_acceptance_and_restores_callbacks_without_ticks() {
    use sims_formats::behavior::Instruction;
    for reject_final in [false, true] {
        let (mut p, mut initial) = walking_fixture();
        let m = p.modules.get_mut("objects").unwrap();
        let mut words = m.object_definition_words[&1].clone();
        words[14] = crate::lot_ground::DESTINATION_GUID as i16;
        m.object_definition_words.insert(3, words);
        let mut functions = m.object_functions[&1].clone();
        functions[5].action = 4100;
        functions[9].action = 4101;
        m.object_functions.insert(3, functions);
        let mut cb = m.routines[&4099].clone();
        cb.body.instructions = vec![
            Instruction {
                opcode: 2,
                on_true: 1,
                on_false: 255,
                operands: [1, 0, 1, 0, 0, 3, 8, 7],
            },
            Instruction {
                opcode: 2,
                on_true: 254,
                on_false: if reject_final { 255 } else { 254 },
                operands: [1, 0, 1, 0, 0, 2, 8, 7],
            },
        ];
        m.routines.insert(4100, cb.clone());
        cb.body.instructions = vec![Instruction {
            opcode: 2,
            on_true: 254,
            on_false: 255,
            operands: [2, 0, 1, 0, 0, 3, 8, 7],
        }];
        m.routines.insert(4101, cb);
        m.routines.get_mut(&4099).unwrap().body.instructions[1].on_true = 255;
        let target = initial.vm.world.objects.get_mut(&4).unwrap();
        target.definition.as_mut().unwrap().object = 3;
        target.definition.as_mut().unwrap().guid = crate::lot_ground::DESTINATION_GUID;
        target.data.extend([(4, 1), (8, 16), (42, 259)]);
        initial
            .vm
            .world
            .objects
            .get_mut(&2)
            .unwrap()
            .data
            .insert(8, 4);
        initial.vm = Vm::new(initial.vm.world.clone(), 104);
        for id in [1, 3, 4] {
            initial.vm.start_object_lifecycle(&p, id).unwrap();
        }
        initial
            .architecture
            .as_mut()
            .unwrap()
            .bind_footprints(&p)
            .unwrap();
        for id in [1, 2] {
            initial
                .native_lot
                .as_mut()
                .unwrap()
                .placement
                .bounds
                .insert(id, [0; 4]);
        }
        let before = initial.vm.world.positions[&4];
        let mut outcomes = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            s.begin_ground_placement(&p, 4, [40, 40], 1).unwrap();
            let mut saw_callback = false;
            for slice in 0..1000 {
                let frozen = s.clone();
                assert!(s.advance_fixture_tick().is_err());
                assert!(s.run_fixture_tick(&p, 1000).is_err());
                assert_eq!(s, frozen);
                s.advance_ground_placement(&p, budget).unwrap();
                if s.vm.threads.values().any(|t| t.callback.is_some()) {
                    saw_callback = true;
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    bad.state.ground_placement = None;
                    assert!(bad.restore(&p).is_err());
                }
                let mut bad = crate::Snapshot::capture(&p, &s);
                bad.state.vm.tick += 1;
                assert!(bad.restore(&p).is_err());
                s = serde_json::from_slice::<crate::Snapshot>(
                    &serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap(),
                )
                .unwrap()
                .restore(&p)
                .unwrap();
                if s.ground_placement.as_ref().unwrap().complete() {
                    break;
                }
                assert!(slice < 999);
            }
            assert_eq!(saw_callback, budget == 1);
            assert_eq!(s.vm.tick, initial.vm.tick);
            assert_eq!(s.vm.rng_state, 104);
            assert_eq!(s.vm.world.objects[&4].temps[1], 2);
            assert_eq!(
                s.take_ground_placement().unwrap(),
                (4, (!reject_final).then_some([40, 40]))
            );
            assert_eq!(
                s.vm.world.positions[&4],
                if reject_final {
                    before
                } else {
                    Placement {
                        x: 40,
                        y: 40,
                        level: 1,
                    }
                }
            );
            assert!(
                !s.native_lot
                    .as_ref()
                    .unwrap()
                    .observers
                    .iter()
                    .any(|e| matches!(e, lot_placement::Request::RefreshAppearance(4)))
            );
            outcomes.push(s);
        }
        assert_eq!(outcomes[0], outcomes[1]);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_find_location_saves_callbacks_and_rechecks_before_commit() {
    use sims_formats::behavior::Instruction;
    for (reject_final, person, mode) in [false, true]
        .into_iter()
        .flat_map(|reject| [false, true].map(move |person| (reject, person)))
        .flat_map(|(reject, person)| [0, 5].map(move |mode| (reject, person, mode)))
    {
        let (mut p, mut initial) = walking_fixture();
        if mode == 5 {
            let source = FloorSource {
                coverings: vec![0; 4096],
                walls: vec![Default::default(); 4096],
                object_heads: vec![0; 4096],
                flags: vec![0; 4096],
            };
            let lot = Lot::rebuild(
                64,
                vec![[0; 4]; 4096],
                vec![255; 4096],
                vec![255; 4096],
                vec![source.clone(), source],
            )
            .unwrap();
            let architecture = Architecture::from_original(lot, &p).unwrap();
            architecture.publish_room_context(&mut initial.vm.world);
            let native = initial.native_lot.as_mut().unwrap();
            let mut links = native.placement.links.clone();
            for floor in &mut links.heads {
                let mut expanded = vec![0; 4096];
                for x in 0..6 {
                    for y in 0..6 {
                        expanded[x * 64 + y] = floor[x * 6 + y];
                    }
                }
                *floor = expanded;
            }
            links.size = 64;
            let bounds = native.placement.bounds.clone();
            native.placement = lot_placement::State::new(links, &architecture).unwrap();
            native.placement.bounds = bounds;
            initial.room.width = architecture.width;
            initial.room.height = architecture.height;
            initial.architecture = Some(architecture);
        }
        let m = p.modules.get_mut("objects").unwrap();
        let body = &mut m.routines.get_mut(&4098).unwrap().body;
        body.locals = 1;
        body.instructions = vec![Instruction {
            opcode: 0x10,
            on_true: 254,
            on_false: 255,
            operands: [mode, 0, 3, 0, 0, 0, 0, 0],
        }];
        let mut words = m.object_definition_words[&1].clone();
        words[14] = 3;
        if person {
            words[9] = 2;
        }
        m.object_definition_words.insert(3, words);
        let mut functions = m.object_functions[&1].clone();
        functions[5].action = 4100;
        functions[9].action = 4101;
        m.object_functions.insert(3, functions);
        let mut cb = m.routines[&4099].clone();
        cb.body.instructions = vec![
            Instruction {
                opcode: 2,
                on_true: 1,
                on_false: 255,
                operands: [1, 0, 1, 0, 0, 3, 8, 7],
            },
            Instruction {
                opcode: 2,
                on_true: 254,
                on_false: if reject_final { 255 } else { 254 },
                operands: [1, 0, 1, 0, 0, 2, 8, 7],
            },
        ];
        m.routines.insert(4100, cb.clone());
        cb.body.instructions = vec![Instruction {
            opcode: 2,
            on_true: 254,
            on_false: 255,
            operands: [2, 0, 1, 0, 0, 3, 8, 7],
        }];
        m.routines.insert(4101, cb);
        m.routines.get_mut(&4099).unwrap().body.instructions[1].on_true = 255;
        let obj = initial.vm.world.objects.get_mut(&4).unwrap();
        obj.definition.as_mut().unwrap().object = 3;
        obj.definition.as_mut().unwrap().guid = 3;
        obj.data.extend([(4, 1), (8, 16), (42, 259)]);
        if person {
            obj.person = Some((0..256).map(|i| (i, 0)).collect());
            initial
                .vm
                .world
                .person
                .as_mut()
                .unwrap()
                .person_order
                .push(4);
            initial.native_lot.as_mut().unwrap().membership =
                RoomMembership::constructed(&initial.vm.world);
            // Keep object 3 as the one collision callback participant.
            // Person/person acceptance reads the *other* Sim's flags;
            // ordinary/person acceptance reads the ordinary object's.
            initial
                .vm
                .world
                .objects
                .get_mut(&1)
                .unwrap()
                .data
                .insert(8, 16);
        }
        initial
            .vm
            .world
            .objects
            .get_mut(&2)
            .unwrap()
            .data
            .insert(8, 4);
        // First two native draws from104 end in4,4: offsets -8,-8.
        // Both searches therefore reach the same occupied tile, while
        // mode5 must retain its consumed stream across every callback.
        initial.vm = Vm::new(initial.vm.world.clone(), 104);
        for id in [1, 3, 4] {
            initial.vm.start_object_lifecycle(&p, id).unwrap();
        }
        let thread = initial.vm.object_threads[&1];
        let frame = initial
            .vm
            .threads
            .get_mut(&thread)
            .unwrap()
            .stack
            .last_mut()
            .unwrap();
        frame.stack_object = 4;
        frame.locals = vec![1];
        initial
            .architecture
            .as_mut()
            .unwrap()
            .bind_footprints(&p)
            .unwrap();
        initial
            .native_lot
            .as_mut()
            .unwrap()
            .placement
            .bounds
            .insert(1, [0; 4]);
        initial
            .native_lot
            .as_mut()
            .unwrap()
            .placement
            .bounds
            .insert(2, [0; 4]);
        // The wrapper's 0x422022 call builds a local tile coordinate. It
        // does not clear the shared placement error/blocker before search.
        let mut probe = initial.clone();
        probe.vm.run_slice(&p, thread, 1).unwrap();
        let sims_vm::Status::AwaitingPlacement(request) = probe.vm.threads[&thread].status.clone()
        else {
            panic!("Find Location request absent")
        };
        crate::lot_find_location::Execution::begin(&mut probe, &p, thread, &request).unwrap();
        let native: serde_json::Value = serde_json::from_str(include_str!(
            "../../../sims-vm/tests/fixtures/find-location-detach.json"
        ))
        .unwrap();
        for row in native["search_diagnostics"].as_array().unwrap() {
            assert_eq!(
                serde_json::json!(probe.native_lot.as_ref().unwrap().diagnostics.error),
                row["original"]["placement_error"]
            );
            assert_eq!(
                serde_json::json!(probe.native_lot.as_ref().unwrap().diagnostics.blocker),
                row["original"]["placement_blocker"]
            );
        }
        let mut results = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut saved = false;
            let mut finished = false;
            for _ in 0..1000 {
                let step = s.run_fixture_tick(&p, budget).unwrap();
                if s.native_lot.as_ref().unwrap().find_location.is_some()
                    && s.vm.threads.values().any(|t| t.callback.is_some())
                {
                    saved = true;
                    assert!(s.advance_fixture_tick().is_err());
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    bad.state.native_lot.as_mut().unwrap().find_location = None;
                    assert!(bad.restore(&p).is_err());
                }
                s = serde_json::from_slice::<crate::Snapshot>(
                    &serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap(),
                )
                .unwrap()
                .restore(&p)
                .unwrap();
                finished |= step
                    .events
                    .iter()
                    .any(|e| e.detail.starts_with("native Find Location #4"));
                if step.stop != crate::StepStop::Budget {
                    break;
                }
            }
            assert!(finished);
            assert_eq!(
                saved,
                budget == 1,
                "mode {mode} reject {reject_final} person {person} budget {budget}; result {:?}, rng {}, pos {:?}, calls {:?}",
                s.vm.threads[&thread].status,
                s.vm.rng_state,
                s.vm.world.positions[&4],
                s.vm.world.objects[&4].temps
            );
            assert_eq!(s.vm.tick, initial.vm.tick);
            assert_eq!(s.vm.rng_state, if mode == 5 { 3_857_371_430 } else { 104 });
            assert_eq!(
                s.vm.threads[&thread].status,
                sims_vm::Status::Returned(!reject_final)
            );
            assert_eq!(s.vm.world.objects[&4].temps[1], 2);
            assert_eq!(
                s.native_lot
                    .as_ref()
                    .unwrap()
                    .observers
                    .iter()
                    .filter(|e| matches!(e, lot_placement::Request::RefreshAppearance(4)))
                    .count(),
                usize::from(!reject_final)
            );
            assert_eq!(
                s.vm.world.positions[&4],
                if reject_final {
                    initial.vm.world.positions[&4]
                } else {
                    Placement {
                        x: 40,
                        y: 40,
                        level: 1,
                    }
                }
            );
            if !reject_final {
                assert!(s.vm.world.objects[&4].temps[2] > 0);
            }
            results.push(s);
        }
        assert_eq!(results[0], results[1]);
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_multipart_directional_find_location_saves_each_member_callback() {
    use sims_formats::behavior::Instruction;
    for reject_final in [false, true] {
        let (mut p, mut initial) = walking_fixture();
        let m = p.modules.get_mut("objects").unwrap();
        let body = &mut m.routines.get_mut(&4098).unwrap().body;
        body.locals = 1;
        body.instructions = vec![Instruction {
            opcode: 0x10,
            on_true: 254,
            on_false: 255,
            operands: [3, 0, 3, 0, 0, 0, 0, 0],
        }];
        let mut words = m.object_definition_words[&1].clone();
        words[14] = 3;
        words[10] = 7;
        words[11] = 0;
        words[63] = 1;
        m.object_definition_words.insert(3, words.clone());
        words[14] = 4;
        words[11] = 1;
        words[63] = 0;
        m.object_definition_words.insert(4, words);
        m.object_definition_order.extend([3, 4]);
        let mut functions = m.object_functions[&1].clone();
        functions[5].action = 4100;
        functions[9].action = 4101;
        m.object_functions.insert(3, functions.clone());
        m.object_functions.insert(4, functions);
        let mut cb = m.routines[&4099].clone();
        cb.body.instructions = vec![
            Instruction {
                opcode: 2,
                on_true: 1,
                on_false: 255,
                operands: [1, 0, 1, 0, 0, 3, 8, 7],
            },
            Instruction {
                opcode: 2,
                on_true: 254,
                on_false: if reject_final { 255 } else { 254 },
                operands: [1, 0, 1, 0, 0, 2, 8, 7],
            },
        ];
        m.routines.insert(4100, cb.clone());
        cb.body.instructions = vec![Instruction {
            opcode: 2,
            on_true: 254,
            on_false: 255,
            operands: [2, 0, 1, 0, 0, 3, 8, 7],
        }];
        m.routines.insert(4101, cb);
        m.routines.get_mut(&4099).unwrap().body.instructions[1].on_true = 255;
        let obj = initial.vm.world.objects.get_mut(&4).unwrap();
        obj.definition.as_mut().unwrap().object = 3;
        obj.definition.as_mut().unwrap().guid = 3;
        obj.data.extend([(4, 1), (8, 16), (42, 259)]);
        initial
            .vm
            .world
            .objects
            .get_mut(&2)
            .unwrap()
            .data
            .insert(8, 4);
        initial.vm.world.objects.get_mut(&4).unwrap().multitile = Some(sims_vm::multitile::Part {
            head: None,
            next: Some(5),
            origin: [0; 3],
            offset: [0; 3],
        });
        let mut part = initial.vm.world.objects[&4].clone();
        part.definition.as_mut().unwrap().object = 4;
        part.definition.as_mut().unwrap().guid = 4;
        part.data.insert(11, 5);
        part.multitile = Some(sims_vm::multitile::Part {
            head: Some(4),
            next: None,
            origin: [0, 1, 0],
            offset: [0, 1, 0],
        });
        initial.vm.world.objects.insert(5, part);
        initial
            .vm
            .world
            .positions
            .insert(5, initial.vm.world.positions[&4]);
        initial
            .vm
            .world
            .person
            .as_mut()
            .unwrap()
            .object_order
            .insert(0, 5);
        initial
            .native_lot
            .as_mut()
            .unwrap()
            .placement
            .links
            .next
            .insert(5, 0);
        initial
            .native_lot
            .as_mut()
            .unwrap()
            .placement
            .bounds
            .insert(5, [0; 4]);
        initial.unplaced.insert(5);
        initial.vm = Vm::new(initial.vm.world.clone(), 1);
        for id in [1, 3, 4, 5] {
            initial.vm.start_object_lifecycle(&p, id).unwrap();
        }
        let thread = initial.vm.object_threads[&1];
        let frame = initial
            .vm
            .threads
            .get_mut(&thread)
            .unwrap()
            .stack
            .last_mut()
            .unwrap();
        frame.stack_object = 4;
        frame.locals = vec![1];
        initial
            .architecture
            .as_mut()
            .unwrap()
            .bind_footprints(&p)
            .unwrap();
        initial
            .native_lot
            .as_mut()
            .unwrap()
            .placement
            .bounds
            .insert(1, [0; 4]);
        initial
            .native_lot
            .as_mut()
            .unwrap()
            .placement
            .bounds
            .insert(2, [0; 4]);
        // The wrapper's 0x422022 call builds a local tile coordinate. It
        // does not clear the shared placement error/blocker before search.
        let mut probe = initial.clone();
        probe.vm.run_slice(&p, thread, 1).unwrap();
        let sims_vm::Status::AwaitingPlacement(request) = probe.vm.threads[&thread].status.clone()
        else {
            panic!("Find Location request absent")
        };
        crate::lot_find_location::Execution::begin(&mut probe, &p, thread, &request).unwrap();
        let native: serde_json::Value = serde_json::from_str(include_str!(
            "../../../sims-vm/tests/fixtures/find-location-detach.json"
        ))
        .unwrap();
        for row in native["search_diagnostics"].as_array().unwrap() {
            assert_eq!(
                serde_json::json!(probe.native_lot.as_ref().unwrap().diagnostics.error),
                row["original"]["placement_error"]
            );
            assert_eq!(
                serde_json::json!(probe.native_lot.as_ref().unwrap().diagnostics.blocker),
                row["original"]["placement_blocker"]
            );
        }
        let mut results = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut saved = false;
            let mut finished = false;
            for _ in 0..1000 {
                let step = s.run_fixture_tick(&p, budget).unwrap();
                if s.native_lot.as_ref().unwrap().find_location.is_some()
                    && s.vm.threads.values().any(|t| t.callback.is_some())
                {
                    saved = true;
                    assert!(s.advance_fixture_tick().is_err());
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    bad.state.native_lot.as_mut().unwrap().find_location = None;
                    assert!(bad.restore(&p).is_err());
                }
                s = crate::Snapshot::capture(&p, &s).restore(&p).unwrap();
                finished |= step
                    .events
                    .iter()
                    .any(|e| e.detail.starts_with("native Find Location #4"));
                if step.stop != crate::StepStop::Budget {
                    break;
                }
            }
            assert!(finished);
            assert_eq!(saved, budget == 1);
            assert_eq!(s.vm.tick, initial.vm.tick);
            assert_eq!(
                s.vm.threads[&thread].status,
                sims_vm::Status::Returned(!reject_final)
            );
            assert_eq!(s.vm.world.objects[&4].temps[1], 2);
            assert_eq!(
                s.native_lot
                    .as_ref()
                    .unwrap()
                    .observers
                    .iter()
                    .filter(|e| matches!(e, lot_placement::Request::RefreshAppearance(4)))
                    .count(),
                0
            );
            assert_eq!(
                s.vm.world.positions[&4],
                if reject_final {
                    initial.vm.world.positions[&4]
                } else {
                    Placement {
                        x: 40,
                        y: 40,
                        level: 1,
                    }
                }
            );
            if !reject_final {
                assert!(s.vm.world.objects[&4].temps[2] > 0);
                assert!(s.vm.world.objects[&5].temps[2] > 0);
                assert_eq!(
                    s.vm.world.positions[&5],
                    Placement {
                        x: 56,
                        y: 40,
                        level: 1
                    }
                );
                assert!(!s.unplaced.contains(&4) && !s.unplaced.contains(&5));
            }
            results.push(s);
        }
        assert_eq!(results[0], results[1]);
    }
}
