mod native_layer_ownership;

use native_layer_ownership::{compare, resources};
use serde_json::{Value, json};
use sims_sim::{
    character::Controls,
    pose::{
        self, LocalBone,
        animator::{Animator, Settings},
        reach::{self, Resource},
        sampling::BoneInput,
    },
};

fn snapshot(animator: &Animator, controls: &Controls, profile_dirty: bool) -> Value {
    json!({"animator":animator,"controls":{
        "mode":controls.ordinary.mode,"base_posture":controls.ordinary.base_posture,
        "pickup":controls.ordinary.pickup,"carry_mode":controls.ordinary.carry.mode,
        "carry_selection":controls.ordinary.carry.selection,
        "engaged":controls.engaged,"seen":controls.animation_events},
        "clock_mode":controls.ordinary.clock_mode,"profile_dirty":profile_dirty})
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_reach_selection_start_finish_preserve_layer_and_event_order() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-reach-animator.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let (rig, program) = resources();
    let clip = &program.animations["synthetic"].clip;
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
                    Some(clip),
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
        let mut profile_dirty = flag("profile_dirty");
        let mut calls = vec![];
        let result = match c["op"].as_str().unwrap() {
            "start" => {
                if let Some(mut selection) = reach::prepare(
                    c["height"].as_f64().unwrap() as f32,
                    n("posture") as i16,
                    n("age") as i16,
                    &mut profile_dirty,
                ) {
                    if selection.reload_profile {
                        calls.push(json!({"service":"reload", "state":snapshot(&animator,&controls,profile_dirty)}));
                        if let Some(age) = c["reload_age"].as_i64() {
                            selection.reread_age(age as i16);
                        }
                    }
                    // Save at the profile/lookup handoff, before the original
                    // has cleared numbered events or replaced the foreground.
                    let saved =
                        serde_json::to_vec(&(&animator, &controls, profile_dirty, selection))
                            .unwrap();
                    let restored: (Animator, Controls, bool, reach::Selection) =
                        serde_json::from_slice(&saved).unwrap();
                    assert_eq!(
                        (&animator, &controls, profile_dirty, selection),
                        (&restored.0, &restored.1, restored.2, restored.3)
                    );
                    (animator, controls, profile_dirty) = (restored.0, restored.1, restored.2);
                    calls.push(json!({"service":"strings", "table":selection.strings,
                        "number":selection.index+1, "state":snapshot(&animator,&controls,profile_dirty)}));
                    let name = c["name"].as_str().filter(|name| !name.is_empty());
                    calls.push(json!({"service":"resolve","name":name}));
                    let resource = if name.is_none() || !flag("present") {
                        Resource::Absent
                    } else if !flag("ready") {
                        Resource::NotReady
                    } else {
                        Resource::Ready(clip)
                    };
                    json!(
                        reach::start_selected(
                            &mut animator,
                            &mut controls,
                            &rig,
                            resource,
                            n("forward") as u8,
                            n("tick")
                        )
                        .unwrap()
                    )
                } else {
                    json!(false)
                }
            }
            "finished" => json!(reach::finished(&animator).unwrap()),
            "finish" => {
                reach::finish(&mut animator, &mut controls, n("posture") as i16, n("tick"))
                    .unwrap();
                Value::Null
            }
            op => panic!("unknown operation {op}"),
        };
        let mut actual = snapshot(&animator, &controls, profile_dirty);
        actual["result"] = result;
        actual["calls"] = json!(calls);
        compare(&actual, &row["original"], &format!("case {case} {c}"));
        let restored: (Animator, Controls, bool) = serde_json::from_slice(
            &serde_json::to_vec(&(&animator, &controls, profile_dirty)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            (animator, controls, profile_dirty),
            restored,
            "restore {case}"
        );
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 384);
}
