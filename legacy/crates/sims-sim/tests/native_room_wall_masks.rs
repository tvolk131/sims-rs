use serde_json::Value;
use sims_sim::lot_presenter::room_masks::{Camera, SplitRooms, View};

fn compare(report: Value, cases: usize) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), cases);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let camera: Camera = serde_json::from_value(c["camera"].clone()).unwrap();
        let mut heights = vec![[0u8; 4]; 4096];
        for v in c["heights"].as_array().unwrap() {
            let p: [usize; 2] = serde_json::from_value(v[0].clone()).unwrap();
            heights[p[0] * 64 + p[1]] = serde_json::from_value(v[1].clone()).unwrap();
        }
        let empty = sims_formats::lot::WallTile {
            segments: 0,
            flags: 0,
            styles: [0; 2],
            patterns: [0; 4],
        };
        let mut walls = vec![empty; 4096];
        for v in c["walls"].as_array().unwrap() {
            let p: [usize; 2] = serde_json::from_value(v[0].clone()).unwrap();
            walls[p[0] * 64 + p[1]].segments = v[1].as_u64().unwrap() as u8;
        }
        let mut splits = SplitRooms::new();
        for v in c["splits"].as_array().unwrap() {
            splits.insert(
                serde_json::from_value(v[0].clone()).unwrap(),
                serde_json::from_value(v[1].clone()).unwrap(),
            );
        }
        let tiles: Vec<[i8; 3]> = serde_json::from_value(c["tiles"].clone()).unwrap();
        let view = View {
            size: 64,
            rotation: c["rotation"].as_u64().unwrap() as u8,
            bounds: serde_json::from_value(c["bounds"].clone()).unwrap(),
            camera: &camera,
            heights: &heights,
            walls: &walls,
            splits: &splits,
        };
        let result = view
            .room(
                c["room"].as_u64().unwrap() as u16,
                c["outdoors"].as_bool().unwrap(),
                &tiles,
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(&result.rows).unwrap(),
            row["original"]["rows"],
            "native room mask {n}"
        );
        let restored: sims_sim::lot_presenter::walls::cutaway::Mask =
            serde_json::from_slice(&serde_json::to_vec(&result).unwrap()).unwrap();
        assert_eq!(result, restored);
        for (p, expected) in c["samples"]
            .as_array()
            .unwrap()
            .iter()
            .zip(row["original"]["projected"].as_array().unwrap())
        {
            let p: [i8; 3] = serde_json::from_value(p.clone()).unwrap();
            assert_eq!(
                serde_json::json!({"point":view.point(p).unwrap(),"rectangle":view.rectangle(p).unwrap(),"heights":view.terrain(p).unwrap()}),
                *expected,
                "native projections case {n} point {p:?}"
            );
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_room_wall_masks_and_terrain_projection() {
    compare(
        serde_json::from_str(include_str!("fixtures/room-wall-mask.json")).unwrap(),
        120,
    );
}
#[test]
#[ignore = "requires private original house room-mask evidence"]
fn original_house_room_wall_masks_and_terrain_projection() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/house-room-wall-mask.json");
    compare(
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap(),
        92,
    );
}
