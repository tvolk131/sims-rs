use serde_json::Value;
use sims_sim::lot_presenter::room_masks::{Camera, SplitRooms, View};
fn compare(report: Value, expected: usize) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), expected);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let camera: Camera = serde_json::from_value(c["camera"].clone()).unwrap();
        let mut heights = vec![[0u8; 4]; 4096];
        for v in c["heights"].as_array().unwrap() {
            let p: [usize; 2] = serde_json::from_value(v[0].clone()).unwrap();
            heights[p[0] * 64 + p[1]] = serde_json::from_value(v[1].clone()).unwrap();
        }
        let walls = vec![
            sims_formats::lot::WallTile {
                segments: 0,
                flags: 0,
                styles: [0; 2],
                patterns: [0; 4]
            };
            4096
        ];
        let splits = SplitRooms::new();
        let view = View {
            size: 64,
            rotation: c["rotation"].as_u64().unwrap() as u8,
            bounds: serde_json::from_value(c["bounds"].clone()).unwrap(),
            camera: &camera,
            heights: &heights,
            walls: &walls,
            splits: &splits,
        };
        let level = c["level"].as_i64().unwrap() as i32;
        assert_eq!(
            c["screens"].as_array().unwrap().len(),
            row["original"].as_array().unwrap().len()
        );
        for (s, expected) in c["screens"]
            .as_array()
            .unwrap()
            .iter()
            .zip(row["original"].as_array().unwrap())
        {
            let screen = serde_json::from_value(s.clone()).unwrap();
            let coarse = view.coarse_point(screen, level).unwrap();
            let fine = view.fine_point(screen, level).unwrap();
            assert_eq!(
                serde_json::json!({"coarse":coarse,"fine":fine}),
                *expected,
                "inverse case {n} screen {screen:?}"
            );
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_coarse_and_fine_inverse_projection() {
    compare(
        serde_json::from_str(include_str!("fixtures/inverse-projection.json")).unwrap(),
        192,
    );
}
#[test]
#[ignore = "requires private original-house inverse-projection evidence"]
fn original_house_inverse_projection() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/house-inverse-projection.json");
    compare(
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap(),
        24,
    );
}

#[test]
#[ignore = "requires private original-house combined mouse/projection evidence"]
fn original_house_mouse_with_native_inverse_projection() {
    use sims_sim::lot_presenter::hover::{Geometry, Mouse, Request, View as MouseView};
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/house-hover-inverse.json");
    let report: Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 24);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let camera: Camera = serde_json::from_value(c["camera"].clone()).unwrap();
        let mut heights = vec![[0u8; 4]; 4096];
        for v in c["heights"].as_array().unwrap() {
            let p: [usize; 2] = serde_json::from_value(v[0].clone()).unwrap();
            heights[p[0] * 64 + p[1]] = serde_json::from_value(v[1].clone()).unwrap();
        }
        let walls = vec![
            sims_formats::lot::WallTile {
                segments: 0,
                flags: 0,
                styles: [0; 2],
                patterns: [0; 4]
            };
            4096
        ];
        let splits = SplitRooms::new();
        let view = View {
            size: 64,
            rotation: c["rotation"].as_u64().unwrap() as u8,
            bounds: serde_json::from_value(c["bounds"].clone()).unwrap(),
            camera: &camera,
            heights: &heights,
            walls: &walls,
            splits: &splits,
        };
        let primary: Vec<i16> = serde_json::from_value(c["primary"].clone()).unwrap();
        let geometry = Geometry {
            size: 64,
            rotation: view.rotation,
            bounds: view.bounds,
            primary: &primary,
        };
        let mut mouse_view: MouseView = serde_json::from_value(c["view"].clone()).unwrap();
        let mut mouse = Mouse::begin(
            &mut mouse_view,
            serde_json::from_value(c["screen"].clone()).unwrap(),
            c["flags"].as_u64().unwrap() as u32,
            c["base_reply"].as_u64().unwrap() as u8,
        )
        .unwrap();
        let mut requests = vec![];
        let mut effect = Value::Null;
        while let Some(request) = mouse.request().cloned() {
            (mouse_view, mouse) =
                serde_json::from_slice(&serde_json::to_vec(&(&mouse_view, &mouse)).unwrap())
                    .unwrap();
            mouse.validate(&mouse_view).unwrap();
            assert_eq!(mouse.request(), Some(&request));
            match request {
                Request::Project { screen, flag } => {
                    assert_eq!(flag, 1);
                    let point = view.fine_point(screen, mouse_view.level).unwrap();
                    requests.push(serde_json::json!({"screen":screen,"flag":flag}));
                    mouse
                        .projected(&mut mouse_view, &geometry, screen, point)
                        .unwrap();
                }
                Request::ComposeVisible | Request::ClearMask => {
                    effect = Value::String(
                        if request == Request::ComposeVisible {
                            "compose"
                        } else {
                            "clear"
                        }
                        .into(),
                    );
                    mouse.acknowledge(&mouse_view, &request).unwrap();
                }
            }
        }
        let expected = &row["original"];
        assert_eq!(
            serde_json::to_value(&mouse_view).unwrap(),
            expected["view"],
            "combined native view {n}"
        );
        assert_eq!(
            serde_json::to_value(&requests).unwrap(),
            expected["requests"],
            "combined native projection order {n}"
        );
        assert_eq!(effect, expected["effect"]);
        assert_eq!(
            mouse.returned(),
            Some(expected["returned"].as_u64().unwrap() as u8)
        );
    }
}
