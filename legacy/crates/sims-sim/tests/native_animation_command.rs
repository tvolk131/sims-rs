mod native_layer_ownership;

use native_layer_ownership::{compare, resources};
use serde_json::{Value, json};
use sims_sim::{
    character::Controls,
    pose::{
        self, LocalBone,
        animator::{Animator, Settings, Slot},
        command::{Input, Lookup, Outcome},
        sampling::BoneInput,
    },
};
use std::collections::BTreeMap;

#[cfg(feature = "private-fixtures")]
#[test]
fn original_commands_keep_layers_events_clocks_and_frame_write_order() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-animation-command.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let (rig, program) = resources();
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let n = |key: &str| c[key].as_i64().unwrap() as i32;
        let flag = |key: &str| c[key].as_bool().unwrap();
        let settings: Settings = serde_json::from_value(c["initial"]["settings"].clone()).unwrap();
        let mut animator = Animator::new(0, 0, 0, false, 123);
        for slot in c["layers"].as_array().unwrap() {
            animator
                .start(
                    serde_json::from_value(slot.clone()).unwrap(),
                    &rig,
                    Some(&program.animations["synthetic"].clip),
                    false,
                    0,
                    settings,
                )
                .unwrap();
        }
        if flag("ended") {
            let mut bones = vec![BoneInput {
                local: LocalBone {
                    translation: [0.; 3],
                    rotation: [0., 0., 0., 1.],
                },
                priority: 0,
                blend_marker: 0,
            }];
            animator
                .begin(2000, true, false, false, &mut bones)
                .unwrap();
            assert!(animator.next(&program, &mut bones).unwrap().is_none());
        }
        let mut value = serde_json::to_value(&animator).unwrap();
        value["numbered_events"] = c["events"].clone();
        if let Some(id) = animator.slot(Slot::Foreground) {
            for entry in value["layers"].as_array_mut().unwrap() {
                if entry["id"] == json!(id) {
                    entry["layer"]["interruptible"] = c["interruptible"].clone();
                }
            }
        }
        animator = serde_json::from_value(value).unwrap();
        let mut controls = Controls {
            ordinary: pose::ordinary::State {
                clock_mode: n("clock_mode") as u8,
                base_posture: n("base_posture"),
                pickup: n("pickup"),
                carry: sims_vm::animation::CarrySelector {
                    mode: n("carry_mode"),
                    selection: n("carry_selection"),
                },
                mode: n("mode"),
                rate: 1.,
                target_rate: 1.,
            },
            movement: pose::movement::Movement::new(pose::construction::State::new(0).movement),
            settings,
            movement_anchors: [None; 2],
            geometry_generation: 0,
            animation_events: n("seen"),
            engaged: n("engaged"),
        };
        controls.movement.state.apply_rate = n("apply_rate") as u8;
        let mut person: BTreeMap<i16, i16> = [
            (0, n("posture") as i16),
            (19, 99),
            (65, n("species") as i16),
        ]
        .into();
        let mut frame: Vec<i16> = serde_json::from_value(c["frame"].clone()).unwrap();
        let mut lookups = Vec::<Value>::new();
        let mut interrupts = 0;
        let mut notifications = 0;
        (animator, controls, person) =
            serde_json::from_slice(&serde_json::to_vec(&(&animator, &controls, &person)).unwrap())
                .unwrap();
        let result = (|| {
            let mut number = n("number") as i16;
            if (0..=3).contains(&controls.ordinary.mode) && n("flags") & 4 != 0 {
                if number < 0 || i32::from(number) >= n("args") {
                    return Err("index".to_string());
                }
                number = frame[number as usize];
            }
            pose::command::invoke(
                &mut animator,
                &mut controls,
                &mut person,
                &rig,
                Input {
                    number,
                    flags: n("flags") as u8,
                    event_count: n("count") as i8,
                    tick: n("tick"),
                    definition_flags: n("definition_flags") as u8,
                },
                |lookup| {
                    lookups.push(json!(lookup));
                    Ok((if lookup == Lookup::Operand {
                        flag("present")
                    } else {
                        flag("posture_present")
                    })
                    .then_some(&program.animations["synthetic"].clip))
                },
                |_| {
                    notifications += 1;
                    Ok(())
                },
                || {
                    interrupts += 1;
                    Ok(flag("interrupt"))
                },
                |event| {
                    let destination = if n("flags") & 32 != 0 {
                        if n("index") >= n("locals") {
                            return Err("local".into());
                        }
                        n("args") + n("index")
                    } else {
                        if n("args") == 0 {
                            return Err("arg".into());
                        }
                        0
                    };
                    frame[destination as usize] = event;
                    Ok(())
                },
            )
        })();
        let actual = json!({"animator":animator,"error":result.as_ref().err().map(|_|8),
            "returned":result.as_ref().ok().map(|r|match r {Outcome::Completed=>1,Outcome::Event(_)=>0,Outcome::Waiting=>2}),
            "frame":frame,"person19":person[&19],"controls":{"mode":controls.ordinary.mode,"base_posture":controls.ordinary.base_posture,
                "pickup":controls.ordinary.pickup,"carry_mode":controls.ordinary.carry.mode,"carry_selection":controls.ordinary.carry.selection,
                "engaged":controls.engaged,"seen":controls.animation_events},"clock_mode":controls.ordinary.clock_mode,
            "apply_rate":controls.movement.state.apply_rate,"lookups":lookups,"interrupts":interrupts,"notifications":notifications});
        compare(&actual, &row["original"], &format!("case {case} {c}"));
        let saved = serde_json::to_vec(&(&animator, &controls, &person)).unwrap();
        let restored: (Animator, Controls, BTreeMap<i16, i16>) =
            serde_json::from_slice(&saved).unwrap();
        assert_eq!(
            (animator, controls, person),
            restored,
            "case {case} restored state"
        );
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 475);
}
