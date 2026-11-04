//! Built-in outfit decisions and RNG consumption compared with original 0x45d962.
use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_builtin_outfit_fields_and_rng() {
    let bytes = std::env::var_os("SIMS_OUTFIT_ORACLE").map_or_else(
        || include_bytes!("fixtures/change-outfit.json").to_vec(),
        |path| std::fs::read(path).unwrap(),
    );
    let report: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = |key: &str| c[key].as_i64().unwrap() as i16;
        let p = Program {
            globals: "fixture".into(),
            modules: BTreeMap::from([(
                "fixture".into(),
                Module {
                    source: "fixture".into(),
                    routines: BTreeMap::from([(
                        4096,
                        Routine {
                            name: "builtin outfit".into(),
                            body: Behavior {
                                format: 0x8002,
                                kind: 0,
                                args: 0,
                                locals: 0,
                                instructions: vec![Instruction {
                                    opcode: 6,
                                    on_true: 254,
                                    on_false: 255,
                                    operands: [
                                        n("requested") as u8,
                                        1,
                                        n("flags") as u8,
                                        0,
                                        0,
                                        0,
                                        0,
                                        0,
                                    ],
                                }],
                            },
                        },
                    )]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let mut world = World::default();
        world.objects.insert(
            1,
            Object {
                person: Some(BTreeMap::from([
                    (8, n("previous")),
                    (21, n("variant")),
                    (58, n("age")),
                ])),
                ..Default::default()
            },
        );
        let mut vm = Vm::new(world, c["seed"].as_u64().unwrap());
        let thread = vm
            .spawn(
                &p,
                Start {
                    owner: "fixture",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: 1,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        let run = vm.run_slice(&p, thread, 10).unwrap();
        assert_eq!(run.stop, Stop::Returned(true), "{c}");
        let fields = vm.world.objects[&1].person.as_ref().unwrap();
        assert_eq!(
            json!({"error":null,"returned":true,"outfit":fields[&8],"variant":fields[&21],"rng":vm.rng_state}),
            row["original"],
            "{c}"
        );
    }
    eprintln!(
        "{} native outfit cases matched",
        report["cases"].as_array().unwrap().len()
    );
}
