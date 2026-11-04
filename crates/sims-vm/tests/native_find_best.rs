use serde_json::Value;
use sims_formats::{
    behavior::{Behavior, Instruction},
    execution::ObjectFunction,
};
use sims_vm::*;
use std::collections::BTreeMap;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_function_search_empty_unconditional_filters_and_distance() {
    let mut report: Value = serde_json::from_str(include_str!("fixtures/find-best.json")).unwrap();
    let callbacks: Value =
        serde_json::from_str(include_str!("fixtures/domestic-find-best.json")).unwrap();
    report["cases"]
        .as_array_mut()
        .unwrap()
        .extend(callbacks["cases"].as_array().unwrap().clone());
    let process: Value =
        serde_json::from_str(include_str!("fixtures/process-tuning-consumers.json")).unwrap();
    report["cases"]
        .as_array_mut()
        .unwrap()
        .extend(process["find_best"].as_array().unwrap().clone());
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let mode = c["mode"].as_i64().unwrap() as i16;
        let mut bytes = [0; 8];
        bytes[..2].copy_from_slice(&mode.to_le_bytes());
        bytes[4] = 10;
        bytes[6] = c["flags"].as_u64().unwrap_or(0) as u8;
        let mut m = Module {
            source: "test".into(),
            ..Default::default()
        };
        m.routines.insert(
            4096,
            Routine {
                name: "synthetic".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 0,
                    locals: 0,
                    instructions: vec![
                        Instruction {
                            opcode: 14,
                            on_true: 1,
                            on_false: 2,
                            operands: bytes
                        };
                        3
                    ],
                },
            },
        );
        let mut world = World::default();
        // The original fixtures supplied this native process DWORD. It is not
        // inferred from a default World or from Program's FCNS cache.
        let mut owner = autonomy_constants::Owner::default();
        owner.process.words[autonomy_constants::Field::FunctionalDistance.index()] = Some(
            c["coefficient_bits"]
                .as_u64()
                .map_or(3.0f32.to_bits(), |v| v as u32),
        );
        world.autonomy_constants = Some(owner);
        if row
            .get("process_reads")
            .is_some_and(|r| r.as_array().unwrap().is_empty())
        {
            // Original zero-score/filter path never reads the coefficient.
            world.autonomy_constants = None;
        }
        let field =
            |key: &str, i: usize, default: i16| c[key][i].as_i64().map_or(default, |v| v as i16);
        let selected = find_best::selector(mode);
        let mut placements = BTreeMap::from([(
            1,
            autonomy::Placement {
                x: 0,
                y: 0,
                level: 1,
            },
        )]);
        for id in 1..=3i16 {
            let i = (id - 2).max(0) as usize;
            let mut words = vec![0; 108];
            words[9] = if id == 1 { 2 } else { field("kinds", i, 0) };
            m.object_definition_words.insert(id as u16, words);
            let mut table = vec![
                ObjectFunction {
                    action: 0,
                    condition: 0
                };
                30
            ];
            let mut object = Object {
                definition: Some(Definition {
                    source: "test".into(),
                    object: id as u16,
                    guid: id as u32,
                    slots: 0,
                }),
                data: (0..72).map(|i| (i, 0)).collect(),
                ..Default::default()
            };
            object.data.insert(29, 1);
            if id == 1 {
                object.person = Some(BTreeMap::from([(
                    65,
                    c["species"].as_i64().unwrap_or(0) as i16,
                )]));
                object.data.insert(66, 777);
                object.temps = [11, 22, 33, 44, 2, 66, 77, 88];
            } else {
                object.data.extend([
                    (25, field("cooldowns", i, 0)),
                    (8, field("disabled", i, 0)),
                    (29, field("rooms", i, 1)),
                ]);
                if let Some((score, function)) = selected {
                    if let Some(index) = score {
                        object.data.insert(index, field("scores", i, 50));
                    }
                    if let Some(index) = function {
                        table[index].action = field("functions", i, 4096) as u16;
                        if c.get("adjusted").is_some() {
                            table[index].condition = 4098 + i as u16;
                            let expr = |scope: u8, index: i16, source: u8, value: i16, next: u8| {
                                Instruction {
                                    opcode: 2,
                                    on_true: next,
                                    on_false: 253,
                                    operands: [
                                        index.to_le_bytes()[0],
                                        index.to_le_bytes()[1],
                                        value.to_le_bytes()[0],
                                        value.to_le_bytes()[1],
                                        0,
                                        5,
                                        scope,
                                        source,
                                    ],
                                }
                            };
                            m.routines.insert(
                                4098 + i as u16,
                                Routine {
                                    name: "eligibility".into(),
                                    body: Behavior {
                                        format: 0x8002,
                                        kind: 0,
                                        args: 4,
                                        locals: 0,
                                        instructions: vec![
                                            expr(8, 1, 9, 0, 1),
                                            expr(8, 0, 10, 0, 2),
                                            expr(
                                                3,
                                                66,
                                                7,
                                                field("adjusted", i, 0),
                                                if c["accepted"][i] == true { 254 } else { 255 },
                                            ),
                                        ],
                                    },
                                },
                            );
                        }
                    }
                    if mode == 15 {
                        object.data.insert(59, field("categories", i, 4));
                    }
                }
                let xyz = c["positions"][i].as_array().unwrap();
                placements.insert(
                    id,
                    autonomy::Placement {
                        x: xyz[0].as_i64().unwrap() as i32,
                        y: xyz[1].as_i64().unwrap() as i32,
                        level: xyz[2].as_i64().unwrap() as i32,
                    },
                );
            }
            m.object_functions.insert(id as u16, table);
            world.objects.insert(id, object);
        }
        world.person = Some(person::WorldState {
            object_order: if c["reverse"] == true {
                vec![3, 2]
            } else {
                vec![2, 3]
            },
            autonomy: Some(autonomy::Context {
                placements,
                ..Default::default()
            }),
            ..Default::default()
        });
        let p = Program {
            globals: "test".into(),
            modules: BTreeMap::from([("test".into(), m)]),
            ..Default::default()
        };
        let mut vm = Vm::new(world, 1);
        let t = vm
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: c["stack"].as_i64().unwrap_or(2) as i16,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        let mut run = vm.run_slice(&p, t, 1).unwrap();
        while !vm.threads[&t].find_best.is_empty() {
            vm = Snapshot::capture(&p, &vm)
                .restore(&p)
                .unwrap_or_else(|e| panic!("{e}: {c}"));
            let mut tampered = Snapshot::capture(&p, &vm);
            tampered.state.threads.get_mut(&t).unwrap().find_best[0].cursor = 0;
            assert!(tampered.restore(&p).is_err());
            run = vm.run_slice(&p, t, 1).unwrap();
            assert_ne!(run.stop, Stop::Faulted, "{:?}", run.trace);
        }
        if let Some(temps) = n.get("temps") {
            assert_eq!(serde_json::json!(vm.world.objects[&1].temps), *temps, "{c}");
        }
        if n["error"] == 23 {
            assert_eq!(run.stop, Stop::Faulted, "{c}");
            assert!(run.trace[0].effect.contains("0x17"));
        } else {
            assert_eq!(run.stop, Stop::Budget, "{c}: {:?}", run.trace);
            assert_eq!(
                vm.threads[&t].stack[0].location.instruction,
                if n["returned"] == true { 1 } else { 2 },
                "{c}"
            );
        }
        assert_eq!(
            vm.threads[&t].stack[0].stack_object,
            n["stack"].as_i64().unwrap() as i16,
            "{c}"
        );
        assert_eq!(
            vm.world.objects[&1].data[&66],
            n["score"].as_i64().unwrap() as i16,
            "{c}"
        );
    }
}
