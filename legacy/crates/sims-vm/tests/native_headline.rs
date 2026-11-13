use serde_json::{Value, json};
use sims_vm::headline::{Catalog, State};
#[cfg(feature = "private-fixtures")]
#[test]
fn original_headline_priority_resource_flags_and_lifetime_vectors() {
    let report: Value = serde_json::from_str(include_str!("fixtures/headline.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let i = |k| c[k].as_i64().unwrap() as i32;
        let mut s = State {
            duration: i("active"),
            sprite: 123,
            frames: 3,
            frame: 2,
            step: 1,
            ticks: 8,
            background: 101,
            overlay: 103,
            priority: i("priority"),
            ..Default::default()
        };
        let original = s.clone();
        let mut b = [0u8; 8];
        b[0..2].copy_from_slice(&(i("flags") as u16).to_le_bytes());
        b[2] = i("index") as u8;
        b[3] = i("kind") as u8;
        b[4..6].copy_from_slice(&(i("duration") as i16).to_le_bytes());
        b[6..8].copy_from_slice(&(i("options") as u16).to_le_bytes());
        let target = if b[0] & 1 != 0 { i("target") } else { 1 };
        if ![1, 2].contains(&target) {
            assert_eq!(n["error"], 21);
            continue;
        }
        let catalog = Catalog {
            frames: n["lookups"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| (id.as_u64().unwrap() as u16, i("frames") as u32))
                .collect(),
            ..Default::default()
        };
        if c["slot"] == true {
            s.apply(&b, i("temp") as i16, &catalog).unwrap();
        }
        let words = |s: &State| {
            json!([
                s.duration,
                i32::from(s.sprite),
                0,
                s.frames,
                s.frame,
                s.step,
                s.ticks,
                i32::from(s.background),
                i32::from(s.overlay),
                s.priority,
                i32::from(s.flag)
            ])
        };
        // Original slot +0x18 is an opaque sprite-cache pointer, not game state.
        for (who, state) in [
            ("caller", if target == 1 { &s } else { &original }),
            ("target", if target == 2 { &s } else { &original }),
        ] {
            let mut expected = n[who].clone();
            expected[2] = json!(0);
            assert_eq!(words(state), expected, "{c} {who}");
        }
        assert_eq!(n["returned"], 1);
    }
    for row in report["ticks"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let i = |k| c[k].as_i64().unwrap() as i32;
        let mut s = State {
            duration: i("duration"),
            frames: i("frames"),
            frame: i("frame"),
            step: i("step"),
            ticks: i("ticks"),
            ..Default::default()
        };
        let changed = s.advance().unwrap();
        assert_eq!(json!(s.duration as u32), n["duration"]);
        assert_eq!(json!(s.frame as u32), n["frame"]);
        assert_eq!(json!(s.ticks as u32), n["ticks"]);
        assert_eq!(json!(u32::from(changed)), n["changed"]);
    }
}
