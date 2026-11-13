use serde_json::Value;
use sims_sim::pose::{
    model::{Bounds, Matrix},
    model_visibility,
};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_model_visibility_projects_corners_and_preserves_native_screen_edges() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-model-visibility.json")).unwrap();
    for (n, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let values: [f32; 6] = serde_json::from_value(input["bounds"].clone()).unwrap();
        let bounds = Bounds {
            minimum: values[..3].try_into().unwrap(),
            maximum: values[3..].try_into().unwrap(),
        };
        let matrix = |key: &str| serde_json::from_value::<Matrix>(input[key].clone()).unwrap();
        let actual = model_visibility::project(
            bounds,
            matrix("scale"),
            matrix("placement"),
            matrix("camera"),
            serde_json::from_value(input["viewport"].clone()).unwrap(),
        )
        .unwrap();
        let expected: model_visibility::Projection =
            serde_json::from_value(row["original"].clone()).unwrap();
        assert_eq!(actual.visible, expected.visible, "case {n}");
        assert_eq!(actual.rectangle, expected.rectangle, "case {n}");
        assert_eq!(actual.matrix.kind, expected.matrix.kind, "case {n}");
        for (a, b) in actual.matrix.values.into_iter().zip(expected.matrix.values) {
            assert!((a - b).abs() <= 3e-5, "case {n} matrix {a}/{b}");
        }
        assert_eq!(actual.screen.is_some(), expected.screen.is_some());
        if let (Some(a), Some(b)) = (actual.screen, expected.screen) {
            for (a, b) in a.into_iter().zip(b) {
                assert!((a - b).abs() <= 0.001, "case {n} corner {a}/{b}");
            }
        }
    }
}
