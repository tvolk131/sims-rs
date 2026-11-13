use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;
#[cfg(feature = "private-fixtures")]
#[test]
fn native_relationship_directions_growth_memory_and_notifications() {
    let bytes = std::env::var_os("SIMS_RELATIONSHIP_ORACLE").map_or_else(
        || include_bytes!("fixtures/relationship.json").to_vec(),
        |p| std::fs::read(p).unwrap(),
    );
    compare(&bytes);
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_legacy_relationship_args_and_growth() {
    compare(include_bytes!("fixtures/home-legacy-relationship.json"));
}

#[test]
fn relationship_read_overwrites_opaque_destination_but_write_requires_its_value() {
    for writing in [false, true] {
        let mut module = Module {
            source: "probe".into(),
            ..Default::default()
        };
        module.routines.insert(
            4096,
            Routine {
                name: "opaque relationship destination".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 0,
                    locals: 4,
                    instructions: vec![
                        Instruction {
                            opcode: 26,
                            on_true: 1,
                            on_false: 255,
                            operands: [0, 0, if writing { 4 } else { 0 }, 0, 25, 0, 3, 0],
                        },
                        Instruction {
                            opcode: 2,
                            on_true: 254,
                            on_false: 255,
                            operands: [0, 0, 3, 0, 0, 5, 8, 25],
                        },
                    ],
                },
            },
        );
        let mut words = vec![0; 108];
        words[9] = 4;
        module.object_definition_words.insert(1, words);
        let p = Program {
            globals: "probe".into(),
            modules: BTreeMap::from([("probe".into(), module)]),
            ..Default::default()
        };
        let mut w = World::default();
        for id in [1, 2] {
            w.objects.insert(
                id,
                Object {
                    definition: Some(Definition {
                        source: "probe".into(),
                        object: 1,
                        guid: 1,
                        slots: 0,
                    }),
                    relationships: BTreeMap::from([(3 - id, vec![-12345])]),
                    ..Default::default()
                },
            );
        }
        let mut vm = Vm::new(w, 1);
        let id = vm
            .spawn(
                &p,
                Start {
                    owner: "probe",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: 2,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        let t = vm.threads.get_mut(&id).unwrap();
        let f = t.stack.last_mut().unwrap();
        f.opaque_locals.insert(3);
        t.frame_memory[f.storage_offset + 8 + f.args.len() + 3] = None;
        let run = vm.run_slice(&p, id, 1).unwrap();
        if writing {
            assert_eq!(run.stop, Stop::Faulted);
            let Status::Faulted(f) = &vm.threads[&id].status else {
                panic!()
            };
            assert!(f.message.contains("opaque native pointer word"));
        } else {
            let f = vm.threads[&id].stack.last().unwrap();
            assert_eq!(f.locals[3], -12345);
            assert!(!f.opaque_locals.contains(&3));
            let mut resumed = Snapshot::capture(&p, &vm).restore(&p).unwrap();
            assert_eq!(vm.run_slice(&p, id, 10).unwrap().stop, Stop::Returned(true));
            assert_eq!(
                resumed.run_slice(&p, id, 10).unwrap().stop,
                Stop::Returned(true)
            );
            assert_eq!(resumed, vm);
            assert_eq!(vm.world.objects[&1].temps[0], -12345);
        }
        assert_eq!(vm.world.objects[&1].relationships[&2], [-12345]);
    }
}
fn compare(bytes: &[u8]) {
    let report: Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let legacy = c["legacy"] == true;
        let n = |k: &str| c[k].as_i64().unwrap() as i16;
        let mut b = [
            n("column") as u8,
            n("direction") as u8,
            n("flags") as u8,
            n("local_index") as u8,
            n("scope") as u8,
            0,
            0,
            0,
        ];
        b[6..].copy_from_slice(&n("index").to_le_bytes());
        if legacy {
            b = [
                u8::from(n("flags") & 4 != 0),
                n("column") as u8,
                1,
                n("direction") as u8,
                n("flags") as u8 & 3,
                0,
                0,
                0,
            ];
        }
        let mut module = Module {
            source: "probe".into(),
            ..Default::default()
        };
        module.routines.insert(
            4096,
            Routine {
                name: "relationship".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 4,
                    locals: 4,
                    instructions: vec![Instruction {
                        opcode: if legacy { 24 } else { 26 },
                        on_true: 254,
                        on_false: 255,
                        operands: b,
                    }],
                },
            },
        );
        for id in [1, 2] {
            let mut words = vec![0; 108];
            words[9] = if id == 1 || c["persons"] == true {
                2
            } else {
                0
            };
            module.object_definition_words.insert(id, words);
        }
        let p = Program {
            globals: "probe".into(),
            modules: BTreeMap::from([("probe".into(), module)]),
            ..Default::default()
        };
        let values: Vec<i32> = serde_json::from_value(c["values"].clone()).unwrap();
        let mut w = World::default();
        let mut directory = person::WorldState::default();
        for id in [1, 2] {
            let mut obj = Object {
                definition: Some(Definition {
                    source: "probe".into(),
                    object: id as u16,
                    guid: 0,
                    slots: 0,
                }),
                person: Some(BTreeMap::from([(31, id)])),
                relationships: BTreeMap::from([(3 - id, values.clone())]),
                ..Default::default()
            };
            obj.temps[0] = n("value");
            w.objects.insert(id, obj);
            directory.neighbors.insert(
                id,
                person::Neighbor {
                    relationships: BTreeMap::from([(3 - id, values.clone())]),
                    ..Default::default()
                },
            );
        }
        w.person = Some(directory);
        let mut vm = Vm::new(w, 1);
        let thread = vm
            .spawn(
                &p,
                Start {
                    owner: "probe",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: n("stack"),
                    args: if legacy {
                        vec![n("local"), n("value"), 503, 504]
                    } else {
                        vec![501, 502, 503, 504]
                    },
                    check: false,
                },
            )
            .unwrap();
        vm.threads
            .get_mut(&thread)
            .unwrap()
            .stack
            .last_mut()
            .unwrap()
            .locals = vec![n("local"), 2, 3, 4];
        let run = vm.run_slice(&p, thread, 10).unwrap();
        let (returned, error) = match run.stop {
            Stop::Returned(b) => (Some(b), None),
            Stop::Faulted => {
                let Status::Faulted(f) = &vm.threads[&thread].status else {
                    panic!()
                };
                let code = f
                    .message
                    .split("native error 0x")
                    .nth(1)
                    .and_then(|s| s.split(')').next())
                    .map(|s| u32::from_str_radix(s, 16).unwrap());
                assert!(code.is_some(), "{c}: {}", f.message);
                (None, code)
            }
            other => panic!("{other:?}"),
        };
        let d = vm.world.person.as_ref().unwrap();
        let actual = json!({"error":error,"returned":returned,"value":if legacy{vm.threads[&thread].frame_memory[9].unwrap()}else{vm.world.objects[&1].temps[0]},
   "objects":[vm.world.objects[&1].relationships,vm.world.objects[&2].relationships],
   "neighbors":[d.neighbors[&1].relationships,d.neighbors[&2].relationships],
   "dirty":[d.neighbors[&1].dirty,d.neighbors[&2].dirty],"registry_dirty":d.relationships_dirty});
        assert_eq!(actual, row["original"], "{c}");
    }
    eprintln!(
        "{} original relationship cases matched",
        report["cases"].as_array().unwrap().len()
    );
}
