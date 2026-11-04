use sims_formats::lot::WallTile;
use sims_sim::lot_presenter::{terrain_shade, terrain_walls};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_wall_query_all_masks_rotations_edges_and_floors() {
    let all: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-terrain-walls.json")).unwrap();
    let rows = all["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 1024);
    assert_eq!(all["source_unchanged"], true);
    for (n, row) in rows.iter().enumerate() {
        let i = &row["input"];
        let point: [i8; 3] = serde_json::from_value(i["point"].clone()).unwrap();
        let rotation = i["rotation"].as_u64().unwrap() as u8;
        let physical: [i8; 3] = serde_json::from_value(row["physical"].clone()).unwrap();
        assert_eq!(terrain_shade::inverse(point, rotation).unwrap(), physical);
        let mut cells = vec![
            WallTile {
                segments: 0,
                flags: 0,
                styles: [0; 2],
                patterns: [0; 4],
            };
            64 * 64
        ];
        let [x, y, _] = physical;
        cells[x as usize * 64 + y as usize].segments = i["segments"].as_u64().unwrap() as u8;
        assert_eq!(
            u64::from(terrain_walls::segments(64, &cells, point, rotation).unwrap()),
            row["segments"].as_u64().unwrap(),
            "case{n}"
        );
    }
}
