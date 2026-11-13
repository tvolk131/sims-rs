use super::*;
use sims_formats::behavior::Instruction;

fn fixture(grouped: bool) -> (Program, Simulation, Definition) {
    let (mut p, mut s) = crate::lot_runtime::tests::fixture();
    let m = p.modules.get_mut("objects").unwrap();
    let mut words = m.object_definition_words[&1].clone();
    words[9] = 8;
    words[14] = 99;
    words[11] = -1;
    //Actual Cursors.iff's legacy160-byte OBJD has no +cc Animator flag.
    words[0] = 136;
    words[1] = 0;
    words.truncate(80);
    m.object_definition_words.insert(99, words);
    let mut functions = m.object_functions[&1].clone();
    functions[0].action = 4101;
    functions[1].action = 4102;
    m.object_functions.insert(99, functions);
    let mut routine = m.routines[&4096].clone();
    routine.body.instructions = vec![
        Instruction {
            opcode: 2,
            on_true: 1,
            on_false: 255,
            operands: [1, 0, 1, 0, 0, 3, 8, 7],
        },
        Instruction {
            opcode: 2,
            on_true: 254,
            on_false: 255,
            operands: [0, 0, 0, 0, 0, 5, 8, 8],
        },
    ];
    m.routines.insert(4101, routine.clone());
    m.routines.insert(4100, routine);
    m.object_functions.get_mut(&1).unwrap()[12].action = 4100;
    let mut main = m.routines[&4096].clone();
    main.body.args = 4;
    main.body.instructions = vec![
        Instruction {
            opcode: 2,
            on_true: 1,
            on_false: 255,
            operands: [0, 0, 7, 0, 0, 5, 9, 7],
        },
        Instruction {
            opcode: 0,
            on_true: 0,
            on_false: 1,
            operands: [0; 8],
        },
    ];
    m.routines.insert(4102, main);
    if grouped {
        let mut words = m.object_definition_words[&1].clone();
        words[10] = 1;
        m.object_definition_words.insert(2, words);
        m.object_functions.insert(2, m.object_functions[&1].clone());
        for id in [1, 3] {
            let o = s.vm.world.objects.get_mut(&id).unwrap();
            o.definition.as_mut().unwrap().object = 2;
            o.multitile = Some(sims_vm::multitile::Part {
                head: (id == 3).then_some(1),
                next: (id == 1).then_some(3),
                origin: [0; 3],
                offset: [0; 3],
            });
        }
    }
    s.architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&p)
        .unwrap();
    s.vm.world.scheduler_storage = Some(Default::default());
    s.vm.world.refresh = Some(Default::default());
    for id in 1..=4 {
        let mut o = s.vm.world.objects.remove(&id).unwrap();
        o.data.insert(43, 2);
        o.native_flags = Some(0x240);
        s.vm.world.register_scheduler_object(id, &mut o).unwrap();
        s.vm.world.objects.insert(id, o);
        s.vm.world.refresh.as_mut().unwrap().draw.insert(
            id,
            sims_vm::refresh::DrawState::constructed(s.vm.world.positions[&id].level),
        );
        if !s.vm.object_threads.contains_key(&id) {
            s.vm.start_object_lifecycle(&p, id).unwrap();
        }
    }
    (
        p,
        s,
        Definition {
            source: "objects".into(),
            object: 99,
            guid: 99,
            slots: 0,
        },
    )
}
fn cold(s: &Simulation, p: &Program) -> Simulation {
    let bytes = serde_json::to_vec(&crate::Snapshot::capture(p, s)).unwrap();
    serde_json::from_slice::<crate::Snapshot>(&bytes)
        .unwrap()
        .restore(p)
        .unwrap()
}

fn ready_to_place(grouped: bool, selected: ObjectId) -> (Program, Simulation) {
    let (mut p, mut s, d) = fixture(grouped);
    let m = p.modules.get_mut("objects").unwrap();
    for words in m.object_definition_words.values_mut() {
        words[18] = 500;
    }
    for functions in m.object_functions.values_mut() {
        functions[9].action = 4100;
    }
    if grouped {
        let mut master = m.object_definition_words[&2].clone();
        master[11] = -1;
        m.object_definition_words.insert(100, master);
        m.object_definition_order.push(100);
    }
    s.vm.world.globals.insert(32, 2);
    s.vm.world.globals.insert(15, 2);
    s.architecture
        .as_mut()
        .unwrap()
        .original
        .as_mut()
        .unwrap()
        .pool
        .fill(255);
    s.vm.world.budget = Some(sims_vm::budget::State {
        balance: 1000,
        ..Default::default()
    });
    for o in s.vm.world.objects.values_mut() {
        o.data.insert(4, 1);
        o.data.insert(42, 3);
    }
    assert!(
        s.begin_cursor_factory(&p, Some(d), selected, 1, false, Default::default())
            .unwrap()
    );
    for _ in 0..100 {
        s.advance_cursor_factory(&p, 1000).unwrap();
        if s.cursor_factory.as_ref().unwrap().placement().is_some() {
            break;
        }
    }
    assert!(s.cursor_factory.as_ref().unwrap().placement().is_some());
    // Test destination is an empty interior tile. The actual factory captures
    // the pre-pickup point; this changes only that caller input in the fixture.
    s.cursor_factory.as_mut().unwrap().position = [56, 56];
    for o in
        s.vm.world
            .objects
            .values_mut()
            .filter(|o| o.cursor.is_some())
    {
        o.data.insert(4, 1);
        o.data.insert(8, 20);
        o.data.insert(42, 3911);
    }
    (p, s)
}

#[test]
fn cursor_placement_keeps_furniture_before_cursor_and_cold_resumes_every_slice() {
    for (grouped, selected) in [(false, 1), (true, 3), (false, 0)] {
        let (p, initial) = ready_to_place(grouped, selected);
        let mut finals = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            s.begin_cursor_factory_placement(&p, false).unwrap();
            let mut furniture_first = false;
            for n in 0..2000 {
                s.advance_cursor_factory(&p, budget).unwrap();
                let f = s.cursor_factory.as_ref().unwrap();
                let cursor_id = id(&s.vm, f.selected_cursor.unwrap()).unwrap();
                if selected != 0
                    && s.vm.world.positions[&selected].x == 56
                    && s.vm.world.positions[&cursor_id].x < 0
                {
                    furniture_first = true;
                }
                let focus = f.pending_focus();
                if budget == 1 {
                    s = cold(&s, &p);
                }
                if focus.is_some() {
                    break;
                }
                assert!(n < 1999, "placement did not finish: {:?}", s.cursor_factory);
            }
            let f = s.cursor_factory.as_ref().unwrap();
            assert!(f.pending_focus().is_some());
            for &h in f.created() {
                let o = &s.vm.world.objects[&id(&s.vm, h).unwrap()];
                assert_eq!(o.cursor.as_ref().unwrap().position, [56, 56]);
                assert_eq!(o.cursor.as_ref().unwrap().valid, 1);
                assert_eq!(o.native_flags.unwrap() & 0x80, 0x80);
            }
            if selected != 0 {
                assert_eq!(s.vm.world.positions[&selected].x, 56);
            }
            if budget == 1 && selected != 0 {
                assert!(furniture_first);
            }
            assert_eq!(s.vm.world.budget, initial.vm.world.budget);
            assert_eq!(s.vm.tick, initial.vm.tick);
            finals.push(s);
        }
        assert_eq!(finals[0], finals[1]);
    }
}

#[test]
fn cursor_budget_failure_still_positions_furniture_but_invalidates_purchase() {
    for (purchase, mode, valid, error) in [(1, 2, 0, 21), (0, 2, 1, 0), (1, 1, 1, 0)] {
        let (p, mut s) = ready_to_place(false, 1);
        s.vm.world.budget.as_mut().unwrap().balance = 499;
        s.vm.world.globals.insert(32, mode);
        let cursor = id(
            &s.vm,
            s.cursor_factory.as_ref().unwrap().selected_cursor.unwrap(),
        )
        .unwrap();
        s.vm.world
            .objects
            .get_mut(&cursor)
            .unwrap()
            .cursor
            .as_mut()
            .unwrap()
            .purchase = purchase;
        s.begin_cursor_factory_placement(&p, false).unwrap();
        for _ in 0..100 {
            s.advance_cursor_factory(&p, 1000).unwrap();
            if s.cursor_factory.as_ref().unwrap().pending_focus().is_some() {
                break;
            }
        }
        assert!(s.cursor_factory.as_ref().unwrap().pending_focus().is_some());
        assert_eq!(s.vm.world.positions[&1].x, 56);
        assert_eq!(
            s.vm.world.objects[&cursor].cursor.as_ref().unwrap().valid,
            valid
        );
        assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, error);
        assert_eq!(s.vm.world.budget.as_ref().unwrap().balance, 499);
    }
}

#[test]
fn cursor_factory_returns_selected_part_only_after_focus_and_owned_notification() {
    for (grouped, selected) in [(false, 1), (true, 3), (false, 0)] {
        let (p, mut s) = ready_to_place(grouped, selected);
        s.begin_cursor_factory_placement(&p, false).unwrap();
        for _ in 0..100 {
            s.advance_cursor_factory(&p, 1000).unwrap();
            if s.cursor_factory.as_ref().unwrap().pending_focus().is_some() {
                break;
            }
        }
        let h = s.cursor_factory.as_ref().unwrap().pending_focus().unwrap();
        let native_id = id(&s.vm, h).unwrap();
        assert!(s.finish_cursor_factory(&p).is_err());
        assert!(s.start_cursor_created(h).is_err());
        assert!(s.complete_cursor_factory_focus(h + 1000).is_err());
        s.complete_cursor_factory_focus(h).unwrap();
        s = cold(&s, &p);
        assert_eq!(
            s.cursor_factory.as_ref().unwrap().published_focus(),
            Some(h)
        );
        assert!(s.complete_cursor_created(h).is_err());
        assert!(s.finish_cursor_factory(&p).is_err());
        s.start_cursor_created(h).unwrap();
        s = cold(&s, &p);
        assert!(s.start_cursor_created(h).is_err());
        let before = s.clone();
        s.advance_cursor_factory(&p, 1000).unwrap();
        assert_eq!(
            s, before,
            "the VM must not acknowledge its application child"
        );
        assert!(s.complete_cursor_created(h + 1000).is_err());
        s.complete_cursor_created(h).unwrap();
        s = cold(&s, &p);
        assert_eq!(s.cursor_factory.as_ref().unwrap().returned(), Some(h));
        assert_eq!(s.finish_cursor_factory(&p).unwrap(), native_id);
        assert!(s.cursor_factory.is_none());
        assert!(s.finish_cursor_factory(&p).is_err());
        s = cold(&s, &p);
        if selected != 0 {
            assert_eq!(
                s.vm.world.objects[&native_id]
                    .cursor
                    .as_ref()
                    .unwrap()
                    .underlying,
                Some(handle(&s.vm, selected).unwrap())
            );
        }
    }
}

fn returned_cursor(grouped: bool, selected: ObjectId) -> (Program, Simulation, ObjectId) {
    let (p, mut s) = ready_to_place(grouped, selected);
    s.begin_cursor_factory_placement(&p, false).unwrap();
    for _ in 0..100 {
        s.advance_cursor_factory(&p, 1000).unwrap();
        if s.cursor_factory.as_ref().unwrap().pending_focus().is_some() {
            break;
        }
    }
    let h = s.cursor_factory.as_ref().unwrap().pending_focus().unwrap();
    s.complete_cursor_factory_focus(h).unwrap();
    s.start_cursor_created(h).unwrap();
    s.complete_cursor_created(h).unwrap();
    let object = s.finish_cursor_factory(&p).unwrap();
    for o in s.vm.world.objects.values_mut() {
        o.data.insert(23, 2);
    }
    (p, s, object)
}

#[test]
fn move_tool_cursor_detach_preserves_order_and_cold_resumes_every_slice() {
    for (grouped, selected) in [(false, 1), (true, 3), (false, 0)] {
        let (p, initial, cursor) = returned_cursor(grouped, selected);
        let mut finals = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            s.native_lot.as_mut().unwrap().observers.clear();
            s.begin_cursor_detach(&p, cursor).unwrap();
            assert!(s.run_fixture_tick(&p, 10).is_err());
            assert!(s.finish_cursor_detach(&p).is_err());
            assert!(s.advance_cursor_rotation(&p, 1).is_err());
            assert!(s.finish_cursor_rotation(&p).is_err());
            let mut callbacks = false;
            for n in 0..3000 {
                s.advance_cursor_detach(&p, budget).unwrap();
                let callback = s.vm.threads.values().any(|t| t.callback.is_some());
                if callback && !callbacks {
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    bad.state.cursor_rotation = None;
                    assert!(bad.restore(&p).is_err());
                }
                callbacks |= callback;
                if budget == 1 {
                    s = cold(&s, &p);
                }
                if s.cursor_rotation.as_ref().unwrap().complete() {
                    break;
                }
                assert!(n < 2999, "move detach stalled");
            }
            assert_eq!(s.vm.tick, initial.vm.tick);
            assert_eq!(s.vm.rng_state, initial.vm.rng_state);
            assert_eq!(s.vm.world.budget, initial.vm.world.budget);
            assert_eq!(s.vm.world.positions[&cursor].x, -16);
            if selected != 0 {
                assert_eq!(s.vm.world.positions[&selected].x, -16);
                let effects = &s.native_lot.as_ref().unwrap().observers;
                let cursor_removed = effects
                    .iter()
                    .position(|e| *e == crate::lot_placement::Request::ViewRemoved(cursor))
                    .unwrap();
                let furniture_removed = effects
                    .iter()
                    .position(|e| *e == crate::lot_placement::Request::ViewRemoved(selected))
                    .unwrap();
                assert!(cursor_removed < furniture_removed);
            }
            s.finish_cursor_detach(&p).unwrap();
            finals.push(cold(&s, &p));
        }
        assert_eq!(finals[0], finals[1]);
    }
}

#[test]
fn cursor_destruction_releases_or_discards_original_furniture_and_cold_resumes_nested_deletion() {
    for (grouped, selected) in [(false, 0), (false, 1), (true, 1), (true, 3)] {
        for valid in [false, true] {
            let (mut p, mut initial, object) = returned_cursor(grouped, selected);
            p.modules
                .get_mut("objects")
                .unwrap()
                .object_definition_words
                .get_mut(&99)
                .unwrap()[9] = 9;
            initial.architecture.as_mut().unwrap().footprints.clear();
            initial
                .architecture
                .as_mut()
                .unwrap()
                .bind_footprints(&p)
                .unwrap();
            let chain = sims_vm::multitile::chain(&initial.vm.world, object).unwrap();
            for id in &chain {
                initial
                    .vm
                    .world
                    .objects
                    .get_mut(id)
                    .unwrap()
                    .cursor
                    .as_mut()
                    .unwrap()
                    .valid = i16::from(valid);
            }
            let furniture = if selected == 0 {
                vec![]
            } else if grouped {
                vec![1, 3]
            } else {
                vec![1]
            };
            let mut finals = vec![];
            for budget in [1, 1000] {
                let mut s = initial.clone();
                s.begin_synchronous_removal(&p, object).unwrap();
                let mut nested = false;
                let mut dangling = false;
                let mut focus_clears = 0;
                let mut step = Step {
                    stop: StepStop::Budget,
                    trace: vec![],
                    events: vec![],
                };
                for n in 0..6000 {
                    let mut remaining = budget;
                    let done = s
                        .drive_synchronous_removal(&p, &mut remaining, &mut step)
                        .unwrap();
                    nested |= s.deletion.iter().any(|d| furniture.contains(&d.target));
                    dangling |= s.vm.world.objects.values().any(|o| {
                        o.cursor
                            .as_ref()
                            .and_then(|c| c.underlying)
                            .is_some_and(|h| {
                                s.vm.world
                                    .scheduler_storage
                                    .as_ref()
                                    .unwrap()
                                    .retired_handle(h)
                            })
                    });
                    if s.pending_cursor_cleanup_focus() {
                        s.complete_cursor_cleanup_focus().unwrap();
                        focus_clears += 1;
                    }
                    if budget == 1 {
                        s = cold(&s, &p);
                    }
                    if done {
                        break;
                    }
                    assert!(n < 5999, "cursor destructor stalled: {:?}", s.deletion);
                }
                assert_eq!(s.take_synchronous_removal().unwrap(), object);
                for id in &chain {
                    assert!(!s.vm.world.objects.contains_key(id));
                }
                for id in &furniture {
                    assert_eq!(s.vm.world.objects.contains_key(id), valid);
                    if valid {
                        assert_eq!(s.vm.world.positions[id], initial.vm.world.positions[id]);
                    }
                }
                if budget == 1 && !valid && selected != 0 {
                    assert!(nested);
                    assert!(dangling);
                }
                assert_eq!(
                    focus_clears,
                    chain.len() - usize::from(valid && selected != 0)
                );
                assert_eq!(s.vm.tick, initial.vm.tick);
                assert_eq!(s.vm.rng_state, initial.vm.rng_state);
                assert_eq!(s.vm.world.budget, initial.vm.world.budget);
                finals.push(cold(&s, &p));
            }
            assert_eq!(
                finals[0], finals[1],
                "grouped={grouped},selected={selected},valid={valid}"
            );
        }
    }
}

#[test]
fn cursor_destruction_resumes_reset_callbacks_and_rejects_unowned_retired_pointers() {
    let (mut p, mut initial, object) = returned_cursor(true, 3);
    let m = p.modules.get_mut("objects").unwrap();
    m.object_definition_words.get_mut(&99).unwrap()[9] = 9;
    m.routines.insert(4103, m.routines[&4100].clone());
    for definition in [2, 99] {
        m.object_functions.get_mut(&definition).unwrap()[3].action = 4103;
    }
    initial.architecture.as_mut().unwrap().footprints.clear();
    initial
        .architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&p)
        .unwrap();
    initial.native_lot.as_mut().unwrap().observers.clear();
    let chain = sims_vm::multitile::chain(&initial.vm.world, object).unwrap();
    for id in &chain {
        initial
            .vm
            .world
            .objects
            .get_mut(id)
            .unwrap()
            .cursor
            .as_mut()
            .unwrap()
            .valid = 0;
    }
    let mut finals = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        s.begin_synchronous_removal(&p, object).unwrap();
        let mut callbacks = 0;
        let mut retired_checked = false;
        let mut callback_restored = false;
        for n in 0..6000 {
            let mut remaining = budget;
            let mut step = Step {
                stop: StepStop::Budget,
                trace: vec![],
                events: vec![],
            };
            let done = s
                .drive_synchronous_removal(&p, &mut remaining, &mut step)
                .unwrap();
            callbacks += step
                .trace
                .iter()
                .filter(|t| t.location.bhav == 4103 && t.location.instruction == 0)
                .count();
            if s.pending_cursor_cleanup_focus() {
                s.complete_cursor_cleanup_focus().unwrap();
            }
            let retired = s.deletion.iter().enumerate().find_map(|(i, d)| {
                d.cursor
                    .as_ref()
                    .filter(|e| !e.retired_references(&s, &p, i).unwrap().is_empty())
                    .map(|_| i)
            });
            if let Some(i) = retired {
                retired_checked = true;
                assert!(sims_vm::Snapshot::capture(&p, &s.vm).restore(&p).is_err());
                let mut bad = crate::Snapshot::capture(&p, &s);
                bad.state.deletion[i].cursor = None;
                assert!(bad.restore(&p).is_err());
                let mut bad = crate::Snapshot::capture(&p, &s);
                bad.state.deletion[i].entry_definition = None;
                assert!(bad.restore(&p).is_err());
                let mut bad = serde_json::to_value(crate::Snapshot::capture(&p, &s)).unwrap();
                bad["state"]["deletion"][i]["cursor"]["depth"] = serde_json::json!(i + 1);
                assert!(
                    serde_json::from_value::<crate::Snapshot>(bad)
                        .unwrap()
                        .restore(&p)
                        .is_err()
                );
            }
            if budget == 1 {
                callback_restored |= s.vm.threads.values().any(|t| t.callback.is_some());
                s = cold(&s, &p);
            }
            if done {
                break;
            }
            assert!(n < 5999, "nested cursor callback stalled");
        }
        assert_eq!(callbacks, 4, "two cursor Resets, two furniture Resets");
        if budget == 1 {
            assert!(retired_checked && callback_restored);
        }
        s.take_synchronous_removal().unwrap();
        assert_eq!(s.vm.tick, initial.vm.tick);
        assert_eq!(s.vm.rng_state, initial.vm.rng_state);
        assert_eq!(s.vm.world.budget, initial.vm.world.budget);
        finals.push(cold(&s, &p));
    }
    assert_eq!(finals[0], finals[1]);
}

#[test]
fn cursor_destruction_deferred_group_retains_its_queue_owner_across_repositioning() {
    let (mut p, mut initial, object) = returned_cursor(true, 3);
    p.modules
        .get_mut("objects")
        .unwrap()
        .object_definition_words
        .get_mut(&99)
        .unwrap()[9] = 9;
    initial.architecture.as_mut().unwrap().footprints.clear();
    initial
        .architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&p)
        .unwrap();
    let chain = sims_vm::multitile::chain(&initial.vm.world, object).unwrap();
    for id in &chain {
        initial
            .vm
            .world
            .objects
            .get_mut(id)
            .unwrap()
            .cursor
            .as_mut()
            .unwrap()
            .valid = 0;
    }
    let mut detach = crate::lot_removal::detach::Detach::new(
        &initial.vm.world,
        &initial.native_lot.as_ref().unwrap().placement.links,
        object,
    )
    .unwrap();
    let mut step = Step {
        stop: StepStop::Budget,
        trace: vec![],
        events: vec![],
    };
    assert!(
        detach
            .advance(&mut initial, &p, &mut 1000, &mut step)
            .unwrap()
    );
    initial.removals.push(sims_vm::removal::Pending {
        target: object,
        suppress_reset: false,
    });
    let mut finals = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        for n in 0..6000 {
            let mut remaining = budget;
            let done = s.flush_removals(&p, &mut remaining, &mut step).unwrap();
            if s.pending_cursor_cleanup_focus() {
                s.complete_cursor_cleanup_focus().unwrap();
            }
            s.sync_spatial_world();
            if budget == 1 {
                s = cold(&s, &p);
            }
            if done {
                break;
            }
            assert!(n < 5999, "deferred cursor destruction stalled");
        }
        assert!(s.deletion.is_empty() && s.removals.is_empty());
        for id in chain.iter().chain([1, 3].iter()) {
            assert!(!s.vm.world.objects.contains_key(id));
        }
        finals.push(cold(&s, &p));
    }
    assert_eq!(finals[0], finals[1]);
}

#[test]
fn cursor_forget_preserves_furniture_and_empty_selected_skips_other_parts() {
    for (grouped, selected, empty_selected) in [
        (false, 1, false),
        (true, 3, false),
        (true, 3, true),
        (false, 0, false),
    ] {
        let (p, mut initial, object) = returned_cursor(grouped, selected);
        if empty_selected {
            initial
                .vm
                .world
                .objects
                .get_mut(&object)
                .unwrap()
                .cursor
                .as_mut()
                .unwrap()
                .underlying = None;
        }
        let chain = sims_vm::multitile::chain(&initial.vm.world, object).unwrap();
        let mut finals = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            s.begin_cursor_cleanup(&p, object, cleanup::Kind::Forget)
                .unwrap();
            assert!(s.run_fixture_tick(&p, 1).is_err());
            assert!(s.begin_cursor_release(&p, object).is_err());
            assert!(s.finish_cursor_cleanup(&p).is_err());
            for n in 0..100 {
                let step = s.advance_cursor_cleanup(&p, budget).unwrap();
                assert!(step.trace.is_empty());
                if budget == 1 {
                    s = cold(&s, &p);
                }
                if s.cursor_cleanup.as_ref().unwrap().pending_focus_clear() {
                    break;
                }
                assert!(n < 99);
            }
            assert!(!s.cursor_cleanup.as_ref().unwrap().complete());
            s.complete_cursor_cleanup_focus().unwrap();
            s = cold(&s, &p);
            s.finish_cursor_cleanup(&p).unwrap();
            let mut expected = initial.clone();
            if !empty_selected {
                for id in &chain {
                    expected
                        .vm
                        .world
                        .objects
                        .get_mut(id)
                        .unwrap()
                        .cursor
                        .as_mut()
                        .unwrap()
                        .underlying = None;
                }
            }
            assert_eq!(
                s, expected,
                "42191c only clears pointers; it does not detach, restore flags, charge, or delete"
            );
            finals.push(s);
        }
        assert_eq!(finals[0], finals[1]);
    }
}

#[test]
fn cursor_discard_destroys_furniture_before_clearing_and_restores_every_slice() {
    for (grouped, selected) in [(false, 1), (true, 1), (true, 3)] {
        let (mut p, mut initial, object) = returned_cursor(grouped, selected);
        // Function3 yields a saved synchronous Reset boundary; it observes
        // the original cursor references before44123f has returned.
        let m = p.modules.get_mut("objects").unwrap();
        m.routines.insert(4103, m.routines[&4100].clone());
        m.object_functions
            .get_mut(&(if grouped { 2 } else { 1 }))
            .unwrap()[3]
            .action = 4103;
        initial.native_lot.as_mut().unwrap().observers.clear();
        let chain = sims_vm::multitile::chain(&initial.vm.world, object).unwrap();
        let furniture = if grouped { vec![1, 3] } else { vec![1] };
        let mut finals = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            s.begin_cursor_cleanup(&p, object, cleanup::Kind::Discard)
                .unwrap();
            let mut dangling = false;
            let mut callbacks = vec![];
            for n in 0..4000 {
                let step = s.advance_cursor_cleanup(&p, budget).unwrap();
                callbacks.extend(
                    step.trace
                        .iter()
                        .filter(|t| t.location.bhav == 4103 && t.location.instruction == 0)
                        .map(|t| t.thread),
                );
                let retired = s
                    .cursor_cleanup
                    .as_ref()
                    .unwrap()
                    .retired_references(&s, &p)
                    .unwrap();
                if !retired.is_empty() {
                    dangling = true;
                    assert!(sims_vm::Snapshot::capture(&p, &s.vm).restore(&p).is_err());
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    bad.state.cursor_cleanup = None;
                    assert!(bad.restore(&p).is_err());
                    let mut bad = serde_json::to_value(crate::Snapshot::capture(&p, &s)).unwrap();
                    bad["state"]["cursor_cleanup"]["references"][0]["furniture"] =
                        serde_json::json!(u32::MAX);
                    assert!(
                        serde_json::from_value::<crate::Snapshot>(bad)
                            .unwrap()
                            .restore(&p)
                            .is_err()
                    );
                }
                if budget == 1 {
                    s = cold(&s, &p);
                }
                if s.cursor_cleanup.as_ref().unwrap().pending_focus_clear() {
                    break;
                }
                assert!(n < 3999, "discard stalled: {:?}", s.cursor_cleanup);
            }
            assert_eq!(callbacks.len(), furniture.len());
            if budget == 1 {
                assert!(
                    dangling,
                    "the native delete-before-clear boundary must remain observable"
                );
            }
            s.complete_cursor_cleanup_focus().unwrap();
            s = cold(&s, &p);
            s.finish_cursor_cleanup(&p).unwrap();
            for id in &furniture {
                assert!(!s.vm.world.objects.contains_key(id));
            }
            for id in &chain {
                assert!(
                    s.vm.world.objects[id]
                        .cursor
                        .as_ref()
                        .unwrap()
                        .underlying
                        .is_none()
                );
                assert_eq!(s.vm.world.positions[id], initial.vm.world.positions[id]);
            }
            assert_eq!(s.vm.tick, initial.vm.tick);
            assert_eq!(s.vm.rng_state, initial.vm.rng_state);
            assert_eq!(s.vm.world.budget, initial.vm.world.budget);
            finals.push(s);
        }
        assert_eq!(finals[0], finals[1]);
    }
}

#[test]
fn cursor_discard_empty_receiver_skips_detach_delete_and_reposition() {
    let (p, initial, object) = returned_cursor(false, 0);
    let mut s = initial.clone();
    s.begin_cursor_cleanup(&p, object, cleanup::Kind::Discard)
        .unwrap();
    s.advance_cursor_cleanup(&p, 1000).unwrap();
    assert!(s.cursor_cleanup.as_ref().unwrap().pending_focus_clear());
    s.complete_cursor_cleanup_focus().unwrap();
    s.finish_cursor_cleanup(&p).unwrap();
    assert_eq!(s, initial);
}

#[test]
fn cursor_discard_captures_point_before_detach_and_skips_rejected_reposition() {
    for rejected in [false, true] {
        let (p, mut s, object) = returned_cursor(true, 3);
        let original = s.vm.world.positions[&object];
        if rejected {
            s.vm.world
                .objects
                .get_mut(&object)
                .unwrap()
                .cursor
                .as_mut()
                .unwrap()
                .position = [-100, -100];
        }
        s.begin_cursor_cleanup(&p, object, cleanup::Kind::Discard)
            .unwrap();
        s.advance_cursor_cleanup(&p, 1).unwrap();
        //Mutation after capture must not replace the caller's stack local.
        s.vm.world
            .objects
            .get_mut(&object)
            .unwrap()
            .cursor
            .as_mut()
            .unwrap()
            .position = [72, 72];
        for n in 0..4000 {
            s.advance_cursor_cleanup(&p, 1).unwrap();
            s = cold(&s, &p);
            if s.cursor_cleanup.as_ref().unwrap().pending_focus_clear() {
                break;
            }
            assert!(n < 3999);
        }
        s.complete_cursor_cleanup_focus().unwrap();
        s.finish_cursor_cleanup(&p).unwrap();
        assert!(!s.vm.world.objects.contains_key(&1));
        assert!(!s.vm.world.objects.contains_key(&3));
        if rejected {
            assert_eq!(s.vm.world.positions[&object].x, -16);
        } else {
            assert_eq!(s.vm.world.positions[&object], original);
        }
    }
}

#[test]
fn immediate_group_removal_dispatches_all_parts_before_deletion_marks() {
    let (p, initial, _) = fixture(true);
    let mut finals = vec![];
    for budget in [1, 1000] {
        let mut s = initial.clone();
        s.begin_synchronous_removal(&p, 3).unwrap();
        let mut saw_mark = false;
        let mut step = Step {
            stop: StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        for n in 0..4000 {
            let mut remaining = budget;
            let done = s
                .drive_synchronous_removal(&p, &mut remaining, &mut step)
                .unwrap();
            if s.vm
                .world
                .objects
                .get(&1)
                .is_some_and(|o| o.native_flags.unwrap() & 0x40 != 0)
            {
                saw_mark = true;
                assert!(
                    [1, 3]
                        .iter()
                        .all(|id| s.vm.world.positions.get(id).is_none_or(|q| q.x == -16))
                );
                let removed: Vec<_> = s
                    .native_lot
                    .as_ref()
                    .unwrap()
                    .observers
                    .iter()
                    .filter_map(|r| {
                        if let crate::lot_placement::Request::ViewRemoved(id) = r {
                            Some(*id)
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(&removed[..2], &[1, 3]);
            }
            if budget == 1 {
                s = cold(&s, &p);
            }
            if done {
                break;
            }
            assert!(n < 3999);
        }
        if budget == 1 {
            assert!(saw_mark);
        }
        assert_eq!(s.take_synchronous_removal().unwrap(), 3);
        assert!(!s.vm.world.objects.contains_key(&1));
        assert!(!s.vm.world.objects.contains_key(&3));
        finals.push(s);
    }
    assert_eq!(finals[0], finals[1]);
}

#[test]
fn cursor_discard_reads_furniture_pointer_again_after_virtual_detach() {
    let (p, mut s, object) = returned_cursor(false, 1);
    s.begin_cursor_cleanup(&p, object, cleanup::Kind::Discard)
        .unwrap();
    for n in 0..4000 {
        s.advance_cursor_cleanup(&p, 1).unwrap();
        s = cold(&s, &p);
        let owner = serde_json::to_value(s.cursor_cleanup.as_ref().unwrap()).unwrap();
        if owner["phase"] == "Delete" && owner["removal"].is_null() {
            break;
        }
        assert!(n < 3999);
    }
    assert_eq!(s.vm.world.positions[&1].x, -16);
    let replacement = handle(&s.vm, 3).unwrap();
    s.vm.world
        .objects
        .get_mut(&object)
        .unwrap()
        .cursor
        .as_mut()
        .unwrap()
        .underlying = Some(replacement);
    for n in 0..4000 {
        s.advance_cursor_cleanup(&p, 1).unwrap();
        s = cold(&s, &p);
        if s.cursor_cleanup.as_ref().unwrap().pending_focus_clear() {
            break;
        }
        assert!(n < 3999);
    }
    s.complete_cursor_cleanup_focus().unwrap();
    s.finish_cursor_cleanup(&p).unwrap();
    assert!(s.vm.world.objects.contains_key(&1));
    assert!(!s.vm.world.objects.contains_key(&3));
    assert_eq!(s.vm.world.positions[&1].x, -16);
}

#[test]
fn cursor_release_places_original_receiver_after_clearing_all_parts_and_cold_resumes() {
    for (grouped, selected) in [(false, 1), (true, 1), (true, 3)] {
        let (mut p, initial, cursor) = returned_cursor(grouped, selected);
        let m = p.modules.get_mut("objects").unwrap();
        m.routines.insert(4103, m.routines[&4100].clone());
        for f in m.object_functions.values_mut() {
            f[11].action = 4103;
        }
        let cursors = sims_vm::multitile::chain(&initial.vm.world, cursor).unwrap();
        let furniture = if grouped { vec![1, 3] } else { vec![1] };
        let mut finals = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            s.native_lot.as_mut().unwrap().observers.clear();
            s.begin_cursor_release(&p, cursor).unwrap();
            assert!(s.run_fixture_tick(&p, 10).is_err());
            assert!(s.begin_cursor_rotation(&p, cursor, 1, false).is_err());
            assert!(s.finish_cursor_release(&p).is_err());
            let mut callbacks = vec![];
            let mut live = false;
            for n in 0..2000 {
                let step = s.advance_cursor_release(&p, budget).unwrap();
                for t in step
                    .trace
                    .iter()
                    .filter(|t| t.location.bhav == 4103 && t.location.instruction == 0)
                {
                    assert!(cursors.iter().all(|id| {
                        s.vm.world.objects[id]
                            .cursor
                            .as_ref()
                            .unwrap()
                            .underlying
                            .is_none()
                    }));
                    callbacks.push(
                        *s.vm
                            .object_threads
                            .iter()
                            .find(|(_, v)| **v == t.thread)
                            .unwrap()
                            .0,
                    );
                }
                if s.vm.threads.values().any(|t| t.callback.is_some()) {
                    live = true;
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    bad.state.cursor_release = None;
                    assert!(bad.restore(&p).is_err());
                }
                if budget == 1 {
                    s = cold(&s, &p);
                }
                if s.cursor_release.as_ref().unwrap().outcome().is_some() {
                    break;
                }
                assert!(n < 1999, "release stalled");
            }
            assert_eq!(callbacks, furniture);
            if budget == 1 {
                assert!(live);
            }
            assert!(s.finish_cursor_release(&p).unwrap());
            for &id in &cursors {
                assert_eq!(
                    s.vm.world.objects[&id].cursor.as_ref().unwrap().underlying,
                    None
                );
                assert_eq!(s.vm.world.positions[&id].x, -16);
                assert!(
                    s.vm.world.objects.contains_key(&id),
                    "destruction belongs to the caller"
                );
            }
            for &id in &furniture {
                assert_eq!(s.vm.world.positions[&id], initial.vm.world.positions[&id]);
                assert_eq!(s.vm.world.objects[&id].native_flags.unwrap() & 0x1f, 0);
                assert_eq!(s.vm.world.objects[&id].idle.as_ref().unwrap().flags & 1, 1);
            }
            let events = &s.native_lot.as_ref().unwrap().observers;
            let cursor_removed = events
                .iter()
                .position(|r| *r == crate::lot_placement::Request::ViewRemoved(cursor))
                .unwrap();
            let furniture_removed = events
                .iter()
                .position(|r| *r == crate::lot_placement::Request::ViewRemoved(selected))
                .unwrap();
            assert!(cursor_removed < furniture_removed);
            assert_eq!(s.vm.tick, initial.vm.tick);
            assert_eq!(s.vm.rng_state, initial.vm.rng_state);
            assert_eq!(s.vm.world.budget, initial.vm.world.budget);
            finals.push(cold(&s, &p));
        }
        assert_eq!(finals[0], finals[1]);
    }
}

#[test]
fn cursor_release_captures_selected_point_before_detach_and_preserves_later_cache_writes() {
    let (p, mut s, cursor) = returned_cursor(true, 3);
    let initial = s.vm.world.positions.clone();
    s.begin_cursor_release(&p, cursor).unwrap();
    s.advance_cursor_release(&p, 1).unwrap();
    // Supplied observer mutation after native421a0e captured its arguments.
    let c =
        s.vm.world
            .objects
            .get_mut(&cursor)
            .unwrap()
            .cursor
            .as_mut()
            .unwrap();
    c.position = [72, 88];
    c.level = 2;
    s = cold(&s, &p);
    for n in 0..1000 {
        s.advance_cursor_release(&p, 1).unwrap();
        s = cold(&s, &p);
        if s.cursor_release.as_ref().unwrap().outcome().is_some() {
            break;
        }
        assert!(n < 999);
    }
    assert!(s.finish_cursor_release(&p).unwrap());
    for id in [1, 3] {
        assert_eq!(s.vm.world.positions[&id], initial[&id]);
    }
    let c = s.vm.world.objects[&cursor].cursor.as_ref().unwrap();
    assert_eq!((c.position, c.level), ([72, 88], 2));
}

#[test]
fn cursor_release_rejects_empty_or_invalid_without_touching_furniture_or_price() {
    for selected in [0, 1, 3] {
        let (mut p, mut s, cursor) = returned_cursor(selected == 3, selected);
        if selected != 0 {
            s.vm.world
                .objects
                .get_mut(&cursor)
                .unwrap()
                .cursor
                .as_mut()
                .unwrap()
                .valid = 0;
        }
        // A rejected release must not need the catalog price at all.
        p.modules
            .get_mut("objects")
            .unwrap()
            .object_definition_order
            .clear();
        let before = s.clone();
        s.begin_cursor_release(&p, cursor).unwrap();
        let step = s.advance_cursor_release(&p, 1).unwrap();
        assert!(step.trace.is_empty());
        assert!(!s.finish_cursor_release(&p).unwrap());
        assert_eq!(s, before);
    }
}

#[test]
fn cursor_release_notifies_contained_objects_in_forward_slot_order_then_dynamic_groups() {
    use crate::{
        lot_links::Descriptor,
        lot_placement::{VirtualOperation, live::Execution},
    };
    for grouped in [false, true] {
        let (mut p, mut initial, cursor) = returned_cursor(grouped, if grouped { 3 } else { 1 });
        if !grouped {
            container_slots(&mut p, &mut initial, 1, &[4, 4]);
            for (child, slot) in [(3, 0), (4, 1)] {
                let op = VirtualOperation::remove(
                    &initial.vm.world,
                    &initial.native_lot.as_ref().unwrap().placement.links,
                    child,
                    true,
                )
                .unwrap();
                let mut e = Execution::new_virtual(op);
                let mut step = Step {
                    stop: StepStop::Budget,
                    trace: vec![],
                    events: vec![],
                };
                assert!(
                    initial
                        .drive_removal_placement(&p, &mut e, &mut 1000, &mut step)
                        .unwrap()
                );
                let op = VirtualOperation::set(
                    &initial.vm.world,
                    &initial.native_lot.as_ref().unwrap().placement.links,
                    child,
                    Descriptor {
                        on_lot: true,
                        position: [56, 56],
                        level: 1,
                        container: Some(1),
                        slot,
                    },
                    true,
                )
                .unwrap();
                let mut e = Execution::new_virtual(op);
                assert!(
                    initial
                        .drive_removal_placement(&p, &mut e, &mut 1000, &mut step)
                        .unwrap()
                );
            }
        }
        let m = p.modules.get_mut("objects").unwrap();
        let mut routine = m.routines[&4100].clone();
        // Both callback return values are ignored by the original wrappers.
        routine.body.instructions.last_mut().unwrap().on_true = 255;
        m.routines.insert(4103, routine.clone());
        routine.body.args = 4;
        m.routines.insert(4104, routine);
        for f in m.object_functions.values_mut() {
            f[11].action = 4103;
            if grouped {
                f[8].action = 4104;
            }
        }
        initial = cold(&initial, &p);
        let mut finals = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            s.begin_cursor_release(&p, cursor).unwrap();
            let mut calls = vec![];
            for n in 0..2000 {
                let step = s.advance_cursor_release(&p, budget).unwrap();
                for t in step.trace.iter().filter(|t| {
                    [4103, 4104].contains(&t.location.bhav) && t.location.instruction == 0
                }) {
                    let object = *s
                        .vm
                        .object_threads
                        .iter()
                        .find(|(_, v)| **v == t.thread)
                        .unwrap()
                        .0;
                    calls.push((t.location.bhav, object));
                }
                if budget == 1 {
                    s = cold(&s, &p);
                }
                if s.cursor_release.as_ref().unwrap().outcome().is_some() {
                    break;
                }
                assert!(n < 1999);
            }
            assert!(s.finish_cursor_release(&p).unwrap());
            assert_eq!(
                calls,
                if grouped {
                    vec![(4103, 1), (4103, 3), (4104, 1), (4104, 3)]
                } else {
                    vec![(4103, 1), (4103, 3), (4103, 4)]
                }
            );
            if !grouped {
                assert_eq!(s.vm.world.objects[&1].slots, vec![Some(3), Some(4)]);
            }
            finals.push(s);
        }
        assert_eq!(finals[0], finals[1]);
    }
}

#[test]
fn cursor_notch_rotation_preserves_native_detach_order_and_cold_continuations() {
    for (grouped, selected) in [(false, 1), (true, 3), (false, 0)] {
        let (p, initial, cursor) = returned_cursor(grouped, selected);
        for notches in [-3, -1, 0, 1, 4, 256] {
            let mut finals = vec![];
            for budget in [1, 1000] {
                let mut s = initial.clone();
                s.native_lot.as_mut().unwrap().observers.clear();
                s.begin_cursor_rotation(&p, cursor, notches, false).unwrap();
                assert!(s.run_fixture_tick(&p, 10).is_err());
                assert!(s.finish_cursor_rotation(&p).is_err());
                let mut callbacks = false;
                for n in 0..3000 {
                    s.advance_cursor_rotation(&p, budget).unwrap();
                    let live_callback = s.vm.threads.values().any(|t| t.callback.is_some());
                    if budget == 1 && live_callback && !callbacks {
                        let mut bad = crate::Snapshot::capture(&p, &s);
                        bad.state.cursor_rotation = None;
                        assert!(
                            bad.restore(&p).is_err(),
                            "rotation callback must retain its synchronous owner"
                        );
                    }
                    callbacks |= live_callback;
                    if budget == 1 {
                        s = cold(&s, &p);
                    }
                    if s.cursor_rotation.as_ref().unwrap().complete() {
                        break;
                    }
                    assert!(n < 2999, "rotation stalled");
                }
                let target = if selected == 0 { cursor } else { selected };
                assert_eq!(
                    s.vm.world.objects[&target].data[&1],
                    (2 * notches) as i16 & 7
                );
                assert_eq!(s.vm.tick, initial.vm.tick);
                assert_eq!(s.vm.rng_state, initial.vm.rng_state);
                assert_eq!(s.vm.world.budget, initial.vm.world.budget);
                let effects = &s.native_lot.as_ref().unwrap().observers;
                let cursor_removed = effects
                    .iter()
                    .position(|e| *e == crate::lot_placement::Request::ViewRemoved(cursor))
                    .unwrap();
                if selected != 0 {
                    let target_removed = effects
                        .iter()
                        .position(|e| *e == crate::lot_placement::Request::ViewRemoved(selected))
                        .unwrap();
                    assert!(cursor_removed < target_removed);
                    assert_eq!(s.vm.world.positions[&selected].x, 56);
                    assert_eq!(s.vm.world.positions[&cursor].x, 56);
                } else if (2 * notches) & 7 == 0 {
                    assert_eq!(
                        s.vm.world.positions[&cursor].x, -16,
                        "zero-delta group call still detaches"
                    );
                }
                if budget == 1 && selected != 0 {
                    assert!(callbacks);
                }
                s.finish_cursor_rotation(&p).unwrap();
                s = cold(&s, &p);
                finals.push(s);
            }
            assert_eq!(finals[0], finals[1]);
        }
    }
}

#[test]
fn cursor_orientation_uses_real_rotation_children_and_restores_every_slice() {
    for (grouped, selected) in [(false, 1), (true, 3), (false, 0)] {
        let (p, initial, cursor) = returned_cursor(grouped, selected);
        for direction in [0, 2, 6, 1, 65536] {
            let mut finals = vec![];
            for budget in [1, 1000] {
                let mut s = initial.clone();
                s.begin_cursor_orientation(&p, cursor, direction, false)
                    .unwrap();
                assert!(s.run_fixture_tick(&p, 10).is_err());
                assert!(s.finish_cursor_orientation(&p).is_err());
                assert!(s.advance_cursor_rotation(&p, 1).is_err());
                assert!(s.finish_cursor_rotation(&p).is_err());
                let mut callback = false;
                for n in 0..10000 {
                    s.advance_cursor_orientation(&p, budget).unwrap();
                    let live = s.vm.threads.values().any(|t| t.callback.is_some());
                    if live && !callback {
                        let mut bad = crate::Snapshot::capture(&p, &s);
                        bad.state.cursor_rotation = None;
                        assert!(bad.restore(&p).is_err());
                    }
                    callback |= live;
                    if budget == 1 {
                        s = cold(&s, &p);
                    }
                    if s.cursor_rotation.as_ref().unwrap().complete() {
                        break;
                    }
                    assert!(n < 9999, "orientation stalled");
                }
                s.finish_cursor_orientation(&p).unwrap();
                let target = if selected == 0 { cursor } else { selected };
                assert_eq!(
                    s.vm.world.objects[&target].data[&1],
                    if direction % 2 == 0 && direction < 8 {
                        direction as i16
                    } else {
                        0
                    }
                );
                assert_eq!(s.vm.tick, initial.vm.tick);
                assert_eq!(s.vm.rng_state, initial.vm.rng_state);
                assert_eq!(s.vm.world.budget, initial.vm.world.budget);
                if budget == 1 && selected != 0 && direction != 0 {
                    assert!(callback);
                }
                if direction == 0 {
                    assert_eq!(s, initial);
                }
                finals.push(cold(&s, &p));
            }
            assert_eq!(finals[0], finals[1]);
        }
    }
}

#[test]
fn empty_cursor_group_rotation_rolls_back_direction_after_failed_onlot_check() {
    let (p, mut s, cursor) = returned_cursor(false, 0);
    s.vm.world
        .objects
        .get_mut(&cursor)
        .unwrap()
        .data
        .insert(4, 0);
    s.begin_cursor_rotation(&p, cursor, 1, false).unwrap();
    for n in 0..1000 {
        s.advance_cursor_rotation(&p, 1).unwrap();
        s = cold(&s, &p);
        if s.cursor_rotation.as_ref().unwrap().complete() {
            break;
        }
        assert!(n < 999);
    }
    assert_eq!(s.vm.world.objects[&cursor].data[&1], 0);
    assert_eq!(s.vm.world.positions[&cursor].x, -16);
    assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, 24);
    //The fallback is the actual group method, with no invented validity write.
    assert_eq!(
        s.vm.world.objects[&cursor].cursor.as_ref().unwrap().valid,
        1
    );
    s.finish_cursor_rotation(&p).unwrap();
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_base_notch_calls_match_captured_original_observer_histories() {
    use crate::lot_placement::{Operation, Request};
    use serde_json::{Value, json};
    let report: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/creation-rotation.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 906);
    let mut calls = 0;
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let (mut p, mut s, _) = fixture(false);
        s.vm.world.objects.get_mut(&4).unwrap().data.extend([
            (1, c["initial"].as_i64().unwrap() as i16),
            (23, c["step"].as_i64().unwrap() as i16),
        ]);
        s.vm.world.positions.insert(
            4,
            sims_vm::autonomy::Placement {
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
        let mut events = vec![];
        //The captured wrapper supplies the call sequence. This comparison
        //exercises the new single-call433f75 implementation independently of
        //the existing Rust Create Object direction-copy wrapper.
        for turn in row["original"]["turns"].as_array().unwrap() {
            calls += 1;
            assert_eq!(json!(s.vm.world.objects[&4].data[&1]), *turn);
            let mut op = Operation::rotate(4, 1);
            while let Some(e) = op
                .next(
                    &mut s.vm.world,
                    &p,
                    s.architecture.as_ref().unwrap(),
                    &mut s.native_lot.as_mut().unwrap().placement,
                )
                .unwrap()
            {
                let direction = s.vm.world.objects[&4].data[&1];
                events.push(match e {
                    Request::Invalidate {
                        object: 4,
                        children_only: false,
                    } => json!({"kind":"invalidate","direction":direction}),
                    Request::Adjacency(tile) => {
                        json!({"kind":"adjacency","direction":direction,"tile":tile})
                    }
                    Request::Walls(4) => json!({"kind":"walls","direction":direction}),
                    _ => panic!("unexpected notch event {e:?}"),
                });
                if c["mutate"] == true
                    && matches!(e, Request::Adjacency(_))
                    && events.iter().filter(|e| e["kind"] == "adjacency").count() == 1
                {
                    s.vm.world.objects.get_mut(&4).unwrap().data.insert(1, 5);
                    s.vm.world.positions.insert(
                        4,
                        sims_vm::autonomy::Placement {
                            x: 40,
                            y: 56,
                            level: 1,
                        },
                    );
                }
                op = serde_json::from_slice(&serde_json::to_vec(&op).unwrap()).unwrap();
            }
        }
        assert_eq!(json!(events), row["original"]["events"], "{c}");
        assert_eq!(
            json!(s.vm.world.objects[&4].data[&1]),
            row["original"]["direction"],
            "{c}"
        );
    }
    eprintln!("{calls} original base Rotate calls across906 captured wrapper histories");
}

#[test]
fn cursor_rotates_contained_children_in_reverse_slot_order_with_native_filter() {
    use crate::{
        lot_links::Descriptor,
        lot_placement::{Request, VirtualOperation, live::Execution},
    };
    let (mut p, mut initial, cursor) = returned_cursor(false, 1);
    container_slots(&mut p, &mut initial, 1, &[4, 4]);
    for (child, slot) in [(3, 0), (4, 1)] {
        let links = &initial.native_lot.as_ref().unwrap().placement.links;
        let remove = VirtualOperation::remove(&initial.vm.world, links, child, true).unwrap();
        {
            let mut e = Execution::new_virtual(remove);
            let mut step = Step {
                stop: StepStop::Budget,
                trace: vec![],
                events: vec![],
            };
            assert!(
                initial
                    .drive_removal_placement(&p, &mut e, &mut 1000, &mut step)
                    .unwrap()
            );
        }
        let op = VirtualOperation::set(
            &initial.vm.world,
            &initial.native_lot.as_ref().unwrap().placement.links,
            child,
            Descriptor {
                on_lot: true,
                position: [56, 56],
                level: 1,
                container: Some(1),
                slot,
            },
            true,
        )
        .unwrap();
        let mut e = Execution::new_virtual(op);
        let mut step = Step {
            stop: StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        assert!(
            initial
                .drive_removal_placement(&p, &mut e, &mut 1000, &mut step)
                .unwrap()
        );
    }
    initial = cold(&initial, &p);
    for upper_step in [1, 2] {
        let mut s = initial.clone();
        s.vm.world
            .objects
            .get_mut(&4)
            .unwrap()
            .data
            .insert(23, upper_step);
        s.native_lot.as_mut().unwrap().observers.clear();
        s.begin_cursor_rotation(&p, cursor, 1, false).unwrap();
        for n in 0..1000 {
            s.advance_cursor_rotation(&p, 1).unwrap();
            s = cold(&s, &p);
            if s.cursor_rotation.as_ref().unwrap().complete() {
                break;
            }
            assert!(n < 999);
        }
        assert_eq!(s.vm.world.objects[&3].data[&1], 2);
        assert_eq!(
            s.vm.world.objects[&4].data[&1],
            if upper_step == 2 { 2 } else { 0 }
        );
        let effects = &s.native_lot.as_ref().unwrap().observers;
        let low = effects
            .iter()
            .position(|r| {
                *r == Request::Invalidate {
                    object: 3,
                    children_only: false,
                }
            })
            .unwrap();
        let high = effects.iter().position(|r| {
            *r == Request::Invalidate {
                object: 4,
                children_only: false,
            }
        });
        if upper_step == 2 {
            assert!(high.unwrap() < low);
        } else {
            assert!(high.is_none());
        }
        assert_eq!(s.vm.world.objects[&1].slots, vec![Some(3), Some(4)]);
        s.finish_cursor_rotation(&p).unwrap();
    }
}

#[test]
fn cursor_can_position_while_furniture_buy_restrictions_reject() {
    for cheat in [false, true] {
        let (p, mut s) = ready_to_place(false, 1);
        s.native_lot.as_mut().unwrap().placement.flags[0][3 * 6 + 3] |= 0x20;
        s.begin_cursor_factory_placement(&p, cheat).unwrap();
        for _ in 0..100 {
            s.advance_cursor_factory(&p, 1).unwrap();
            s = cold(&s, &p);
            if s.cursor_factory.as_ref().unwrap().pending_focus().is_some() {
                break;
            }
        }
        let f = s.cursor_factory.as_ref().unwrap();
        assert!(f.pending_focus().is_some());
        let cursor = id(&s.vm, f.selected_cursor.unwrap()).unwrap();
        assert_eq!(s.vm.world.positions[&cursor].x, 56);
        assert_eq!(
            s.vm.world.objects[&cursor].cursor.as_ref().unwrap().valid,
            i16::from(cheat)
        );
        assert_eq!(s.vm.world.positions[&1].x, if cheat { 56 } else { -16 });
        assert_eq!(
            s.native_lot.as_ref().unwrap().diagnostics.error,
            if cheat { 0 } else { 1 }
        );
    }
}

#[test]
fn buy_acceptance_checks_signed_tile_bytes_and_centering_after_virtual_check() {
    for (point, accepted, error) in [
        ([56, 56], true, 0),
        ([55, 56], false, 0),
        ([8, 56], false, 1),
        ([2056, 56], false, 1),
    ] {
        let (p, mut s) = ready_to_place(false, 1);
        let q = acceptance::Query {
            object: handle(&s.vm, 1).unwrap(),
            position: point,
            level: 1,
            container: None,
        };
        let mut c = acceptance::Check::new(&s, q, false).unwrap();
        let mut step = Step {
            stop: StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        for _ in 0..100 {
            c.advance(&mut s, &p, &mut 1, &mut step).unwrap();
            c.validate(&s.vm, &p).unwrap();
            c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
            if c.outcome().is_some() {
                break;
            }
        }
        assert_eq!(c.outcome(), Some(accepted), "point={point:?}");
        assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, error);
    }
}

fn container_slots(p: &mut Program, s: &mut Simulation, parent: ObjectId, heights: &[i32]) {
    let entries = heights
        .iter()
        .map(|&height| sims_formats::execution::Slot {
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
            height,
            facing: -2,
            resolution: 16,
        })
        .collect();
    p.modules.get_mut("objects").unwrap().slots.insert(
        1,
        sims_formats::execution::Slots {
            version: 10,
            entries,
        },
    );
    let o = s.vm.world.objects.get_mut(&parent).unwrap();
    o.definition.as_mut().unwrap().slots = 1;
    o.slots = vec![None; heights.len()];
}

#[test]
fn buy_containment_retains_original_point_centering_and_height_restrictions() {
    for (height, cheat, centered, accepted, error) in [
        (4, false, true, true, 0),
        (5, false, true, false, 11),
        (7, false, true, false, 11),
        (8, false, true, true, 0),
        (5, true, true, true, 0),
        (4, false, false, false, 0),
    ] {
        let (mut p, mut s) = ready_to_place(false, 1);
        container_slots(&mut p, &mut s, 3, &[height]);
        s.vm.world.objects.get_mut(&1).unwrap().data.insert(4, -1);
        let q = acceptance::Query {
            object: handle(&s.vm, 1).unwrap(),
            position: [if centered { 56 } else { 55 }, 56],
            level: 1,
            container: Some((3, 0)),
        };
        let mut c = acceptance::Check::new(&s, q, cheat).unwrap();
        let mut step = Step {
            stop: StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        for _ in 0..100 {
            c.advance(&mut s, &p, &mut 1, &mut step).unwrap();
            c.validate(&s.vm, &p).unwrap();
            c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
            if c.outcome().is_some() {
                break;
            }
        }
        assert_eq!(
            c.outcome(),
            Some(accepted),
            "height={height}, cheat={cheat}, centered={centered}"
        );
        assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, error);
    }
}

#[test]
fn cursor_slot_search_preserves_candidate_order_and_runs_real_rotation() {
    for direction in [0, 6] {
        let (mut p, mut s) = ready_to_place(false, 1);
        container_slots(&mut p, &mut s, 3, &[5, 4]);
        s.native_lot
            .as_mut()
            .unwrap()
            .placement
            .bounds
            .insert(3, [48, 48, 32, 32]);
        s.vm.world
            .objects
            .get_mut(&3)
            .unwrap()
            .data
            .insert(1, direction);
        s.vm.world.objects.get_mut(&1).unwrap().data.insert(4, 8);
        s.vm.world.objects.get_mut(&1).unwrap().data.insert(23, 2);
        s.cursor_factory.as_mut().unwrap().position = [40, 40];
        s.begin_cursor_factory_placement(&p, false).unwrap();
        for n in 0..1000 {
            s.advance_cursor_factory(&p, 1).unwrap();
            s = cold(&s, &p);
            if s.cursor_factory
                .as_ref()
                .unwrap()
                .placement_request()
                .is_some()
            {
                break;
            }
            assert!(
                n < 999,
                "slot search did not reach rotation: {:?}",
                s.cursor_factory
            );
        }
        let f = s.cursor_factory.as_ref().unwrap();
        let h = f.selected_cursor.unwrap();
        assert_eq!(
            f.placement_request(),
            Some(placement::Request::Rotate {
                cursor: h,
                notches: i32::from(direction) / 2
            })
        );
        let c = s.vm.world.objects[&id(&s.vm, h).unwrap()]
            .cursor
            .as_ref()
            .unwrap();
        assert_eq!((c.container, c.slot), (3, 1));
        assert_eq!(s.vm.world.positions[&1].x, -16);
        assert_eq!(s.vm.world.objects[&1].data[&1], 0);
        for n in 0..2000 {
            s.advance_cursor_factory(&p, 1).unwrap();
            s = cold(&s, &p);
            if s.cursor_factory.as_ref().unwrap().pending_focus().is_some() {
                break;
            }
            assert!(
                n < 1999,
                "slot rotation did not return: {:?}",
                s.cursor_factory
            );
        }
        assert_eq!(s.vm.world.objects[&1].data[&1], direction);
        assert_eq!(s.vm.world.objects[&1].container, Some((3, 1)));
        assert_eq!(
            s.vm.world.objects[&id(&s.vm, h).unwrap()]
                .cursor
                .as_ref()
                .unwrap()
                .valid,
            1
        );
    }
}

#[test]
fn cursor_failed_slot_search_uses_native_height_mask_diagnostic() {
    for (mask, error) in [(8, 36), (2, 11)] {
        let (mut p, mut s) = ready_to_place(false, 1);
        container_slots(&mut p, &mut s, 3, &[5]);
        s.native_lot
            .as_mut()
            .unwrap()
            .placement
            .bounds
            .insert(3, [48, 48, 32, 32]);
        s.vm.world.objects.get_mut(&1).unwrap().data.insert(4, mask);
        s.cursor_factory.as_mut().unwrap().position = [40, 40];
        s.begin_cursor_factory_placement(&p, false).unwrap();
        for n in 0..500 {
            s.advance_cursor_factory(&p, 1).unwrap();
            s = cold(&s, &p);
            if s.cursor_factory.as_ref().unwrap().pending_focus().is_some() {
                break;
            }
            assert!(n < 499);
        }
        assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, error);
        assert_eq!(s.vm.world.positions[&1].x, -16);
        let f = s.cursor_factory.as_ref().unwrap();
        let cursor_id = id(&s.vm, f.pending_focus().unwrap()).unwrap();
        assert_eq!(s.vm.world.positions[&cursor_id].x, 40);
        assert_eq!(
            s.vm.world.objects[&cursor_id]
                .cursor
                .as_ref()
                .unwrap()
                .valid,
            0
        );
    }
}

#[test]
fn cursor_set_caches_container_only_at_unchanged_point_and_rejects_unbound_save() {
    for same in [false, true] {
        let (p, mut s) = ready_to_place(false, 1);
        let input = s.cursor_factory.as_ref().unwrap().placement().unwrap();
        let cursor_id = id(&s.vm, input.cursor).unwrap();
        let c =
            s.vm.world
                .objects
                .get_mut(&cursor_id)
                .unwrap()
                .cursor
                .as_mut()
                .unwrap();
        c.position = if same { input.position } else { [40, 40] };
        c.level = input.level;
        c.container = 3;
        c.slot = 9;
        let e = placement::Execution::new(&s, input, 4, true, false).unwrap();
        let f = s.cursor_factory.as_mut().unwrap();
        f.phase = Phase::Place;
        f.placement = Some(e);
        s.advance_cursor_factory(&p, 1).unwrap();
        let c = s.vm.world.objects[&cursor_id].cursor.as_ref().unwrap();
        assert_eq!((c.container, c.slot), if same { (3, 4) } else { (0, 0) });
        s = cold(&s, &p);
        let mut malformed = serde_json::to_value(crate::Snapshot::capture(&p, &s)).unwrap();
        malformed["state"]["cursor_factory"]["placement"]["input"]["position"][0] =
            serde_json::json!(24);
        let bad: crate::Snapshot = serde_json::from_value(malformed).unwrap();
        assert!(bad.restore(&p).unwrap_err().contains("another factory"));
    }
}
#[test]
fn live_cursor_factory_preserves_pickup_order_both_init_passes_and_cold_updates() {
    for (grouped, selected) in [(false, 1), (true, 1), (true, 3), (false, 0)] {
        let (p, initial, d) = fixture(grouped);
        let mut outcomes = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            assert!(
                s.begin_cursor_factory(&p, Some(d.clone()), selected, 0, false, Default::default())
                    .unwrap()
            );
            let mut saw_constructor = false;
            let mut saw_callback = false;
            let mut saw_visit = false;
            for i in 0..1000 {
                let frozen = s.clone();
                assert!(s.begin_native_selection(None, false).is_err());
                assert!(s.begin_temporary_portrait_person(0).is_err());
                assert!(s.enable_registry_scheduler(0).is_err());
                assert!(s.run_fixture_tick(&p, 100).is_err());
                assert!(s.advance_fixture_tick().is_err());
                assert_eq!(s, frozen);
                s.advance_cursor_factory(&p, budget).unwrap();
                let f = s.cursor_factory.as_ref().unwrap();
                for &h in f.created() {
                    let object = id(&s.vm, h).unwrap();
                    if s.cursor_constructor_pending(object) {
                        saw_constructor = true;
                        let mut bad = crate::Snapshot::capture(&p, &s);
                        bad.state.vm.world.positions.get_mut(&object).unwrap().x = -16;
                        assert!(bad.restore(&p).is_err());
                        let mut bad = crate::Snapshot::capture(&p, &s);
                        bad.state.cursor_factory = None;
                        assert!(bad.restore(&p).is_err());
                    }
                }
                if s.object_pickup.is_some() {
                    let all_post = f.created().iter().all(|&h| {
                        s.vm.world.objects[&id(&s.vm, h).unwrap()]
                            .cursor
                            .as_ref()
                            .unwrap()
                            .postconstructed
                    });
                    assert_eq!(all_post, grouped);
                    assert!(s.take_object_pickup().is_err());
                }
                saw_callback |= s.vm.threads.values().any(|t| t.callback.is_some());
                saw_visit |= s.vm.scheduled_object_visit.is_some();
                s = cold(&s, &p);
                if s.cursor_factory.as_ref().unwrap().placement().is_some() {
                    break;
                }
                assert!(i < 999);
            }
            let f = s.cursor_factory.as_ref().unwrap();
            let q = f.placement().unwrap();
            assert_eq!(q.level, 1);
            assert_eq!(q.position, if selected == 0 { [128; 2] } else { [40; 2] });
            assert_eq!(f.created().len(), if grouped { 2 } else { 1 });
            for &h in f.created() {
                let o = &s.vm.world.objects[&id(&s.vm, h).unwrap()];
                assert_eq!(o.temps[1], 2, "both base and outer Init must run");
                assert_eq!(
                    o.cursor.as_ref().unwrap().underlying.is_some(),
                    selected != 0
                );
            }
            assert_eq!(s.vm.world.objects[&1].temps[1], i16::from(selected != 0));
            assert_eq!(s.vm.world.objects[&3].temps[1], i16::from(grouped));
            assert_eq!(s.vm.tick, initial.vm.tick);
            assert_eq!(s.vm.rng_state, initial.vm.rng_state);
            if budget == 1 {
                assert!(saw_constructor && saw_callback);
                assert_eq!(saw_visit, grouped);
            }
            let before = s.clone();
            s.advance_cursor_factory(&p, 100).unwrap();
            assert_eq!(s, before, "unbound placement must not be acknowledged");
            outcomes.push(s);
        }
        assert_eq!(outcomes[0], outcomes[1]);
    }
}
#[test]
fn pickup_permission_uses_native_diagnostic_order_reverse_slots_and_ground_flags() {
    let (_, mut s, _) = fixture(false);
    s.vm.world.objects.get_mut(&1).unwrap().slots = vec![Some(2), Some(3)];
    s.vm.world.objects.get_mut(&2).unwrap().data.insert(62, 1);
    s.vm.world.objects.get_mut(&3).unwrap().data.insert(43, 0);
    assert!(!permission::check(&mut s, 1, false).unwrap());
    assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, 0x25);
    s.vm.world.objects.get_mut(&3).unwrap().data.insert(43, 2);
    assert!(!permission::check(&mut s, 1, false).unwrap());
    assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, 0x26);
    s.vm.world.objects.get_mut(&2).unwrap().data.insert(62, -1);
    s.native_lot.as_mut().unwrap().placement.flags[1][14] = 0x20;
    s.vm.world.positions.get_mut(&1).unwrap().level = 2;
    assert!(permission::check(&mut s, 1, false).unwrap());
    s.native_lot.as_mut().unwrap().placement.flags[0][14] = 0x20;
    assert!(!permission::check(&mut s, 1, false).unwrap());
    assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, 0x27);
    assert!(permission::check(&mut s, 1, true).unwrap());
    assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, 0);
    assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.blocker, None);
}
#[test]
fn cursor_permission_denial_and_absent_resource_create_nothing() {
    let (p, mut s, d) = fixture(false);
    s.vm.world.objects.get_mut(&1).unwrap().data.insert(43, 0);
    let before = s.clone();
    assert!(
        !s.begin_cursor_factory(&p, None, 1, 1, false, Default::default())
            .unwrap()
    );
    assert_eq!(s, before);
    assert!(
        !s.begin_cursor_factory(&p, Some(d), 1, 1, false, Default::default())
            .unwrap()
    );
    let mut expected = before;
    expected.native_lot.as_mut().unwrap().diagnostics.error = 0x25;
    assert_eq!(s, expected);
}

#[test]
fn cursor_update_has_a_direct_owner_and_malformed_owner_cannot_claim_callbacks() {
    let (p, mut s, d) = fixture(true);
    s.vm.lot_rng_state = Some(123);
    s.vm.world.globals.extend([(10, 1), (17, 0), (19, 0)]);
    s.enable_registry_scheduler(0).unwrap();
    assert!(
        s.begin_cursor_factory(&p, Some(d), 3, 1, false, Default::default())
            .unwrap()
    );
    let mut checked_update = false;
    for n in 0..1000 {
        s.advance_cursor_factory(&p, 1).unwrap();
        s = cold(&s, &p);
        let f = s.cursor_factory.as_ref().unwrap();
        if f.reset.is_some() {
            let mut saved = serde_json::to_value(crate::Snapshot::capture(&p, &s)).unwrap();
            saved["state"]["cursor_factory"]["reset"]["call"]["update"] = serde_json::json!(0);
            assert!(
                serde_json::from_value::<crate::Snapshot>(saved)
                    .unwrap()
                    .restore(&p)
                    .is_err()
            );
        }
        if let Some(child) = &s.object_pickup {
            let mut malformed = crate::Snapshot::capture(&p, &s);
            malformed
                .state
                .cursor_factory
                .as_mut()
                .unwrap()
                .created
                .clear();
            assert!(malformed.restore(&p).is_err());
            assert!(child.cursor_binding().is_some());
        }
        if s.vm.scheduled_object_visit.is_some() {
            assert!(f.owns_visit(&s.vm));
            assert!(s.native_scheduler.as_ref().unwrap().request().is_none());
            let mut missing = crate::Snapshot::capture(&p, &s);
            missing.state.cursor_factory = None;
            assert!(missing.restore(&p).is_err());
            checked_update = true;
        }
        if f.placement().is_some() {
            break;
        }
        assert!(n < 999);
    }
    assert!(checked_update);
    let mut bad = crate::Snapshot::capture(&p, &s);
    let f = bad.state.cursor_factory.as_mut().unwrap();
    f.selected_cursor = f.created.first().copied();
    assert!(bad.restore(&p).is_err());
    for target in [1, 3] {
        assert_eq!(s.vm.world.objects[&target].native_flags, Some(0x241));
    }
}

#[test]
fn pickup_permission_secondary_visits_head_and_signed_tile_bytes_are_preserved() {
    let (_, mut s, _) = fixture(true);
    s.vm.world.objects.get_mut(&1).unwrap().data.insert(43, 0);
    assert!(!permission::check(&mut s, 3, false).unwrap());
    assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, 0x25);
    s.vm.world.objects.get_mut(&1).unwrap().data.insert(43, 2);
    s.vm.world.objects.get_mut(&3).unwrap().data.insert(8, 0x20);
    assert!(!permission::check(&mut s, 1, false).unwrap());
    assert_eq!(s.native_lot.as_ref().unwrap().diagnostics.error, 0x26);
    let (_, mut s, _) = fixture(false);
    s.native_lot.as_mut().unwrap().placement.flags[0].fill(0x20);
    for coordinate in [-16, 128 * 16, 256 * 16] {
        let q = s.vm.world.positions.get_mut(&4).unwrap();
        q.x = coordinate;
        q.y = coordinate;
        assert_eq!(
            permission::check(&mut s, 4, false).unwrap(),
            coordinate != 256 * 16
        );
    }
    s.vm.world.objects.get_mut(&4).unwrap().slots.push(Some(4));
    assert!(permission::check(&mut s, 4, false).is_err());
}

#[cfg(feature = "private-fixtures")]
#[test]
fn sale_permission_matches_original_base_multipart_and_signed_slots() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/native-sale-permission.json"
    ))
    .unwrap();
    assert_eq!(
        corpus["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 864);
    for (n, case) in cases.iter().enumerate() {
        let input = &case["input"];
        let (_, mut s, _) = fixture(input["grouped"].as_bool().unwrap());
        for o in s.vm.world.objects.values_mut() {
            o.data.extend([(43, 2), (62, 0), (8, 0)]);
            o.slots.clear();
        }
        s.native_lot.as_mut().unwrap().placement.flags[0].fill(0);
        for (i, id) in [1, 3].into_iter().enumerate() {
            s.vm.world.objects.get_mut(&id).unwrap().data.insert(
                43,
                if input["pickup"][i].as_bool().unwrap() {
                    2
                } else {
                    0
                } | if input["sale"][i].as_bool().unwrap() {
                    8
                } else {
                    0
                },
            );
        }
        let owner = input["slot_owner"].as_i64().unwrap() as i16;
        s.vm.world.objects.get_mut(&owner).unwrap().slots = input["slots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| Some(id.as_i64().unwrap() as i16))
            .collect();
        let cheat = input["cheat"].as_bool().unwrap();
        s.native_lot.as_mut().unwrap().diagnostics = crate::lot_acceptance::person::Diagnostics {
            error: 99,
            blocker: Some(4),
        };
        let mut expected = s.clone();
        expected.native_lot.as_mut().unwrap().diagnostics.error =
            case["original"]["diagnostic"].as_i64().unwrap() as i32;
        if !cheat {
            expected.native_lot.as_mut().unwrap().diagnostics.blocker = None;
        }
        let target = input["target"].as_i64().unwrap() as i16;
        assert_eq!(
            permission::check_sale(&mut s, target, cheat).unwrap(),
            case["original"]["allowed"].as_bool().unwrap(),
            "case{n}"
        );
        assert_eq!(
            s, expected,
            "case{n}: permission changed more than original diagnostics"
        );
    }
}

#[test]
fn snapshot_undo_placement_owns_native_rotation_check_and_function11_across_cold_saves() {
    use crate::edit_history::{
        self,
        object::{self, driver},
        snapshot,
    };
    struct View<'a>(snapshot::live::View<'a>);
    impl snapshot::Registry for View<'_> {
        fn lookup(&self, id: i16) -> Result<Option<u32>, String> {
            self.0.lookup(id)
        }
        fn on_lot(&mut self, h: u32) -> Result<bool, String> {
            self.0.on_lot(h)
        }
    }
    impl snapshot::CaptureView for View<'_> {
        fn normalized_master(&self, h: u32) -> Result<Option<u32>, String> {
            self.0.normalized_master(h)
        }
        fn script_id(&self, h: u32) -> Result<i16, String> {
            self.0.script_id(h)
        }
        fn capture(&mut self, h: u32) -> Result<snapshot::Position, String> {
            self.0.capture(h)
        }
        fn definition(&self, h: u32) -> Result<u32, String> {
            self.0.definition(h)
        }
        fn facing(&self, h: u32) -> Result<i16, String> {
            snapshot::CaptureView::facing(&self.0, h)
        }
    }
    impl snapshot::placement::View for View<'_> {
        fn facing(&self, h: u32) -> Result<i16, String> {
            snapshot::placement::View::facing(&self.0, h)
        }
    }
    impl snapshot::lifecycle::View for View<'_> {
        fn manager(&self) -> Result<u32, String> {
            Ok(1)
        }
    }
    for grouped in [false, true] {
        for accepted in [false, true] {
            let (mut p, mut initial, _) = fixture(grouped);
            for functions in p
                .modules
                .get_mut("objects")
                .unwrap()
                .object_functions
                .values_mut()
            {
                functions[11].action = 4100;
            }
            initial.vm.world.globals.insert(32, 2);
            initial.vm.world.globals.insert(15, 2);
            initial
                .architecture
                .as_mut()
                .unwrap()
                .original
                .as_mut()
                .unwrap()
                .pool
                .fill(255);
            for o in initial.vm.world.objects.values_mut() {
                o.data.insert(4, 1);
                o.data.insert(42, 3);
                o.data.insert(23, 2);
            }
            let mut h = edit_history::Store::default();
            let mut objects = object::Objects::default();
            let mut refs = snapshot::References::new(1);
            let mut next = 8;
            object::construction::construct(
                object::construction::Input {
                    command: 7,
                    script_id: 1,
                    kind: 1,
                    cost: 0,
                    budget: 900,
                },
                &mut h,
                &mut objects,
                &mut refs,
                &mut snapshot::live::View {
                    world: &mut initial.vm.world,
                    program: &p,
                    links: &initial.native_lot.as_ref().unwrap().placement.links,
                    definition: &|_| Ok(1),
                },
                &mut || {
                    let id = next;
                    next += 1;
                    Ok(Some(id))
                },
            )
            .unwrap();
            let target = objects.get_mut(7).unwrap();
            target.snapshots[0].position = if accepted { [56, 56] } else { [0, 0] };
            target.snapshots[0].facing = Some(2);
            let mut finals = vec![];
            for budget in [1, 1000] {
                let (mut s, mut store, mut os, mut rs) =
                    (initial.clone(), h.clone(), objects.clone(), refs.clone());
                let mut budgets = object::budget::Budgets::default();
                let mut call = driver::Execution::new(edit_history::command::Request {
                    command: 7,
                    method: edit_history::command::Method::Undo,
                })
                .unwrap();
                let mut callbacks = 0;
                let mut rotation = 0;
                let mut placement = 0;
                let mut check = 0;
                for n in 0..4000 {
                    if s.object_pickup.is_some() {
                        s.advance_object_pickup(&p, budget).unwrap();
                        if s.object_pickup.as_ref().unwrap().complete() {
                            s.finish_object_command_pickup(&p, &mut call).unwrap();
                        }
                    } else if s.object_placement.is_some() {
                        if !matches!(
                            s.object_placement.as_ref().unwrap().caller().service,
                            driver::Service::Placement(snapshot::placement::Request::Place(_))
                        ) {
                            let mut bad =
                                serde_json::to_value(crate::Snapshot::capture(&p, &s)).unwrap();
                            bad["state"]["object_placement"]["cheat"] = serde_json::json!(true);
                            assert!(
                                serde_json::from_value::<crate::Snapshot>(bad)
                                    .unwrap()
                                    .restore(&p)
                                    .is_err()
                            );
                        }
                        let frozen = s.clone();
                        assert!(s.advance_fixture_tick().is_err());
                        assert!(s.run_fixture_tick(&p, 1).is_err());
                        assert_eq!(s, frozen);
                        assert!(s.begin_object_pickup(&p, 1, false).is_err());
                        s.advance_object_command_placement(&p, budget).unwrap();
                        if s.object_placement.as_ref().unwrap().complete() {
                            let mut wrong =
                                driver::Execution::new(edit_history::command::Request {
                                    command: 8,
                                    method: edit_history::command::Method::Undo,
                                })
                                .unwrap();
                            let before = s.clone();
                            assert!(s.finish_object_command_placement(&p, &mut wrong).is_err());
                            assert_eq!(s, before);
                            s.finish_object_command_placement(&p, &mut call).unwrap();
                        }
                    } else {
                        let mut view = View(snapshot::live::View {
                            world: &mut s.vm.world,
                            program: &p,
                            links: &s.native_lot.as_ref().unwrap().placement.links,
                            definition: &|_| Ok(1),
                        });
                        match call
                            .advance(
                                &mut store,
                                &mut os,
                                &mut rs,
                                &mut budgets,
                                &mut view,
                                budget,
                            )
                            .unwrap()
                        {
                            driver::Step::Complete(value) => {
                                assert_eq!(value, u32::from(!accepted));
                                break;
                            }
                            driver::Step::Budget => (),
                            driver::Step::Service(driver::Service::Placement(
                                snapshot::placement::Request::RemoveFromLot(_),
                            )) => s.begin_object_command_pickup(&p, &call).unwrap(),
                            driver::Step::Service(driver::Service::Placement(r)) => {
                                match r {
                                    snapshot::placement::Request::Rotate(_) => rotation += 1,
                                    snapshot::placement::Request::Check(_) => check += 1,
                                    snapshot::placement::Request::Place(_) => placement += 1,
                                    _ => unreachable!(),
                                }
                                s.begin_object_command_placement(&p, &call, false).unwrap();
                            }
                            other => panic!("unexpected snapshot undo service {other:?}"),
                        }
                    }
                    if budget == 1 {
                        if s.object_placement.is_some()
                            && s.vm.threads.values().any(|t| t.callback.is_some())
                        {
                            callbacks += 1;
                            let mut bad = crate::Snapshot::capture(&p, &s);
                            bad.state.object_placement = None;
                            assert!(bad.restore(&p).is_err());
                        }
                        s = cold(&s, &p);
                        call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
                        call.validate().unwrap();
                    }
                    assert!(n < 3999, "snapshot undo stalled");
                }
                assert!(call.result().is_some());
                assert!(rotation > 0);
                assert_eq!(check, 1);
                assert_eq!(placement, usize::from(accepted));
                if accepted && budget == 1 {
                    assert!(callbacks > 0, "PlaceOnLot function11 was never suspended");
                }
                assert_eq!(s.vm.world.positions[&1].x, if accepted { 56 } else { -16 });
                assert_eq!(s.vm.world.positions[&1].y, if accepted { 56 } else { -16 });
                assert_eq!(s.vm.world.objects[&1].data[&1], 2);
                assert_eq!(s.vm.tick, initial.vm.tick);
                assert_eq!(s.vm.rng_state, initial.vm.rng_state);
                finals.push((s, store, os, rs, call));
            }
            assert_eq!(
                finals[0], finals[1],
                "grouped={grouped} accepted={accepted}"
            );
        }
    }
}

#[test]
fn direct_cursor_point_calls_preserve_children_across_every_saved_slice() {
    use rotation::{Point, PointResult};
    for (grouped, selected) in [(false, 1), (true, 3)] {
        let (p, initial, cursor) = returned_cursor(grouped, selected);
        for point in [
            Point::Check {
                position: [56, 72],
                level: 1,
            },
            Point::Place {
                position: [56, 72],
                level: 1,
                slot: 0,
                notify: true,
            },
            Point::Check {
                position: [-16, -16],
                level: 1,
            },
        ] {
            let mut finals = vec![];
            for budget in [1, 1000] {
                let mut s = initial.clone();
                s.begin_cursor_point(&p, cursor, point, false).unwrap();
                assert!(s.run_fixture_tick(&p, 1).is_err());
                assert!(s.finish_cursor_point(&p).is_err());
                assert!(s.advance_cursor_rotation(&p, 1).is_err());
                assert!(s.finish_cursor_rotation(&p).is_err());
                for n in 0..5000 {
                    s.advance_cursor_point(&p, budget).unwrap();
                    assert_eq!(s.cursor_rotation.as_ref().unwrap().point(), Some(point));
                    if budget == 1 {
                        s = cold(&s, &p);
                    }
                    if s.cursor_rotation.as_ref().unwrap().complete() {
                        break;
                    }
                    assert!(n < 4999);
                }
                let result = s.finish_cursor_point(&p).unwrap();
                assert_eq!(
                    matches!(result, PointResult::Placed),
                    matches!(point, Point::Place { .. })
                );
                assert_eq!(s.vm.tick, initial.vm.tick);
                assert_eq!(s.vm.rng_state, initial.vm.rng_state);
                assert_eq!(s.vm.world.budget, initial.vm.world.budget);
                finals.push((result, cold(&s, &p)));
            }
            assert_eq!(finals[0], finals[1]);
        }
    }
}

mod dormant_service;
