//! Compare resource selection and the wrapper's post-visitor writes. The native
//! oracle supplies factory/placement/visitor boundaries; this does not claim an
//! end-to-end native person construction or rejected-placement destructor test.
use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{Module, Object, Program, Routine, Start, Status, Vm, World, person};
use std::collections::BTreeMap;

#[cfg(feature = "private-fixtures")]
#[test]
fn original_neighbor_resource_selection_and_post_visitor_frame_writes() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/visitor-creation.json")).unwrap();
    let rows = report["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 216);
    let mut tails = 0;
    for row in rows {
        let c = &row["input"];
        let n = &row["original"];
        let number = |key: &str, default: i64| c[key].as_i64().unwrap_or(default);
        let flag = number("flags", 4) as u8;
        let mut operands = [0; 8];
        operands[..4].copy_from_slice(&0x12345678u32.to_le_bytes());
        operands[4..6].copy_from_slice(&[5, flag]);
        let ins = Instruction {
            opcode: 42,
            on_true: 1,
            on_false: 1,
            operands,
        };
        let routine = |args, locals, instructions| Routine {
            name: "wrapper fixture".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args,
                locals,
                instructions,
            },
        };
        let p = Program {
            modules: BTreeMap::from([(
                "fixture".into(),
                Module {
                    source: "fixture".into(),
                    routines: BTreeMap::from([
                        (4096, routine(4, 0, vec![ins.clone(), ins.clone()])),
                        (
                            4097,
                            routine(
                                number("args", 4) as u8,
                                number("locals", 0) as u16,
                                vec![ins],
                            ),
                        ),
                    ]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let present = c["neighbor"] != false;
        let mut slots = vec![None; 25];
        slots[24] = present.then_some(25);
        let mut vm = Vm::new(
            World {
                objects: [1, 3]
                    .into_iter()
                    .map(|id| (id, Object::default()))
                    .collect(),
                person: Some(person::WorldState {
                    neighbor_slots: Some(slots),
                    neighbors: BTreeMap::from([(
                        25,
                        person::Neighbor {
                            guid: 0xaabbccdd,
                            ..Default::default()
                        },
                    )]),
                    ..Default::default()
                }),
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
                    stack_object: 25,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        vm.run_slice(&p, parent, 1).unwrap();
        let Status::AwaitingCreation(mut r) = vm.threads[&parent].status.clone() else {
            panic!("{c}")
        };
        assert_eq!(
            vm.creation_guid(&r).unwrap(),
            n["calls"]
                .as_array()
                .unwrap()
                .iter()
                .any(|call| call[0] == "factory")
                .then_some(0xaabbccdd),
            "{c}"
        );
        // Missing records return before any factory work. Invalid placement is
        // outside this comparison: its synchronous deletion has its own adapter.
        if !present || c["placement"] == false {
            continue;
        }
        tails += 1;
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
        let visitor_ran = n["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|call| call[0] == "visitor");
        let prior_word = if visitor_ran { 222 } else { 111 };
        // Supplied visitor side effects, matching the native oracle boundary.
        vm.world.objects.get_mut(&1).unwrap().temps[0] = if visitor_ran {
            number("after_temp", 456)
        } else {
            number("temp", -123)
        } as i16;
        let t = vm.threads.get_mut(&child).unwrap();
        if let Some(a) = t.stack[0].args.first_mut() {
            *a = prior_word;
        } else if let Some(a) = t.stack[0].locals.first_mut() {
            *a = prior_word;
        }
        t.frame_memory.resize(32, Some(0));
        t.frame_memory[8] = Some(prior_word);
        match number("frames", 1) {
            0 => {
                t.stack.clear();
                t.status = Status::Returned(true);
            }
            2 => {
                let mut f = t.stack[0].clone();
                f.storage_offset = 32;
                t.stack.push(f);
            }
            _ => {}
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
    }
    assert_eq!(tails, 72);
}
