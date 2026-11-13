use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;
fn program(opcode: u16, operands: [u8; 8]) -> Program {
    let m = Module {
        source: "test".into(),
        routines: BTreeMap::from([(
            4096,
            Routine {
                name: "synthetic".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 0,
                    locals: 0,
                    instructions: vec![Instruction {
                        opcode,
                        operands,
                        on_true: 254,
                        on_false: 255,
                    }],
                },
            },
        )]),
        ..Default::default()
    };
    Program {
        globals: "test".into(),
        modules: BTreeMap::from([("test".into(), m)]),
        ..Default::default()
    }
}
fn start(p: &Program, w: World, current: i16) -> (Vm, u64) {
    let mut v = Vm::new(w, 1);
    let t = v
        .spawn(
            p,
            Start {
                owner: "test",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: current,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    (v, t)
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_resource_instance_order_and_cursor_exhaustion() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/resource-enumeration.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let mut p = program(31, [123, 0, 0, 0, 4, 0, 0, 0]);
        if c["loaded"] == true {
            let mut words = vec![0; 108];
            words[14] = 123;
            p.modules
                .get_mut("test")
                .unwrap()
                .object_definition_words
                .insert(1, words);
        }
        let mut w = World {
            person: Some(person::WorldState::default()),
            ..Default::default()
        };
        for id in [1, 2, 3] {
            w.objects.insert(
                id,
                Object {
                    definition: Some(Definition {
                        source: "test".into(),
                        object: if c["matches"].as_array().unwrap().contains(&json!(id)) {
                            1
                        } else {
                            2
                        },
                        guid: if c["matches"].as_array().unwrap().contains(&json!(id)) {
                            123
                        } else {
                            124
                        },
                        slots: 0,
                    }),
                    ..Default::default()
                },
            );
        }
        if c["cached"] == true {
            w.person
                .as_mut()
                .unwrap()
                .resource_instances
                .insert(123, serde_json::from_value(c["order"].clone()).unwrap());
        }
        let (mut v, t) = start(&p, w, c["current"].as_i64().unwrap() as i16);
        let r = v.run_slice(&p, t, 1).unwrap();
        assert_eq!(
            r.stop,
            Stop::Returned(n["returned"].as_bool().unwrap()),
            "{c}: {:?}",
            r.trace
        );
        // Returning pops the frame, so the retained cursor is recorded by the trace.
        let expected = if c["loaded"] == true {
            format!("→ {}", n["value"])
        } else {
            "destination retained".into()
        };
        assert!(r.trace[0].effect.contains(&expected), "{c}: {:?}", r.trace);
        if c["loaded"] == true {
            assert_eq!(
                json!(v.world.person.unwrap().resource_instances[&123]),
                n["order"],
                "{c}"
            );
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_room_exterior_scope() {
    let report: Value = serde_json::from_str(include_str!("fixtures/room-data.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let room = c["room"].as_i64().unwrap() as i16;
        // Expression writes to Temp 1 from room scope, or tries writing the room value.
        let b = if c["write"] == true {
            [1, 0, 1, 0, 0, 5, 23, 7]
        } else {
            [1, 0, 1, 0, 0, 5, 8, 23]
        };
        let p = program(2, b);
        let mut w = World::default();
        let mut o = Object::default();
        o.temps[0] = room;
        w.objects.insert(1, o);
        if c["present"] == true {
            w.room_outdoors.insert(room, c["outdoors"] == true);
        }
        let (mut v, t) = start(&p, w, 1);
        let r = v.run_slice(&p, t, 1).unwrap();
        if !n["error"].is_null() || c["write"] == true {
            assert_eq!(r.stop, Stop::Faulted, "{c}");
        } else {
            assert_eq!(r.stop, Stop::Returned(true), "{c}: {:?}", r.trace);
            assert_eq!(json!(v.world.objects[&1].temps[1]), n["value"], "{c}");
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_room_floor_scope() {
    let report: Value = serde_json::from_str(include_str!("fixtures/room-floor.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let room = c["room"].as_i64().unwrap() as i16;
        let b = if c["write"] == true {
            [2, 0, 1, 0, 0, 5, 23, 7]
        } else {
            [1, 0, 2, 0, 0, 5, 8, 23]
        };
        let p = program(2, b);
        let mut w = World::default();
        let mut o = Object::default();
        o.temps[0] = room;
        w.objects.insert(1, o);
        if c["present"] == true {
            w.room_outdoors.insert(room, false);
        }
        let (mut v, t) = start(&p, w, 1);
        let r = v.run_slice(&p, t, 1).unwrap();
        if !n["error"].is_null() || c["write"] == true {
            assert_eq!(r.stop, Stop::Faulted, "{c}: {:?}", r.trace);
        } else {
            assert_eq!(r.stop, Stop::Returned(true), "{c}: {:?}", r.trace);
            assert_eq!(json!(v.world.objects[&1].temps[1]), n["value"], "{c}");
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_find_location_detach_does_not_search_or_place() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/find-location-detach.json")).unwrap();
    // The expanded corpus also contains lookup/index failures. Their order is
    // covered by native_find_location_detach; this test exercises valid calls.
    let accepted: Vec<_> = report["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["original"]["error"].is_null())
        .collect();
    assert_eq!(accepted.len(), 36);
    for row in accepted {
        let flags = row["input"]["flags"].as_u64().unwrap() as u8;
        let n = &row["original"];
        let mut p = program(16, [1, 0, flags, 0, 0, 0, 0, 0]);
        p.modules
            .get_mut("test")
            .unwrap()
            .routines
            .get_mut(&4096)
            .unwrap()
            .body
            .locals = 1;
        let (mut v, t) = start(
            &p,
            World {
                objects: BTreeMap::from([(1, Object::default()), (2, Object::default())]),
                ..Default::default()
            },
            2,
        );
        v.threads.get_mut(&t).unwrap().stack[0].locals[0] = 1;
        v.run_slice(&p, t, 1).unwrap();
        let Status::AwaitingPlacement(r) = v.threads[&t].status.clone() else {
            panic!()
        };
        assert_eq!(r.target, 2);
        assert_eq!(r.reference, if flags & 1 != 0 { 1 } else { 0 });
        assert_eq!(n["detached"], json!([1]));
        assert_eq!(n["checks"], json!([]));
        assert_eq!(n["placed"], Value::Null);
        v.finish_find_location(&p, t, r.id, true).unwrap();
        assert_eq!(v.threads[&t].status, Status::Returned(n["returned"] == 1));
    }
}
