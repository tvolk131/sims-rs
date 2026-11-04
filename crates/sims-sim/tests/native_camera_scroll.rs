use serde_json::Value;
use sims_sim::lot_presenter::scroll::{self, Context, Cursor, State};

fn compare_extrema(report: Value, count: usize) {
    assert_eq!(report["cases"].as_array().unwrap().len(), count);
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let heights: Vec<[u8; 4]> = serde_json::from_value(c["heights"].clone()).unwrap();
        let actual = scroll::terrain_extrema(
            c["size"].as_i64().unwrap() as i32,
            serde_json::from_value(c["origin"].clone()).unwrap(),
            c["rotation"].as_u64().unwrap() as u8,
            &heights,
        )
        .unwrap();
        assert_eq!(serde_json::to_value(actual).unwrap(), row["original"]);
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_camera_terrain_extrema() {
    compare_extrema(
        serde_json::from_str(include_str!("fixtures/camera-terrain.json")).unwrap(),
        32,
    );
}
#[test]
#[ignore = "requires private original-house terrain oracle"]
fn original_house_camera_terrain_extrema() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/house-camera-terrain.json");
    compare_extrema(
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap(),
        12,
    );
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_bounds_scroll_and_renderer_sync_with_restore() {
    let report: Value = serde_json::from_str(include_str!("fixtures/camera-scroll.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 384);
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut context: Context = serde_json::from_value(c["context"].clone()).unwrap();
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let bounds = scroll::bounds(&context).unwrap();
        assert_eq!(
            serde_json::to_value(bounds).unwrap(),
            row["original"]["bounds"],
            "bounds {n}"
        );
        let mut cursor = Cursor::begin(
            &mut state,
            &context,
            serde_json::from_value(c["target"].clone()).unwrap(),
            c["immediate"].as_u64().unwrap() as u8,
            c["renderer_ready"].as_bool().unwrap(),
            c["rotation"].as_u64().unwrap() as u32,
        )
        .unwrap();
        let mut requests = vec![];
        if let Some(request) = cursor.request().cloned() {
            (state, context, cursor) =
                serde_json::from_slice(&serde_json::to_vec(&(&state, &context, &cursor)).unwrap())
                    .unwrap();
            assert_eq!(cursor.request(), Some(&request));
            let before = cursor.clone();
            let mut wrong = request.clone();
            wrong.origin[0] = wrong.origin[0].wrapping_add(1);
            assert!(cursor.acknowledge(&wrong).is_err());
            assert_eq!(cursor, before);
            requests.push(serde_json::json!({"request":request,"state":state}));
            let mut patched = serde_json::to_value(&state).unwrap();
            for (key, value) in c["callback"].as_object().unwrap() {
                patched[key] = value.clone();
            }
            state = serde_json::from_value(patched).unwrap();
            cursor.acknowledge(&request).unwrap();
            assert!(cursor.acknowledge(&request).is_err());
        }
        assert!(cursor.complete());
        assert_eq!(
            context,
            serde_json::from_value(c["context"].clone()).unwrap()
        );
        assert_eq!(
            serde_json::json!({"bounds":bounds,"requests":requests,"state":state}),
            row["original"],
            "scroll {n}"
        );
    }
}
