//! Original integer and resolver behavior, with synthetic lot storage.
use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_budget_vectors() {
    let report: Value = serde_json::from_str(include_str!("fixtures/budget.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = |k: &str| c[k].as_i64().unwrap();
        let value = n("value") as i16;
        let index = if n("source") == 0 { value } else { 0 };
        let mut b = [
            n("source") as u8,
            8,
            0,
            0,
            n("flags") as u8,
            0,
            n("category") as u8,
            0,
        ];
        b[2..4].copy_from_slice(&index.to_le_bytes());
        let p = Program {
            globals: "test".into(),
            modules: BTreeMap::from([(
                "test".into(),
                Module {
                    source: "test".into(),
                    routines: BTreeMap::from([(
                        4096,
                        Routine {
                            name: "budget".into(),
                            body: Behavior {
                                format: 0x8002,
                                kind: 0,
                                args: 4,
                                locals: 4,
                                instructions: vec![Instruction {
                                    opcode: 25,
                                    on_true: 254,
                                    on_false: 255,
                                    operands: b,
                                }],
                            },
                        },
                    )]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let expenses: [i32; 9] = serde_json::from_value(c["expenses"].clone()).unwrap();
        let mut world = World {
            budget: Some(budget::State {
                balance: n("balance") as i32,
                expenses,
                notifications: vec![],
            }),
            globals: BTreeMap::from([(13, 123), (14, 456), (32, n("mode") as i16)]),
            ..Default::default()
        };
        let mut obj = Object::default();
        obj.temps[0] = value;
        world.objects.insert(1, obj);
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
                    args: vec![value, 0, 0, 0],
                    check: false,
                },
            )
            .unwrap();
        vm.threads.get_mut(&id).unwrap().stack[0].locals[0] = value;
        vm.run_slice(&p, id, 1).unwrap();
        let r = &row["original"];
        if r["error"].is_null() {
            assert_eq!(
                vm.threads[&id].status,
                Status::Returned(r["returned"] == 1),
                "{c}"
            );
        } else {
            assert_eq!(r["error"], 56);
            assert!(
                matches!(&vm.threads[&id].status,Status::Faulted(f) if f.message.contains("0x38"))
            );
        }
        let state = vm.world.budget.as_ref().unwrap();
        assert_eq!(json!(state.balance), r["balance"], "{c}");
        assert_eq!(json!(state.expenses), r["expenses"], "{c}");
        assert_eq!(
            json!([vm.world.globals[&13], vm.world.globals[&14]]),
            r["money_words"],
            "{c}"
        );
        assert_eq!(
            json!(
                state
                    .notifications
                    .iter()
                    .map(|&v| [218, v])
                    .collect::<Vec<_>>()
            ),
            r["signals"],
            "{c}"
        );
        Snapshot::capture(&p, &vm).restore(&p).unwrap();
    }
}
