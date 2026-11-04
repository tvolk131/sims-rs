use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;
fn setup(ins: Instruction, target: i16) -> (Program, Vm, u64) {
    let mut m = Module {
        source: "test".into(),
        ..Default::default()
    };
    m.routines.insert(
        4096,
        Routine {
            name: "probe".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args: 0,
                locals: 0,
                instructions: vec![ins],
            },
        },
    );
    let p = Program {
        modules: BTreeMap::from([("test".into(), m)]),
        ..Default::default()
    };
    let mut w = World::default();
    w.objects
        .extend([(1, Object::default()), (2, Object::default())]);
    let mut v = Vm::new(w, 1);
    let id = v
        .spawn(
            &p,
            Start {
                owner: "test",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: target,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    (p, v, id)
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_family_membership_drives_neighbor_home_lot() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/domestic-home-lot.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let e = &row["original"];
        let (p, mut v, id) = setup(
            Instruction {
                opcode: 2,
                on_true: 254,
                on_false: 255,
                operands: [0; 8],
            },
            c["neighbor"].as_i64().unwrap() as i16,
        );
        let mut w = person::WorldState::default();
        for id in 1..=2 {
            w.neighbors.insert(
                id,
                person::Neighbor {
                    guid: 0x7fd96b54 + if id == 1 { 0 } else { id as u32 },
                    ..Default::default()
                },
            );
        }
        w.families.insert(
            1,
            c["members"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_i64().unwrap() as i16)
                .collect(),
        );
        w.family_homes.insert(1, c["lot"].as_i64().unwrap() as i16);
        v.world.person = Some(w);
        let r = v.inspect_memory(&p, id, 24, 6, c["write"].as_bool().unwrap());
        if e["error"].is_null() {
            let r = r.unwrap();
            assert_eq!(r.value, e["value"].as_i64().unwrap() as i16, "{c}");
            assert!(!r.writable);
        } else {
            assert!(r.is_err(), "{c}");
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_neighbor_home_uses_first_family_in_native_order() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/domestic-home-lot.json")).unwrap();
    let rows = report["ordered_cases"].as_array().unwrap();
    assert_eq!(rows.len(), 128);
    for row in rows {
        let c = &row["input"];
        let (p, mut v, id) = setup(
            Instruction {
                opcode: 2,
                on_true: 254,
                on_false: 255,
                operands: [0; 8],
            },
            c["neighbor"].as_i64().unwrap() as i16,
        );
        let mut people = person::WorldState {
            family_order: Some(serde_json::from_value(c["order"].clone()).unwrap()),
            family_homes: [(1, 7), (2, 9)].into(),
            ..Default::default()
        };
        for n in 1..=2 {
            people.neighbors.insert(
                n,
                person::Neighbor {
                    guid: if n == 1 || c["same_guid"] == true {
                        1
                    } else {
                        2
                    },
                    // The lookup must ignore this cached person word.
                    person: [(61, 99)].into(),
                    ..Default::default()
                },
            );
        }
        for (i, members) in c["members"].as_array().unwrap().iter().enumerate() {
            people.families.insert(
                i as i16 + 1,
                serde_json::from_value(members.clone()).unwrap(),
            );
        }
        v.world.person = Some(people);
        let got = v
            .inspect_memory(&p, id, 24, 6, c["write"].as_bool().unwrap())
            .unwrap();
        assert_eq!(row["original"]["error"], serde_json::Value::Null);
        assert_eq!(
            serde_json::json!(got.value),
            row["original"]["value"],
            "{c}"
        );
        assert!(!got.writable);
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_special_effect_phases_and_render_boundary() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/domestic-effects.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let e = &row["original"];
        let flags = c["flags"].as_u64().unwrap() as u8;
        let value = c["value"].as_i64().unwrap() as i16;
        let mut operands = [0; 8];
        operands[..2].copy_from_slice(&value.to_le_bytes());
        operands[2] = 1;
        operands[3] = 2;
        operands[4] = flags;
        let (p, mut v, id) = setup(
            Instruction {
                opcode: 35,
                on_true: 254,
                on_false: 255,
                operands,
            },
            c["target"].as_i64().unwrap() as i16,
        );
        v.world.actions = Some(action::WorldState {
            live_family: true,
            ..Default::default()
        });
        v.threads.get_mut(&id).unwrap().stack[0].continuation = c["state"].as_u64().unwrap() as u32;
        let run = v.run_slice(&p, id, 1).unwrap();
        if !e["error"].is_null() {
            assert_eq!(run.stop, Stop::Faulted, "{c}");
            continue;
        }
        assert_eq!(
            matches!(run.stop, Stop::Sleeping),
            e["returned"] == 2,
            "{c}"
        );
        assert_eq!(
            v.effects.len(),
            e["effects"].as_array().unwrap().len(),
            "{c}"
        );
        if let Some(effect) = v.effects.first() {
            let expected = &e["effects"][0];
            assert_eq!(effect.object, expected[1].as_i64().unwrap() as i16);
            assert_eq!(effect.duration_ms, expected[3].as_i64().unwrap() as i32);
            assert_eq!(effect.kind, expected[4].as_u64().unwrap() as u8);
            assert_eq!(effect.size, expected[5].as_u64().unwrap() as u8);
        }
        let mut resumed = Snapshot::capture(&p, &v).restore(&p).unwrap();
        if run.stop == Stop::Sleeping {
            for _ in 0..2 {
                v.advance_tick().unwrap();
                resumed.advance_tick().unwrap();
                assert_eq!(
                    v.run_slice(&p, id, 8).unwrap(),
                    resumed.run_slice(&p, id, 8).unwrap()
                );
            }
            if c["target"] == 0 {
                assert!(matches!(v.threads[&id].status, Status::Faulted(_)));
            } else {
                assert_eq!(v.threads[&id].status, Status::Returned(true));
            }
            assert_eq!(v, resumed);
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_room_area_scope_is_signed_readonly_and_checks_missing_rooms() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/domestic-room.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let e = &row["original"];
        let (p, mut v, id) = setup(
            Instruction {
                opcode: 2,
                on_true: 254,
                on_false: 255,
                operands: [0; 8],
            },
            1,
        );
        let room = c["room"].as_i64().unwrap() as i16;
        v.world.objects.get_mut(&1).unwrap().temps[0] = room;
        if c["present"].as_bool().unwrap() {
            v.world
                .room_half_tiles
                .insert(room, c["half_tiles"].as_i64().unwrap() as i32);
        }
        let result = v.inspect_memory(&p, id, 23, 3, c["write"].as_bool().unwrap());
        if e["error"].is_null() {
            let r = result.unwrap();
            assert_eq!(r.value, e["value"].as_i64().unwrap() as i16, "{c}");
            assert!(!r.writable);
        } else {
            assert!(result.unwrap_err().contains("0x28"), "{c}");
        }
    }
}
