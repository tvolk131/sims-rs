use super::*;
use crate::{Body, Point, Room, Snapshot, StepStop};
use sims_formats::{
    animation::{Animation, Samples},
    behavior::{Behavior, Instruction},
};
use sims_vm::{Module, Object, Routine, Start, Vm, World, program::AnimationAsset};
use std::collections::BTreeMap;
fn program(mode: u8) -> Program {
    let names: Vec<_> = (0..11).map(|i| format!("synthetic-turn-{i}")).collect();
    let module = Module {
        source: "person".into(),
        strings: BTreeMap::from([(150, names.clone())]),
        routines: BTreeMap::from([(
            4096,
            Routine {
                name: "turn".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 4,
                    locals: 0,
                    instructions: vec![Instruction {
                        opcode: 22,
                        on_true: 254,
                        on_false: 255,
                        operands: [mode, 0, 0, 0, 0, 0, 0, 0],
                    }],
                },
            },
        )]),
        ..Module::default()
    };
    Program {
        neighbor_resource_flags: None,
        text_language: None,
        resource_storage: None,
        careers: None,
        accessories: BTreeMap::new(),
        globals: "person".into(),
        modules: BTreeMap::from([("person".into(), module)]),
        animations: names
            .into_iter()
            .map(|name| {
                (
                    name.clone(),
                    AnimationAsset {
                        source: "synthetic".into(),
                        content_hash: String::new(),
                        samples_source: String::new(),
                        samples_hash: String::new(),
                        clip: Animation {
                            name: name.clone(),
                            skill: name,
                            duration_ms: 1566.,
                            distance: 0.,
                            moving: false,
                            translation_count: 0,
                            rotation_count: 0,
                            frames: 47,
                            motions: vec![],
                            events: vec![],
                        },
                        samples: Samples {
                            translations: vec![],
                            rotations: vec![],
                        },
                    },
                )
            })
            .collect(),
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_standing_path_state_sequences() {
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../sims-vm/tests/fixtures/turn-timing.json"
    ))
    .unwrap();
    let carried: serde_json::Value =
        serde_json::from_str(include_str!("../tests/fixtures/home-carried-turn.json")).unwrap();
    let p = program(1);
    for row in report["sequences"]
        .as_array()
        .unwrap()
        .iter()
        .chain(carried["sequences"].as_array().unwrap())
    {
        let c = &row["input"];
        let d: [i32; 2] = serde_json::from_value(c["delta"].clone()).unwrap();
        let request = TurnRequest {
            id: 1,
            origin: sims_vm::Location {
                source: "person".into(),
                bhav: 4096,
                instruction: 0,
            },
            caller: 1,
            target: 2,
            mode: 2,
        };
        let mut turn = Turn::new(
            &p,
            &request,
            1,
            [[32, 48], [32, 48], [32 + d[0], 48 + d[1]]],
            c["heading"].as_f64().unwrap() as f32,
            0,
        )
        .unwrap();
        for (i, n) in row["original"].as_array().unwrap().iter().enumerate() {
            turn.progress = [0., 0.5, 1., 1.][i];
            turn.update(&p, i as u64).unwrap();
            let phase = match turn.phase {
                Phase::PathReady => 9,
                Phase::Turning => 4,
                Phase::Complete => 3,
                _ => panic!(),
            };
            assert_eq!(serde_json::json!(phase), n["phase"], "{c} step {i}");
            assert_eq!(
                serde_json::json!(turn.heading.to_bits()),
                n["heading_bits"],
                "{c} step {i}"
            );
            if turn.phase == Phase::Turning {
                assert_eq!(
                    serde_json::json!(turn.base.to_bits()),
                    n["base_bits"],
                    "{c}"
                );
                assert_eq!(
                    serde_json::json!(turn.target.to_bits()),
                    n["target_bits"],
                    "{c}"
                );
                assert_eq!(
                    turn.clip.as_ref().unwrap(),
                    &format!("synthetic-turn-{}", n["clip"].as_u64().unwrap())
                );
            }
        }
    }
}
#[test]
fn turning_persists_every_tick_and_primitive_boundary_without_moving() {
    for mode in 1..=3 {
        for direction in 0..8 {
            let p = program(mode);
            let mut vm = Vm::new(
                World {
                    objects: BTreeMap::from([
                        (
                            1,
                            Object {
                                data: BTreeMap::from([(1, direction), (9, 99)]),
                                person: Some(BTreeMap::from([(0, 0), (58, 30), (65, 0)])),
                                ..Object::default()
                            },
                        ),
                        (
                            2,
                            Object {
                                data: BTreeMap::from([(1, 0)]),
                                ..Object::default()
                            },
                        ),
                    ]),
                    ..World::default()
                },
                1,
            );
            let id = vm
                .spawn(
                    &p,
                    Start {
                        owner: "person",
                        bhav: 4096,
                        caller: 1,
                        callee: 1,
                        stack_object: 2,
                        args: vec![],
                        check: false,
                    },
                )
                .unwrap();
            let bodies = BTreeMap::from([
                (
                    1,
                    Body {
                        level: 1,
                        position: Point::new(48, 32),
                        facing: 1 << direction,
                        radius: 2,
                        elevation: 0,
                    },
                ),
                (
                    2,
                    Body {
                        level: 1,
                        position: Point::new(64, 48),
                        facing: 1,
                        radius: 2,
                        elevation: 0,
                    },
                ),
            ]);
            let mut a = Simulation::new(
                vm,
                Room {
                    width: 96,
                    height: 96,
                    obstacles: vec![],
                },
                bodies,
            )
            .unwrap();
            a.vm.world.camera_rotation = direction as u8 % 4;
            let mut b = a.clone();
            // Save before the spatial service accepts the VM's request.
            b.vm.run_slice(&p, id, 1).unwrap();
            b = Snapshot::capture(&p, &b).restore(&p).unwrap();
            a.run_fixture_tick(&p, 100).unwrap();
            b.run_fixture_tick(&p, 1).unwrap();
            assert_eq!(a, b);
            let mut finished = false;
            for _ in 0..80 {
                b = Snapshot::from_json(
                    &serde_json::to_vec(&Snapshot::capture(&p, &b)).unwrap(),
                    &p,
                )
                .unwrap();
                assert_eq!(a, b);
                if a.turns.values().any(|t| t.phase == Phase::Turning) {
                    let mut bad = Snapshot::capture(&p, &a);
                    bad.state.turns.get_mut(&1).unwrap().progress = 0.777;
                    assert!(bad.restore(&p).is_err());
                }
                a.advance_fixture_tick().unwrap();
                b.advance_fixture_tick().unwrap();
                let one = a.run_fixture_tick(&p, 100).unwrap();
                loop {
                    if b.run_fixture_tick(&p, 1).unwrap().stop != StepStop::Budget {
                        break;
                    }
                }
                assert_eq!(a, b);
                assert_eq!(a.bodies[&1].position, Point::new(48, 32));
                if one.stop == StepStop::Complete {
                    finished = true;
                    break;
                }
            }
            assert!(finished);
            assert_eq!(a.vm.threads[&id].status, sims_vm::Status::Returned(true));
            assert_eq!(
                a.vm.world.objects[&1].data[&9],
                if mode == 1 { 0 } else { 99 }
            );
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_zero_animation_reset_ignores_source_and_clears_person19() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("../tests/fixtures/animation-reset.json")).unwrap();
    for row in report["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["phase"] == 0)
    {
        let mut p = program(0);
        let ins = &mut p
            .modules
            .get_mut("person")
            .unwrap()
            .routines
            .get_mut(&4096)
            .unwrap()
            .body
            .instructions[0];
        ins.opcode = 44;
        ins.operands[4] = row["source"].as_u64().unwrap() as u8;
        let mut vm = Vm::new(
            World {
                objects: BTreeMap::from([(
                    1,
                    Object {
                        data: BTreeMap::from([(1, 0)]),
                        person: Some(BTreeMap::from([
                            (0, 0),
                            (19, 99),
                            (42, 1),
                            (58, 30),
                            (64, 0),
                            (65, 0),
                        ])),
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
            1,
        );
        let id = vm
            .spawn(
                &p,
                Start {
                    owner: "person",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: 1,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        let mut s = Simulation::new(
            vm,
            Room {
                width: 64,
                height: 64,
                obstacles: vec![],
            },
            BTreeMap::from([(
                1,
                Body {
                    level: 1,
                    position: Point::new(32, 32),
                    facing: 1,
                    radius: 2,
                    elevation: 0,
                },
            )]),
        )
        .unwrap();
        assert_eq!(
            s.run_fixture_tick(&p, 100).unwrap().stop,
            StepStop::Complete
        );
        assert_eq!(row["original"]["returned"], 1);
        assert_eq!(s.vm.threads[&id].status, sims_vm::Status::Returned(true));
        assert_eq!(
            serde_json::json!(s.vm.world.objects[&1].person.as_ref().unwrap()[&19]),
            row["original"]["person19"]
        );
        assert!(s.animations[&1].looping);
        Snapshot::capture(&p, &s).restore(&p).unwrap();
    }
}

#[test]
fn fractional_final_turn_frame_does_not_apply_nominal_root_rotation_twice() {
    let p = program(1);
    for (heading, delta) in [(0., [16, 16]), (7., [-16, 0])] {
        let request = TurnRequest {
            id: 1,
            origin: sims_vm::Location {
                source: "person".into(),
                bhav: 4096,
                instruction: 0,
            },
            caller: 1,
            target: 2,
            mode: 2,
        };
        let mut turn = Turn::new(
            &p,
            &request,
            1,
            [[32, 48], [32, 48], [32 + delta[0], 48 + delta[1]]],
            heading,
            0,
        )
        .unwrap();
        turn.phase = Phase::Turning;
        turn.progress = 1.;
        turn.heading = turn.base - 0.01;
        let (_, _, sample) = turn.sample(&p, 1.).unwrap();
        assert_eq!(sample, turn.base * std::f32::consts::FRAC_PI_4);
    }
}
