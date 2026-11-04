use sims_sim::lot_presenter::terrain_clip::Frame;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_rectangle_and_clip_gates() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-terrain-clip.json")).unwrap();
    let rows = v["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 1024);
    for (n, r) in rows.iter().enumerate() {
        let i = &r["input"];
        let mut f = Frame::new(
            serde_json::from_value(i["point"].clone()).unwrap(),
            serde_json::from_value(i["screen"].clone()).unwrap(),
            serde_json::from_value(i["corners"].clone()).unwrap(),
            i["zoom"].as_u64().unwrap() as u32,
            serde_json::from_value(i["bounds"].clone()).unwrap(),
        );
        for _ in 0..8 {
            if f.complete() || f.raster_pending() {
                break;
            }
            f = serde_json::from_slice(&serde_json::to_vec(&f).unwrap()).unwrap();
            assert!(
                f.advance(
                    [
                        i["flags"][0].as_u64().unwrap() as u8,
                        u8::from(i["flags"][1].as_bool().unwrap())
                    ],
                    i["height"].as_i64().unwrap() as i32,
                    serde_json::from_value(i["clip"].clone()).unwrap()
                )
                .unwrap()
            );
        }
        assert_eq!(f.raster_pending(), r["target"] == 0x4c451cu32, "case{n}");
        assert_eq!(
            serde_json::to_value(f.rectangle()).unwrap(),
            r["rectangle"],
            "case{n}"
        );
    }
}
