use super::*;
use crate::{Snapshot, lot_placement, lot_runtime::tests::fixture};
use serde_json::Value;
use sims_formats::behavior::Instruction;

fn setup(group: bool) -> (Program, Simulation) {
    let (mut p, mut s) = fixture();
    let m = p.modules.get_mut("objects").unwrap();
    let mut ordinary = m.object_definition_words[&1].clone();
    ordinary[14] = 2;
    m.object_definition_words.insert(2, ordinary);
    m.object_functions.insert(2, m.object_functions[&1].clone());
    let bystander =
        s.vm.world
            .objects
            .get_mut(&4)
            .unwrap()
            .definition
            .as_mut()
            .unwrap();
    bystander.object = 2;
    bystander.guid = 2;
    if group {
        m.object_definition_words.get_mut(&1).unwrap()[10] = 7;
        for (id, head, next) in [
            (1, None, Some(3)),
            (3, Some(1), Some(2)),
            (2, Some(1), None),
        ] {
            s.vm.world.objects.get_mut(&id).unwrap().multitile = Some(sims_vm::multitile::Part {
                head,
                next,
                origin: [0; 3],
                offset: [10, 12, 0],
            });
        }
    }
    s.architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&p)
        .unwrap();
    s.vm = sims_vm::Vm::new(s.vm.world.clone(), 1);
    for id in 1..=4 {
        s.vm.start_object_lifecycle(&p, id).unwrap();
    }
    s.vm.world.globals.extend([(34, 0), (36, 0)]);
    (p, s)
}

fn callback(p: &mut Program, function: usize, routine: u16, global: u8) {
    let m = p.modules.get_mut("objects").unwrap();
    m.object_functions.get_mut(&1).unwrap()[function].action = routine;
    let mut body = m.routines[&4096].clone();
    body.body.instructions = vec![
        Instruction {
            opcode: 2,
            on_true: 1,
            on_false: 255,
            operands: [global, 0, 1, 0, 0, 3, 6, 7],
        },
        Instruction {
            opcode: 2,
            on_true: 255,
            on_false: 255,
            operands: [0, 0, 0, 0, 0, 5, 8, 8],
        },
    ];
    m.routines.insert(routine, body);
}

fn restore(p: &Program, s: &Simulation) -> Simulation {
    serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&Snapshot::capture(p, s)).unwrap())
        .unwrap()
        .restore(p)
        .unwrap()
}

#[test]
fn deletion_person_reset_nested_removal_matches_original_and_restores() {
    compare_deletion_person_reset(false);
}

#[test]
fn deletion_person_reset_bound_flags_match_original_and_restore() {
    compare_deletion_person_reset(true);
}

fn compare_deletion_person_reset(bound: bool) {
    let report: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/reset-removal.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 32);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let native = &row["original"];
        let flags = c["primitive_flags"].as_u64().unwrap() as u8;
        let repeats = c["repeats"].as_u64().unwrap() as usize;
        let suppress = c["deletion_flags"].as_u64().unwrap() & 0x1000 != 0;
        let (mut p, mut initial) = setup(false);
        let m = p.modules.get_mut("objects").unwrap();
        let words = m.object_definition_words.get_mut(&2).unwrap();
        words[9] = 2;
        words[19] = 100;
        m.semiglobal = Some("objects".into());
        let mut profile = vec![String::new(); 15];
        profile[12] = "male".into();
        profile[13] = "30".into();
        profile[14] = "lgt".into();
        m.strings.insert(100, profile);
        m.strings.insert(8192, vec!["".into(), "lgt".into()]);
        m.object_functions.get_mut(&2).unwrap()[3].action = 4105;
        let mut routine = m.routines[&4096].clone();
        routine.body.instructions = (0..repeats)
            .map(|i| Instruction {
                opcode: 18,
                on_true: (i + 1) as u8,
                on_false: 253,
                operands: [0, 0, flags, 0, 0, 0, 0, 0],
            })
            .chain([Instruction {
                opcode: 2,
                on_true: 254,
                on_false: 253,
                operands: [0, 0, 77, 0, 0, 5, 8, 7],
            }])
            .collect();
        m.routines.insert(4105, routine);
        let object = initial.vm.world.objects.get_mut(&4).unwrap();
        object.native_flags = bound.then(|| c["deletion_flags"].as_u64().unwrap() as u32);
        object.person = Some((0..256).map(|i| (i, 0)).collect());
        object.person.as_mut().unwrap().extend([(31, 1), (68, 1)]);
        let people = initial.vm.world.person.as_mut().unwrap();
        people.person_order = vec![4];
        people.neighbors.insert(
            1,
            sims_vm::person::Neighbor {
                guid: 2,
                person: (0..256).map(|i| (i, 0)).collect(),
                ..Default::default()
            },
        );
        initial.vm.world.actions = Some(sims_vm::action::WorldState {
            actors: [(4, Default::default())].into(),
            ..Default::default()
        });
        // This authored fixture changes an already-bound ordinary definition
        // into a person; rebuild that one test input's shape before validation.
        initial.architecture.as_mut().unwrap().footprints.remove(&2);
        initial
            .architecture
            .as_mut()
            .unwrap()
            .bind_footprints(&p)
            .unwrap();
        initial.native_lot.as_mut().unwrap().membership =
            sims_vm::person_loading::RoomMembership::constructed(&initial.vm.world);
        initial.removals.push(Pending {
            target: 4,
            suppress_reset: suppress,
        });
        let mut deletion = Deletion::new(4);
        deletion.notified = true;
        initial.deletion.push(deletion);
        let mut outcomes = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut observed = 0;
            for checkpoint in 0..20 {
                let mut step = Step {
                    stop: crate::StepStop::Waiting,
                    trace: vec![],
                    events: vec![],
                };
                let mut remaining = budget;
                let done = s
                    .drive_deletion_person_reset(&p, &mut remaining, &mut step)
                    .unwrap();
                let e = s.deletion.last().unwrap().person_reset.as_ref().unwrap();
                if let Some(sims_vm::person_reset::Request::Script(thread)) = e.pending {
                    if matches!(s.vm.threads[&thread].status, Status::AwaitingRemoval(_)) {
                        let event = &native["events"][observed * 3];
                        assert_eq!(event["kind"], "remove");
                        assert_eq!(event["state"]["live"], !s.vm.threads[&thread].check);
                        assert_eq!(event["state"]["flags"], e.reset.native_flags());
                        assert_eq!(
                            event["state"]["neighbor68"],
                            s.vm.world.person.as_ref().unwrap().neighbors[&1].person[&68]
                        );
                        assert_eq!(s.removals, initial.removals);
                        observed += 1;
                    }
                }
                s = restore(&p, &s);
                if done {
                    break;
                }
                assert!(checkpoint < 19);
            }
            assert_eq!(observed, if budget == 1 { repeats } else { 0 });
            assert!(
                s.deletion
                    .last()
                    .unwrap()
                    .person_reset
                    .as_ref()
                    .unwrap()
                    .complete
            );
            assert_eq!(
                s.removals, initial.removals,
                "do not requeue or alter suppression"
            );
            assert_eq!(s.vm.world.objects[&4].temps[0], native["temps"][0]);
            assert_eq!(
                s.vm.world.person.as_ref().unwrap().neighbors[&1].person[&68],
                native["after"]["neighbor68"]
            );
            assert_eq!(
                s.deletion
                    .last()
                    .unwrap()
                    .person_reset
                    .as_ref()
                    .unwrap()
                    .reset
                    .native_flags(),
                native["flags"]
            );
            if bound {
                assert_eq!(
                    s.vm.world.objects[&4].native_flags.unwrap(),
                    native["flags"]
                );
            }
            assert_eq!(s.vm.tick, initial.vm.tick);
            let t = &s.vm.threads[&s.vm.object_threads[&4]];
            assert!(t.callback.is_none() && !t.check && t.status == Status::Ready);
            assert_eq!(t.stack.len(), 1);
            assert_eq!(t.stack[0].location.instruction, 0, "Main stays frozen");
            outcomes.push(s);
        }
        assert_eq!(outcomes[0], outcomes[1]);
    }
}

#[test]
fn deletion_base_callback_marks_another_object_and_resumes_nested_detach() {
    for flags in 0..=3 {
        let (mut p, mut initial) = setup(false);
        // Object 4 is already off-lot and queued; object 1 is still linked.
        // Its detach callback itself spans multiple instruction budgets.
        callback(&mut p, 10, 4106, 34);
        let m = p.modules.get_mut("objects").unwrap();
        m.object_functions.get_mut(&2).unwrap()[3].action = 4105;
        let mut routine = m.routines[&4096].clone();
        routine.body.instructions = vec![
            Instruction {
                opcode: 18,
                on_true: 1,
                on_false: 253,
                operands: [0, 0, flags, 0, 0, 0, 0, 0],
            },
            Instruction {
                opcode: 2,
                on_true: 2,
                on_false: 253,
                operands: [0, 0, 1, 0, 0, 5, 10, 7],
            },
            Instruction {
                opcode: 18,
                on_true: 3,
                on_false: 253,
                operands: [1, 0, flags, 0, 0, 0, 0, 0],
            },
            Instruction {
                opcode: 2,
                on_true: 254,
                on_false: 253,
                operands: [36, 0, 1, 0, 0, 3, 6, 7],
            },
        ];
        m.routines.insert(4105, routine);
        initial.removals.push(Pending {
            target: 4,
            suppress_reset: false,
        });
        let mut outcomes = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut suspended_detach = false;
            let mut completed = false;
            for _ in 0..100 {
                let mut step = Step {
                    stop: crate::StepStop::Waiting,
                    trace: vec![],
                    events: vec![],
                };
                let mut remaining = budget;
                let ready = s
                    .drive_native_removal(&p, &mut remaining, &mut step)
                    .unwrap();
                let done = ready && s.flush_removals(&p, &mut remaining, &mut step).unwrap();
                suspended_detach |= s.native_lot.as_ref().unwrap().removal.is_some();
                assert!(
                    step.trace.iter().all(|t| t.location.bhav != 4096),
                    "ordinary Main ran during deletion"
                );
                s = restore(&p, &s);
                if done {
                    completed = true;
                    break;
                }
            }
            assert!(completed);
            assert_eq!(suspended_detach, budget == 1);
            assert_eq!(
                s.vm.world.globals[&36], 1,
                "callback resumes once after nested marking"
            );
            assert_eq!(
                s.vm.world.globals[&34], 2,
                "mark detach and base Reset detach each run once"
            );
            assert!(!s.vm.world.objects.contains_key(&4) && !s.vm.world.objects.contains_key(&1));
            assert_eq!(s.vm.world.objects.len(), 2);
            assert!(s.removals.is_empty() && s.deletion.is_empty());
            assert_eq!(s.vm.tick, initial.vm.tick);
            outcomes.push(s);
        }
        assert_eq!(outcomes[0], outcomes[1]);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn live_group_deletion_matches_native_reset_order_and_restores_after_head_release() {
    let report: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/group-reset-deletion.json"
    ))
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 24);
    let mut cases = 0;
    let mut promoted_saves = 0;
    for row in report["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["input"]["argument"] == 0)
    {
        let c = &row["input"];
        let group = c["group"] == true;
        let selected = c["target"].as_i64().unwrap() as i16;
        let (mut p, mut initial) = setup(group);
        let mut detach = lot_placement::live::Execution::new_virtual(
            lot_placement::VirtualOperation::remove(
                &initial.vm.world,
                &initial.native_lot.as_ref().unwrap().placement.links,
                selected,
                true,
            )
            .unwrap(),
        );
        let mut step = Step {
            stop: crate::StepStop::Waiting,
            trace: vec![],
            events: vec![],
        };
        assert!(
            initial
                .drive_live_placement(&p, &mut detach, &mut 1000, &mut step)
                .unwrap()
        );
        // Observe each native virtual detach through its actual member
        // callbacks, and compare Reset's branch against the original trace.
        callback(&mut p, 10, 4100, 34);
        if c["reset_tree"] == true {
            callback(&mut p, 3, 4101, 36);
        }
        initial.removals.push(Pending {
            target: selected,
            suppress_reset: false,
        });
        let threads = initial.vm.object_threads.clone();
        let mut expected_detach = vec![];
        let mut expected_reset = vec![];
        let mut expected_deleted = vec![];
        for e in row["original"]["events"].as_array().unwrap() {
            let id = e["args"][0].as_i64().unwrap() as i16;
            match e["kind"].as_str().unwrap() {
                "detach" if group => {
                    let states = e["state"].as_array().unwrap();
                    let mut part = states.iter().find(|s| s["id"] == id).unwrap();
                    if let Some(head) = part["head"].as_i64() {
                        part = states.iter().find(|s| s["id"] == head).unwrap();
                    }
                    loop {
                        expected_detach.push(part["id"].as_i64().unwrap() as i16);
                        let Some(next) = part["next"].as_i64() else {
                            break;
                        };
                        part = states.iter().find(|s| s["id"] == next).unwrap();
                    }
                }
                "detach" => expected_detach.push(id),
                "reset_callback" => expected_reset.push(id),
                "registry_unlink" => {
                    expected_deleted.push(format!("deleted #{id} after lot object updates"))
                }
                _ => {}
            }
        }
        let mut outcomes = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut detaches = vec![];
            let mut resets = vec![];
            let mut deleted = vec![];
            let mut saves = 0;
            for checkpoint in 0..100 {
                let mut step = Step {
                    stop: crate::StepStop::Waiting,
                    trace: vec![],
                    events: vec![],
                };
                let mut remaining = budget;
                let done = s.flush_removals(&p, &mut remaining, &mut step).unwrap();
                for trace in &step.trace {
                    if trace.location.instruction == 0 {
                        let id = *threads.iter().find(|(_, t)| **t == trace.thread).unwrap().0;
                        match trace.location.bhav {
                            4100 => detaches.push(id),
                            4101 => resets.push(id),
                            _ => panic!("ordinary Main ran during destruction"),
                        }
                    }
                }
                deleted.extend(
                    step.events
                        .into_iter()
                        .filter(|e| e.detail.starts_with("deleted #"))
                        .map(|e| e.detail),
                );
                if !done {
                    saves += 1;
                    assert!(s.advance_fixture_tick().is_err());
                    let mut bad = Snapshot::capture(&p, &s);
                    bad.state.deletion.last_mut().unwrap().base_reset = None;
                    assert!(bad.restore(&p).is_err());
                    if group {
                        assert_eq!(s.deletion[0].group.as_ref().unwrap().marked, vec![1, 3, 2]);
                        let mut bad = Snapshot::capture(&p, &s);
                        bad.state.deletion[0].group.as_mut().unwrap().next = Some(999);
                        assert!(bad.restore(&p).is_err());
                        if !s.vm.world.objects.contains_key(&1) {
                            promoted_saves += 1;
                            let head = if s.vm.world.objects.contains_key(&3) {
                                3
                            } else {
                                2
                            };
                            assert_eq!(
                                s.vm.world.objects[&head].multitile.as_ref().unwrap().head,
                                None
                            );
                            assert_eq!(
                                s.vm.world.objects[&head].multitile.as_ref().unwrap().offset,
                                [10, 12, 0]
                            );
                        }
                    }
                }
                s = restore(&p, &s);
                if done {
                    break;
                }
                assert!(checkpoint < 99);
            }
            assert_eq!(saves > 0, budget == 1);
            assert_eq!(detaches, expected_detach, "{c}");
            assert_eq!(resets, expected_reset, "{c}");
            assert_eq!(deleted, expected_deleted, "{c}");
            assert!(s.removals.is_empty() && s.deletion.is_empty());
            assert_eq!(s.vm.tick, initial.vm.tick);
            outcomes.push(s);
        }
        assert_eq!(outcomes[0], outcomes[1]);
        cases += 1;
    }
    assert_eq!(cases, 12);
    assert!(promoted_saves > 0);
}

#[test]
fn group_removal_preserves_selected_self_yield_and_resolved_queue_identity() {
    for selected in [1, 2] {
        for self_remove in [false, true] {
            for dynamic in [false, true] {
                let (mut p, mut s) = setup(true);
                callback(&mut p, 10, 4100, 36);
                let m = p.modules.get_mut("objects").unwrap();
                if dynamic {
                    m.object_functions.get_mut(&1).unwrap()[8].action = 4096;
                }
                let caller = if self_remove { selected } else { 4 };
                let original = if self_remove { 1 } else { 2 };
                let mut words = m.object_definition_words[&original].clone();
                words[14] = 3;
                m.object_definition_words.insert(3, words);
                let mut functions = m.object_functions[&original].clone();
                functions[1].action = 4098;
                m.object_functions.insert(3, functions);
                let mut main = m.routines[&4096].clone();
                main.body.instructions = vec![
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
                        operands: [34, 0, 1, 0, 0, 3, 6, 7],
                    },
                ];
                m.routines.insert(4098, main);
                let d =
                    s.vm.world
                        .objects
                        .get_mut(&caller)
                        .unwrap()
                        .definition
                        .as_mut()
                        .unwrap();
                d.object = 3;
                d.guid = 3;
                s.architecture
                    .as_mut()
                    .unwrap()
                    .bind_footprints(&p)
                    .unwrap();
                s.vm = sims_vm::Vm::new(s.vm.world.clone(), 1);
                for id in 1..=4 {
                    s.vm.start_object_lifecycle(&p, id).unwrap();
                }
                let thread = s.vm.object_threads[&caller];
                s.vm.threads
                    .get_mut(&thread)
                    .unwrap()
                    .stack
                    .last_mut()
                    .unwrap()
                    .stack_object = selected;
                let expected = if dynamic { selected } else { 1 };
                let mut queued = false;
                for checkpoint in 0..100 {
                    let step = s.run_fixture_tick(&p, 1).unwrap();
                    assert_ne!(
                        step.stop,
                        crate::StepStop::Faulted,
                        "selected={selected}, self={self_remove}, dynamic={dynamic}, faults={:?}",
                        s.vm.threads
                            .iter()
                            .filter(|(_, t)| matches!(t.status, sims_vm::Status::Faulted(_)))
                            .map(|(id, t)| (id, &t.status))
                            .collect::<Vec<_>>()
                    );
                    if let Some(e) = s.native_lot.as_ref().unwrap().removal.as_ref() {
                        assert_eq!(e.target(), expected);
                        let crate::lot_removal::Origin::Primitive { request, .. } = &e.origin
                        else {
                            panic!()
                        };
                        assert_eq!(request.target, selected);
                    }
                    if let Some(r) = s.removals.first() {
                        queued = true;
                        assert_eq!(r.target, expected);
                    }
                    s = restore(&p, &s);
                    if step.stop != crate::StepStop::Budget {
                        break;
                    }
                    assert!(checkpoint < 99);
                }
                assert!(queued);
                assert_eq!(
                    s.vm.world.objects.keys().copied().collect::<Vec<_>>(),
                    vec![4]
                );
                assert_eq!(s.vm.world.globals[&34], if self_remove { 0 } else { 1 });
                assert_eq!(s.vm.world.globals[&36], 9);
            }
        }
    }
}
