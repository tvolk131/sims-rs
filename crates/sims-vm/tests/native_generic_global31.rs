use serde_json::Value;
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;

#[cfg(feature = "private-fixtures")]
#[test]
fn generic_14_matches_original_single_word_global_assignment() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/generic-global31.json")).unwrap();
    let p = Program {
        globals: "test".into(),
        modules: BTreeMap::from([(
            "test".into(),
            Module {
                source: "test".into(),
                routines: BTreeMap::from([(
                    4096,
                    Routine {
                        name: "assign global 31".into(),
                        body: Behavior {
                            format: 0x8002,
                            kind: 0,
                            args: 0,
                            locals: 0,
                            instructions: vec![Instruction {
                                opcode: 1,
                                on_true: 254,
                                on_false: 255,
                                operands: [14, 0, 255, 255, 255, 255, 255, 255],
                            }],
                        },
                    },
                )]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    for row in fixture["cases"].as_array().unwrap() {
        let value = row["input"]["value"].as_i64().unwrap() as i16;
        let mut world = World::default();
        world.globals = (0..256).map(|i| (i, i)).collect();
        world
            .globals
            .insert(31, row["input"]["previous"].as_i64().unwrap() as i16);
        let mut caller = Object::default();
        caller.temps[0] = value;
        let mut target = Object::default();
        target.temps[0] = value ^ 0x7777;
        world.objects = BTreeMap::from([(1, caller), (2, target)]);
        let mut expected = world.clone();
        expected
            .globals
            .insert(31, row["original"]["global31"].as_i64().unwrap() as i16);
        let mut vm = Vm::new(world, 123);
        let rng = (vm.rng_state, vm.lot_rng_state, vm.crt_rng_state);
        let id = vm
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: 2,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        vm.run_slice(&p, id, 1).unwrap();
        assert_eq!(vm.threads[&id].status, Status::Returned(true));
        assert_eq!(vm.world, expected);
        assert_eq!((vm.rng_state, vm.lot_rng_state, vm.crt_rng_state), rng);
        assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    }
}
