use serde_json::Value;
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{
    Module, Object, Program, Routine, Snapshot, Start, Status, Vm, World, autonomy::Placement,
};

#[cfg(feature = "private-fixtures")]
#[test]
fn generic_tile_depth_matches_native_camera_rotation_and_target_lookup() {
    let report: Value = serde_json::from_str(include_str!("fixtures/tile-depth.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 485);
    let p = Program {
        modules: [(
            "test".into(),
            Module {
                source: "test".into(),
                routines: [(
                    4096,
                    Routine {
                        name: "Tile depth".into(),
                        body: Behavior {
                            format: 0x8002,
                            kind: 0,
                            args: 4,
                            locals: 0,
                            instructions: vec![Instruction {
                                opcode: 1,
                                on_true: 254,
                                on_false: 255,
                                operands: [12, 0, 255, 255, 128, 128, 123, 0],
                            }],
                        },
                    },
                )]
                .into(),
                ..Default::default()
            },
        )]
        .into(),
        ..Default::default()
    };
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let mut vm = Vm::new(
            World {
                camera_rotation: c["camera"].as_u64().unwrap() as u8,
                objects: [
                    (
                        1,
                        Object {
                            temps: [1, 2, 3, 0, 7, -1, 8, 255],
                            ..Default::default()
                        },
                    ),
                    (2, Object::default()),
                ]
                .into(),
                positions: [
                    (
                        1,
                        Placement {
                            x: 11 * 16 + 9,
                            y: 7 * 16 + 3,
                            level: 1,
                        },
                    ),
                    (
                        2,
                        Placement {
                            x: c["tile"][1].as_i64().unwrap() as i32 * 16 + 5,
                            y: c["tile"][0].as_i64().unwrap() as i32 * 16 + 13,
                            level: c["level"].as_i64().unwrap() as i32,
                        },
                    ),
                ]
                .into(),
                ..Default::default()
            },
            1,
        );
        let id = vm
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: c["target"].as_i64().unwrap() as i16,
                    args: vec![0; 4],
                    check: false,
                },
            )
            .unwrap();
        assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
        let positions = vm.world.positions.clone();
        vm.run_slice(&p, id, 1).unwrap();
        let expected: [i16; 8] = serde_json::from_value(n["temps"].clone()).unwrap();
        assert_eq!(vm.world.objects[&1].temps, expected, "{c}");
        assert_eq!(vm.world.positions, positions);
        if let Some(error) = n["error"].as_u64() {
            let Status::Faulted(f) = &vm.threads[&id].status else {
                panic!("{c}")
            };
            assert!(
                f.message.contains(&format!("0x{error:02x}")),
                "{c}: {}",
                f.message
            );
        } else {
            assert_eq!(vm.threads[&id].status, Status::Returned(true), "{c}");
        }
        let mut invalid = Snapshot::capture(&p, &vm);
        invalid.state.world.camera_rotation = 4;
        assert!(invalid.restore(&p).unwrap_err().contains("camera rotation"));
    }
}
