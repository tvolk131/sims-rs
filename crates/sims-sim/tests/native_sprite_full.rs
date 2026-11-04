use sha2::{Digest, Sha256};
use sims_sim::lot_presenter::sprite_full::{self, Input};
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_full_sprite_each_color_and_depth_store() {
    let all: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-sprite-full.json")).unwrap();
    assert_eq!(all["cases"].as_array().unwrap().len(), 128);
    let mut stores = 0;
    for (n, r) in all["cases"].as_array().unwrap().iter().enumerate() {
        let i = &r["input"];
        let frame = hex(i["frame"].as_str().unwrap());
        let palette = hex(i["palette"].as_str().unwrap());
        let rows = serde_json::from_value::<Vec<i32>>(i["rows"].clone()).unwrap();
        let writes = sprite_full::plan(Input {
            frame: &frame,
            palette: &palette,
            rows: &rows,
            base: i["base"].as_i64().unwrap() as i32,
            x: i["x"].as_i64().unwrap() as i32,
            y: i["y"].as_i64().unwrap() as i32,
            color_pitch: i["pitch"].as_i64().unwrap() as i32,
            depth_pitch: i["pitch"].as_i64().unwrap() as i32,
        })
        .unwrap();
        assert_eq!(
            serde_json::to_value(&writes).unwrap(),
            r["events"],
            "stores case{n}"
        );
        let mut color = hex(i["color"].as_str().unwrap());
        let mut depth = hex(i["depth"].as_str().unwrap());
        for w in &writes {
            sprite_full::apply(w, &mut color, &mut depth).unwrap();
        }
        assert_eq!(
            Sha256::digest(&color)
                .iter()
                .map(|v| format!("{v:02x}"))
                .collect::<String>(),
            r["color_sha256"].as_str().unwrap(),
            "color case{n}"
        );
        assert_eq!(
            Sha256::digest(&depth)
                .iter()
                .map(|v| format!("{v:02x}"))
                .collect::<String>(),
            r["depth_sha256"].as_str().unwrap(),
            "depth case{n}"
        );
        stores += writes.len();
    }
    assert!(stores > 10000);
}
