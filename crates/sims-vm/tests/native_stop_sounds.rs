//! Original source-selection vectors with a synthetic mixer boundary.
use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_stop_sound_source_and_ordered_commands() {
    let report: Value = serde_json::from_str(include_str!("fixtures/stop-sounds.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = |k: &str| c[k].as_i64().unwrap() as i16;
        let mut b = [1, 2, 3, 4, 5, 6, 7, 8];
        b[..2].copy_from_slice(&n("selector").to_le_bytes());
        let p = Program {
            globals: "test".into(),
            modules: BTreeMap::from([(
                "test".into(),
                Module {
                    source: "test".into(),
                    routines: BTreeMap::from([(
                        4096,
                        Routine {
                            name: "stop".into(),
                            body: Behavior {
                                format: 0x8002,
                                kind: 0,
                                args: 0,
                                locals: 0,
                                instructions: vec![Instruction {
                                    opcode: 48,
                                    on_true: 254,
                                    on_false: 255,
                                    operands: b,
                                }],
                            },
                        },
                    )]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let mut world = World::default();
        world.objects.insert(n("caller"), Object::default());
        let mut vm = Vm::new(world, 1);
        let id = vm
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: n("caller"),
                    callee: n("caller"),
                    stack_object: n("target"),
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        // A preceding play must retain its place before the later stop, even when
        // both commands are drained after this tick.
        vm.sounds.push(SoundEvent {
            tick: 0,
            object: n("caller"),
            stop: false,
            name: "synthetic".into(),
            flags: 0,
            sample_rate: 0,
            volume: 0,
        });
        vm.run_slice(&p, id, 1).unwrap();
        assert_eq!(vm.threads[&id].status, Status::Returned(true));
        assert!(row["original"]["error"].is_null());
        assert_eq!(row["original"]["returned"], 1);
        assert_eq!(json!([vm.sounds[1].object]), row["original"]["stopped"]);
        assert!(!vm.sounds[0].stop && vm.sounds[1].stop);
        assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    }
}
