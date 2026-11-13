//! Pinned original4b10c4 instructions, with synthetic planes and clipping cases.
use sims_sim::lot_presenter::{object_shadow as shadow, sprite_full, sprite_gradient::Tables};
fn bytes(v: &serde_json::Value) -> Vec<u8> {
    v.as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
#[cfg(feature = "private-fixtures")]
#[test]
fn shadow_composite_matches_original_ordered_color_stores() {
    let all: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-object-shadow.json")).unwrap();
    assert_eq!(all["source_unchanged"], true);
    assert_eq!(all["depth_and_inputs_unchanged"], true);
    let tables = bytes(&all["tables"]);
    let cases = all["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 72);
    for (n, c) in cases.iter().enumerate() {
        let (mut color, mut depth) = (bytes(&c["color"]), bytes(&c["depth"]));
        let original_depth = depth.clone();
        let mask = bytes(&c["mask"]);
        let rows: Vec<i32> = serde_json::from_value(c["rows"].clone()).unwrap();
        let writes = shadow::plan(shadow::Input {
            origin: serde_json::from_value(c["origin"].clone()).unwrap(),
            size: serde_json::from_value(c["size"].clone()).unwrap(),
            mask: &mask,
            mask_pitch: c["mask_pitch"].as_i64().unwrap() as i32,
            bias: c["bias"].as_i64().unwrap() as i32,
            rows: &rows,
            depth_offset: c["offset"].as_i64().unwrap() as i32,
            clip: serde_json::from_value(c["clip"].clone()).unwrap(),
            color: &color,
            depth: &depth,
            pitches: serde_json::from_value(all["pitches"].clone()).unwrap(),
            rgb555: c["mode"] == 1,
            tables: Tables {
                bytes: &tables,
                green_shift: c["shift"].as_u64().unwrap() as u32,
            },
        })
        .unwrap_or_else(|e| panic!("case{n}: {e}"));
        assert_eq!(
            serde_json::to_value(&writes).unwrap(),
            c["writes"],
            "case{n} ordered native stores"
        );
        for w in &writes {
            sprite_full::apply(w, &mut color, &mut depth).unwrap();
        }
        assert_eq!(color, bytes(&c["output"]), "case{n} color");
        assert_eq!(depth, original_depth, "case{n} depth");
    }
}
