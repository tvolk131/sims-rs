//! Cursor storage/registration tests. These are constructed Rust fixtures,
//! not a claim of fresh original-executable comparison.
use sims_formats::{
    behavior::{Behavior, Instruction},
    execution::{ObjectFunction, Slot, Slots},
};
use sims_vm::{cursor::State, multitile::Part, *};
use std::collections::BTreeMap;

fn fixture() -> (Program, Vm, Definition) {
    let mut m = Module {
        source: "cursor-test".into(),
        ..Default::default()
    };
    for number in 1..=4 {
        let mut w = vec![0i16; 108];
        w[9] = if number == 4 { 8 } else { 4 };
        w[10] = if number == 2 || number == 3 { 1 } else { 0 };
        w[11] = if number == 3 { 0x0100 } else { 0 };
        w[56] = if number == 3 { 1 } else { 0 };
        w[14] = number as i16;
        w[58] = 9;
        w[65] = 2;
        if number == 4 {
            w[11] = -1;
        }
        m.object_definition_words.insert(number, w);
        m.object_functions.insert(
            number,
            vec![
                ObjectFunction {
                    action: 4096,
                    condition: 0,
                },
                ObjectFunction {
                    action: 4097,
                    condition: 0,
                },
            ],
        );
    }
    for (number, value) in [(4096, 17u8), (4097, 39)] {
        m.routines.insert(
            number,
            Routine {
                name: "test phase marker".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 0,
                    locals: 0,
                    instructions: vec![Instruction {
                        opcode: 2,
                        on_true: 254,
                        on_false: 253,
                        operands: [0, 0, value, 0, 0, 5, 0, 7],
                    }],
                },
            },
        );
    }
    let p = Program {
        globals: m.source.clone(),
        modules: BTreeMap::from([(m.source.clone(), m)]),
        ..Default::default()
    };
    let mut w = World {
        person: Some(Default::default()),
        refresh: Some(Default::default()),
        scheduler_storage: Some(Default::default()),
        ..Default::default()
    };
    for number in 1..=3 {
        let d = definition(number);
        let mut o = Object::construct(d.clone(), &p, 0).unwrap();
        o.data.extend([(11, number as i16), (14, -1), (29, -5)]);
        if number > 1 {
            o.multitile = Some(Part {
                head: (number == 3).then_some(2),
                next: (number == 2).then_some(3),
                origin: multitile::origin(
                    &p.modules["cursor-test"].object_definition_words[&number],
                )
                .unwrap(),
                offset: [0; 3],
            });
        }
        let id = number as i16;
        w.register_scheduler_object(id, &mut o).unwrap();
        w.objects.insert(id, o);
        w.positions.insert(
            id,
            autonomy::Placement {
                x: -16,
                y: -16,
                level: 1,
            },
        );
        w.person.as_mut().unwrap().object_order.insert(0, id);
        w.refresh
            .as_mut()
            .unwrap()
            .draw
            .insert(id, refresh::DrawState::constructed(1));
    }
    multitile::update_offsets(&mut w, 2).unwrap();
    (p, Vm::new(w, 1), definition(4))
}
fn definition(number: u16) -> Definition {
    Definition {
        source: "cursor-test".into(),
        object: number,
        guid: number as u32,
        slots: 0,
    }
}
fn handle(v: &Vm, id: i16) -> u32 {
    v.world
        .scheduler_storage
        .as_ref()
        .unwrap()
        .handle(id)
        .unwrap()
}
fn register(
    v: &mut Vm,
    p: &Program,
    d: &Definition,
    id: i16,
    target: Option<i16>,
    head: Option<i16>,
) {
    let h = target.map(|id| handle(v, id));
    v.register_cursor_storage(p, id, d.clone(), h, head, None)
        .unwrap();
    v.finish_cursor_postconstruction(p, id).unwrap();
}
fn cold(p: &Program, v: &Vm) -> Vm {
    Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(p, v)).unwrap(), p).unwrap()
}
#[test]
fn cursor_registration_stops_before_reset_and_retains_furniture_identity() {
    let (p, mut v, d) = fixture();
    let furniture = v.world.objects[&1].clone();
    let furniture_handle = handle(&v, 1);
    v.world
        .person
        .as_mut()
        .unwrap()
        .resource_instances
        .insert(d.guid, vec![]);
    v.register_cursor_storage(&p, 10, d, Some(furniture_handle), None, Some((1, 2)))
        .unwrap();
    assert_eq!(cold(&p, &v), v);
    assert_eq!(
        v.world.positions[&10],
        autonomy::Placement {
            x: 0,
            y: 0,
            level: 0
        }
    );
    assert!(v.world.objects[&10].data.values().all(|&x| x == 0));
    assert!(v.world.objects[&10].dynamic_sprite_flags.is_empty());
    assert_eq!(
        v.world.objects[&10].multitile.as_ref().unwrap().origin,
        [0; 3]
    );
    //A function12 callback may change arbitrary cursor data before+3c.
    v.world.objects.get_mut(&10).unwrap().data.insert(44, 123);
    v.finish_cursor_postconstruction(&p, 10).unwrap();
    assert_eq!(v.world.objects[&10].data[&44], 123);
    assert!(v.finish_cursor_postconstruction(&p, 10).is_err());
    let o = &v.world.objects[&10];
    assert_eq!(
        o.cursor,
        Some(State {
            postconstructed: true,
            ..State::constructed(Some(furniture_handle), Some((1, 2)))
        })
    );
    assert_eq!(o.multitile.as_ref().unwrap().origin, [-1, 255, 0]);
    assert_eq!(o.multitile.as_ref().unwrap().offset, [0; 3]);
    assert_eq!(o.native_flags, Some(0x200));
    assert_eq!(o.native_keep_dynamic, Some(0));
    assert_eq!(o.native_tree_error, Some(0));
    assert_eq!(o.idle.as_ref().unwrap().registry_word(), 0xffff);
    assert_eq!(o.data[&11], 0); //Native registry +f0 is separate from script word11.
    assert_eq!(
        [o.data[&3], o.data[&14], o.data[&29], o.data[&67]],
        [-1, -1, -5, 0]
    );
    for word in [18, 21, 24, 41, 45, 46, 47, 48] {
        assert_eq!(o.data[&word], 0);
    }
    assert_eq!(o.attributes, vec![0; 9]);
    assert_eq!(o.dynamic_sprite_flags, vec![0; 2]);
    assert!(v.object_threads.is_empty() && v.lifecycles.is_empty());
    assert_eq!(v.world.objects[&1], furniture);
    assert_eq!(handle(&v, 1), furniture_handle);
    assert_eq!(v.world.person.as_ref().unwrap().object_order, [10, 3, 2, 1]);
    assert_eq!(
        v.world.person.as_ref().unwrap().resource_instances[&4],
        [10]
    );
    assert_eq!(cold(&p, &v), v);
    assert_eq!(
        State::constructed(None, Some((7, 3))),
        State::constructed(None, None)
    );
}
#[test]
fn cursor_offsets_copy_furniture_without_splicing_its_chain() {
    for direction in 0..8 {
        let (p, mut v, d) = fixture();
        v.world
            .objects
            .get_mut(&2)
            .unwrap()
            .data
            .insert(1, direction);
        register(&mut v, &p, &d, 10, Some(2), None);
        register(&mut v, &p, &d, 11, Some(3), Some(10));
        register(&mut v, &p, &d, 12, None, Some(10));
        v.world
            .objects
            .get_mut(&12)
            .unwrap()
            .multitile
            .as_mut()
            .unwrap()
            .offset = [71, 72, 73];
        let old_tail = v.world.objects[&11].multitile.as_ref().unwrap().offset;
        v.world
            .objects
            .get_mut(&2)
            .unwrap()
            .data
            .insert(1, (direction + 2) % 8);
        //Offsets may legitimately be stale between the direction write and +88.
        assert_eq!(cold(&p, &v), v);
        cursor::refresh_offsets(&mut v.world, &p, 11).unwrap();
        assert_eq!(multitile::chain(&v.world, 11).unwrap(), [10, 11, 12]);
        assert_eq!(multitile::chain(&v.world, 3).unwrap(), [2, 3]);
        for (c, f) in [(10, 2), (11, 3)] {
            assert_eq!(
                v.world.objects[&c].multitile.as_ref().unwrap().offset,
                v.world.objects[&f].multitile.as_ref().unwrap().offset
            );
        }
        if direction % 2 == 0 {
            assert_ne!(
                v.world.objects[&11].multitile.as_ref().unwrap().offset,
                old_tail
            );
        }
        assert_eq!(
            v.world.objects[&12].multitile.as_ref().unwrap().offset,
            [71, 72, 73]
        );
        let before = v.clone();
        cursor::refresh_offsets(&mut v.world, &p, 12).unwrap();
        let mut expected = before;
        expected
            .world
            .objects
            .get_mut(&12)
            .unwrap()
            .multitile
            .as_mut()
            .unwrap()
            .offset = [0; 3];
        assert_eq!(v, expected);
        assert_eq!(cold(&p, &v), v);
    }
}
#[test]
fn cursor_cannot_use_the_ordinary_batched_group_reset() {
    let (p, mut v, d) = fixture();
    register(&mut v, &p, &d, 10, Some(2), None);
    register(&mut v, &p, &d, 11, Some(3), Some(10));
    let before = v.clone();
    assert!(
        v.prepare_multitile_reset(&p, 11, Default::default())
            .unwrap_err()
            .contains("ordered subclass")
    );
    assert_eq!(v, before);
}
#[test]
fn cursor_rejects_reused_script_ids_and_mixed_or_broken_chains() {
    let (p, mut v, d) = fixture();
    register(&mut v, &p, &d, 10, Some(1), None);
    let old = handle(&v, 1);
    v.world.unregister_scheduler_object(1).unwrap();
    let mut replacement = v.world.objects.remove(&1).unwrap();
    v.world
        .register_scheduler_object(1, &mut replacement)
        .unwrap();
    v.world.objects.insert(1, replacement);
    assert_ne!(handle(&v, 1), old);
    assert!(
        cursor::refresh_offsets(&mut v.world, &p, 10)
            .unwrap_err()
            .contains("lifetime ended")
    );
    assert!(Snapshot::capture(&p, &v).restore(&p).is_err());
    let (p, mut v, d) = fixture();
    register(&mut v, &p, &d, 10, None, None);
    v.world.objects.get_mut(&10).unwrap().multitile = None;
    assert!(Snapshot::capture(&p, &v).restore(&p).is_err());
    let (p, mut v, d) = fixture();
    register(&mut v, &p, &d, 10, None, None);
    v.world
        .objects
        .get_mut(&10)
        .unwrap()
        .multitile
        .as_mut()
        .unwrap()
        .next = Some(2);
    v.world
        .objects
        .get_mut(&2)
        .unwrap()
        .multitile
        .as_mut()
        .unwrap()
        .head = Some(10);
    assert!(Snapshot::capture(&p, &v).restore(&p).is_err());
}
#[test]
fn cursor_postconstructor_counts_only_physical_resource_slots_after_sentinel() {
    let slot = |kind| Slot {
        kind,
        offset: [0.; 3],
        standing: 0,
        sitting: 0,
        ground: 0,
        flags: 0,
        snap_target: 0,
        proximity: [0; 3],
        max_size: 0,
        unknown: 0,
        gradient: 0.,
        height: 0,
        facing: 0,
        resolution: 0,
    };
    for mode in 0..3 {
        let (mut p, mut v, mut d) = fixture();
        if mode != 0 {
            d.slots = 128;
            p.modules
                .get_mut("cursor-test")
                .unwrap()
                .object_definition_words
                .get_mut(&4)
                .unwrap()[20] = 128;
        }
        if mode == 2 {
            p.modules.get_mut("cursor-test").unwrap().slots.insert(
                128,
                Slots {
                    version: 10,
                    entries: vec![slot(0), slot(1), slot(0), slot(3)],
                },
            );
        }
        register(&mut v, &p, &d, 10, None, None);
        let count = if mode == 2 { 2 } else { 0 };
        assert_eq!(v.world.objects[&10].slots.len(), count);
        assert_eq!(v.world.objects[&10].data[&67], count as i16);
        assert_eq!(cold(&p, &v), v);

        // The ordinary431aaf path must publish the same count before Init,
        // including absent SLOT resources. Observe it from original VM scope 3
        // rather than only checking the Rust slot vector after construction.
        let m = p.modules.get_mut("cursor-test").unwrap();
        m.object_definition_words.get_mut(&1).unwrap()[20] = d.slots as i16;
        m.routines.get_mut(&4096).unwrap().body.instructions[0].operands =
            [0, 0, 67, 0, 0, 5, 8, 3];
        d.object = 1;
        d.guid = 1;
        v.begin_object_factory(&p, 20, d, false, Default::default(), [0; 2])
            .unwrap();
        let thread = v.object_threads[&20];
        finish_init(&p, &mut v, thread, false);
        assert_eq!(v.world.objects[&20].temps[0], count as i16);
    }
}

fn finish_init(p: &Program, v: &mut Vm, thread: ThreadId, cold_each: bool) {
    for _ in 0..100 {
        if v.threads[&thread].callback.is_none() {
            return;
        }
        assert_eq!(v.threads[&thread].status, Status::Ready);
        v.run_slice(p, thread, 1).unwrap();
        if cold_each {
            *v = cold(p, v);
        }
    }
    panic!("Init did not finish");
}
fn update(p: &Program, v: &mut Vm, id: i16) {
    v.begin_scheduled_object_visit(id).unwrap();
    let thread = v.object_threads[&id];
    let result = v.run_slice(p, thread, 100).unwrap();
    assert_ne!(result.stop, Stop::Budget);
    v.finish_object_update(id, true).unwrap();
    v.end_scheduled_object_visit().unwrap();
}
fn reset_request(
    p: &Program,
    v: &mut Vm,
    r: cursor::reset::Request,
    cold_each: bool,
    trace: &mut Vec<String>,
) {
    use cursor::reset::Request::*;
    let id = |v: &Vm, h| {
        v.world
            .scheduler_storage
            .as_ref()
            .unwrap()
            .object_for_handle(h)
            .unwrap()
    };
    match r {
        BaseReset {
            object,
            update: advance,
        } => {
            let object = id(v, object);
            trace.push(format!("base {object}"));
            let thread = v
                .begin_cursor_base_reset(p, object, Default::default())
                .unwrap();
            let runs = v.lifecycles[&object].run_init;
            if runs {
                trace.push(format!("init {object}"));
                finish_init(p, v, thread, cold_each);
                if advance != 0 {
                    trace.push(format!("update {object}"));
                    update(p, v, object);
                }
                v.refresh_registry_flags(p, object).unwrap();
                trace.push(format!("registry {object}"));
            }
        }
        Offsets(h) => {
            let object = id(v, h);
            cursor::refresh_offsets(&mut v.world, p, object).unwrap();
            trace.push(format!("offsets {object}"));
        }
        Init(h) => {
            let object = id(v, h);
            trace.push(format!("init {object}"));
            if let Some(t) = v.begin_loaded_function(p, object, 0).unwrap() {
                finish_init(p, v, t, cold_each);
            }
        }
        Update(h) => {
            let object = id(v, h);
            trace.push(format!("update {object}"));
            update(p, v, object);
        }
        RegistryFlags(h) => {
            let object = id(v, h);
            v.refresh_registry_flags(p, object).unwrap();
            trace.push(format!("registry {object}"));
        }
        Invalidate(h) => {
            let object = id(v, h);
            v.refresh_object(object, object, 0, 0).unwrap();
            trace.push(format!("invalidate {object}"));
        }
        Flags { object, low } => {
            //Supplied flag-service fixture. The actual4320da host is separately
            //covered by lot_selection; this test owns cursor dispatch/order.
            let object = id(v, object);
            let targets = if v.world.objects[&object].multitile.is_some() {
                multitile::chain(&v.world, object).unwrap()
            } else {
                vec![object]
            };
            for target in targets {
                let o = v.world.objects.get_mut(&target).unwrap();
                o.native_flags = Some((o.native_flags.unwrap() & !31) | low);
            }
            trace.push(format!("flags {object} {low}"));
        }
    }
}
#[test]
fn cursor_reset_runs_base_and_outer_init_in_order_and_resumes_each_boundary() {
    use cursor::reset::{Borrowed, Call};
    for advance in [0, 1, -1] {
        for selected in [10, 11] {
            let (mut p, mut v, d) = fixture();
            p.modules.get_mut("cursor-test").unwrap().routines.insert(
                4097,
                Routine {
                    name: "sleeping Main".into(),
                    body: Behavior {
                        format: 0x8002,
                        kind: 0,
                        args: 4,
                        locals: 0,
                        instructions: vec![
                            Instruction {
                                opcode: 2,
                                on_true: 1,
                                on_false: 253,
                                operands: [0, 0, 7, 0, 0, 5, 9, 7],
                            },
                            Instruction {
                                opcode: 0,
                                on_true: 0,
                                on_false: 1,
                                operands: [0; 8],
                            },
                        ],
                    },
                },
            );
            register(&mut v, &p, &d, 10, Some(2), None);
            register(&mut v, &p, &d, 11, Some(3), Some(10));
            v.world
                .objects
                .get_mut(&2)
                .unwrap()
                .data
                .extend([(23, 23), (1, 2)]);
            v.world
                .objects
                .get_mut(&3)
                .unwrap()
                .data
                .extend([(23, 34), (1, 2)]);
            let mut restored = v.clone();
            let mut traces = vec![];
            for (state, cold_each) in [(&mut v, false), (&mut restored, true)] {
                let mut call = Call::new(handle(state, selected), advance).unwrap();
                let mut trace = vec![];
                for _ in 0..100 {
                    let Some(request) = call
                        .advance(&mut Borrowed {
                            world: &mut state.world,
                        })
                        .unwrap()
                    else {
                        break;
                    };
                    assert_eq!(
                        call.advance(&mut Borrowed {
                            world: &mut state.world
                        })
                        .unwrap(),
                        Some(request)
                    );
                    if cold_each {
                        *state = cold(&p, state);
                        call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
                    }
                    reset_request(&p, state, request, cold_each, &mut trace);
                    call.acknowledge(request).unwrap();
                    assert!(call.acknowledge(request).is_err());
                    if cold_each {
                        *state = cold(&p, state);
                        call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
                    }
                }
                assert!(call.complete());
                traces.push(trace);
            }
            assert_eq!(v, restored);
            assert_eq!(traces[0], traces[1]);
            let mut expected = vec![];
            for object in [10, 11] {
                expected.extend([format!("base {object}"), format!("init {object}")]);
                if advance != 0 {
                    expected.push(format!("update {object}"));
                }
                expected.push(format!("registry {object}"));
            }
            expected.push("offsets 10".into());
            for object in [10, 11] {
                expected.push(format!("init {object}"));
                if advance != 0 {
                    expected.push(format!("update {object}"));
                }
                expected.push(format!("registry {object}"));
            }
            //Secondary423175 delegates to head42231d, which finishes its own
            //post-pass, before the secondary42231d repeats that post-pass.
            for _ in 0..if selected == 10 { 1 } else { 2 } {
                expected.extend([
                    "invalidate 2".into(),
                    "flags 2 1".into(),
                    "invalidate 3".into(),
                    "flags 3 1".into(),
                ]);
            }
            assert_eq!(traces[0], expected);
            assert_eq!(v.world.objects[&10].data[&23], 23);
            assert_eq!(v.world.objects[&11].data[&23], 34);
            assert_eq!(v.world.objects[&11].data[&1], 2);
            assert_eq!(v.tick, 0);
        }
    }
}
#[test]
fn cursor_reset_rereads_furniture_and_next_after_services() {
    use cursor::reset::{Borrowed, Call, Request};
    let (p, mut v, d) = fixture();
    register(&mut v, &p, &d, 10, Some(1), None);
    register(&mut v, &p, &d, 11, None, Some(10));
    v.world
        .objects
        .get_mut(&3)
        .unwrap()
        .data
        .extend([(23, 912), (1, 6)]);
    v.world.objects.get_mut(&3).unwrap().native_flags = Some(0x216);
    let mut call = Call::new(handle(&v, 10), 0).unwrap();
    let mut trace = vec![];
    loop {
        let request = call
            .advance(&mut Borrowed {
                world: &mut v.world,
            })
            .unwrap()
            .unwrap();
        if matches!(request, Request::Invalidate(_)) {
            let replacement = handle(&v, 3);
            v.world
                .objects
                .get_mut(&10)
                .unwrap()
                .cursor
                .as_mut()
                .unwrap()
                .underlying = Some(replacement);
            call.acknowledge(request).unwrap();
            v = cold(&p, &v);
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
            let r = call
                .advance(&mut Borrowed {
                    world: &mut v.world,
                })
                .unwrap()
                .unwrap();
            assert_eq!(
                r,
                Request::Flags {
                    object: replacement,
                    low: 0x17
                }
            );
            assert_eq!(v.world.objects[&10].data[&23], 912);
            assert_eq!(v.world.objects[&10].data[&1], 6);
            assert_eq!(v.world.objects[&10].native_flags, Some(0x200));
            //A changed next link after the flags callback ends this traversal.
            v.world
                .objects
                .get_mut(&10)
                .unwrap()
                .multitile
                .as_mut()
                .unwrap()
                .next = None;
            call.acknowledge(r).unwrap();
            assert!(
                call.advance(&mut Borrowed {
                    world: &mut v.world
                })
                .unwrap()
                .is_none()
            );
            break;
        }
        reset_request(&p, &mut v, request, false, &mut trace);
        call.acknowledge(request).unwrap();
    }
}

#[test]
fn cursor_reset_rejects_malformed_saved_iterations_before_accessing_objects() {
    use cursor::reset::{Borrowed, Call};
    let (p, mut v, d) = fixture();
    register(&mut v, &p, &d, 10, None, None);
    let mut call = Call::new(handle(&v, 10), 0).unwrap();
    let request = call
        .advance(&mut Borrowed {
            world: &mut v.world,
        })
        .unwrap()
        .unwrap();
    call.acknowledge(request).unwrap();
    let valid = serde_json::to_value(&call).unwrap();
    for field in ["current", "visited"] {
        let mut bad = valid.clone();
        bad["stack"][0][field] = if field == "current" {
            serde_json::Value::Null
        } else {
            serde_json::json!([handle(&v, 10), handle(&v, 10)])
        };
        let mut bad: Call = serde_json::from_value(bad).unwrap();
        assert!(bad.validate().is_err());
        assert!(
            bad.advance(&mut Borrowed {
                world: &mut v.world
            })
            .is_err()
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Node {
    head: Option<u32>,
    next: Option<u32>,
    underlying: Option<u32>,
    word23: i16,
    direction: i16,
    flags: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Nodes(BTreeMap<u32, Node>);
impl cursor::reset::View for Nodes {
    fn head(&self, o: u32) -> Result<Option<u32>, String> {
        Ok(self.0[&o].head)
    }
    fn next(&self, o: u32) -> Result<Option<u32>, String> {
        Ok(self.0[&o].next)
    }
    fn underlying(&self, o: u32) -> Result<Option<u32>, String> {
        Ok(self.0[&o].underlying)
    }
    fn copy_words(&mut self, c: u32, f: u32) -> Result<u32, String> {
        let source = self.0[&f].clone();
        let cursor = self.0.get_mut(&c).unwrap();
        cursor.word23 = source.word23;
        cursor.direction = source.direction;
        Ok(source.flags)
    }
}
fn wrapper_case(input: &serde_json::Value, resume: bool) -> serde_json::Value {
    use cursor::reset::{Call, Request};
    let mut nodes: Nodes = serde_json::from_value(input["nodes"].clone()).unwrap();
    let mut call = Call::new(
        input["selected"].as_u64().unwrap() as u32,
        input["update"].as_i64().unwrap() as i16,
    )
    .unwrap();
    let mut trace = vec![];
    for _ in 0..100 {
        let Some(request) = call.advance(&mut nodes).unwrap() else {
            break;
        };
        if resume {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
            nodes = serde_json::from_slice(&serde_json::to_vec(&nodes).unwrap()).unwrap();
        }
        trace.push(request);
        if request == Request::Invalidate(10) {
            match input["mutation"].as_str().unwrap() {
                "swap" => nodes.0.get_mut(&1).unwrap().underlying = Some(11),
                "words" => {
                    let n = nodes.0.get_mut(&10).unwrap();
                    n.word23 = -32768;
                    n.direction = -9;
                }
                "flags" => nodes.0.get_mut(&10).unwrap().flags = 0x12345678,
                _ => (),
            }
        }
        if let Request::Flags { object, low } = request {
            let n = nodes.0.get_mut(&object).unwrap();
            n.flags = (n.flags & !31) | low;
            if input["mutation"] == "next" && object == 10 {
                nodes.0.get_mut(&1).unwrap().next = None;
            }
        }
        call.acknowledge(request).unwrap();
        if resume {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
            nodes = serde_json::from_slice(&serde_json::to_vec(&nodes).unwrap()).unwrap();
        }
    }
    assert!(call.complete());
    serde_json::json!({"trace":trace,"nodes":nodes})
}
#[test]
#[ignore = "prepared or original cursor wrapper corpus supplied by SIMS_CURSOR_RESET_CASES"]
fn cursor_reset_wrapper_corpus() {
    let input = std::env::var("SIMS_CURSOR_RESET_CASES").unwrap();
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    let compare = std::env::var_os("SIMS_CURSOR_RESET_COMPARE_NATIVE").is_some();
    if compare {
        assert_eq!(report["native_execution"], true);
    }
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 30);
    for (index, c) in cases.iter().enumerate() {
        let direct = wrapper_case(&c["input"], false);
        assert_eq!(direct, wrapper_case(&c["input"], true), "case {index}");
        if compare {
            assert_eq!(direct, c["original"], "original case {index}");
        }
    }
    eprintln!(
        "{} wrapper cases; original comparison={compare}",
        cases.len()
    );
}
