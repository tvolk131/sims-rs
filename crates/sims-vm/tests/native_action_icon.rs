use serde_json::Value;
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_generic_action_icon_updates_only_icon_owner() {
    let report: Value = serde_json::from_str(include_str!("fixtures/action-icon.json")).unwrap();
    let p = Program {
        globals: "test".into(),
        modules: BTreeMap::from([(
            "test".into(),
            Module {
                source: "test".into(),
                routines: BTreeMap::from([(
                    4096,
                    Routine {
                        name: "icon".into(),
                        body: Behavior {
                            format: 0x8002,
                            kind: 0,
                            args: 0,
                            locals: 0,
                            instructions: vec![Instruction {
                                opcode: 1,
                                on_true: 254,
                                on_false: 255,
                                operands: [2, 0, 0, 0, 0, 0, 0, 0],
                            }],
                        },
                    },
                )]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let mut w = World::default();
        w.objects.insert(
            1,
            Object {
                person: (c["person"] == true).then(BTreeMap::new),
                ..Default::default()
            },
        );
        w.objects.insert(2, Object::default());
        w.actions = Some(action::WorldState {
            actors: BTreeMap::from([(
                1,
                action::Actor {
                    active: (c["active"] == true).then_some(action::Action {
                        id: 99,
                        target: 2,
                        icon_owner: 2,
                        priority: 100,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        });
        let mut vm = Vm::new(w, 1);
        let id = vm
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: c["target"].as_i64().unwrap() as i16,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        let run = vm.run_slice(&p, id, 1).unwrap();
        if let Some(error) = n["error"].as_u64() {
            assert_eq!(run.stop, Stop::Faulted, "{c}");
            assert!(
                run.trace[0].effect.contains(&format!("0x{error:02x}")),
                "{:?}",
                run.trace
            );
        } else {
            assert_eq!(
                run.stop,
                Stop::Returned(n["returned"].as_bool().unwrap()),
                "{c}"
            );
        }
        if let Some(a) = vm.world.actions.as_ref().unwrap().actors[&1]
            .active
            .as_ref()
        {
            assert_eq!(a.icon_owner, n["icon"].as_i64().unwrap() as i16, "{c}");
            assert_eq!(a.target, n["target"].as_i64().unwrap() as i16, "{c}");
            assert_eq!(a.id, n["serial"].as_u64().unwrap() as u32);
        }
    }
}
