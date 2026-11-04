use serde_json::Value;
use sims_sim::pose::transform::{Placement, Source, Terrain, View};

#[cfg(feature = "private-fixtures")]
#[test]
fn native_animator_transform_preserves_cache_terrain_floor_and_facing_semantics() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-animator-transform.json")).unwrap();
    let rows = report["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 256);
    for (case, row) in rows.iter().enumerate() {
        let input = &row["input"];
        let expected = &row["original"];
        let size = input["size"].as_u64().unwrap() as usize;
        let corners: [u8; 4] = serde_json::from_value(input["corners"].clone()).unwrap();
        let heights = (0..size)
            .flat_map(|x| {
                (0..size).map(move |y| corners.map(|v| v.wrapping_add((x * 3 + y * 5) as u8)))
            })
            .collect::<Vec<_>>();
        let mut placement = Placement {
            base: serde_json::from_value(input["base"].clone()).unwrap(),
            previous: serde_json::from_value(input["previous"].clone()).unwrap(),
            offset: serde_json::from_value(input["offset"].clone()).unwrap(),
            dirty: input["dirty"].as_bool().unwrap(),
        };
        if input["reset"].as_bool().unwrap() {
            placement.invalidate();
            let reset: Placement = serde_json::from_value(expected["reset"].clone()).unwrap();
            assert_eq!(placement, reset, "{case} reset");
        }
        let source = Source {
            position: serde_json::from_value(input["position"].clone()).unwrap(),
            facing: input["facing"].as_i64().unwrap() as i16,
            level: input["level"].as_i64().unwrap() as i32,
            flags: serde_json::from_value(input["flags"].clone()).unwrap(),
        };
        let view = View {
            rotation: input["rotation"].as_u64().unwrap() as u8,
            floor_vector: serde_json::from_value(
                input["floor_vectors"][input["zoom"].as_u64().unwrap() as usize].clone(),
            )
            .unwrap(),
        };
        let transform = placement
            .transform(
                source,
                view,
                Terrain {
                    size,
                    corners: &heights,
                },
            )
            .unwrap();
        assert_eq!(
            serde_json::json!(placement.dirty),
            expected["dirty"],
            "{case} dirty"
        );
        for (key, actual) in [
            ("base", placement.base.as_slice()),
            ("previous", placement.previous.as_slice()),
            ("matrix", transform.matrix.as_slice()),
        ] {
            for (i, (a, b)) in actual
                .iter()
                .zip(expected[key].as_array().unwrap())
                .enumerate()
            {
                let b = b.as_f64().unwrap() as f32;
                assert!((*a - b).abs() <= 3e-5, "case {case} {key}[{i}]: {a} / {b}");
            }
        }
        let calls = expected["calls"].as_array().unwrap();
        assert_eq!(
            serde_json::json!(transform.terrain_point),
            calls[0]["point"],
            "{case}"
        );
        let pool = calls
            .iter()
            .find(|c| c["service"] == "00478f76")
            .map(|c| c["point"].clone());
        assert_eq!(
            transform.pool_query.map(|p| serde_json::json!(p)),
            pool,
            "{case}"
        );
        let mut restored: Placement =
            serde_json::from_slice(&serde_json::to_vec(&placement).unwrap()).unwrap();
        assert_eq!(placement, restored);
        assert_eq!(
            transform,
            restored
                .transform(
                    source,
                    view,
                    Terrain {
                        size,
                        corners: &heights
                    }
                )
                .unwrap(),
            "{case} repeated placement"
        );
    }
}
