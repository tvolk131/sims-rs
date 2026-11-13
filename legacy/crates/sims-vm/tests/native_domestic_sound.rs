use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_sound_uses_object_context_and_missing_events_are_nonfatal() {
    let report: Value = serde_json::from_str(include_str!("fixtures/domestic-sound.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let routine = |instructions| Routine {
            name: "synthetic".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args: 0,
                locals: 0,
                instructions,
            },
        };
        let mut owner = Module {
            source: "private".into(),
            semiglobal: Some("semi".into()),
            ..Default::default()
        };
        owner.routines.insert(
            4096,
            routine(vec![Instruction {
                opcode: 8192,
                on_true: 254,
                on_false: 255,
                operands: [0; 8],
            }]),
        );
        let mut semi = Module {
            source: "semi".into(),
            ..Default::default()
        };
        semi.routines.insert(
            8192,
            routine(vec![Instruction {
                opcode: 23,
                on_true: 254,
                on_false: 255,
                operands: [2, 0, 0, 1, c["flags"].as_u64().unwrap() as u8, 0, 0, 0],
            }]),
        );
        // Deliberately conflicting lexical sound: the code resource is not the
        // object execution context used by the original FWAV lookup.
        semi.sounds.insert(2, "wrong_lexical_resource".into());
        if c["private"] == true {
            owner.sounds.insert(2, "private_sound".into());
        }
        let mut global = Module {
            source: "global".into(),
            ..Default::default()
        };
        if c["global"] == true {
            global.sounds.insert(2, "global_sound".into());
        }
        let p = Program {
            globals: "global".into(),
            modules: BTreeMap::from([
                ("private".into(), owner),
                ("semi".into(), semi),
                ("global".into(), global),
            ]),
            ..Default::default()
        };
        let mut w = World::default();
        w.objects.insert(1, Object::default());
        w.objects.insert(2, Object::default());
        let mut vm = Vm::new(w, 1);
        let id = vm
            .spawn(
                &p,
                Start {
                    owner: "private",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: c["target"].as_i64().unwrap() as i16,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        vm.run_slice(&p, id, 100).unwrap();
        assert_eq!(vm.threads[&id].status, Status::Returned(true));
        assert_eq!(row["original"]["returned"], 1);
        assert_eq!(
            json!(vm.sounds.iter().map(|s| s.object).collect::<Vec<_>>()),
            row["original"]["played"],
            "{c}"
        );
        if let Some(sound) = vm.sounds.first() {
            assert_eq!(
                sound.name,
                if c["private"] == true {
                    "private_sound"
                } else {
                    "global_sound"
                }
            );
        }
        assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    }
}
