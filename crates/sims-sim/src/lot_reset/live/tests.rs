use super::*;
use serde_json::{Value, json};
use sims_formats::behavior::Instruction;

#[test]
fn reconstruction_resets_every_group_member_before_init_and_retains_saved_location() {
    for group in [false, true] {
        let (mut p, mut initial) = crate::lot_runtime::tests::fixture();
        let m = p.modules.get_mut("objects").unwrap();
        m.object_functions.get_mut(&1).unwrap()[0].action = 4100;
        if group {
            m.object_definition_words.get_mut(&1).unwrap()[10] = 7;
        }
        let mut init = m.routines[&4096].clone();
        init.body.instructions[0].operands = [0, 0, 11, 0, 0, 5, 0, 3];
        m.routines.insert(4100, init);
        for (id, head, next) in [
            (1, None, Some(3)),
            (3, Some(1), Some(2)),
            (2, Some(1), None),
        ] {
            let o = initial.vm.world.objects.get_mut(&id).unwrap();
            o.attributes = vec![99; 8];
            o.temps = [99; 8];
            if group {
                o.multitile = Some(sims_vm::multitile::Part {
                    head,
                    next,
                    origin: [0; 3],
                    offset: [0; 3],
                });
            }
        }
        initial.vm = Vm::new(initial.vm.world.clone(), 1);
        for id in 1..=4 {
            initial.vm.start_object_lifecycle(&p, id).unwrap();
            initial.vm.restart_loaded_main(&p, id).unwrap();
        }
        let now = ObjectDate {
            year: 1997,
            month: 3,
            day: 10,
        };
        for selected in [1, 2, 3] {
            for deleted in [false, true] {
                let parts = if group { vec![1, 3, 2] } else { vec![selected] };
                let mut outcomes = vec![];
                for budget in [1, 3, 1000] {
                    let mut s = initial.clone();
                    let mut reset = Execution::new(selected);
                    let mut step = Step {
                        stop: crate::StepStop::Budget,
                        trace: vec![],
                        events: vec![],
                    };
                    let done = reset
                        .advance(&mut s, &p, now, deleted, &mut 0, &mut step)
                        .unwrap();
                    assert_eq!(done, deleted);
                    assert!(step.trace.is_empty());
                    for id in &parts {
                        assert_eq!(
                            s.vm.world.objects[id].attributes,
                            vec![if deleted { 99 } else { 0 }; 8]
                        );
                    }
                    let mut init_order = vec![];
                    let mut checkpoints = 0;
                    while !reset.complete() {
                        step.trace.clear();
                        reset
                            .advance(&mut s, &p, now, deleted, &mut { budget }, &mut step)
                            .unwrap();
                        init_order.extend(
                            step.trace
                                .iter()
                                .filter(|t| t.location.bhav == 4100)
                                .map(|t| {
                                    *s.vm
                                        .object_threads
                                        .iter()
                                        .find(|(_, thread)| **thread == t.thread)
                                        .unwrap()
                                        .0
                                }),
                        );
                        reset.validate(&s, &p).unwrap();
                        let (next_reset, next_sim): (Execution, Simulation) =
                            serde_json::from_slice(&serde_json::to_vec(&(&reset, &s)).unwrap())
                                .unwrap();
                        assert_eq!(next_reset, reset);
                        assert_eq!(next_sim, s);
                        sims_vm::Snapshot::capture(&p, &s.vm).restore(&p).unwrap();
                        if let Phase::Update { parts, .. } = &reset.phase {
                            let mut bad = reset.clone();
                            if let Phase::Update {
                                cursor, started, ..
                            } = &mut bad.phase
                            {
                                *cursor = parts.len();
                                *started = false;
                            }
                            if parts
                                .iter()
                                .any(|id| s.vm.world.objects[id].last_update != Some(s.vm.tick))
                            {
                                assert!(
                                    bad.validate(&s, &p).is_err(),
                                    "unfinished update accepted as completed"
                                );
                            }
                        }
                        s = next_sim;
                        reset = next_reset;
                        checkpoints += 1;
                        assert!(checkpoints < 100);
                    }
                    assert_eq!(init_order, if deleted { vec![] } else { parts.clone() });
                    for id in &parts {
                        let object = &s.vm.world.objects[id];
                        assert_eq!(object.attributes[0], if deleted { 99 } else { *id });
                        if !deleted {
                            assert_eq!(s.vm.world.positions[id], initial.vm.world.positions[id]);
                            assert_eq!(object.last_update, Some(0));
                        } else {
                            assert_eq!(s.vm.world.positions[id].x, -16);
                        }
                    }
                    assert_eq!(s.vm.tick, 0);
                    outcomes.push(s);
                }
                assert_eq!(outcomes[0], outcomes[1]);
                assert_eq!(outcomes[0], outcomes[2]);
            }
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn callback_reset_matches_original_fields_frames_order_and_budgeted_resume() {
    let report: Value = serde_json::from_str(include_str!(
        "../../../../sims-vm/tests/fixtures/load-services.json"
    ))
    .unwrap();
    assert_eq!(report["resets"].as_array().unwrap().len(), 54);
    for row in report["resets"].as_array().unwrap() {
        let c = &row["input"];
        let original = &row["original"];
        for incoming in [
            Status::Ready,
            Status::Sleeping { wake_tick: 500 },
            Status::Returned(true),
        ] {
            let (mut p, mut initial) = crate::lot_runtime::tests::fixture();
            let module = p.modules.get_mut("objects").unwrap();
            module.object_functions.get_mut(&1).unwrap()[3].action =
                c["bhav"].as_u64().unwrap() as u16;
            let mut main = module.routines[&4096].clone();
            main.body.args = 4;
            // A visible effect if a driver mistakenly executes the new Main.
            main.body.instructions[0].operands = [0, 0, 111, 0, 0, 5, 6, 7];
            module.routines.insert(4096, main.clone());
            let mut callback = main;
            callback.body.instructions = vec![
                Instruction {
                    opcode: 2,
                    on_true: 1,
                    on_false: 253,
                    operands: [0, 0, 0, 0, 0, 5, 0, 9],
                },
                Instruction {
                    opcode: 2,
                    on_true: 2,
                    on_false: 253,
                    operands: [62, 0, 65, 1, 0, 5, 3, 7],
                },
                Instruction {
                    opcode: 2,
                    on_true: if c["returned"] == true { 254 } else { 255 },
                    on_false: 253,
                    operands: [8, 0, 255, 255, 0, 5, 3, 7],
                },
            ];
            module.routines.insert(7901, callback);
            initial.vm.restart_loaded_main(&p, 4).unwrap();
            let object = initial.vm.world.objects.get_mut(&1).unwrap();
            object.attributes = vec![100];
            object.temps = [-123, 2, 3, 4, 5, 6, 7, 8];
            object
                .data
                .extend([(8, c["flags"].as_u64().unwrap() as i16), (62, 987)]);
            object.idle = Some(sims_vm::idle::IdleState {
                countdown: 51,
                ..Default::default()
            });
            let thread = initial
                .vm
                .restore_object_stack(
                    &p,
                    1,
                    &[sims_vm::loading::SavedFrame {
                        owner: "objects".into(),
                        tree: 4096,
                        node: 0,
                        stack_object: 0,
                        args: vec![0; 4],
                        locals: vec![],
                        primitive_state: 0,
                    }],
                    128,
                    false,
                )
                .unwrap();
            let t = initial.vm.threads.get_mut(&thread).unwrap();
            t.last_result = c["previous"].as_bool().unwrap();
            t.status = incoming.clone();
            t.frame_memory.resize(64, Some(0));
            t.frame_memory[12..32].fill(Some(0));
            t.frame_memory[32..].fill(Some(0x5a5a));
            let main_before = t.stack[0].clone();
            if matches!(incoming, Status::Returned(_)) {
                t.stack.clear();
                initial.vm.lifecycles.get_mut(&1).unwrap().phase =
                    sims_vm::lifecycle::Phase::Finished;
            }
            let owner = c["owner"].as_i64().unwrap() as i16;
            initial.vm.world.ui_owner.owner = (owner != 0).then_some(owner);
            let globals = initial.vm.world.globals.clone();
            let mut outcome = None;
            for budget in [1, 3, 1000] {
                let mut s = initial.clone();
                let mut reset = Execution::new(1);
                let mut trace = vec![];
                loop {
                    let mut step = Step {
                        stop: crate::StepStop::Budget,
                        trace: vec![],
                        events: vec![],
                    };
                    let done = reset
                        .advance(
                            &mut s,
                            &p,
                            ObjectDate::default(),
                            false,
                            &mut { budget },
                            &mut step,
                        )
                        .unwrap();
                    trace.extend(step.trace);
                    reset.validate(&s, &p).unwrap();
                    let (saved_reset, saved_sim): (Execution, Simulation) =
                        serde_json::from_slice(&serde_json::to_vec(&(&reset, &s)).unwrap())
                            .unwrap();
                    assert_eq!(saved_reset, reset);
                    assert_eq!(saved_sim, s);
                    saved_reset.validate(&saved_sim, &p).unwrap();
                    sims_vm::Snapshot::capture(&p, &s.vm)
                        .restore(&p)
                        .unwrap_or_else(|error| {
                            panic!(
                                "{c}/{incoming:?}/{budget}: {error}; {:?}; {:?}",
                                reset.phase, s.vm.lifecycles
                            )
                        });
                    if let Phase::Callback(e) = &reset.phase {
                        let depth = s.vm.threads[&thread].callback.as_ref().unwrap().base_depth;
                        assert!(e.owns_callback(&s.vm, &p, thread, depth));
                        assert!(!e.owns_callback(&s.vm, &p, thread, depth + 1));
                        let mut bad = reset.clone();
                        bad.object = 2;
                        assert!(bad.validate(&s, &p).is_err());
                    }
                    reset = saved_reset;
                    s = saved_sim;
                    if done {
                        break;
                    }
                    assert!(trace.len() < 100);
                }
                let t = &s.vm.threads[&thread];
                let o = &s.vm.world.objects[&1];
                let actual = json!({"last_result":t.last_result,"data8":o.data[&8],"data62":o.data[&62],
                    "attribute0":o.attributes[0],"schedule":o.idle.as_ref().unwrap().countdown,
                    "owner":s.vm.world.ui_owner.owner.unwrap_or(0)});
                for key in [
                    "last_result",
                    "data8",
                    "data62",
                    "attribute0",
                    "schedule",
                    "owner",
                ] {
                    assert_eq!(
                        actual[key], original[key],
                        "{c}/{incoming:?}/{budget}: {key}"
                    );
                }
                assert_eq!(t.stack.len(), 1);
                assert_eq!(t.stack[0], main_before);
                let events: Vec<_> =
                    s.vm.world
                        .ui_owner
                        .effects
                        .iter()
                        .map(|effect| {
                            assert_eq!(*effect, sims_vm::ui_owner::Effect::Changed);
                            json!([0x107, 0])
                        })
                        .collect();
                assert_eq!(json!(events), original["events"]);
                assert_eq!(s.vm.world.globals, globals, "Main ran inside Reset");
                assert_eq!(
                    s.vm.world.positions, initial.vm.world.positions,
                    "callback reset moved object"
                );
                assert_eq!(
                    s.native_lot, initial.native_lot,
                    "callback reset emitted placement effects"
                );
                let expected_trace: Vec<_> = original["steps"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|s| s["bhav"] != 65535 && s["pc"].as_u64().unwrap() <= 250)
                    .map(|s| json!([s["bhav"], s["pc"]]))
                    .collect();
                assert_eq!(
                    trace
                        .iter()
                        .map(|t| json!([t.location.bhav, t.location.instruction]))
                        .collect::<Vec<_>>(),
                    expected_trace
                );
                if let Some(old) = &outcome {
                    assert_eq!(&s, old, "budget changed Reset {c}");
                }
                outcome = Some(s);
            }
        }
    }
}
