use sims_sim::lot_presenter::level_geometry::{
    segment_bounds, segment_heights, segment_overlaps, special_kind,
};
#[cfg(feature = "private-fixtures")]
#[test]
fn geometry_segment_math_matches_original_instructions() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-geometry-segments.json")).unwrap();
    let cases = v["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 4352);
    for (n, c) in cases.iter().enumerate() {
        let i = &c["input"];
        let value = match c["kind"].as_str().unwrap() {
            "classification" => {
                serde_json::json!(u32::from(special_kind(i.as_u64().unwrap() as u32)))
            }
            "bounds" => serde_json::json!(segment_bounds(
                i["mask"].as_u64().unwrap() as u32,
                i["zoom"].as_u64().unwrap() as u8,
                i["height"].as_i64().unwrap() as i32
            )),
            "overlap" => serde_json::json!(segment_overlaps(
                std::array::from_fn(|k| i["rect"][k].as_i64().unwrap() as i32),
                std::array::from_fn(|k| i["clip"][k].as_i64().unwrap() as i32)
            )),
            "heights" => serde_json::json!(segment_heights(
                i["orientation"].as_u64().unwrap() as u32,
                std::array::from_fn(|k| i["corners"][k].as_u64().unwrap() as u8)
            )),
            _ => panic!("unknown native case"),
        };
        //53cd2a defines AL; its remaining EAX bits retain the argument.
        let expected = if c["kind"] == "classification" {
            serde_json::json!(c["original"].as_u64().unwrap() & 255)
        } else {
            c["original"].clone()
        };
        assert_eq!(value, expected, "case{n}");
    }
}
