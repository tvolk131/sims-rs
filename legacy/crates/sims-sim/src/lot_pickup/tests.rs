use super::*;
use std::collections::BTreeMap;

#[derive(Default, Clone, Serialize, Deserialize)]
struct Node {
    group: bool,
    dynamic: bool,
    defined_group: bool,
    head: Option<u32>,
    next: Option<u32>,
    slots: Vec<Option<u32>>,
}
#[derive(Default, Clone, Serialize, Deserialize)]
struct Objects(BTreeMap<u32, Node>);
impl View for Objects {
    fn multipart(&self, o: u32) -> Result<bool, String> {
        Ok(self.0[&o].group)
    }
    fn dynamic(&self, o: u32) -> Result<bool, String> {
        Ok(self.0[&o].dynamic)
    }
    fn grouped_definition(&self, o: u32) -> Result<bool, String> {
        Ok(self.0[&o].defined_group)
    }
    fn head(&self, o: u32) -> Result<Option<u32>, String> {
        Ok(self.0[&o].head)
    }
    fn next(&self, o: u32) -> Result<Option<u32>, String> {
        Ok(self.0[&o].next)
    }
    fn slots(&self, o: u32) -> Result<usize, String> {
        Ok(self.0[&o].slots.len())
    }
    fn child(&self, o: u32, n: usize) -> Result<Option<u32>, String> {
        Ok(self.0[&o].slots.get(n).copied().flatten())
    }
}
fn cold<T: Serialize + serde::de::DeserializeOwned>(v: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(v).unwrap()).unwrap()
}
fn run(
    mut objects: Objects,
    mut call: Call,
    mutate: impl Fn(&mut Objects, Request),
    restored: bool,
) -> Vec<Request> {
    let mut trace = vec![];
    for _ in 0..100 {
        if restored {
            (objects, call) = cold(&(objects, call));
        }
        let Some(r) = call.advance(&objects).unwrap() else {
            assert!(call.complete());
            return trace;
        };
        assert_eq!(call.advance(&objects).unwrap(), Some(r));
        assert!(call.acknowledge(Request::Physical(999)).is_err());
        trace.push(r);
        mutate(&mut objects, r);
        call.acknowledge(r).unwrap();
        assert!(call.acknowledge(r).is_err());
    }
    panic!("pickup failed to complete")
}
#[test]
fn ordinary_slots_capture_length_after_parent_and_reread_contents_after_children() {
    for restored in [false, true] {
        let mut objects = Objects::default();
        objects.0.insert(1, Node::default());
        let trace = run(
            objects,
            Call::new(1, false),
            |o, r| match r {
                Request::Function12(1) => o.0.get_mut(&1).unwrap().slots = vec![Some(2), Some(3)],
                Request::Function12(2) => {
                    o.0.get_mut(&1).unwrap().slots = vec![Some(2), Some(4), Some(5)]
                }
                _ => (),
            },
            restored,
        );
        assert_eq!(
            trace,
            vec![
                Request::Function12(1),
                Request::Function12(2),
                Request::Function12(4),
                Request::Physical(1)
            ]
        );
    }
}
#[test]
fn shrinking_slots_skip_removed_children_and_group_reads_next_after_callbacks() {
    for restored in [false, true] {
        let objects = Objects(BTreeMap::from([
            (
                1,
                Node {
                    group: true,
                    defined_group: true,
                    next: Some(2),
                    slots: vec![Some(4), Some(5)],
                    ..Default::default()
                },
            ),
            (
                2,
                Node {
                    group: true,
                    defined_group: true,
                    head: Some(1),
                    ..Default::default()
                },
            ),
            (
                3,
                Node {
                    group: true,
                    defined_group: true,
                    head: Some(1),
                    ..Default::default()
                },
            ),
        ]));
        let trace = run(
            objects,
            Call::new(2, false),
            |o, r| {
                if r == Request::Function12(4) {
                    let n = o.0.get_mut(&1).unwrap();
                    n.slots.clear();
                    n.next = Some(3);
                }
            },
            restored,
        );
        assert_eq!(
            trace,
            vec![
                Request::Function12(1),
                Request::Function12(4),
                Request::Function12(3),
                Request::Physical(2)
            ]
        );
    }
}
#[test]
fn dynamic_split_precedes_head_read_and_base_checks_definition_fresh() {
    for restored in [false, true] {
        let objects = Objects(BTreeMap::from([(
            1,
            Node {
                group: true,
                dynamic: true,
                defined_group: true,
                head: Some(99),
                ..Default::default()
            },
        )]));
        let trace = run(
            objects,
            Call::new(1, true),
            |o, r| {
                if r == Request::Split(1) {
                    o.0.get_mut(&1).unwrap().head = None;
                }
                if r == Request::Function12(1) {
                    o.0.get_mut(&1).unwrap().defined_group = false;
                }
            },
            restored,
        );
        assert_eq!(
            trace,
            vec![
                Request::Split(1),
                Request::Function12(1),
                Request::Function12(1),
                Request::Physical(1)
            ]
        );
    }
}

#[test]
fn borrowed_pickup_retains_lifetime_identity_across_script_id_reuse() {
    let (p, mut s) = crate::lot_runtime::tests::fixture();
    s.vm.world.scheduler_storage = Some(Default::default());
    let mut object = s.vm.world.objects.remove(&1).unwrap();
    s.vm.world
        .register_scheduler_object(1, &mut object)
        .unwrap();
    s.vm.world.objects.insert(1, object);
    let view = live::Borrowed {
        world: &s.vm.world,
        program: &p,
    };
    let handle = view.handle(1).unwrap();
    let mut call = Call::new(handle, false);
    assert_eq!(
        call.advance(&view).unwrap(),
        Some(Request::Function12(handle))
    );
    s.vm.world.unregister_scheduler_object(1).unwrap();
    let mut replacement = s.vm.world.objects.remove(&1).unwrap();
    s.vm.world
        .register_scheduler_object(1, &mut replacement)
        .unwrap();
    s.vm.world.objects.insert(1, replacement);
    call.acknowledge(Request::Function12(handle)).unwrap();
    let view = live::Borrowed {
        world: &s.vm.world,
        program: &p,
    };
    assert_ne!(view.handle(1).unwrap(), handle);
    assert!(cold(&call).advance(&view).is_err());
}

#[test]
#[ignore = "generated private wrapper inputs, optionally original execution results"]
fn pickup_wrapper_corpus_matches_cold_resumption_and_supplied_original() {
    let path = std::env::var("SIMS_PICKUP_CASES").unwrap();
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let native = std::env::var_os("SIMS_PICKUP_COMPARE_NATIVE").is_some();
    assert_eq!(document["native_execution"], native);
    let cases = document["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 72);
    for row in cases {
        let c = &row["input"];
        let nodes: BTreeMap<u32, Node> = serde_json::from_value(c["nodes"].clone()).unwrap();
        let selected = c["selected"].as_u64().unwrap() as u32;
        let split = c["split"].as_bool().unwrap();
        let mut results = vec![];
        for restored in [false, true] {
            let trace = run(
                Objects(nodes.clone()),
                Call::new(selected, split),
                |o, r| {
                    if let Request::Split(id) = r {
                        let n = o.0.get_mut(&id).unwrap();
                        n.head = None;
                        n.next = None;
                    }
                    if let Request::Function12(id) = r {
                        if c["mutation"] == "parent_grow" && id == selected {
                            o.0.get_mut(&id).unwrap().slots.push(Some(6));
                        }
                        if id == 4 {
                            match c["mutation"].as_str().unwrap() {
                                "grow" => {
                                    o.0.get_mut(&1).unwrap().slots = vec![Some(4), Some(6), Some(5)]
                                }
                                "shrink" => o.0.get_mut(&1).unwrap().slots.clear(),
                                "relink" => o.0.get_mut(&1).unwrap().next = Some(3),
                                "definition" => {
                                    o.0.get_mut(&selected).unwrap().defined_group = false
                                }
                                _ => (),
                            }
                        }
                    }
                },
                restored,
            );
            let events: Vec<_> = trace
                .into_iter()
                .map(|r| match r {
                    Request::Function12(o) => serde_json::json!(["Function12", o]),
                    Request::Split(o) => serde_json::json!(["Split", o]),
                    Request::Physical(o) => serde_json::json!(["Physical", o]),
                })
                .collect();
            if native {
                assert_eq!(serde_json::json!(events), row["original"], "{c}");
            }
            results.push(events);
        }
        assert_eq!(results[0], results[1], "{c}");
    }
    eprintln!(
        "{} prepared cases; original comparison={native}",
        cases.len()
    );
}

#[test]
fn live_pickup_runs_real_function12_and_physical_removal_with_cold_resume() {
    use sims_formats::behavior::Instruction;
    for (grouped, split) in [(false, false), (true, false), (true, true)] {
        let (mut p, mut initial) = crate::lot_runtime::tests::fixture();
        let m = p.modules.get_mut("objects").unwrap();
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
                on_true: 254,
                on_false: 255,
                operands: [0, 0, 0, 0, 0, 5, 8, 8],
            },
        ];
        m.routines.insert(4100, callback);
        m.object_functions.get_mut(&1).unwrap()[12].action = 4100;
        if grouped {
            let mut words = m.object_definition_words[&1].clone();
            words[10] = 1;
            m.object_definition_words.insert(2, words);
            m.object_functions.insert(2, m.object_functions[&1].clone());
            if split {
                m.object_functions.get_mut(&2).unwrap()[8].action = 4096;
            }
            for id in [1, 3] {
                let o = initial.vm.world.objects.get_mut(&id).unwrap();
                o.definition.as_mut().unwrap().object = 2;
                o.multitile = Some(sims_vm::multitile::Part {
                    head: (id == 3).then_some(1),
                    next: (id == 1).then_some(3),
                    origin: [0; 3],
                    offset: [0; 3],
                });
            }
        }
        initial.vm.world.scheduler_storage = Some(Default::default());
        for id in 1..=4 {
            let mut object = initial.vm.world.objects.remove(&id).unwrap();
            initial
                .vm
                .world
                .register_scheduler_object(id, &mut object)
                .unwrap();
            initial.vm.world.objects.insert(id, object);
            if !initial.vm.object_threads.contains_key(&id) {
                initial.vm.start_object_lifecycle(&p, id).unwrap();
            }
        }
        let selected = if grouped { 3 } else { 1 };
        let handle = initial
            .vm
            .world
            .scheduler_storage
            .as_ref()
            .unwrap()
            .handle(selected)
            .unwrap();
        let mut outcomes = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            s.begin_object_pickup(&p, handle, split).unwrap();
            let mut saw_callback = false;
            for i in 0..300 {
                let frozen = s.clone();
                assert!(s.advance_fixture_tick().is_err());
                assert!(s.run_fixture_tick(&p, 100).is_err());
                assert_eq!(s, frozen);
                s.advance_object_pickup(&p, budget).unwrap();
                if s.vm.threads.values().any(|t| t.callback.is_some()) {
                    saw_callback = true;
                    let mut bad = crate::Snapshot::capture(&p, &s);
                    bad.state.object_pickup = None;
                    assert!(bad.restore(&p).is_err());
                }
                let mut bad = crate::Snapshot::capture(&p, &s);
                bad.state.vm.tick += 1;
                assert!(bad.restore(&p).is_err());
                s = cold(&crate::Snapshot::capture(&p, &s)).restore(&p).unwrap();
                if s.object_pickup.as_ref().unwrap().complete() {
                    break;
                }
                assert!(i < 299);
            }
            assert_eq!(saw_callback, budget == 1);
            assert_eq!(s.take_object_pickup().unwrap(), handle);
            assert_eq!(s.vm.tick, initial.vm.tick);
            assert_eq!(s.vm.rng_state, initial.vm.rng_state);
            assert_eq!(s.vm.world.objects.len(), 4);
            assert!(s.deletion.is_empty() && s.removals.is_empty());
            for id in 1..=4 {
                let removed = if split {
                    id == 3
                } else {
                    id == 1 || grouped && id == 3
                };
                assert_eq!(s.vm.world.objects[&id].temps[1], i16::from(removed));
                if removed {
                    assert_eq!(s.vm.world.positions[&id].x, -16);
                    assert_eq!(s.vm.world.positions[&id].y, -16);
                } else {
                    assert_eq!(s.vm.world.positions[&id], initial.vm.world.positions[&id]);
                }
            }
            outcomes.push(s);
        }
        assert_eq!(outcomes[0], outcomes[1]);
    }
}
