//! Native predicate vectors use synthetic interaction-use lists, not menu queues.
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;

#[cfg(feature = "private-fixtures")]
#[test]
fn original_interaction_use_records_match_live_frames() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/test-interacting.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut operands = [0xa5; 8];
        operands[0] = c["mode"].as_u64().unwrap() as u8;
        operands[1] = 7;
        operands[2..4].copy_from_slice(&(c["selected"].as_i64().unwrap() as i16).to_le_bytes());
        let p = Program {
            globals: "probe".into(),
            modules: [(
                "probe".into(),
                Module {
                    source: "probe".into(),
                    routines: [(
                        4096,
                        Routine {
                            name: "use query".into(),
                            body: Behavior {
                                format: 0x8002,
                                kind: 0,
                                args: 4,
                                locals: 0,
                                instructions: vec![Instruction {
                                    opcode: 0x25,
                                    on_true: 0,
                                    on_false: 0,
                                    operands,
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
        let mut w = World::default();
        for actor in [1, 2] {
            w.objects.insert(
                actor,
                Object {
                    person: (actor == 1 || c["other_person"] == true).then(Default::default),
                    ..Default::default()
                },
            );
        }
        let action = action::Action {
            target: 2,
            icon_owner: 2,
            priority: 100,
            ..Default::default()
        };
        w.actions = Some(action::WorldState {
            actors: [(
                1,
                action::Actor {
                    active: Some(action.clone()),
                    queue: action::Queue {
                        pending: [action].into(),
                    },
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        });
        let mut v = Vm::new(w, 1);
        let mut threads = vec![];
        for actor in [1, 2] {
            let id = v
                .spawn(
                    &p,
                    Start {
                        owner: "probe",
                        bhav: 4096,
                        caller: actor,
                        callee: actor,
                        stack_object: c["target"].as_i64().unwrap() as i16,
                        args: vec![0; 4],
                        check: false,
                    },
                )
                .unwrap();
            threads.push(id);
            let t = v.threads.get_mut(&id).unwrap();
            let template = t.stack[0].clone();
            for target in c["records"][(actor - 1) as usize].as_array().unwrap() {
                let mut frame = template.clone();
                frame.storage_offset = t.stack.len() * 12;
                frame.action_target = Some(target.as_i64().unwrap() as i16);
                t.stack.push(frame);
            }
            t.frame_memory.resize(t.stack.len() * 12, Some(0));
        }
        let before_other = v.threads[&threads[1]].clone();
        let before_world = v.world.clone();
        let result = v.run_slice(&p, threads[0], 1).unwrap();
        if row["original"]["error"].is_null() {
            assert_eq!(result.stop, Stop::Budget, "{c}: {:?}", result.trace);
            assert_eq!(
                v.threads[&threads[0]].last_result,
                row["original"]["returned"].as_bool().unwrap(),
                "{c}"
            );
        } else {
            assert_eq!(result.stop, Stop::Faulted, "{c}");
            assert!(
                result.trace[0].effect.contains("0x1c"),
                "{c}: {:?}",
                result.trace
            );
        }
        assert_eq!(v.world, before_world, "predicate changed world: {c}");
        assert_eq!(
            v.threads[&threads[1]], before_other,
            "predicate changed other person: {c}"
        );
    }
}
