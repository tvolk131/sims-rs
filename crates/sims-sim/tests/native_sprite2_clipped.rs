//! Original machine-code component vectors with synthetic planes and runs.
//! These do not claim a completed household tick.
use sims_sim::lot_presenter::sprite2_clipped::{self as blit, Input};
fn bytes(v: &serde_json::Value) -> Vec<u8> {
    let s = v.as_str().unwrap().as_bytes();
    s.chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
#[cfg(feature = "private-fixtures")]
#[test]
fn scalar_clipped_spr2_matches_original_ordered_stores() {
    let all: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-sprite2-clipped.json")).unwrap();
    assert_eq!(all["source_unchanged"], true);
    assert_eq!(all["native_parity_credit"], 0);
    let cases = all["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 36);
    let palette = bytes(&all["palette"]);
    let z = bytes(&all["z"]);
    let mix = bytes(&all["mix"]);
    let pitch = serde_json::from_value(all["pitch"].clone()).unwrap();
    for (n, c) in cases.iter().enumerate() {
        let frame = bytes(&c["frame"]);
        let (mut color, mut depth, mut old, mut colors) = (
            bytes(&c["color"]),
            bytes(&c["depth"]),
            bytes(&c["old"]),
            bytes(&c["colors"]),
        );
        let writes = blit::plan(Input {
            frame: &frame,
            palette: &palette,
            z: &z,
            mix: &mix,
            color: &color,
            depth: &depth,
            pitch,
            origin: serde_json::from_value(c["origin"].clone()).unwrap(),
            margins: serde_json::from_value(c["margins"].clone()).unwrap(),
            base: c["base"].as_i64().unwrap() as i32,
        })
        .unwrap_or_else(|e| panic!("case{n}: {e}"));
        assert_eq!(
            serde_json::to_value(&writes).unwrap(),
            c["writes"],
            "case{n} ordered native stores"
        );
        for w in &writes {
            blit::apply(w, &mut color, &mut depth, &mut old, &mut colors).unwrap();
        }
        assert_eq!(color, bytes(&c["output"]["color"]), "case{n} color");
        assert_eq!(depth, bytes(&c["output"]["depth"]), "case{n} depth");
        assert_eq!(old, bytes(&c["output"]["old"]), "case{n} old scratch");
        assert_eq!(
            colors,
            bytes(&c["output"]["colors"]),
            "case{n} source scratch"
        );
    }
}
