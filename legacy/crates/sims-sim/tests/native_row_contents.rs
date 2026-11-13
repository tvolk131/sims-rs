use sims_sim::lot_presenter::row_tail::{Frame, Inputs, Request};
#[cfg(feature = "private-fixtures")]
#[test]
fn original_interior_water_and_overlay() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-row-contents.json")).unwrap();
    let rows = v["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 1024);
    for (n, r) in rows.iter().enumerate() {
        let i = &r["input"];
        let point: [i8; 3] = serde_json::from_value(i["point"].clone()).unwrap();
        let mut water = vec![255; 4096];
        water[point[0] as usize * 64 + point[1] as usize] = i["water"].as_u64().unwrap() as u8;
        let mut f = Frame::new(point, point, [0, 0], [1, -1, 0], 1, 0);
        for _ in 0..16 {
            f = serde_json::from_slice(&serde_json::to_vec(&f).unwrap()).unwrap();
            if f.at_native_cut(0x4b0111)
                || matches!(
                    f.pending(),
                    Some(Request::WaterRaster | Request::OverlayRaster)
                )
            {
                break;
            }
            let more = if f.pending() == Some(Request::OtherContents) {
                f.advance_contents(&water, 64, i["overlay"].as_bool().unwrap())
            } else {
                f.advance(Inputs {
                    size: 64,
                    active_size: i["active"].as_i64().unwrap() as i32,
                    rotation: 0,
                    pool: None,
                    bounds: [0, 63, 0, 63],
                    neighbors: [[0, 0, 0]; 3],
                    covering: 0,
                    floor_enabled: false,
                    tile_width: 32,
                })
            };
            assert!(more.unwrap(), "case{n}");
        }
        let target = match f.pending() {
            Some(Request::WaterRaster) => 0x4afa2a,
            Some(Request::OverlayRaster) => 0x4b004d,
            _ => {
                assert!(f.at_native_cut(0x4b0111));
                0x4b0111
            }
        };
        assert_eq!(r["target"], target, "case{n}");
        assert_eq!(f.point(), point);
        assert_eq!(f.index(), 0);
        assert_eq!(f.screen(), [0, 0]);
    }
}
