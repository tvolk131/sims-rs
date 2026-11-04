use serde_json::Value;
use sims_sim::lot_presenter::visibility::{Inputs, Pending, Room, State};
use std::collections::BTreeMap;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_cursor_clipping_and_suspended_dirty_publication() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-world-cursor.json")).unwrap();
    assert_eq!(
        report["source_sha256"],
        "b15ce2d878ba88f9158abc4c6715edc6cc7dfd3ca9dfcc7e1c6bb561dd87f852"
    );
    assert_eq!(report["executed_caller_matches_fresh_image"], true);
    assert_eq!(report["cases"].as_array().unwrap().len(), 512);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let rooms: BTreeMap<u16, Room> = serde_json::from_value(c["rooms"].clone()).unwrap();
        let history: Vec<u16> = serde_json::from_value(c["visible_rooms"].clone()).unwrap();
        let pending = state
            .begin(Inputs {
                size: c["size"].as_i64().unwrap() as i32,
                rotation: c["rotation"].as_u64().unwrap() as u8,
                bounds: serde_json::from_value(c["bounds"].clone()).unwrap(),
                visible_level: 1,
                mode: 0,
                include_visible: 1,
                visible_rooms: &history,
                rooms: &rooms,
                selected: None,
            })
            .unwrap()
            .unwrap();
        // Native dirty and include-visible publication occur after OS/projection.
        assert_eq!(state.dirty, 0);
        assert_eq!(state.include_visible, 0);
        let mut cold: State = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        let cold_pending: Pending =
            serde_json::from_slice(&serde_json::to_vec(&pending).unwrap()).unwrap();
        let client: [i32; 2] = serde_json::from_value(c["client_point"].clone()).unwrap();
        if client[0] >= 0 && client[0] < 800 && client[1] >= 0 && client[1] < 600 {
            let fine: [i32; 2] = serde_json::from_value(c["fine_point"].clone()).unwrap();
            let tile = [(fine[1] >> 4) as i8, (fine[0] >> 4) as i8, 1];
            let outdoors = history.iter().any(|id| rooms[id].outdoors);
            assert_eq!(
                pending.cursor(&mut state, tile, outdoors).unwrap(),
                cold_pending.cursor(&mut cold, tile, outdoors).unwrap()
            );
        }
        pending.finish(&mut state).unwrap();
        cold_pending.finish(&mut cold).unwrap();
        assert_eq!(state, cold);
        assert_eq!(
            serde_json::to_value(state).unwrap(),
            row["original"],
            "native cursor case {n}"
        );
    }
}
