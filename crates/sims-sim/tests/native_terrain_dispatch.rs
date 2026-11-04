use sims_sim::lot_presenter::terrain_clip::Frame;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_wrapper_dispatch_flags_and_rectangle() {
    let all: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-terrain-dispatch.json")).unwrap();
    let rows = all["cases"].as_array().unwrap();
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
        let flags = [
            i["flags"][0].as_u64().unwrap() as u8,
            u8::from(i["flags"][1].as_bool().unwrap()),
        ];
        let clip = serde_json::from_value(i["clip"].clone()).unwrap();
        let h = i["height"].as_i64().unwrap() as i32;
        for _ in 0..8 {
            if f.complete() || f.raster_pending() {
                break;
            }
            assert!(f.advance(flags, h, clip).unwrap());
        }
        let spec = if f.raster_pending() {
            f.raster_spec(
                h,
                clip,
                i["restrict_slope"].as_bool().unwrap(),
                i["draw"].as_bool().unwrap(),
            )
            .unwrap()
        } else {
            None
        };
        if let Some(s) = spec {
            assert_eq!(
                serde_json::json!({"left":s.left,"bottom":s.bottom,"clipped":s.clipped}),
                r["dispatch"],
                "case{n}"
            );
            assert_eq!(
                serde_json::to_value(s.rectangle).unwrap(),
                r["rectangle"],
                "case{n}"
            );
        } else {
            assert!(r["dispatch"].is_null(), "case{n}");
        }
    }
}
