use serde_json::Value;
use sims_sim::lot_presenter::visibility::{Inputs, Room, Selected, State};
use std::collections::BTreeMap;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_visibility_composition_clipping_and_dirty_scan() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/cutaway-visibility.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 220);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let rooms: BTreeMap<u16, Room> = serde_json::from_value(c["rooms"].clone()).unwrap();
        let visible: Vec<u16> = serde_json::from_value(c["visible_rooms"].clone()).unwrap();
        let selected: Option<Selected> = serde_json::from_value(c["selected"].clone()).unwrap();
        state
            .update(Inputs {
                size: c["size"].as_i64().unwrap() as i32,
                rotation: c["rotation"].as_u64().unwrap() as u8,
                bounds: serde_json::from_value(c["bounds"].clone()).unwrap(),
                visible_level: c["visible_level"].as_i64().unwrap() as i32,
                mode: c["mode"].as_i64().unwrap() as i32,
                include_visible: c["include_visible"].as_u64().unwrap() as u8,
                visible_rooms: &visible,
                rooms: &rooms,
                selected: selected.as_ref(),
            })
            .unwrap();
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            row["original"],
            "native visibility case {n}"
        );
        let restored: State = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(state, restored);
    }
}
