use sims_sim::lot_presenter::{
    room_masks::Camera,
    tile_scan::{Frame, Inputs},
};
#[cfg(feature = "private-fixtures")]
#[test]
fn original_scan_callback_order_clipping_and_live_fields_match_across_every_cold_cut() {
    let f: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-window-scan.json")).unwrap();
    assert_eq!(f["cases"].as_array().unwrap().len(), 1024);
    for (i, c) in f["cases"].as_array().unwrap().iter().enumerate() {
        let v = &c["input"];
        let mut size = v["size"].as_i64().unwrap() as i32;
        let bounds = serde_json::from_value(v["bounds"].clone()).unwrap();
        let mut camera: Camera = serde_json::from_value(v["camera"].clone()).unwrap();
        let clip = serde_json::from_value(v["clip"].clone()).unwrap();
        let diagonal: Vec<u8> = serde_json::from_value(v["diagonal"].clone()).unwrap();
        let fast = v["fast"].as_u64().unwrap() as u8;
        let mut flags: [u8; 3] = serde_json::from_value(v["flags"].clone()).unwrap();
        let level = v["level"].as_i64().unwrap() as i32;
        let above = v["above"].as_u64().unwrap() as u32;
        let special = v["special"].as_u64().unwrap() as u8;
        let mut frame = Frame::new(
            level,
            above,
            special,
            serde_json::from_value(v["step"].clone()).unwrap(),
        );
        let mut events = vec![];
        for _ in 0..65536 {
            let mut cold: Frame =
                serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
            if frame.complete() {
                break;
            }
            if let Some(r) = frame.pending().cloned() {
                events.push(serde_json::to_value(&r).unwrap());
                let mut step = r.step;
                if v["change_after_first"] == true && events.len() == 1 {
                    camera.origin[0] += 13;
                    camera.origin[1] -= 7;
                    camera.floor_heights[camera.zoom as usize] += 9;
                    size = 3;
                    flags = [0, 0, 1];
                    step[2] = 17;
                }
                frame.acknowledge(&r, r.point, step).unwrap();
                cold.acknowledge(&r, r.point, step).unwrap();
            } else {
                let input = || Inputs {
                    size,
                    bounds,
                    camera: &camera,
                    diagonal: &diagonal,
                    clip,
                    fast,
                    flags,
                };
                assert!(frame.advance(input()).unwrap(), "case{i}");
                assert!(cold.advance(input()).unwrap());
            }
            assert_eq!(frame, cold, "cold case{i}");
        }
        assert!(frame.complete(), "case{i}");
        assert_eq!(
            serde_json::json!(events),
            c["expected"]["events"],
            "case{i}"
        );
        assert_eq!(c["expected"]["only_supplied_owner_changes"], true);
    }
}
