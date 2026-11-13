//! Synthetic scripts assert query continuation, shared check registers, dynamic
//! advertisements and action dispatch without distributing original assets.
use sims_formats::{
    behavior::{Behavior, Instruction},
    tuning::{Constant, Constants},
};
use sims_vm::{
    autonomy::{Context, Placement},
    program::{AutonomyAdvertisement, MenuEntry},
    *,
};
use std::collections::{BTreeMap, BTreeSet};
fn expr(ls: u8, li: i16, op: u8, rs: u8, ri: i16, next: u8) -> Instruction {
    let mut b = [0; 8];
    b[..2].copy_from_slice(&li.to_le_bytes());
    b[2..4].copy_from_slice(&ri.to_le_bytes());
    b[5..].copy_from_slice(&[op, ls, rs]);
    Instruction {
        opcode: 2,
        on_true: next,
        on_false: 253,
        operands: b,
    }
}
fn routine(instructions: Vec<Instruction>) -> Routine {
    Routine {
        name: "synthetic".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args: 4,
            locals: 0,
            instructions,
        },
    }
}
fn fixture() -> (Program, Vm, u64) {
    let mut m = Module {
        source: "fixture".into(),
        ..Default::default()
    };
    m.routines.insert(
        4096,
        routine(vec![
            Instruction {
                opcode: 3,
                on_true: 1,
                on_false: 2,
                operands: [0; 8],
            },
            Instruction {
                opcode: 30,
                on_true: 2,
                on_false: 2,
                operands: [0; 8],
            },
            Instruction {
                opcode: 0,
                on_true: 254,
                on_false: 255,
                operands: [0; 8],
            },
        ]),
    );
    m.routines.insert(
        4097,
        routine(vec![
            expr(8, 0, 3, 7, 1, 1),
            expr(1, 0, 5, 8, 0, 2),
            expr(12, 6, 5, 8, 0, 3),
            expr(12, 6, 6, 7, 2000, 254),
        ]),
    );
    m.routines
        .insert(4098, routine(vec![expr(0, 0, 5, 10, 0, 254)]));
    for id in [1, 2, 5, 6] {
        m.named_constants.insert(
            id,
            Constants {
                version: 1,
                entries: vec![],
            },
        );
    }
    m.named_constants
        .get_mut(&2)
        .unwrap()
        .entries
        .push(Constant {
            name: "random selection count".into(),
            value: 1.,
            comment: String::new(),
        });
    for id in [501, 502, 503, 504] {
        m.strings
            .insert(id, vec!["(-100;-100)(100;100)".into(); 10]);
    }
    for id in [1, 2, 3] {
        let mut words = vec![0; 108];
        words[7] = 1;
        words[9] = if id == 1 { 2 } else { 4 };
        m.object_definition_words.insert(id, words);
        let mut ads = vec![
            motives::Advertisement {
                personality: 0,
                minimum: 0,
                delta: 0
            };
            16
        ];
        ads[13].minimum = 1000;
        ads[13].delta = 1000;
        m.object_interactions.insert(
            id,
            [10, 11]
                .into_iter()
                .map(|menu| MenuEntry {
                    id: menu,
                    flags: 256,
                    check: 4097,
                    action: 4098,
                    autonomy: AutonomyAdvertisement {
                        threshold: 50,
                        attenuation_code: 1,
                        motives: ads.clone(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .collect(),
        );
    }
    let p = Program {
        globals: "fixture".into(),
        modules: BTreeMap::from([("fixture".into(), m)]),
        ..Default::default()
    };
    let mut w = World {
        tree_advertisements: vec![[0; 3]; 16],
        ..Default::default()
    };
    // Explicit supplied process owner for this authored query fixture. Missing
    // FCNS fields are no longer silently filled by the query implementation.
    w.autonomy_constants = Some(autonomy_constants::Owner {
        process: autonomy_constants::Process {
            loaded: None,
            words: [
                Some(0.2f32.to_bits()),
                Some(0.05f32.to_bits()),
                Some(0.002f32.to_bits()),
                Some(0.02f32.to_bits()),
                Some(0.1f32.to_bits()),
                Some(0.002f32.to_bits()),
                Some(0.02f32.to_bits()),
                Some(0.1f32.to_bits()),
                Some(0.2f32.to_bits()),
                Some(1),
                None,
                None,
            ],
        },
        ..Default::default()
    });
    for id in [1, 2, 3] {
        w.objects.insert(
            id,
            Object {
                definition: Some(Definition {
                    source: "fixture".into(),
                    object: id as u16,
                    guid: 0,
                    slots: 0,
                }),
                attributes: vec![0; 8],
                data: (0..72)
                    .map(|i| {
                        (
                            i,
                            if i == 29 {
                                if id == 3 { 2 } else { 1 }
                            } else {
                                0
                            },
                        )
                    })
                    .collect(),
                ..Default::default()
            },
        );
    }
    let actor = w.objects.get_mut(&1).unwrap();
    actor.person = Some(
        (0..256)
            .map(|i| (i, if i == 36 { 50 } else { 0 }))
            .collect(),
    );
    actor.motives = Some([0; 16]);
    actor.temps[0] = 5;
    w.actions = Some(action::WorldState {
        live_family: true,
        actors: BTreeMap::from([(1, action::Actor::default())]),
        ..Default::default()
    });
    w.person = Some(person::WorldState {
        free_will: true,
        controlled_family: 1,
        autonomy: Some(Context {
            residential: true,
            advertised: Some(BTreeSet::from([2, 3])),
            full_scan: Some(BTreeSet::from([1])),
            placements: (1..=3)
                .map(|i| {
                    (
                        i,
                        Placement {
                            x: i32::from(i) * 16,
                            y: 0,
                            level: 1,
                        },
                    )
                })
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    });
    let mut vm = Vm::new(w, 123);
    let id = vm
        .spawn(
            &p,
            Start {
                owner: "fixture",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 1,
                args: vec![0; 4],
                check: false,
            },
        )
        .unwrap();
    (p, vm, id)
}
#[test]
fn checks_share_temps_then_restore_and_resume_into_selected_original_action() {
    let (p, mut vm, id) = fixture();
    let mut uninterrupted = vm.clone();
    let mut traces = vec![];
    let mut checkpoints = 0;
    loop {
        let run = vm.run_slice(&p, id, 1).unwrap();
        assert_ne!(run.stop, Stop::Faulted, "{:?}", run.trace);
        traces.extend(run.trace);
        vm = Snapshot::from_json(
            &serde_json::to_vec(&Snapshot::capture(&p, &vm)).unwrap(),
            &p,
        )
        .unwrap();
        checkpoints += 1;
        if run.stop != Stop::Budget {
            break;
        }
        assert!(checkpoints < 100);
    }
    let mut other = vec![];
    loop {
        let run = uninterrupted.run_slice(&p, id, 1000).unwrap();
        other.extend(run.trace);
        if run.stop != Stop::Budget {
            break;
        }
    }
    assert_eq!(traces, other);
    assert_eq!(vm, uninterrupted);
    assert!(checkpoints > 20);
    assert_eq!(vm.world.objects[&1].temps[0], 5);
    assert_eq!(vm.world.objects[&2].attributes[0], 7);
    assert_eq!(vm.world.objects[&3].attributes[0], 9);
    assert_eq!(vm.world.objects[&1].attributes[0], 3);
    assert_eq!(vm.world.objects[&1].data[&20], 10);
    assert_eq!(vm.world.tree_advertisements[13], [0, 1000, -1000]);
    assert_eq!(
        vm.world.actions.as_ref().unwrap().actors[&1]
            .previous
            .as_ref()
            .unwrap()
            .target,
        3
    );
}
#[test]
fn malformed_search_cursor_and_callback_binding_are_rejected() {
    let (p, mut vm, id) = fixture();
    vm.run_slice(&p, id, 1).unwrap();
    let mut snapshot = Snapshot::capture(&p, &vm);
    snapshot
        .state
        .threads
        .get_mut(&id)
        .unwrap()
        .autonomy
        .as_mut()
        .unwrap()
        .next_object = usize::MAX;
    assert!(snapshot.restore(&p).is_err());
    let mut snapshot = Snapshot::capture(&p, &vm);
    snapshot
        .state
        .threads
        .get_mut(&id)
        .unwrap()
        .autonomy
        .as_mut()
        .unwrap()
        .waiting
        .as_mut()
        .unwrap()
        .object = 3;
    assert!(snapshot.restore(&p).is_err());
}

#[cfg(feature = "private-fixtures")]
#[test]
fn eligibility_sequence_matches_original_shared_registers_and_room_advertisements() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/missing-eligibility.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let (p0, mut vm, id) = fixture();
        let mut p = p0;
        vm.world
            .person
            .as_mut()
            .unwrap()
            .autonomy
            .as_mut()
            .unwrap()
            .advertised = Some([2].into());
        vm.world
            .objects
            .get_mut(&2)
            .unwrap()
            .data
            .insert(29, row["input"]["target_room"].as_i64().unwrap() as i16);
        p.modules
            .get_mut("fixture")
            .unwrap()
            .routines
            .get_mut(&4097)
            .unwrap()
            .body
            .instructions[3]
            .on_true = row["input"]["return"].as_u64().unwrap() as u8;
        let missing = row["input"]["missing"] == true;
        if missing {
            p.modules.get_mut("fixture").unwrap().routines.remove(&4097);
        }
        let mut results = vec![];
        for _ in 0..100 {
            let run = vm.run_slice(&p, id, 1).unwrap();
            assert_ne!(run.stop, Stop::Faulted, "{:?}", run.trace);
            if run.trace.iter().any(|t| {
                t.effect == "Find Best Action scored eligibility callback"
                    || t.effect == "Find Best Action rejected absent eligibility tree"
            }) {
                let original = &row["original"]["checks"][results.len()];
                assert_eq!(
                    serde_json::json!(vm.world.objects[&1].temps[0]),
                    original["temp"]
                );
                let ads: Vec<_> = vm
                    .world
                    .tree_advertisements
                    .iter()
                    .flatten()
                    .copied()
                    .collect();
                assert_eq!(serde_json::json!(ads), original["ads"]);
                let thread = &vm.threads[&id];
                let parent = &thread.stack[0];
                let end = parent.storage_offset + 8 + parent.args.len() + parent.locals.len();
                assert_eq!(
                    serde_json::json!(&thread.frame_memory[end..end + 8]),
                    row["original"]["sentinel"]
                );
                results.push(());
            }
            vm = Snapshot::from_json(
                &serde_json::to_vec(&Snapshot::capture(&p, &vm)).unwrap(),
                &p,
            )
            .unwrap();
            if run.stop != Stop::Budget {
                break;
            }
        }
        assert_eq!(results.len(), 2);
        assert_eq!(
            serde_json::json!(vm.world.objects[&1].temps[0]),
            row["original"]["restored_temp"]
        );
        assert_eq!(
            vm.world.actions.as_ref().unwrap().actors[&1]
                .previous
                .is_some(),
            !missing && row["input"]["return"] == 254
        );
    }
}

#[test]
fn native_registry_and_current_zoning_drive_query_across_snapshot() {
    for advertised in [false, true] {
        let (p, mut vm, id) = fixture();
        vm.world.globals.insert(10, 7);
        let people = vm.world.person.as_mut().unwrap();
        people.lot_zoning.insert(7, 1);
        let context = people.autonomy.as_mut().unwrap();
        context.full_scan = None;
        context.advertised = None;
        // Actor 1, tick 1 would otherwise use subset 2 and exclude object 3.
        vm.tick = 1;
        for (&object, state) in &mut vm.world.objects {
            state.idle = Some(idle::IdleState::from_registry_word(
                if object == 3 && advertised {
                    0x0009ffff
                } else {
                    0x0001ffff
                },
            ));
        }
        let mut uninterrupted = vm.clone();
        for _ in 0..100 {
            let run = vm.run_slice(&p, id, 1).unwrap();
            assert_ne!(run.stop, Stop::Faulted, "{:?}", run.trace);
            if let Some(q) = &vm.threads[&id].autonomy {
                assert_eq!(q.subset, 0);
            }
            vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
            if run.stop != Stop::Budget {
                break;
            }
        }
        loop {
            if uninterrupted.run_slice(&p, id, 1000).unwrap().stop != Stop::Budget {
                break;
            }
        }
        assert_eq!(vm, uninterrupted);
        let previous = vm.world.actions.as_ref().unwrap().actors[&1]
            .previous
            .as_ref();
        assert_eq!(previous.map(|a| a.target), advertised.then_some(3));
        // Object 2 has the same usable TTAB but no cached native advertisement.
        assert_eq!(vm.world.objects[&2].attributes[0], 0);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn person_join_uses_scanned_distance_and_survives_callback_save_resume() {
    let (mut p, mut vm, id) = fixture();
    let m = p.modules.get_mut("fixture").unwrap();
    m.routines.get_mut(&4097).unwrap().body.args = 5;
    m.object_definition_words.get_mut(&2).unwrap()[9] = 2;
    for entry in m.object_interactions.get_mut(&2).unwrap() {
        entry.autonomy.threshold = 0;
    }
    let target_entries = m.object_interactions.get_mut(&3).unwrap();
    target_entries[0].flags |= 2;
    target_entries[0].autonomy.joining_index = 11;
    // Join tail does not apply the ordinary-entry autonomy threshold gate.
    target_entries[1].autonomy.threshold = 100;
    let person = vm.world.objects[&1].person.clone();
    vm.world.objects.get_mut(&2).unwrap().person = person;
    vm.world.actions.as_mut().unwrap().actors.insert(
        2,
        action::Actor {
            active: Some(action::Action {
                target: 3,
                menu_id: 10,
                routine: 4098,
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    vm.world
        .person
        .as_mut()
        .unwrap()
        .autonomy
        .as_mut()
        .unwrap()
        .advertised = Some(BTreeSet::from([2]));
    let mut baseline = vm.clone();
    let mut saw_join = false;
    for _ in 0..100 {
        let run = vm.run_slice(&p, id, 1).unwrap();
        assert_ne!(run.stop, Stop::Faulted, "{:?}", run.trace);
        if let Some(query) = &vm.threads[&id].autonomy {
            for pending in &query.pending {
                assert_eq!(
                    (pending.object, pending.menu, pending.distance),
                    (3, 11, 1.)
                );
                assert!(pending.join);
                saw_join = true;
            }
            if let Some(frame) = vm.threads[&id].stack.get(query.parent_depth) {
                assert_eq!(frame.args[1], 1, "join check context was lost");
                assert_eq!(frame.args[4], 4097, "native fifth callback word was lost");
                let report: serde_json::Value =
                    serde_json::from_str(include_str!("fixtures/autonomy-join.json")).unwrap();
                let native = report["checks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| {
                        r["input"]["kind"] == 1
                            && r["input"]["autonomous"] == 1
                            && r["input"]["third"] == 0
                            && r["input"]["check"] == 4097
                    })
                    .unwrap();
                assert_eq!(serde_json::json!(frame.args), native["original"]);
            }
        }
        vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
        if run.stop != Stop::Budget {
            break;
        }
    }
    assert!(saw_join);
    loop {
        if baseline.run_slice(&p, id, 1000).unwrap().stop != Stop::Budget {
            break;
        }
    }
    assert_eq!(vm, baseline);
    let previous = vm.world.actions.as_ref().unwrap().actors[&1]
        .previous
        .as_ref()
        .unwrap();
    assert_eq!((previous.target, previous.menu_id), (3, 11));
}

#[test]
fn faulted_eligibility_retains_valid_query_and_callback_snapshot() {
    let (mut p, mut vm, id) = fixture();
    p.modules
        .get_mut("fixture")
        .unwrap()
        .routines
        .get_mut(&4097)
        .unwrap()
        .body
        .instructions[0]
        .opcode = 254;
    loop {
        let step = vm.run_slice(&p, id, 1).unwrap();
        if step.stop == Stop::Faulted {
            break;
        }
        assert_eq!(step.stop, Stop::Budget);
    }
    assert!(vm.threads[&id].autonomy.is_some());
    assert!(vm.threads[&id].callback.is_some());
    assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    let mut malformed = Snapshot::capture(&p, &vm);
    malformed
        .state
        .threads
        .get_mut(&id)
        .unwrap()
        .autonomy
        .as_mut()
        .unwrap()
        .next_object = usize::MAX;
    assert!(malformed.restore(&p).is_err());
}

#[test]
fn query_uses_process_owner_when_fcns_is_absent_and_preserves_unknown_history() {
    let (p, vm, id) = fixture();
    let mut without = p.clone();
    without
        .modules
        .get_mut("fixture")
        .unwrap()
        .named_constants
        .remove(&2);
    let mut a = vm.clone();
    let mut b = vm.clone();
    loop {
        let left = a.run_slice(&p, id, 1).unwrap();
        let right = b.run_slice(&without, id, 1).unwrap();
        assert_eq!(left, right);
        assert_ne!(left.stop, Stop::Faulted, "{:?}", left.trace);
        assert_eq!(a, b);
        b = Snapshot::from_json(
            &serde_json::to_vec(&Snapshot::capture(&without, &b)).unwrap(),
            &without,
        )
        .unwrap();
        assert_eq!(a, b);
        if left.stop != Stop::Budget {
            break;
        }
    }
    let mut missing = vm;
    missing.world.autonomy_constants = None;
    let rng = missing.rng_state;
    let run = missing.run_slice(&p, id, 1).unwrap();
    assert_eq!(run.stop, Stop::Faulted);
    assert_eq!(missing.rng_state, rng);
    assert!(missing.world.autonomy_constants.is_none());
    assert!(format!("{:?}", run.trace).contains("native autonomy process word"));
}

#[test]
fn native_personal_score_cache_is_used_and_cold_resumes_without_resource_curves() {
    let (p, mut vm, id) = fixture();
    let mut curves=motives::Tuning::from_program(&p).unwrap().adult_scores;
    curves[0].points[0][1]+=17.0;
    let actor=vm.world.objects.get_mut(&1).unwrap();
    actor.native_flags=Some(0);
    actor.native_motive_scores=Some(motives::score_owner::Owner::capture(&curves).unwrap());
    let mut without=p.clone();
    for table in [501,503]{without.modules.get_mut("fixture").unwrap().strings.remove(&table);}
    let mut a=vm.clone();let mut b=vm.clone();let mut checkpoints=0;
    loop{
        let left=a.run_slice(&p,id,1).unwrap();let right=b.run_slice(&without,id,1).unwrap();
        assert_eq!(left,right);assert_ne!(left.stop,Stop::Faulted,"{:?}",left.trace);assert_eq!(a,b);
        b=Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(&without,&b)).unwrap(),&without).unwrap();
        assert_eq!(a,b);checkpoints+=1;assert!(checkpoints<100);
        if left.stop!=Stop::Budget{break;}
    }
    let rng=vm.rng_state;vm.world.objects.get_mut(&1).unwrap().native_motive_scores=None;
    let failed=vm.run_slice(&p,id,1).unwrap();assert_eq!(failed.stop,Stop::Faulted);assert_eq!(vm.rng_state,rng);
    assert!(format!("{:?}",failed.trace).contains("native personal score curve owner unavailable"));
}
