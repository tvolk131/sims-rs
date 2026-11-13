use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{Module, Object, Program, Routine, Start, Status, Vm, World};

#[cfg(feature = "private-fixtures")]
#[test]
fn detachment_decodes_references_and_errors_in_original_order() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/find-location-detach.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 128);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let val = |k: &str| c[k].as_i64().unwrap();
        let operands = [1, val("index") as u8, val("flags") as u8, 0, 0, 0, 0, 0];
        let p = Program {
            modules: [(
                "test".into(),
                Module {
                    source: "test".into(),
                    routines: [(
                        4096,
                        Routine {
                            name: "Find Location".into(),
                            body: Behavior {
                                format: 0x8002,
                                kind: 0,
                                args: 4,
                                locals: val("locals") as u16,
                                instructions: vec![Instruction {
                                    opcode: 0x10,
                                    on_true: 254,
                                    on_false: 255,
                                    operands,
                                }],
                            },
                        },
                    )]
                    .into(),
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        };
        let mut vm = Vm::new(
            World {
                objects: [(1, Object::default()), (2, Object::default())].into(),
                ..Default::default()
            },
            1,
        );
        let thread = vm
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: val("target") as i16,
                    args: vec![0; 4],
                    check: false,
                },
            )
            .unwrap();
        vm.threads.get_mut(&thread).unwrap().stack[0]
            .locals
            .fill(val("reference") as i16);
        vm.run_slice(&p, thread, 1).unwrap();
        if let Some(error) = n["error"].as_u64() {
            let Status::Faulted(f) = &vm.threads[&thread].status else {
                panic!("{c}")
            };
            assert!(
                f.message.contains(&format!("0x{error:02x}")),
                "{}: {c}",
                f.message
            );
            assert_eq!(n["detached"], json!([]));
        } else {
            let Status::AwaitingPlacement(r) = vm.threads[&thread].status.clone() else {
                panic!("{c}")
            };
            assert_eq!(r.target, 2);
            assert_eq!(
                r.reference,
                if val("flags") & 1 != 0 {
                    val("reference") as i16
                } else {
                    0
                }
            );
            assert_eq!(n["detached"], json!([1]));
            assert_eq!(n["checks"], json!([]));
            assert!(n["placed"].is_null());
            assert_eq!(n["refreshed"], 0);
            // The separately tested physical service has a void native return.
            vm.finish_find_location(&p, thread, r.id, true).unwrap();
            assert_eq!(
                vm.threads[&thread].status,
                Status::Returned(n["returned"] == 1)
            );
        }
    }
}
