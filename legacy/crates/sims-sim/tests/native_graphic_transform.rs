use serde_json::{Value, json};
use sims_sim::pose::{
    graphic_transform::*,
    model::Matrix,
    transform::{Terrain, View},
};

#[cfg(feature = "private-fixtures")]
#[test]
fn native_base_graphic_placement_preserves_math_and_dependent_callback_order() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-graphic-transform.json")).unwrap();
    for row in fixture["cases"].as_array().unwrap() {
        let c = &row["input"];
        let expected = &row["original"];
        let mut placement: Placement = serde_json::from_value(c.clone()).unwrap();
        let mut matrix: Matrix = serde_json::from_value(c["matrix"].clone()).unwrap();
        let source = Source {
            position: serde_json::from_value(c["position"].clone()).unwrap(),
            facing: c["facing"].as_i64().unwrap() as i16,
            level: c["level"].as_i64().unwrap() as i32,
        };
        let size = c["size"].as_u64().unwrap() as usize;
        let corners: [u8; 4] = serde_json::from_value(c["corners"].clone()).unwrap();
        let heights = (0..size)
            .flat_map(|x| {
                (0..size).map(move |y| corners.map(|v| v.wrapping_add((x * 3 + y * 5) as u8)))
            })
            .collect::<Vec<_>>();
        let view = View {
            rotation: c["rotation"].as_u64().unwrap() as u8,
            floor_vector: serde_json::from_value(
                c["floor_vectors"][c["zoom"].as_u64().unwrap() as usize].clone(),
            )
            .unwrap(),
        };
        let mut update = Update::default();
        let request = update
            .advance(
                &mut placement,
                matrix,
                source,
                view,
                Terrain {
                    size,
                    corners: &heights,
                },
            )
            .unwrap()
            .unwrap();
        let original = (update.clone(), placement.clone(), matrix);
        (update, placement, matrix) =
            serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert_eq!((update.clone(), placement.clone(), matrix), original);
        assert_eq!(
            update
                .advance(
                    &mut placement,
                    matrix,
                    source,
                    view,
                    Terrain {
                        size,
                        corners: &heights
                    }
                )
                .unwrap(),
            Some(request.clone())
        );
        assert_eq!((update.clone(), placement.clone(), matrix), original);
        match &request {
            Request::Publish(result) => {
                let original: Matrix =
                    serde_json::from_value(expected["published"].clone()).unwrap();
                assert_eq!(result.kind, original.kind);
                for (a, b) in result.values.into_iter().zip(original.values) {
                    assert!((a - b).abs() <= 3e-5, "{c}: {a}/{b}");
                }
                assert!(
                    !expected["calls"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|v| v["service"] == "00478f76")
                );
            }
            Request::Position(point) => {
                assert_eq!(json!(point), expected["calls"][0]["set_position"]);
                assert!(expected["published"].is_null());
                if let Some(m) = c["mutation"].as_object() {
                    if let Some(v) = m.get("base") {
                        placement.base = serde_json::from_value(v.clone()).unwrap();
                    }
                    if let Some(v) = m.get("offset") {
                        placement.offset = serde_json::from_value(v.clone()).unwrap();
                    }
                    if let Some(v) = m.get("dirty") {
                        placement.dirty = v.as_bool().unwrap();
                    }
                    if let Some(v) = m.get("explicit") {
                        placement.explicit = v.as_bool().unwrap();
                    }
                    if let Some(v) = m.get("matrix") {
                        matrix = serde_json::from_value(v.clone()).unwrap();
                    }
                }
            }
        }
        let original = (update.clone(), placement.clone(), matrix);
        (update, placement, matrix) =
            serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert_eq!((update.clone(), placement.clone(), matrix), original);
        let wrong = Request::Position([12345, 67890]);
        assert!(update.acknowledge(&mut placement, &wrong).is_err());
        assert_eq!((update.clone(), placement.clone(), matrix), original);
        update.acknowledge(&mut placement, &request).unwrap();
        let completed = (update.clone(), placement.clone());
        assert!(update.acknowledge(&mut placement, &request).is_err());
        assert_eq!((update.clone(), placement.clone()), completed);
        assert!(update.complete());
        assert_eq!(
            placement,
            serde_json::from_value(expected.clone()).unwrap(),
            "{c}"
        );
        assert_eq!(
            matrix,
            serde_json::from_value(expected["matrix"].clone()).unwrap()
        );
        assert_eq!(
            json!(source.position),
            expected["position"],
            "base +78 is a no-op"
        );
    }
}

#[test]
fn invalid_dependent_graphic_coordinate_does_not_publish_partial_state() {
    let mut placement = Placement::constructed();
    placement.explicit = true;
    let mut matrix = Matrix::IDENTITY;
    matrix.values[12] = f32::INFINITY;
    let mut update = Update::default();
    let before = (update.clone(), placement.clone());
    assert!(
        update
            .advance(
                &mut placement,
                matrix,
                Source {
                    position: [0; 2],
                    facing: 0,
                    level: 1
                },
                View {
                    rotation: 0,
                    floor_vector: [0.; 3]
                },
                Terrain {
                    size: 0,
                    corners: &[]
                }
            )
            .is_err()
    );
    assert_eq!((update, placement), before);
}
