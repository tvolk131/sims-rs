use super::*;
use serde_json::{Value, json};
use sims_vm::{Definition, Object, autonomy::Placement, multitile::Part};

fn state(w: &World) -> Value {
    Value::Array(
        w.objects
            .iter()
            .map(|(&id, o)| {
                let p = o.multitile.as_ref().unwrap();
                json!({"id":id,"head":p.head,"next":p.next,"origin":p.origin,"offset":p.offset,
            "direction":o.data[&1],"physical_slot":o.data[&14]})
            })
            .collect(),
    )
}

fn fixture(c: &Value) -> (World, Links) {
    let mut world = World::default();
    let mut links = Links {
        size: c["size"].as_u64().unwrap() as usize,
        heads: serde_json::from_value(c["heads"].clone()).unwrap(),
        next: Default::default(),
    };
    for o in c["objects"].as_array().unwrap() {
        let id = o["id"].as_i64().unwrap() as i16;
        let slots: Vec<i16> = serde_json::from_value(o["slots"].clone()).unwrap();
        let vector = |key: &str| {
            serde_json::from_value(o.get(key).cloned().unwrap_or(json!([0, 0, 0]))).unwrap()
        };
        world.objects.insert(
            id,
            Object {
                definition: Some(Definition {
                    source: "resource".into(),
                    object: o["resource"].as_u64().unwrap_or(1) as u16,
                    guid: 0,
                    slots: 0,
                }),
                data: serde_json::from_value(o["data"].clone()).unwrap(),
                slots: slots[1..]
                    .iter()
                    .map(|&id| (id != 0).then_some(id))
                    .collect(),
                multitile: Some(Part {
                    head: o["head"].as_i64().map(|id| id as i16),
                    next: o["next"].as_i64().map(|id| id as i16),
                    origin: vector("origin"),
                    offset: vector("offset"),
                }),
                ..Default::default()
            },
        );
        let q: [i32; 3] = serde_json::from_value(o["position"].clone()).unwrap();
        world.positions.insert(
            id,
            Placement {
                x: q[0],
                y: q[1],
                level: q[2],
            },
        );
        links.next.insert(id, slots[0]);
    }
    (world, links)
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_dynamic_join_and_neighbor_masks_match_with_callback_mutation_and_restore() {
    let report: Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/dynamic-group.json"),
        )
        .expect(
            "run scripts/research/native_dynamic_group.py to generate the original-engine fixture",
        ),
    )
    .unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 132);
    for (index, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let (mut world, links) = fixture(c);
        let selected = c["selected"].as_i64().unwrap() as i16;
        let mut events = vec![];
        if c["operation"] == "merge" {
            sims_vm::multitile::merge(&mut world, selected, c["incoming"].as_i64().unwrap() as i16)
                .unwrap();
        } else {
            if c["operation"] == "post" {
                join_neighbors(&links, &mut world, selected).unwrap();
            }
            let mut updates = Updates::begin(&links, &mut world, selected).unwrap();
            loop {
                let mut restored: Updates =
                    serde_json::from_value(serde_json::to_value(&updates).unwrap()).unwrap();
                let mut other = world.clone();
                let next = updates.next(&links, &mut world).unwrap();
                assert_eq!(
                    restored.next(&links, &mut other).unwrap(),
                    next,
                    "case {index}"
                );
                assert_eq!(other, world);
                assert_eq!(restored, updates);
                let Some(update) = next else {
                    break;
                };
                events
                    .push(json!({"object":update.object,"args":update.args,"parts":state(&world)}));
                if events.len() == 1 {
                    if let Some(id) = world.objects[&update.object]
                        .multitile
                        .as_ref()
                        .unwrap()
                        .next
                    {
                        if c["mutation"] == "move_next" {
                            world.positions.insert(
                                id,
                                Placement {
                                    x: -16,
                                    y: -16,
                                    level: 1,
                                },
                            );
                        } else if c["mutation"] == "skip_next" {
                            let next = world.objects[&id].multitile.as_ref().unwrap().next;
                            world
                                .objects
                                .get_mut(&update.object)
                                .unwrap()
                                .multitile
                                .as_mut()
                                .unwrap()
                                .next = next;
                        }
                    }
                }
            }
        }
        assert_eq!(
            json!({"parts":state(&world),"events":events}),
            row["original"],
            "native case {index}: {c}"
        );
    }
}

#[test]
fn placed_multipart_factory_saves_every_callback_and_cleans_up_rejection() {
    use sims_formats::behavior::Instruction;
    for dynamic in [false, true] {
        for (contained, copy_direction) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let (mut p, mut initial) = crate::lot_runtime::tests::fixture();
            let m = p.modules.get_mut("objects").unwrap();
            let template = m.routines[&4096].clone();
            let expr = |dest, value, lhs, rhs, next| Instruction {
                opcode: 2,
                on_true: next,
                on_false: 255,
                operands: [dest, 0, value, 0, 0, 5, lhs, rhs],
            };
            for (id, args, instructions) in [
                (
                    4096,
                    0,
                    vec![Instruction {
                        opcode: 42,
                        on_true: 254,
                        on_false: 255,
                        operands: [
                            2,
                            0,
                            0,
                            0,
                            if contained { 4 } else { 3 },
                            1 | if copy_direction { 32 } else { 0 },
                            0,
                            0,
                        ],
                    }],
                ),
                (
                    4097,
                    0,
                    vec![
                        expr(4, 1, 3, 7, 1),
                        expr(42, 3, 3, 7, 2),
                        expr(0, 77, 8, 7, 254),
                    ],
                ),
                (4098, 0, vec![expr(1, 88, 8, 7, 254)]),
                (4099, 1, vec![expr(34, 0, 3, 9, 1), expr(35, 0, 3, 8, 254)]),
            ] {
                let mut r = template.clone();
                r.body.args = args;
                r.body.instructions = instructions;
                m.routines.insert(id, r);
            }
            for (number, index) in [(2, -1), (3, 0), (4, 1)] {
                let mut w = m.object_definition_words[&1].clone();
                w[10] = 7;
                w[11] = index;
                w[14] = number;
                w[63] = i16::from(index == 0);
                m.object_definition_words.insert(number as u16, w);
                m.object_definition_order.push(number as u16);
                let mut f = m.object_functions[&1].clone();
                f[0].action = 4097;
                f[1].action = 4098;
                f[8].action = if dynamic { 4099 } else { 0 };
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
            let mut ordinary = m.object_definition_words[&1].clone();
            ordinary[14] = 6;
            m.object_definition_words.insert(6, ordinary);
            let mut functions = m.object_functions[&1].clone();
            functions[1].action = 4098;
            m.object_functions.insert(6, functions);
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
                d.object = 6;
                d.guid = 6;
                initial.vm.start_object_lifecycle(&p, id).unwrap();
            }
            initial
                .architecture
                .as_mut()
                .unwrap()
                .original
                .as_mut()
                .unwrap()
                .pool
                .fill(255);
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
                .stack_object = 1;
            let mut outcomes = vec![];
            for budget in [1, 1000] {
                let mut s = initial.clone();
                let mut saved_dynamic = false;
                for slice in 0..300 {
                    let step=s.run_fixture_tick(&p,budget).unwrap_or_else(|e| panic!("dynamic {dynamic} contained {contained} budget {budget} slice {slice}: {e}"));
                    assert_ne!(step.stop, crate::StepStop::Faulted, "{:?}", s.vm.threads);
                    saved_dynamic |= s
                        .native_creations
                        .values()
                        .any(|e| serde_json::to_value(e).unwrap()["phase"] == "Dynamic");
                    let snapshot = crate::Snapshot::capture(&p, &s);
                    if s.native_creations
                        .values()
                        .any(|e| !s.vm.world.objects.contains_key(&e.binding().1))
                    {
                        assert!(sims_vm::Snapshot::capture(&p, &s.vm).restore(&p).is_err());
                        let mut orphan = snapshot.clone();
                        orphan.state.native_creations.clear();
                        assert!(orphan.restore(&p).is_err());
                    }
                    s=serde_json::from_slice::<crate::Snapshot>(&serde_json::to_vec(&snapshot).unwrap()).unwrap().restore(&p)
                        .unwrap_or_else(|e| panic!("restore dynamic {dynamic} contained {contained} budget {budget} slice {slice}: {e}; creations {:?}; lifecycle IDs {:?}", snapshot.state.native_creations, snapshot.state.vm.lifecycles.keys().collect::<Vec<_>>()));
                    if step.stop != crate::StepStop::Budget {
                        break;
                    }
                    assert!(slice < 299);
                }
                assert!(s.native_creations.is_empty());
                assert_eq!(
                    s.vm.threads[&creator].status,
                    sims_vm::Status::Returned(!contained)
                );
                assert_eq!(s.vm.world.objects.contains_key(&5), !contained);
                assert_eq!(s.vm.world.objects.contains_key(&6), !contained);
                if !contained {
                    assert_eq!(
                        sims_vm::multitile::chain(&s.vm.world, 5).unwrap(),
                        vec![5, 6]
                    );
                    for (id, mask) in [(5, 16), (6, 1)] {
                        let o = &s.vm.world.objects[&id];
                        assert_eq!(o.temps[0], 77);
                        if dynamic {
                            assert_eq!(o.data[&34], mask);
                            assert_eq!(o.data[&35], 77);
                        }
                    }
                }
                assert_eq!(saved_dynamic, dynamic && !contained && budget == 1);
                outcomes.push(serde_json::to_value(crate::Snapshot::capture(&p, &s)).unwrap());
            }
            assert_eq!(
                outcomes[0], outcomes[1],
                "dynamic {dynamic} contained {contained}"
            );
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_dynamic_split_matches_live_callback_order_and_restore() {
    let report: Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/dynamic-split.json"),
        )
        .expect(
            "run scripts/research/native_dynamic_split.py to generate the original-engine fixture",
        ),
    )
    .unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 492);
    for (index, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let selected = c["selected"].as_i64().unwrap() as i16;
        let (mut world, links) = fixture(c);
        let mut split = Split::begin(&mut world, selected).unwrap();
        let mut events = vec![];
        loop {
            split.validate(&world).unwrap();
            let mut restored: Split =
                serde_json::from_value(serde_json::to_value(&split).unwrap()).unwrap();
            let mut other = world.clone();
            let next = split
                .next(&links, &mut world)
                .unwrap_or_else(|e| panic!("native split case {index}: {e}"));
            assert_eq!(
                restored.next(&links, &mut other).unwrap(),
                next,
                "case {index}"
            );
            assert_eq!(other, world);
            assert_eq!(restored, split);
            let Some(update) = next else {
                break;
            };
            events.push(json!({"object":update.object,"args":update.args,"parts":state(&world)}));
            if events.len() == 1 {
                let part = world.objects[&update.object].multitile.as_ref().unwrap();
                match c["mutation"].as_str().unwrap() {
                    "move_next" => {
                        if let Some(id) = part.next {
                            world.positions.insert(
                                id,
                                Placement {
                                    x: -16,
                                    y: -16,
                                    level: 1,
                                },
                            );
                        }
                    }
                    "skip_next" => {
                        if let Some(id) = part.next {
                            let next = world.objects[&id].multitile.as_ref().unwrap().next;
                            world
                                .objects
                                .get_mut(&update.object)
                                .unwrap()
                                .multitile
                                .as_mut()
                                .unwrap()
                                .next = next;
                        }
                    }
                    "move_selected" => {
                        world.positions.insert(
                            selected,
                            Placement {
                                x: -16,
                                y: -16,
                                level: 1,
                            },
                        );
                    }
                    "place_selected" => {
                        world.positions.insert(
                            selected,
                            Placement {
                                x: 56,
                                y: 56,
                                level: 1,
                            },
                        );
                    }
                    "rejoin" => {
                        let head = if selected == 1 { 2 } else { 1 };
                        let tail = *world
                            .objects
                            .keys()
                            .filter(|&&id| id != selected)
                            .last()
                            .unwrap();
                        world
                            .objects
                            .get_mut(&tail)
                            .unwrap()
                            .multitile
                            .as_mut()
                            .unwrap()
                            .next = Some(selected);
                        world
                            .objects
                            .get_mut(&selected)
                            .unwrap()
                            .multitile
                            .as_mut()
                            .unwrap()
                            .head = Some(head);
                    }
                    "none" => {}
                    mutation => panic!("unhandled split mutation {mutation}"),
                }
            }
        }
        assert!(split.complete());
        split.validate(&world).unwrap();
        assert_eq!(
            json!({"parts":state(&world),"events":events}),
            row["original"],
            "native split case {index}: {c}"
        );
    }
}

#[test]
fn find_location_splits_before_removal_and_saves_each_callback() {
    use sims_formats::behavior::Instruction;
    for (selected, caller_id) in [(1, 4), (2, 4), (3, 4), (1, 1), (2, 2)] {
        let (mut p, mut initial) = crate::lot_runtime::tests::fixture();
        let m = p.modules.get_mut("objects").unwrap();
        let mut caller_words = m.object_definition_words[&1].clone();
        caller_words[14] = 2;
        caller_words[10] = if caller_id == 4 { 0 } else { 7 };
        m.object_definition_words.insert(2, caller_words);
        m.object_definition_words.get_mut(&1).unwrap()[10] = 7;
        let mut functions = m.object_functions[&1].clone();
        functions[8].action = 4099;
        functions[10].action = 4100;
        m.object_functions.insert(1, functions.clone());
        functions[1].action = 4098;
        m.object_functions.insert(2, functions);
        let template = m.routines[&4096].clone();
        let expr = |lhs, rhs, op, a, b, next| Instruction {
            opcode: 2,
            on_true: next,
            on_false: 255,
            operands: [lhs, 0, rhs, 0, 0, op, a, b],
        };
        for (id, args, instructions) in [
            (
                4098,
                0,
                vec![Instruction {
                    opcode: 0x10,
                    on_true: 254,
                    on_false: 255,
                    operands: [1, 0, 2, 0, 0, 0, 0, 0],
                }],
            ),
            (
                4099,
                4,
                vec![
                    expr(34, 1, 3, 6, 7, 1),
                    expr(34, 34, 5, 3, 6, 2),
                    expr(35, 0, 5, 3, 9, 255),
                ],
            ),
            (
                4100,
                0,
                vec![expr(36, 34, 5, 3, 6, 1), expr(1, 1, 3, 8, 7, 255)],
            ),
        ] {
            let mut r = template.clone();
            r.body.args = args;
            r.body.instructions = instructions;
            m.routines.insert(id, r);
        }
        for (id, head, next) in [
            (1, None, Some(3)),
            (3, Some(1), Some(2)),
            (2, Some(1), None),
        ] {
            let o = initial.vm.world.objects.get_mut(&id).unwrap();
            o.multitile = Some(Part {
                head,
                next,
                origin: [0; 3],
                offset: [0; 3],
            });
            o.temps[0] = 77;
            o.temps[2] = 88;
        }
        initial.vm.world.globals.insert(34, 0);
        let d = initial
            .vm
            .world
            .objects
            .get_mut(&caller_id)
            .unwrap()
            .definition
            .as_mut()
            .unwrap();
        d.object = 2;
        d.guid = 2;
        initial
            .architecture
            .as_mut()
            .unwrap()
            .bind_footprints(&p)
            .unwrap();
        initial.vm = sims_vm::Vm::new(initial.vm.world.clone(), 1);
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
        let mut results = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            let mut suspended = false;
            let mut completed = 0;
            for slice in 0..100 {
                let step = s.run_fixture_tick(&p, budget).unwrap_or_else(|e| {
                    panic!(
                        "selected {selected} caller {caller_id} budget {budget} slice {slice}: {e}"
                    )
                });
                assert_ne!(step.stop, crate::StepStop::Faulted, "{:?}", s.vm.threads);
                if matches!(
                    s.native_lot.as_ref().unwrap().find_location,
                    Some(crate::lot_find_location::Execution::DynamicDetach { .. })
                ) {
                    suspended = true;
                    assert!(s.advance_fixture_tick().is_err());
                }
                if s.vm.world.positions[&selected].x < 0 {
                    assert_eq!(
                        s.vm.world.globals[&34], 3,
                        "all dynamic callbacks precede removal"
                    );
                    assert_eq!(
                        s.vm.world.objects[&selected].data[&36], 3,
                        "function 10 runs after function 8"
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
            assert_eq!(suspended, budget == 1);
            let order: Vec<_> = [1, 3, 2]
                .into_iter()
                .filter(|&id| id != selected)
                .chain([selected])
                .collect();
            for (i, &id) in order.iter().enumerate() {
                let o = &s.vm.world.objects[&id];
                assert_eq!(o.data[&34], i as i16 + 1);
                assert_eq!(o.data[&35], 0);
                assert_eq!(
                    [o.temps[0], o.temps[2]],
                    [77, 88],
                    "explicit update args preserve Temps"
                );
                assert_eq!(o.temps[1], i16::from(id == selected));
                assert_eq!(s.unplaced.contains(&id), id == selected);
            }
            assert_eq!(
                sims_vm::multitile::chain(&s.vm.world, selected).unwrap(),
                vec![selected]
            );
            assert_eq!(
                sims_vm::multitile::chain(&s.vm.world, order[0]).unwrap(),
                order[..2]
            );
            assert_eq!(s.vm.world.objects.len(), 4);
            assert!(s.removals.is_empty() && s.deletion.is_empty());
            assert_eq!(
                s.vm.threads[&thread].status,
                sims_vm::Status::Returned(true)
            );
            results.push(s);
        }
        assert_eq!(results[0], results[1]);
    }
}
