use serde::{Deserialize, Serialize};
use serde_json::Value;
use sims_sim::pose::model::{Matrix, Model};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct State {
    model: Model,
    animator: Matrix,
    skeleton: Matrix,
    pose_generation: u32,
}
fn apply(state: &mut State, a: &Value) {
    match a["op"].as_str().unwrap() {
        "scale" => state
            .model
            .set_scale(serde_json::from_value(a["value"].clone()).unwrap())
            .unwrap(),
        "publish" => {
            let matrix: Matrix = serde_json::from_value(a["matrix"].clone()).unwrap();
            state.animator = matrix;
            state.skeleton = state.model.publish(matrix).unwrap();
            state.pose_generation = state.pose_generation.wrapping_add(1);
        }
        "hidden" => {
            let matrix: Matrix = serde_json::from_value(a["matrix"].clone()).unwrap();
            state.skeleton = state.model.hidden(matrix.values).unwrap();
            state.pose_generation = state.pose_generation.wrapping_add(1);
        }
        other => panic!("unexpected operation {other}"),
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_scale_publication_and_hidden_model_transform() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-model-placement.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let mut steps = 0;
    for (i, row) in report["sequences"].as_array().unwrap().iter().enumerate() {
        let mut state: State = serde_json::from_value(row["initial"].clone()).unwrap();
        for (j, step) in row["steps"].as_array().unwrap().iter().enumerate() {
            let mut restored: State =
                serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
            apply(&mut state, &step["action"]);
            apply(&mut restored, &step["action"]);
            assert_eq!(state, restored, "restored {i}/{j}");
            let native: State = serde_json::from_value(step["original"].clone()).unwrap();
            assert_eq!(state, native, "native model {i}/{j}");
            state = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
            steps += 1;
        }
    }
    assert_eq!(steps, 672);
}
