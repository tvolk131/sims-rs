//! Compare original machine-code RNG output, mutation and fault ordering.
use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{Module, Object, Program, Routine, Start, Status, Vm, World};
use std::collections::BTreeMap;
fn main() {
    let report: Value = serde_json::from_slice(
        &std::fs::read(std::env::args().nth(1).expect("private report")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let mut count = 0;
    for row in report["streams"].as_array().unwrap() {
        let mut state = row["seed"].as_u64().unwrap() as u32;
        for expected in row["original"].as_array().unwrap() {
            let output = sims_vm::rng::next(&mut state);
            assert_eq!(json!([output, state]), *expected);
            count += 1;
        }
    }
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = |key: &str| c[key].as_i64().unwrap() as i16;
        let mut operands = [0; 8];
        for (offset, key) in [
            (0, "dest_index"),
            (2, "dest_scope"),
            (4, "range_index"),
            (6, "range_scope"),
        ] {
            operands[offset..offset + 2].copy_from_slice(&n(key).to_le_bytes());
        }
        let routine = Routine {
            name: "RNG probe".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args: 0,
                locals: 0,
                instructions: vec![Instruction {
                    opcode: 8,
                    on_true: 254,
                    on_false: 255,
                    operands,
                }],
            },
        };
        let p = Program {
            globals: "fixture".into(),
            modules: BTreeMap::from([(
                "fixture".into(),
                Module {
                    source: "fixture".into(),
                    routines: BTreeMap::from([(4096, routine)]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let mut world = World::default();
        world.objects.insert(
            1,
            Object {
                temps: [n("range"), 2, 3, 0, 7, -1, 8, 255],
                person: Some(BTreeMap::new()),
                motives: Some([(-12.5f32).to_bits(); 16]),
                ..Default::default()
            },
        );
        world.globals = (0..64)
            .map(|i| (i, if i == 34 { n("range") } else { 300 + i }))
            .collect();
        let mut vm = Vm::new(world, c["seed"].as_u64().unwrap());
        let id = vm
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
        vm.run_slice(&p, id, 1).unwrap();
        let (error, returned) = match &vm.threads[&id].status {
            Status::Returned(b) => (None, Some(u32::from(*b))),
            Status::Faulted(f) => (
                Some(
                    u32::from_str_radix(
                        f.message
                            .split("native error 0x")
                            .nth(1)
                            .unwrap()
                            .split(')')
                            .next()
                            .unwrap(),
                        16,
                    )
                    .unwrap(),
                ),
                None,
            ),
            s => panic!("unexpected {s:?}"),
        };
        let o = &vm.world.objects[&1];
        let actual = json!({"error":error,"returned":returned,"state":vm.rng_state,"temp0":o.temps[0],"temp1":o.temps[1],"motive":o.motives.unwrap()[0],"global34":vm.world.globals[&34]});
        assert_eq!(actual, row["original"], "{c}");
        // Roundtrip exact RNG state, including faulted and completed executions.
        assert_eq!(sims_vm::Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
        count += 1;
    }
    println!(
        "{count} original RNG stream/primitive comparisons match; snapshots preserve every state"
    );
}
