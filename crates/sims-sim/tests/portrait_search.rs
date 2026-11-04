use serde::Deserialize;
use sims_sim::placement::SearchView;
#[derive(Deserialize)]
struct Input {
    size: i32,
    bounds: [i32; 4],
    rotation: i32,
    point: [i32; 2],
    viewport: [i32; 4],
}
#[derive(Deserialize)]
struct Call {
    screen: [i16; 2],
    mode: u32,
}
#[derive(Deserialize)]
struct Output {
    position: [i32; 2],
    calls: Vec<Call>,
}
#[derive(Deserialize)]
struct Row {
    input: Input,
    original: Output,
}
#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Row>,
}

#[cfg(feature = "private-fixtures")]
#[test]
fn unanchored_origin_matches_native_viewport_fallback_rotation_and_signed_bytes() {
    let f: Fixture =
        serde_json::from_str(include_str!("fixtures/portrait-search-origin.json")).unwrap();
    assert_eq!(f.cases.len(), 180);
    for row in f.cases {
        let c = row.input;
        let view = SearchView {
            viewport: c.viewport,
            bounds: c.bounds,
            rotation: c.rotation,
        };
        let restored: SearchView =
            serde_json::from_slice(&serde_json::to_vec(&view).unwrap()).unwrap();
        assert_eq!(view, restored);
        assert_eq!(row.original.calls.len(), 1);
        assert_eq!(view.midpoint(), row.original.calls[0].screen);
        assert_eq!(row.original.calls[0].mode, 1);
        assert_eq!(view.origin(c.size, c.point), row.original.position);
        assert_eq!(restored.origin(c.size, c.point), row.original.position);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn temporary_person_search_matches_native_single_pass_without_final_recheck() {
    use serde_json::{Value, json};
    use sims_sim::placement::{SearchTile, Spiral, search_tile_allowed};
    let f: Value =
        serde_json::from_str(include_str!("fixtures/temporary-person-search.json")).unwrap();
    let rows = f["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 100);
    for (index, row) in rows.iter().enumerate() {
        let c = &row["input"];
        let view = SearchView {
            viewport: serde_json::from_value(c["viewport"].clone()).unwrap(),
            bounds: serde_json::from_value(c["bounds"].clone()).unwrap(),
            rotation: c["rotation"].as_i64().unwrap() as i32,
        };
        let center = view.origin(8, serde_json::from_value(c["projected"].clone()).unwrap());
        let mut cursor = Spiral::new(8, center).unwrap();
        let [column, row_index] = cursor.center_tile();
        let tiles = c["tiles"].as_array().unwrap();
        let room = tiles[column as usize * 8 + row_index as usize]["room"]
            .as_i64()
            .unwrap() as i16;
        let mut checks = vec![];
        let mut found = None;
        while let Some(pos) = cursor.next() {
            cursor = serde_json::from_value(json!(cursor)).unwrap();
            cursor.validate(8, center).unwrap();
            let t = &tiles[(pos[1] >> 4) as usize * 8 + (pos[0] >> 4) as usize];
            let tile = SearchTile {
                segments: t["segments"].as_u64().unwrap() as u8,
                head: t["head"].as_i64().unwrap() as i16,
                room: t["room"].as_i64().unwrap() as i16,
                pool: t["pool"].as_u64().unwrap() as u8,
                ground_flags: t["ground_flags"].as_u64().unwrap() as u8,
            };
            if search_tile_allowed(tile, 1, 1, Some(room), false) {
                checks.push(pos);
                if checks.len() > c["reject"].as_u64().unwrap() as usize {
                    found = Some(pos);
                    break;
                }
            }
        }
        let n = &row["original"];
        assert_eq!(json!(checks), n["checks"], "case {index}");
        assert_eq!(json!(found.is_some()), n["found"], "case {index}");
        assert_eq!(
            json!(found.unwrap_or([0; 2])),
            n["position"],
            "case {index}"
        );
        assert_eq!(
            json!([{"screen":view.midpoint(),"mode":1}]),
            n["camera"],
            "case {index}"
        );
    }
}
