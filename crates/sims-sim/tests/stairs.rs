use serde_json::{Value, json};
use sims_sim::{
    Point, Rect,
    architecture::{Architecture, FloorPlan, Wall},
    portals::{PairPart, partner},
};
#[cfg(feature = "private-fixtures")]
#[test]
fn original_multistorey_part_placement_and_portal_partners() {
    let report: Value = serde_json::from_str(include_str!("fixtures/stairs.json")).unwrap();
    for row in report["placement"].as_array().unwrap() {
        let c = &row["input"];
        let origins: Vec<[i32; 3]> = serde_json::from_value(c["origins"].clone()).unwrap();
        let direction = c["direction"].as_i64().unwrap() as i16;
        let offsets: Vec<_> = origins
            .iter()
            .map(|o| {
                sims_vm::multitile::rotate(std::array::from_fn(|i| o[i] - origins[0][i]), direction)
            })
            .collect();
        let selected = c["selected"].as_u64().unwrap() as usize - 1;
        for (i, &offset) in offsets.iter().enumerate() {
            let (position, level) = sims_vm::multitile::placement(
                serde_json::from_value(c["position"].clone()).unwrap(),
                c["level"].as_i64().unwrap() as i32,
                offsets[selected],
                offset,
            );
            assert_eq!(
                json!({"part":i+1,"position":position,"level":level}),
                row["original"]["moves"][i],
                "{c}"
            );
        }
    }
    for row in report["pair"].as_array().unwrap() {
        let c = &row["input"];
        let raw = c["parts"].as_array().unwrap();
        let parts: Vec<_> = raw
            .iter()
            .enumerate()
            .map(|(i, p)| PairPart {
                id: i as i16 + 1,
                head: (i != 0).then_some(1),
                next: (i + 1 < raw.len()).then_some(i as i16 + 2),
                style: c["style"].as_i64().unwrap() as i16,
                base_graphic: p["base"].as_i64().unwrap() as i16,
                portal: p["portal"].as_i64().unwrap() != 0,
            })
            .collect();
        assert_eq!(
            i64::from(partner(&parts, c["selected"].as_i64().unwrap() as i16).unwrap_or(0)),
            row["original"].as_i64().unwrap(),
            "{c}"
        );
    }
}
#[test]
fn storeys_have_independent_walls_rooms_and_floor_holes() {
    let ground = FloorPlan {
        walls: vec![Wall {
            a: Point::new(16, 0),
            b: Point::new(16, 16),
            door: None,
        }],
        supported: vec![true; 16],
    };
    let mut upper = FloorPlan {
        walls: vec![],
        supported: vec![true; 16],
    };
    upper.supported[5] = false;
    let a = Architecture::with_floors(64, 64, vec![ground, upper], &Default::default()).unwrap();
    assert_ne!(
        a.room_at(1, Point::new(8, 8)),
        a.room_at(2, Point::new(8, 8))
    );
    let across_wall = Rect {
        min: Point::new(13, 5),
        max: Point::new(19, 11),
    };
    assert!(!a.bounds_clear(1, across_wall, &[]));
    assert!(a.bounds_clear(2, across_wall, &[]));
    let hole = Rect {
        min: Point::new(21, 21),
        max: Point::new(27, 27),
    };
    assert!(a.bounds_clear(1, hole, &[]));
    assert!(!a.bounds_clear(2, hole, &[]));
    assert!(!a.bounds_clear(3, across_wall, &[]));
}

#[cfg(feature = "private-fixtures")]
#[test]
fn routing_slot_snap_uses_native_target_floor_and_failure_branch() {
    use sims_formats::{
        behavior::{Behavior, Instruction},
        execution::{Slot, Slots},
    };
    use sims_sim::{Body, Room, Simulation, StepStop};
    use sims_vm::{Definition, Module, Object, Program, Routine, Start, Status, Vm, World};
    use std::collections::BTreeMap;
    let report: Value = serde_json::from_str(include_str!("fixtures/stairs.json")).unwrap();
    for row in report["snap"].as_array().unwrap() {
        let c = &row["input"];
        let direction = c["rotation"].as_u64().unwrap() as u8;
        let level = c["target_level"].as_i64().unwrap() as i32;
        let success = c["success"].as_i64().unwrap() == 1;
        let slot = Slot {
            kind: 3,
            offset: [0.; 3],
            standing: 0,
            sitting: 0,
            ground: 0,
            flags: 2048,
            snap_target: -1,
            proximity: [16; 3],
            max_size: 0,
            unknown: 0,
            gradient: 0.1875,
            height: 0,
            facing: -2,
            resolution: 16,
        };
        let mut module = Module {
            source: "synthetic".into(),
            ..Default::default()
        };
        module.slots.insert(
            128,
            Slots {
                version: 10,
                entries: vec![slot],
            },
        );
        module.routines.insert(
            4096,
            Routine {
                name: "synthetic Snap".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 0,
                    locals: 0,
                    instructions: vec![Instruction {
                        opcode: 0x2e,
                        on_true: 254,
                        on_false: 255,
                        operands: [0, 0, 3, 0, c["flags"].as_u64().unwrap() as u8, 0, 0, 0],
                    }],
                },
            },
        );
        let mut world = World::default();
        for id in [1, 2] {
            let mut words = vec![0; 108];
            words[14] = id;
            words[9] = 4;
            words[0x86 / 2] = 6;
            module.object_definition_words.insert(id as u16, words);
            world.objects.insert(
                id,
                Object {
                    definition: Some(Definition {
                        source: "synthetic".into(),
                        object: id as u16,
                        guid: id as u32,
                        slots: 128,
                    }),
                    person: (id == 1).then(|| BTreeMap::from([(0, 0), (37, 0), (62, 0), (73, 0)])),
                    data: BTreeMap::from([
                        (1, i16::from(direction)),
                        (8, if id == 2 { 16 } else { 0 }),
                        (29, 0),
                        (52, 0),
                        (54, 0),
                    ]),
                    ..Default::default()
                },
            );
        }
        let p = Program {
            globals: "synthetic".into(),
            modules: BTreeMap::from([("synthetic".into(), module)]),
            ..Default::default()
        };
        let body = |x, y| Body {
            level: 1,
            position: Point::new(x, y),
            facing: 1 << direction,
            radius: 3,
            elevation: 0,
        };
        let mut s = Simulation::new(
            Vm::new(world, 1),
            Room {
                width: 128,
                height: 128,
                obstacles: vec![],
            },
            BTreeMap::from([(1, body(48, 32)), (2, body(96, 80))]),
        )
        .unwrap();
        s.bodies.get_mut(&2).unwrap().level = level;
        let mut floors = vec![
            FloorPlan {
                walls: vec![],
                supported: vec![true; 64]
            };
            2
        ];
        if !success {
            floors[level as usize - 1].supported[5 * 8 + 6] = false;
        }
        s.set_floors(&p, floors).unwrap();
        let thread =
            s.vm.spawn(
                &p,
                Start {
                    owner: "synthetic",
                    bhav: 4096,
                    caller: 1,
                    callee: 2,
                    stack_object: 2,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        let step = s.run_fixture_tick(&p, 100).unwrap();
        assert!(
            !matches!(step.stop, StepStop::Faulted | StepStop::Budget),
            "{c}: {step:?}"
        );
        assert_eq!(
            s.vm.threads[&thread].status,
            Status::Returned(row["original"]["returned"] == 1),
            "{c}"
        );
        if success {
            assert_eq!(
                json!([
                    s.bodies[&1].position.y,
                    s.bodies[&1].position.x,
                    s.bodies[&1].level
                ]),
                json!(row["original"]["placement"].as_array().unwrap()[..3]),
                "{c}"
            );
        } else {
            assert_eq!(s.bodies[&1], body(48, 32));
        }
    }
}
