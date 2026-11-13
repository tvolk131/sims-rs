use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{action, program::MenuEntry, *};
use std::collections::BTreeMap;

fn assign(scope: u8, index: i16, rhs_scope: u8, rhs: i16, next: u8) -> Instruction {
    let mut operands = [0; 8];
    operands[..2].copy_from_slice(&index.to_le_bytes());
    operands[2..4].copy_from_slice(&rhs.to_le_bytes());
    operands[5..].copy_from_slice(&[5, scope, rhs_scope]);
    Instruction {
        opcode: 2,
        on_true: next,
        on_false: 253,
        operands,
    }
}
fn routine(instructions: Vec<Instruction>) -> Routine {
    Routine {
        name: "player menu fixture".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args: 5,
            locals: 0,
            instructions,
        },
    }
}
fn fixture() -> (Program, Vm) {
    let m = Module {
        source: "fixture".into(),
        object_definition_words: [(1, vec![0; 100]), (2, vec![0; 100])].into(),
        routines: [
            (
                4096,
                routine(vec![Instruction {
                    opcode: 0,
                    operands: [0; 8],
                    on_true: 254,
                    on_false: 255,
                }]),
            ),
            (
                4097,
                routine(vec![
                    assign(8, 0, 7, 123, 1),
                    assign(0, 0, 9, 4, 2),
                    assign(0, 1, 7, 99, 255),
                ]),
            ),
            (4098, routine(vec![assign(0, 2, 8, 0, 254)])),
            (4099, routine(vec![assign(3, 50, 7, 1, 254)])),
        ]
        .into(),
        object_interactions: [(
            2,
            vec![
                MenuEntry {
                    id: 7,
                    flags: 0x100,
                    action: 4096,
                    check: 4097,
                    ..Default::default()
                },
                MenuEntry {
                    id: 2,
                    flags: 0,
                    action: 4096,
                    check: 4098,
                    ..Default::default()
                },
                MenuEntry {
                    id: 3,
                    flags: 0,
                    action: 4096,
                    check: 4099,
                    ..Default::default()
                },
                MenuEntry {
                    id: 9,
                    flags: 0x100,
                    action: 4096,
                    check: 0,
                    ..Default::default()
                },
            ],
        )]
        .into(),
        ..Default::default()
    };
    let p = Program {
        globals: "fixture".into(),
        modules: [("fixture".into(), m)].into(),
        ..Default::default()
    };
    let mut world = World::default();
    for id in [1, 2] {
        world.objects.insert(
            id,
            Object {
                definition: Some(Definition {
                    source: "fixture".into(),
                    object: id as u16,
                    guid: 0x7c4,
                    slots: 0,
                }),
                person: (id == 1).then(|| (0..256).map(|i| (i, 0)).collect()),
                data: (0..72).map(|i| (i, 0)).collect(),
                attributes: vec![0; 8],
                temps: [42; 8],
                ..Default::default()
            },
        );
    }
    world.actions = Some(action::WorldState {
        serial: 77,
        live_family: true,
        actors: BTreeMap::from([(1, Default::default())]),
        ..Default::default()
    });
    let mut vm = Vm::new(world, 987);
    let t = vm
        .spawn(
            &p,
            Start {
                owner: "fixture",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 1,
                args: vec![1, 2, 3, 4, 5],
                check: false,
            },
        )
        .unwrap();
    vm.object_threads.insert(1, t);
    vm.run_slice(&p, t, 1).unwrap();
    assert!(matches!(vm.threads[&t].status, Status::Sleeping { .. }));
    (p, vm)
}

#[cfg(feature = "private-fixtures")]
#[test]
fn dynamic_labels_follow_original_publication_and_suppress_the_base_entry() {
    let native: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-dynamic-menu.json")).unwrap();
    for row in native["finishes"].as_array().unwrap() {
        let c = &row["input"];
        if c["debug"] != 0 {
            continue;
        }
        let count = c["added"].as_u64().unwrap() as usize;
        let flags = c["result_flags"].as_u64().unwrap() as u32;
        let mut baseline = None;
        // Native target availability and disable flags are assigned when the
        // check returns. Only those bits vary in the ordinary fixture.
        for seed in 0..8u64 {
            let (mut p, mut vm) = fixture();
            let m = p.modules.get_mut("fixture").unwrap();
            m.object_interactions.get_mut(&2).unwrap().truncate(1);
            m.object_interactions.get_mut(&2).unwrap()[0].flags =
                if flags & 1 != 0 { 0x100 } else { 0 };
            m.strings
                .insert(302, (0..count).map(|n| format!("Choice {n}")).collect());
            let mut code = (0..count)
                .map(|n| Instruction {
                    opcode: 50,
                    on_true: (n + 1) as u8,
                    on_false: 253,
                    operands: [46, 1, 0, 0, n as u8 + 1, 0, 0, 0],
                })
                .collect::<Vec<_>>();
            if flags & 16 != 0 {
                code.push(assign(3, 50, 7, 1, (code.len() + 1) as u8));
            }
            code.push(assign(8, 7, 7, 0, if flags & 8 != 0 { 254 } else { 255 }));
            m.routines.insert(4097, routine(code));
            let before = (
                vm.tick,
                vm.rng_state,
                vm.lot_rng_state,
                vm.crt_rng_state,
                vm.world.actions.clone(),
            );
            vm.begin_player_menu(&p, 1, 2).unwrap();
            if seed == 0 {
                vm.advance_player_menu(&p, 0).unwrap();
                let mut old = serde_json::to_value(Snapshot::capture(&p, &vm)).unwrap();
                old["state"]["player_menu"]["waiting"]
                    .as_object_mut()
                    .unwrap()
                    .remove("output_start");
                vm = serde_json::from_value::<Snapshot>(old)
                    .unwrap()
                    .restore(&p)
                    .unwrap();
                if count > 0 {
                    let thread = vm.object_threads[&1];
                    vm.threads.get_mut(&thread).unwrap().status=Status::Faulted(Fault {message:"player-menu Change Action String requires native dynamic entry publication".into(),stack:vec![]});
                    vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
                }
            }
            let mut state = seed + 1;
            let actions = loop {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let budget = 1 + ((state >> 32) % 3) as usize;
                let (done, _) = vm.advance_player_menu(&p, budget).unwrap();
                // Restore every suspended cut, including after a dynamic
                // publication but before the eligibility return.
                vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
                if let Some(a) = done {
                    break a;
                }
            };
            assert_eq!(
                actions.len() as u64,
                row["original"]["count"].as_u64().unwrap(),
                "{c}, seed {seed}"
            );
            if count > 0 {
                for (n, a) in actions.iter().enumerate() {
                    assert_eq!(a.label, Some(format!("Choice {n}")));
                    assert_eq!(a.args, [2, 0, 0, 0]);
                    assert_eq!(a.flags, 12 | (flags & 1));
                    assert_eq!(a.priority, 50);
                    assert_eq!(a.id, 0);
                }
            } else {
                assert!(
                    actions
                        .iter()
                        .all(|a| a.label.is_none() && a.args == [0; 4])
                );
            }
            assert_eq!(
                (
                    vm.tick,
                    vm.rng_state,
                    vm.lot_rng_state,
                    vm.crt_rng_state,
                    vm.world.actions.clone()
                ),
                before
            );
            assert_eq!(vm.world.objects[&1].temps, [42; 8]);
            let outcome = (actions, vm);
            if let Some(expected) = &baseline {
                assert_eq!(
                    &outcome, expected,
                    "slice seed {seed} changed the complete VM"
                );
            } else {
                baseline = Some(outcome);
            }
        }
    }
}

#[test]
fn outer_menu_destructor_waits_for_nested_tile_checks_and_cold_resume() {
    let (mut p, initial) = fixture();
    let m = p.modules.get_mut("fixture").unwrap();
    m.object_interactions.get_mut(&2).unwrap().truncate(1); //Writes Temp0=123, returns false.
    m.object_definition_words.insert(3, vec![0; 100]);
    m.object_interactions.insert(
        3,
        vec![MenuEntry {
            id: 0,
            action: 4096,
            check: 4098,
            ..Default::default()
        }],
    );
    for budget in [1, 1000] {
        let mut vm = initial.clone();
        let mut target = vm.world.objects[&2].clone();
        target.definition.as_mut().unwrap().object = 3;
        vm.world.objects.insert(3, target);
        vm.begin_player_menu(&p, 1, 2).unwrap();
        let completed = loop {
            if let Some(done) = vm.advance_player_menu_retained(&p, budget).unwrap().0 {
                break done;
            }
        };
        assert!(completed.actions.is_empty());
        assert_eq!(vm.world.objects[&1].temps[0], 123);
        //The VM and its caller-owned destructor are both retained across load.
        vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
        let cleanup: player_menu::Cleanup =
            serde_json::from_slice(&serde_json::to_vec(&completed.cleanup).unwrap()).unwrap();
        vm.begin_player_menu(&p, 1, 3).unwrap();
        loop {
            if let Some(actions) = vm.advance_player_menu(&p, budget).unwrap().0 {
                assert_eq!(actions.len(), 1);
                break;
            }
        }
        assert_eq!(
            vm.world.objects[&1].attributes[2], 123,
            "nested check saw prematurely restored Temps"
        );
        assert_eq!(
            vm.world.objects[&1].temps[0], 123,
            "child destructor lost outer Temps"
        );
        cleanup.restore(&mut vm).unwrap();
        assert_eq!(vm.world.objects[&1].temps, [42; 8]);
        assert_eq!(vm.tick, initial.tick);
        assert_eq!(vm.rng_state, initial.rng_state);
        Snapshot::capture(&p, &vm).restore(&p).unwrap();
    }
}

#[test]
fn live_checks_keep_side_effects_restore_temps_once_and_resume_on_same_arena() {
    let (p, initial) = fixture();
    let mut outcomes = vec![];
    for budget in [1, 1000] {
        let mut vm = initial.clone();
        let original_stack = vm.threads[&1].stack.clone();
        vm.begin_player_menu(&p, 1, 2).unwrap();
        let mut instructions = vec![];
        let actions = loop {
            let saved = serde_json::to_vec(&Snapshot::capture(&p, &vm)).unwrap();
            vm = Snapshot::from_json(&saved, &p).unwrap();
            let before = vm.clone();
            assert!(vm.advance_tick().is_err());
            assert!(vm.run_round(&p, 100).is_empty());
            assert_eq!(vm, before);
            let (done, run) = vm.advance_player_menu(&p, budget).unwrap();
            instructions.extend(run.trace);
            if let Some(actions) = done {
                break actions;
            }
        };
        assert_eq!(
            actions.iter().map(|a| a.menu_id).collect::<Vec<_>>(),
            [2, 9]
        );
        assert_eq!(
            actions.iter().map(|a| a.flags).collect::<Vec<_>>(),
            [12, 13]
        );
        assert!(
            actions
                .iter()
                .all(|a| a.priority == 50 && a.id == 0 && a.args == [0; 4])
        );
        assert_eq!(&vm.world.objects[&1].attributes[..3], &[4097, 99, 123]);
        assert_eq!(vm.world.objects[&1].temps, [42; 8]);
        assert_eq!(vm.world.objects[&1].data[&50], 0);
        assert_eq!(vm.world.actions, initial.world.actions);
        assert_eq!(vm.rng_state, initial.rng_state);
        assert_eq!(vm.tick, initial.tick);
        assert_eq!(vm.threads[&1].stack, original_stack);
        assert_eq!(vm.threads[&1].status, initial.threads[&1].status);
        assert_ne!(
            vm.threads[&1].frame_memory,
            initial.threads[&1].frame_memory
        );
        assert!(vm.player_menu.is_none());
        Snapshot::capture(&p, &vm).restore(&p).unwrap();
        outcomes.push((vm, actions, instructions));
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[test]
fn missing_checks_keep_native_sentinel_and_empty_tables_return_empty() {
    let (mut p, mut vm) = fixture();
    p.modules
        .get_mut("fixture")
        .unwrap()
        .object_interactions
        .get_mut(&2)
        .unwrap()
        .truncate(1);
    p.modules.get_mut("fixture").unwrap().routines.remove(&4097);
    vm.begin_player_menu(&p, 1, 2).unwrap();
    assert!(
        vm.advance_player_menu(&p, 10)
            .unwrap()
            .0
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        &vm.threads[&1].frame_memory[16..24],
        &[
            Some(-1),
            Some(0),
            Some(0),
            Some(0),
            Some(1),
            Some(0),
            Some(0),
            Some(0)
        ]
    );
    assert_eq!(vm.world.objects[&1].temps, [42; 8]);
    p.modules
        .get_mut("fixture")
        .unwrap()
        .object_interactions
        .remove(&2);
    let before = vm.clone();
    vm.begin_player_menu(&p, 1, 2).unwrap();
    assert!(vm.advance_player_menu(&p, 0).unwrap().0.unwrap().is_empty());
    assert_eq!(before, vm);
}

#[test]
fn pending_checks_reject_mismatched_owners_and_clock_without_advancing() {
    let (p, mut vm) = fixture();
    vm.begin_player_menu(&p, 1, 2).unwrap();
    vm.advance_player_menu(&p, 0).unwrap();
    let before = vm.clone();
    assert!(vm.begin_player_menu(&p, 1, 2).is_err());
    assert!(vm.run_slice(&p, 2, 1).is_err());
    assert_eq!(before, vm);
    let mut wrong = vm.clone();
    wrong.tick += 1;
    assert!(Snapshot::capture(&p, &wrong).restore(&p).is_err());
    wrong = vm.clone();
    wrong
        .threads
        .get_mut(&1)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .callee = 1;
    assert!(Snapshot::capture(&p, &wrong).restore(&p).is_err());
    Snapshot::capture(&p, &vm).restore(&p).unwrap();
}

#[test]
fn rejected_random_check_keeps_consumed_rng_after_temp_restoration() {
    let (mut p, initial) = fixture();
    let m = p.modules.get_mut("fixture").unwrap();
    m.object_interactions.get_mut(&2).unwrap().truncate(1);
    m.routines.get_mut(&4097).unwrap().body.instructions = vec![Instruction {
        opcode: 8,
        operands: [0, 0, 8, 0, 100, 0, 7, 0],
        on_true: 255,
        on_false: 253,
    }];
    let mut expected = initial.rng_state;
    rng::next(&mut expected);
    for budget in [1, 100] {
        let mut vm = initial.clone();
        vm.begin_player_menu(&p, 1, 2).unwrap();
        let actions = loop {
            vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
            if let Some(actions) = vm.advance_player_menu(&p, budget).unwrap().0 {
                break actions;
            }
        };
        assert!(actions.is_empty());
        assert_eq!(vm.world.objects[&1].temps, initial.world.objects[&1].temps);
        assert_eq!(vm.rng_state, expected);
        assert_ne!(vm.rng_state, initial.rng_state);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn shared_eligibility_matches_140_retained_original_engine_cases() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/player-menu-eligibility.json")).unwrap();
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 140);
    for row in cases {
        let c = &row["input"];
        let n = &row["original"];
        for budget in [1, 1000] {
            let (mut p, mut vm) = fixture();
            let m = p.modules.get_mut("fixture").unwrap();
            let table = m.object_interactions.get_mut(&2).unwrap();
            table.truncate(1);
            table[0].flags = c["ttab_flags"].as_u64().unwrap() as u32;
            m.routines.get_mut(&4097).unwrap().body.instructions = vec![
                assign(8, 0, 7, 99, 1),
                assign(0, 0, 7, 321, c["check_return"].as_u64().unwrap() as u8),
            ];
            let o = vm.world.objects.get_mut(&1).unwrap();
            o.attributes[0] = 100;
            o.temps = [1, 2, 3, 0, 7, -1, 8, 255];
            o.person
                .as_mut()
                .unwrap()
                .insert(58, c["age"].as_i64().unwrap() as i16);
            o.person
                .as_mut()
                .unwrap()
                .insert(65, c["species"].as_i64().unwrap() as i16);
            vm.world
                .objects
                .get_mut(&2)
                .unwrap()
                .data
                .insert(8, c["target_flags"].as_i64().unwrap() as i16);
            vm.begin_player_menu(&p, 1, 2).unwrap();
            let actions = loop {
                vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
                if let Some(actions) = vm.advance_player_menu(&p, budget).unwrap().0 {
                    break actions;
                }
            };
            assert_eq!(!actions.is_empty(), n["active_serial"] == 1, "{c}");
            assert_eq!(
                serde_json::json!(vm.world.objects[&1].attributes[0]),
                n["attribute"],
                "{c}"
            );
            assert_eq!(
                serde_json::json!(vm.world.objects[&1].temps),
                n["temps"],
                "{c}"
            );
            assert_eq!(vm.world.actions.as_ref().unwrap().serial, 77);
            assert!(vm.world.actions.as_ref().unwrap().callbacks.is_empty());
        }
    }
}

#[test]
fn missing_dynamic_label_resource_stops_and_pet_gate_keeps_go_here_exception() {
    let (mut p, mut vm) = fixture();
    p.modules
        .get_mut("fixture")
        .unwrap()
        .routines
        .get_mut(&4097)
        .unwrap()
        .body
        .instructions = vec![Instruction {
        opcode: 50,
        operands: [46, 1, 0, 0, 1, 0, 0, 0],
        on_true: 254,
        on_false: 255,
    }];
    vm.begin_player_menu(&p, 1, 2).unwrap();
    let error = vm.advance_player_menu(&p, 10).unwrap_err();
    assert!(error.contains("resource is not loaded"), "{error}");
    assert!(vm.player_menu.is_some());
    Snapshot::capture(&p, &vm).restore(&p).unwrap();
    let (p, mut vm) = fixture();
    vm.world
        .objects
        .get_mut(&1)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(65, 2);
    vm.world
        .objects
        .get_mut(&2)
        .unwrap()
        .definition
        .as_mut()
        .unwrap()
        .guid = 1;
    let before = vm.clone();
    vm.begin_player_menu(&p, 1, 2).unwrap();
    assert!(
        vm.advance_player_menu(&p, 100)
            .unwrap()
            .0
            .unwrap()
            .is_empty()
    );
    assert_eq!(before, vm);
    vm.world
        .objects
        .get_mut(&2)
        .unwrap()
        .definition
        .as_mut()
        .unwrap()
        .guid = 0x7c4;
    let mut p = p;
    p.modules
        .get_mut("fixture")
        .unwrap()
        .object_interactions
        .get_mut(&2)
        .unwrap()[3]
        .flags |= 1024;
    vm.begin_player_menu(&p, 1, 2).unwrap();
    assert_eq!(
        vm.advance_player_menu(&p, 100).unwrap().0.unwrap()[0].menu_id,
        9
    );
}
