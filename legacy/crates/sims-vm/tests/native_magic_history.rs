use serde_json::Value;
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{
    magic_history::{self, Context},
    *,
};
use std::collections::BTreeMap;

fn program() -> Program {
    Program {
        globals: "test".into(),
        modules: BTreeMap::from([(
            "test".into(),
            Module {
                source: "test".into(),
                routines: BTreeMap::from([(
                    4096,
                    Routine {
                        name: "Generic43".into(),
                        body: Behavior {
                            format: 0x8002,
                            kind: 0,
                            args: 0,
                            locals: 0,
                            instructions: vec![Instruction {
                                opcode: 1,
                                operands: [43, 0, 0, 0, 0, 0, 0, 0],
                                on_true: 254,
                                on_false: 255,
                            }],
                        },
                    },
                )]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    }
}
fn controller(attributes: Vec<i16>) -> Object {
    Object {
        definition: Some(Definition {
            source: "test".into(),
            object: 1,
            guid: magic_history::CONTROLLER,
            slots: 0,
        }),
        attributes,
        ..Default::default()
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_generic43_signed_values_guards_and_unchanged_state() {
    let report: Value = serde_json::from_str(include_str!("fixtures/magic-history.json")).unwrap();
    let p = program();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let expected = &row["original"];
        let mut world = World {
            globals: BTreeMap::from([
                (9, c["selected_family"].as_i64().unwrap() as i16),
                (10, c["lot"].as_i64().unwrap() as i16),
            ]),
            magic_history: Some(if c["context"] == true {
                Context::Retained {
                    enabled: c["enabled"].as_u64().unwrap() != 0,
                    family: c["family"].as_i64().unwrap() as i32,
                    registry_present: c["registry"] == true,
                    values: serde_json::from_value(c["history"].clone()).unwrap(),
                }
            } else {
                Context::Absent
            }),
            person: Some(person::WorldState {
                lot_zoning: BTreeMap::from([(7, c["zone"].as_i64().unwrap() as i32)]),
                ..Default::default()
            }),
            ..Default::default()
        };
        world.objects.insert(1, Object::default());
        let mut target = controller(serde_json::from_value(c["attributes"].clone()).unwrap());
        if c["controller"] != true {
            target.definition.as_mut().unwrap().guid = 123;
        }
        world.objects.insert(2, target);
        let before = world.clone();
        let mut vm = Vm::new(world, 123456);
        vm.lot_rng_state = Some(987654);
        let id = vm
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: 1,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        let run = vm.run_slice(&p, id, 1).unwrap();
        assert_eq!(
            run.stop,
            Stop::Returned(expected["returned"].as_bool().unwrap()),
            "{c}: {:?}",
            run.trace
        );
        assert_eq!(
            serde_json::json!(vm.world.objects[&2].attributes),
            expected["attributes"],
            "{c}"
        );
        assert_eq!(vm.rng_state, 123456);
        assert_eq!(vm.lot_rng_state, Some(987654));
        assert_eq!(vm.world.magic_history, before.magic_history);
        let cache = &vm.world.person.as_ref().unwrap().resource_instances;
        assert_eq!(
            cache.contains_key(&magic_history::CONTROLLER),
            expected["lookups"].as_u64().unwrap() != 0,
            "{c}"
        );
        vm.world.objects.get_mut(&2).unwrap().attributes = before.objects[&2].attributes.clone();
        vm.world.person.as_mut().unwrap().resource_instances.clear();
        assert_eq!(vm.world, before, "{c}");
    }
}

#[test]
fn history_is_retained_and_only_first_cached_instance_receives_it() {
    assert_eq!(
        Context::constructed(0, true),
        Context::Retained {
            enabled: false,
            family: 0,
            registry_present: true,
            values: [0; 6]
        }
    );
    let mut w = World {
        globals: BTreeMap::from([(9, 1), (10, 0)]),
        magic_history: Some(Context::Retained {
            enabled: true,
            family: 1,
            registry_present: true,
            values: [5; 6],
        }),
        person: Some(person::WorldState::default()),
        objects: BTreeMap::from([(2, controller(vec![0; 8])), (3, controller(vec![0; 8]))]),
        ..Default::default()
    };
    // Cache order, rather than object ID or construction order, is authoritative.
    w.person
        .as_mut()
        .unwrap()
        .resource_instances
        .insert(magic_history::CONTROLLER, vec![3, 2]);
    let encoded = serde_json::to_vec(&w).unwrap();
    let restored: World = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(w, restored);
    magic_history::restore(&mut w).unwrap();
    assert_eq!(w.objects[&2].attributes, vec![0; 8]);
    assert_eq!(w.objects[&3].attributes, vec![5, 5, 5, 5, 5, 5, 0, 0]);
    w.magic_history = None;
    let before = w.clone();
    assert!(magic_history::restore(&mut w).is_err());
    assert_eq!(w, before);
    // No retained family can be both nonzero and equal to a zero selected family.
    w.globals.insert(9, 0);
    magic_history::restore(&mut w).unwrap();
}
