use serde_json::{Value, json};
use sims_formats::{
    behavior::{Behavior, Instruction},
    execution::ObjectFunction,
};
use sims_vm::*;
use std::collections::BTreeMap;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_functional_selector_and_frame_push() {
    let bytes = std::env::var_os("SIMS_FUNCTIONAL_ORACLE").map_or_else(
        || include_bytes!("fixtures/functional-tree.json").to_vec(),
        |p| std::fs::read(p).unwrap(),
    );
    let report: Value = serde_json::from_slice(&bytes).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mode = c["mode"].as_u64().unwrap() as u16;
        let person = c["person"].as_bool().unwrap();
        let mut operands = [0; 8];
        operands[..2].copy_from_slice(&mode.to_le_bytes());
        operands[2] = c["flags"].as_u64().unwrap() as u8;
        let routine = |opcode, operands| Routine {
            name: "synthetic".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args: 4,
                locals: 4,
                instructions: vec![Instruction {
                    opcode,
                    on_true: 254,
                    on_false: 255,
                    operands,
                }],
            },
        };
        let caller = Module {
            source: "caller".into(),
            routines: BTreeMap::from([(4096, routine(20, operands))]),
            ..Default::default()
        };
        let mut target = Module {
            source: "target".into(),
            ..Default::default()
        };
        target.object_functions.insert(
            1,
            (0..30)
                .map(|i| ObjectFunction {
                    action: if c["present"] == true { 4100 + i } else { 0 },
                    condition: 0,
                })
                .collect(),
        );
        for i in 0..30 {
            let mut r = routine(2, [0; 8]);
            r.body.locals = 2;
            target.routines.insert(4100 + i, r);
        }
        let p = Program {
            globals: "caller".into(),
            modules: BTreeMap::from([("caller".into(), caller), ("target".into(), target)]),
            ..Default::default()
        };
        let mut a = Object {
            person: person.then(|| BTreeMap::from([(65, c["species"].as_i64().unwrap() as i16)])),
            ..Default::default()
        };
        a.temps = [11, 22, 33, 44, 2, 66, 77, 88];
        let z = Object {
            definition: Some(Definition {
                source: "target".into(),
                object: 1,
                guid: 0,
                slots: 0,
            }),
            data: BTreeMap::from([(62, 0)]),
            ..Default::default()
        };
        let mut vm = Vm::new(
            World {
                actions: Some(action::WorldState {
                    actors: BTreeMap::from([(
                        1,
                        action::Actor {
                            active: Some(action::Action::default()),
                            ..Default::default()
                        },
                    )]),
                    ..Default::default()
                }),
                objects: BTreeMap::from([(1, a), (2, z)]),
                ..Default::default()
            },
            1,
        );
        let t = vm
            .spawn(
                &p,
                Start {
                    owner: "caller",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: 2,
                    args: vec![1, 2, 3, 4],
                    check: false,
                },
            )
            .unwrap();
        vm.threads.get_mut(&t).unwrap().stack[0].locals = vec![501, 502, 503, 504];
        let result = vm.run_slice(&p, t, 1).unwrap();
        let native = &row["original"];
        assert!(native["error"].is_null());
        if native["returned"] == 3 {
            assert_eq!(result.stop, Stop::Budget, "{c}");
            let f = vm.threads[&t].stack.last().unwrap();
            assert_eq!(
                json!({"bhav":f.location.bhav,"pc":f.location.instruction,"stack_object":f.stack_object,"continuation":f.continuation,"args":f.args,"locals":f.locals,"owner":f.code_owner}),
                native["frames"][1],
                "{c}"
            );
        } else {
            assert_eq!(result.stop, Stop::Returned(false), "{c}");
        }
        assert_eq!(
            json!(
                vm.world.actions.as_ref().unwrap().actors[&1]
                    .active
                    .as_ref()
                    .unwrap()
                    .icon_owner
            ),
            native["icon"],
            "{c}"
        );
        assert_eq!(
            vm.threads[&t].stack.last().is_some_and(|f| f.action_icon),
            native["use_icon"] == true,
            "{c}"
        );
        assert_eq!(
            json!(vm.world.objects[&2].data[&62]),
            native["use_count"],
            "{c}"
        );
    }
}
