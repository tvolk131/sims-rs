use serde_json::Value;
use sims_vm::{
    Object, Vm, World,
    person::{Neighbor, WorldState},
};
use std::collections::BTreeMap;

fn words(v: &Value) -> BTreeMap<i16, i16> {
    v.as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, v)| (i as i16, v.as_i64().unwrap() as i16))
        .collect()
}
fn compare(report: Value) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let original = &row["original"];
        let mut world = World {
            person: Some(WorldState {
                neighbor_slots: Some(vec![
                    None,
                    if c["neighbor_present"] == true {
                        Some(2)
                    } else {
                        None
                    },
                    None,
                ]),
                neighbors: BTreeMap::from([(
                    2,
                    Neighbor {
                        person: words(&original["before"]),
                        person_revision: c["revision"].as_i64().unwrap() as i32,
                        dirty: true,
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            }),
            ..Default::default()
        };
        if c["person_present"] == true {
            world.objects.insert(
                1,
                Object {
                    person: Some(words(&original["person"])),
                    ..Default::default()
                },
            );
        }
        let mut vm = Vm::new(world, 123);
        let before = vm.clone();
        vm.writeback_person_neighbor(1).unwrap();
        let n = &vm.world.person.as_ref().unwrap().neighbors[&2];
        assert_eq!(n.person, words(&original["after"]), "{c}");
        assert_eq!(
            n.person_revision,
            original["revision"].as_u64().unwrap() as u32 as i32,
            "{c}"
        );
        assert!(n.dirty, "writeback must preserve existing flags");
        if c["person_present"] == true {
            assert_eq!(
                vm.world.objects[&1].person.as_ref().unwrap(),
                &words(&original["person_after"])
            );
        }
        // This service changes only the selected persistent record fields and
        // its revision; neither source traits, queues nor random streams move.
        let mut restored = vm.clone();
        restored.world.person.as_mut().unwrap().neighbors =
            before.world.person.as_ref().unwrap().neighbors.clone();
        assert_eq!(restored, before);
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_neighbor_writeback_fields_revision_and_lookup() {
    compare(serde_json::from_str(include_str!("fixtures/neighbor-writeback.json")).unwrap());
}
#[test]
#[ignore = "requires private original neighbor writeback corpus"]
fn original_neighbor_writeback_full_corpus() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/neighbor-writeback.json");
    compare(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}

#[test]
fn removal_wrapper_writes_back_before_self_yield_or_true_return() {
    use sims_vm::{Module, Program, Routine, Snapshot, Start, Status};
    for flags in 0..=3 {
        let p = Program {
            globals: "test".into(),
            modules: BTreeMap::from([(
                "test".into(),
                Module {
                    source: "test".into(),
                    routines: BTreeMap::from([(
                        4096,
                        Routine {
                            name: "Remove self".into(),
                            body: sims_formats::behavior::Behavior {
                                format: 0x8002,
                                kind: 0,
                                args: 0,
                                locals: 0,
                                instructions: vec![sims_formats::behavior::Instruction {
                                    opcode: 18,
                                    on_true: 254,
                                    on_false: 255,
                                    operands: [0, 0, flags, 0, 0, 0, 0, 0],
                                }],
                            },
                        },
                    )]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let mut person: BTreeMap<i16, i16> = (0..256).map(|i| (i, 0)).collect();
        person.extend([(31, 2), (13, 777), (83, 888)]);
        let neighbor = Neighbor {
            person: (0..256).map(|i| (i, 0)).collect(),
            person_revision: 10,
            ..Default::default()
        };
        let mut vm = Vm::new(
            World {
                objects: BTreeMap::from([(
                    1,
                    Object {
                        person: Some(person),
                        ..Default::default()
                    },
                )]),
                person: Some(WorldState {
                    object_order: vec![1],
                    person_order: vec![1],
                    neighbor_slots: Some(vec![None, Some(2)]),
                    neighbors: BTreeMap::from([(2, neighbor)]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            123,
        );
        let t = vm
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: 1,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        vm.run_slice(&p, t, 1).unwrap();
        let Status::AwaitingRemoval(r) = vm.threads[&t].status.clone() else {
            panic!()
        };
        assert_eq!(
            vm.world.person.as_ref().unwrap().neighbors[&2].person[&13],
            0
        );
        vm.finish_removal(&p, t, r.id).unwrap();
        let neighbor = &vm.world.person.as_ref().unwrap().neighbors[&2];
        assert_eq!(neighbor.person_revision, 9);
        assert_eq!(neighbor.person[&13], 777);
        assert_eq!(
            neighbor.person[&83], 0,
            "v10-only field must remain unchanged"
        );
        assert!(!neighbor.dirty);
        assert_eq!(
            vm.threads[&t].status,
            if flags & 1 == 0 {
                Status::Sleeping { wake_tick: 1 }
            } else {
                Status::Returned(true)
            }
        );
        assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    }
}
