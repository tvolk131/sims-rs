use serde::Deserialize;
use sims_sim::pose::portrait::{Camera, Transform, collect_models};

#[cfg(feature = "private-fixtures")]
#[test]
fn native_portrait_crop_preserves_padding_rounding_and_edge_expansion() {
    use sims_sim::pose::portrait::{Crop, CropInput};
    #[derive(Deserialize)]
    struct Row {
        input: CropInput,
        original: Crop,
    }
    #[derive(Deserialize)]
    struct Fixture {
        cases: Vec<Row>,
    }
    let f: Fixture = serde_json::from_str(include_str!("fixtures/portrait-crop.json")).unwrap();
    assert_eq!(f.cases.len(), 240);
    for (i, row) in f.cases.into_iter().enumerate() {
        let actual = row.input.crop().unwrap();
        assert_eq!(actual, row.original, "case {i}: {:?}", row.input);
        let restored: CropInput =
            serde_json::from_slice(&serde_json::to_vec(&row.input).unwrap()).unwrap();
        assert_eq!(restored.crop().unwrap(), actual);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_matrix_point_preserves_projective_division_and_cached_tags() {
    #[derive(Deserialize)]
    struct Row {
        matrix: sims_sim::pose::model::Matrix,
        point: [f32; 3],
        original: [f32; 3],
    }
    #[derive(Deserialize)]
    struct Fixture {
        cases: Vec<Row>,
    }
    let f: Fixture =
        serde_json::from_str(include_str!("fixtures/native-matrix-point.json")).unwrap();
    assert_eq!(f.cases.len(), 320);
    for (i, row) in f.cases.into_iter().enumerate() {
        assert_eq!(
            row.matrix.point(row.point).unwrap(),
            row.original,
            "case {i}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_head_selection_preserves_exact_names_order_duplicates_and_nulls() {
    #[derive(Deserialize)]
    struct Binding {
        name: Vec<u8>,
        model: u32,
    }
    #[derive(Deserialize)]
    struct Row {
        input: Vec<Vec<Binding>>,
        original: Vec<u32>,
    }
    #[derive(Deserialize)]
    struct Fixture {
        cases: Vec<Row>,
    }
    let f: Fixture = serde_json::from_str(include_str!("fixtures/portrait-models.json")).unwrap();
    assert_eq!(f.cases.len(), 99);
    for row in f.cases {
        let mut output = vec![999, 777];
        collect_models(
            row.input
                .iter()
                .map(|a| a.iter().map(|b| (b.name.as_slice(), b.model))),
            &mut output,
        );
        assert_eq!(output, row.original);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_portrait_camera_preserves_projection_tags_and_matrix_order() {
    #[derive(Deserialize)]
    struct Row {
        input: Camera,
        original: Transform,
    }
    #[derive(Deserialize)]
    struct Fixture {
        cases: Vec<Row>,
    }
    let f: Fixture = serde_json::from_str(include_str!("fixtures/portrait-camera.json")).unwrap();
    assert_eq!(f.cases.len(), 144);
    for (i, row) in f.cases.into_iter().enumerate() {
        let actual = row.input.transform().unwrap();
        assert_eq!(actual, row.original, "case {i}: {:?}", row.input);
        let restored: Camera =
            serde_json::from_slice(&serde_json::to_vec(&row.input).unwrap()).unwrap();
        assert_eq!(restored.transform().unwrap(), actual);
    }
}
