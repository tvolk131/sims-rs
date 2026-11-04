use serde_json::Value;
use sims_sim::lot_presenter::room_masks::{Camera, SplitRooms, View};
fn compare(report: Value, count: usize, centering: bool) {
    assert_eq!(report["cases"].as_array().unwrap().len(), count);
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let camera: Camera = serde_json::from_value(c["camera"].clone()).unwrap();
        let mut heights = vec![[0; 4]; 4096];
        for h in c["heights"].as_array().unwrap() {
            let i = h[0][0].as_u64().unwrap() as usize * 64 + h[0][1].as_u64().unwrap() as usize;
            heights[i] = serde_json::from_value(h[1].clone()).unwrap();
        }
        let walls = vec![Default::default(); 4096];
        let splits = SplitRooms::new();
        let v = View {
            size: 64,
            rotation: c["rotation"].as_u64().unwrap() as u8,
            bounds: serde_json::from_value(c["bounds"].clone()).unwrap(),
            camera: &camera,
            heights: &heights,
            walls: &walls,
            splits: &splits,
        };
        let actual: Vec<Value> = c["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let physical = serde_json::from_value(p.clone()).unwrap();
                let level = c["level"].as_i64().unwrap() as i32;
                if centering {
                    serde_json::to_value(
                        v.centered_origin(
                            serde_json::from_value(c["viewport"].clone()).unwrap(),
                            serde_json::from_value(c["pending"].clone()).unwrap(),
                            physical,
                            level,
                        )
                        .unwrap(),
                    )
                    .unwrap()
                } else {
                    serde_json::to_value(v.fine_screen(physical, level).unwrap()).unwrap()
                }
            })
            .collect();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            row["original"],
            "forward {n}"
        );
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_fine_forward_projection() {
    compare(
        serde_json::from_str(include_str!("fixtures/forward-projection.json")).unwrap(),
        128,
        false,
    );
}
#[test]
#[ignore = "requires private original-house forward projection oracle"]
fn original_house_fine_forward_projection() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/house-forward-projection.json");
    compare(
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap(),
        24,
        false,
    );
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_immediate_centering() {
    compare(
        serde_json::from_str(include_str!("fixtures/camera-center.json")).unwrap(),
        128,
        true,
    );
}
#[test]
#[ignore = "requires private original-house centering oracle"]
fn original_house_immediate_centering() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/house-camera-center.json");
    compare(
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap(),
        24,
        true,
    );
}
