//! Native floor-wrapper tail after the supplied placement service. This tests
//! actual root argument/local/arena aliasing, late temp reads and exception order.
use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{Module, Object, Program, Routine, Start, Status, Vm, World};

#[cfg(feature = "private-fixtures")]
#[test]
fn native_floor_creation_publishes_parameters_after_placement() {
    let report: Value = serde_json::from_str(include_str!("fixtures/create-floor.json")).unwrap();
    let frame_report: Value =
        serde_json::from_str(include_str!("fixtures/create-frame-floor.json")).unwrap();
    let mut tails = 0;
    for row in report["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(frame_report["cases"].as_array().unwrap())
    {
        let c = &row["input"];
        let n = &row["original"];
        if !n["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|call| call[0] == "place")
        {
            continue;
        }
        tails += 1;
        let number = |key: &str, default: i64| c[key].as_i64().unwrap_or(default);
        let mut operands = [0; 8];
        operands[..4].copy_from_slice(&0x12345678u32.to_le_bytes());
        operands[4] = number("mode", 3) as u8;
        operands[5] = number("flags", 0) as u8;
        operands[6] = number("index", 0) as u8;
        let ins = Instruction {
            opcode: 42,
            on_true: 1,
            on_false: 1,
            operands,
        };
        let routine = |args, locals| Routine {
            name: "native creation tail fixture".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args,
                locals,
                instructions: vec![ins.clone(), ins.clone()],
            },
        };
        let p = Program {
            modules: [(
                "fixture".into(),
                Module {
                    source: "fixture".into(),
                    routines: [
                        (4096, routine(4, 0)),
                        (
                            4097,
                            routine(number("args", 1) as u8, number("locals", 1) as u16),
                        ),
                    ]
                    .into(),
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        };
        let mut vm = Vm::new(
            World {
                objects: [1, 2, 3]
                    .into_iter()
                    .map(|id| (id, Object::default()))
                    .collect(),
                ..Default::default()
            },
            1,
        );
        let parent = vm
            .spawn(
                &p,
                Start {
                    owner: "fixture",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: number("target", 2) as i16,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        vm.run_slice(&p, parent, 1).unwrap();
        let Status::AwaitingCreation(mut r) = vm.threads[&parent].status.clone() else {
            panic!("{c}")
        };
        let child = vm
            .spawn(
                &p,
                Start {
                    owner: "fixture",
                    bhav: 4097,
                    caller: 3,
                    callee: 3,
                    stack_object: 7,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        vm.object_threads.insert(3, child);
        // Supplied SetObject callback changes the temp before the native tail.
        vm.world.objects.get_mut(&1).unwrap().temps[0] = number("after_temp", 456) as i16;
        let t = vm.threads.get_mut(&child).unwrap();
        if let Some(v) = t.stack[0].args.first_mut() {
            *v = 111;
        } else if let Some(v) = t.stack[0].locals.first_mut() {
            *v = 111;
        }
        t.frame_memory.resize(32, Some(0));
        t.frame_memory[8] = Some(111);
        if number("frames", 1) == 0 {
            t.stack.clear();
            t.status = Status::Returned(true);
        }
        r.created = Some(3);
        let request = r.id;
        vm.threads.get_mut(&parent).unwrap().status = Status::AwaitingCreation(r);
        let result = vm.finish_creation_with_parameters(&p, parent, request, 3);
        if n["error"] == 54 {
            assert!(result.unwrap_err().contains("0x36"), "{c}");
        } else {
            result.unwrap();
            assert_eq!(vm.threads[&parent].last_result, n["returned"] == 1, "{c}");
        }
        assert_eq!(
            json!(vm.threads[&parent].stack[0].stack_object),
            n["stack_object"],
            "{c}"
        );
        assert_eq!(
            json!(vm.threads[&child].frame_memory[8]),
            n["root_word"],
            "{c}"
        );
        assert_eq!(
            json!(
                vm.threads[&child]
                    .stack
                    .first()
                    .map_or(7, |f| f.stack_object)
            ),
            n["root_object"],
            "{c}"
        );
        assert_eq!(
            json!(vm.world.objects[&1].temps[0]),
            n["creator_temp"],
            "{c}"
        );
    }
    assert_eq!(tails, 478);
}
