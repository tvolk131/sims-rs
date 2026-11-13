//! Synthetic vectors from recovered code. Rectangle rebuilding on a zero mask
//! is a supplied geometry boundary; only the request state is implemented here.
use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_posture_and_footprint_requests() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/posture-footprint.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = |k: &str| c[k].as_i64().unwrap() as i16;
        assert!(row["original"]["error"].is_null());
        if c["kind"] == "posture" {
            assert_eq!(
                json!(animation::posture_mode(n("posture"), n("previous") as u32)),
                row["original"]["mode"]
            );
            continue;
        }
        let p = Program {
            globals: "test".into(),
            modules: BTreeMap::from([(
                "test".into(),
                Module {
                    source: "test".into(),
                    routines: BTreeMap::from([(
                        4096,
                        Routine {
                            name: "footprint".into(),
                            body: Behavior {
                                format: 0x8002,
                                kind: 0,
                                args: 0,
                                locals: 0,
                                instructions: vec![Instruction {
                                    opcode: 1,
                                    on_true: 254,
                                    on_false: 255,
                                    operands: [15, 0, 0, 0, 0, 0, 0, 0],
                                }],
                            },
                        },
                    )]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let mut object = Object {
            person: Some(BTreeMap::from([(72, n("old")), (73, n("applied"))])),
            ..Default::default()
        };
        object.temps[0] = n("mask");
        let mut world = World::default();
        world.objects.insert(1, object);
        let mut vm = Vm::new(world, 1);
        let id = vm
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
        vm.run_slice(&p, id, 1).unwrap();
        assert_eq!(vm.threads[&id].status, Status::Returned(true));
        let fields = vm.world.objects[&1].person.as_ref().unwrap();
        assert_eq!(json!([fields[&72], fields[&73]]), row["original"]["fields"]);
        assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
        if n("mask") == 0 {
            assert_eq!(row["original"]["refresh"], json!([[0, 244, 256]]));
        } else {
            assert_eq!(row["original"]["refresh"], json!([]));
        }
    }
}
