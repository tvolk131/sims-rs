use serde_json::{Value, json};
use sims_sim::{
    head_look::rig::AbsoluteBone,
    pose::{
        animator::{Animator, Settings, Slot},
        layer::{Layer, Start},
        model::Matrix,
        root_motion::Source,
        transform::Placement,
    },
};

fn animator(c: &Value) -> Animator {
    let mut animator = Animator::new(0, 0, 0, true, -1);
    if c["foreground"] == true {
        animator
            .restore_layer(
                Slot::Foreground,
                Some(
                    Layer::new(
                        "synthetic".into(),
                        1000.,
                        &[],
                        &[],
                        Start {
                            priority: 1,
                            speed: 1.,
                            weight: 1.,
                            now: 0,
                        },
                    )
                    .unwrap(),
                ),
                0,
                Settings {
                    fade_enabled: false,
                    fade_duration: 200,
                    sampling_config: 1,
                },
            )
            .unwrap();
    }
    let mut v = json!(animator);
    v["anchor_valid"] = c["anchor_valid"].clone();
    v["anchor_point"] = c["anchor_point"].clone();
    v["anchor_bone"] = if c["anchor_bone"] == true {
        json!(0)
    } else {
        Value::Null
    };
    serde_json::from_value(v).unwrap()
}
fn placement(c: &Value) -> Placement {
    Placement {
        base: serde_json::from_value(c["base"].clone()).unwrap(),
        previous: [7., 8., 9.],
        offset: [1., 2., 3.],
        dirty: true,
    }
}
fn compare(actual: &Value, expected: &Value, label: &str) {
    for key in ["base", "anchor_point"] {
        for (a, b) in actual[key]
            .as_array()
            .unwrap()
            .iter()
            .zip(expected[key].as_array().unwrap())
        {
            assert!(
                (a.as_f64().unwrap() - b.as_f64().unwrap()).abs() <= 3e-5,
                "{label}.{key}: {a}/{b}"
            );
        }
    }
    assert_eq!(actual["anchor_valid"], expected["anchor_valid"], "{label}");
}
fn step(animator: &mut Animator, placement: &mut Placement, row: &Value, label: &str) -> bool {
    let c = &row["input"];
    let source: Source = serde_json::from_value(c["source"].clone()).unwrap();
    let bone: AbsoluteBone = serde_json::from_value(c["bone"].clone()).unwrap();
    let matrix: Matrix = serde_json::from_value(c["matrix"].clone()).unwrap();
    let mut state = json!(animator);
    state["anchor_bone"] = if c["anchor_bone"] == true {
        json!(0)
    } else {
        Value::Null
    };
    *animator = serde_json::from_value(state).unwrap();
    let mut restored: (Animator, Placement) =
        serde_json::from_slice(&serde_json::to_vec(&(&*animator, &*placement)).unwrap()).unwrap();
    let before = placement.clone();
    let corrected = animator
        .correct_root(&[bone], placement, matrix, source)
        .unwrap();
    assert_eq!(
        corrected,
        restored
            .0
            .correct_root(&[bone], &mut restored.1, matrix, source)
            .unwrap()
    );
    assert_eq!((&*animator, &*placement), (&restored.0, &restored.1));
    assert_eq!(
        (placement.previous, placement.offset, placement.dirty),
        (before.previous, before.offset, before.dirty)
    );
    let a = json!(animator);
    let actual = json!({"base":placement.base,"anchor_valid":a["anchor_valid"],"anchor_point":a["anchor_point"]});
    compare(&actual, &row["original"], label);
    let calls = row["placements"].as_array().unwrap();
    assert_eq!(
        usize::from(corrected),
        calls.len(),
        "placement request {label}"
    );
    for call in calls {
        compare(&actual, call, label);
    }
    corrected
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_root_anchor_correction_preserves_gates_reference_and_restore() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-root-motion.json")).unwrap();
    let mut corrected = 0;
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        corrected += usize::from(step(
            &mut animator(c),
            &mut placement(c),
            row,
            &format!("case {i}"),
        ));
    }
    let mut updates = 0;
    for (i, sequence) in report["sequences"].as_array().unwrap().iter().enumerate() {
        let c = &sequence[0]["input"];
        let mut animator = animator(c);
        let mut placement = placement(c);
        for (j, row) in sequence.as_array().unwrap().iter().enumerate() {
            corrected += usize::from(step(
                &mut animator,
                &mut placement,
                row,
                &format!("sequence {i}/{j}"),
            ));
            updates += 1;
        }
    }
    assert!(report["cases"].as_array().unwrap().len() >= 464 && updates >= 128 && corrected > 100);
    eprintln!(
        "464 native root cases, {updates} retained-state updates, {corrected} placement requests"
    );
}
