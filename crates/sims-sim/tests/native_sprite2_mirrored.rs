//! Component evidence from unmodified original instructions, not gameplay ticks.
use sims_sim::lot_presenter::sprite2_clipped::{self as blit, CopyProfile, Input, Write};
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
fn mirrored_spr2_matches_both_original_entries_and_suspended_stores() {
    let all: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-sprite2-mirrored.json")).unwrap();
    assert_eq!(all["source_unchanged"], true);
    assert_eq!(all["native_parity_credit"], 0);
    let cases = all["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 384);
    let (palette, z, mix) = (bytes(&all["palette"]), bytes(&all["z"]), bytes(&all["mix"]));
    let mut total = 0;
    let mut widths = std::collections::BTreeSet::new();
    for (n, c) in cases.iter().enumerate() {
        let frame = bytes(&c["frame"]);
        let mut owners = (
            bytes(&c["color"]),
            bytes(&c["depth"]),
            bytes(&c["scratch"]["OldColors"]),
            bytes(&c["scratch"]["SourceColors"]),
            bytes(&c["scratch"]["Batch"]),
            bytes(&c["scratch"]["Alphas"]),
        );
        let (writes, argument) = blit::plan_mirrored_with_argument(
            Input {
                frame: &frame,
                palette: &palette,
                z: &z,
                mix: &mix,
                color: &owners.0,
                depth: &owners.1,
                pitch: serde_json::from_value(all["pitch"].clone()).unwrap(),
                origin: serde_json::from_value(c["origin"].clone()).unwrap(),
                margins: serde_json::from_value(c["margins"].clone()).unwrap(),
                base: c["base"].as_i64().unwrap() as i32,
            },
            Some(CopyProfile {
                dispatch: if c["profile"] == "baseline" {
                    [1; 6]
                } else {
                    [0; 6]
                },
                plane_mod32: [0, 0],
            }),
        )
        .unwrap_or_else(|e| panic!("case{n}: {e}"));
        if c["entry"] == "unclipped" {
            assert_eq!(
                serde_json::to_value(argument).unwrap(),
                c["returned_frame_argument"],
                "case{n} mutated frame argument"
            );
        }
        let expected: Vec<Write> = serde_json::from_value(c["writes"].clone()).unwrap();
        assert_eq!(writes, expected, "case{n} ordered atomic stores");
        let cut = writes.len() / 2;
        let mut cold = None;
        for (at, w) in writes.iter().enumerate() {
            if at == cut {
                //A computed store cut, not a captured native CPU boundary.
                let encoded = serde_json::to_vec(&(&owners, &writes, at)).unwrap();
                cold = Some(
                    serde_json::from_slice::<(
                        (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>),
                        Vec<Write>,
                        usize,
                    )>(&encoded)
                    .unwrap(),
                );
            }
            widths.insert(w.width.unwrap_or(0));
            blit::apply_batched(
                w,
                &mut owners.0,
                &mut owners.1,
                &mut owners.2,
                &mut owners.3,
                &mut owners.4,
                &mut owners.5,
            )
            .unwrap();
        }
        if let Some((mut restored, plan, start)) = cold {
            for w in &plan[start..] {
                blit::apply_batched(
                    w,
                    &mut restored.0,
                    &mut restored.1,
                    &mut restored.2,
                    &mut restored.3,
                    &mut restored.4,
                    &mut restored.5,
                )
                .unwrap();
            }
            assert_eq!(restored, owners, "case{n} suspended continuation");
        }
        for (name, actual) in [
            ("Color", &owners.0),
            ("Depth", &owners.1),
            ("OldColors", &owners.2),
            ("SourceColors", &owners.3),
            ("Batch", &owners.4),
            ("Alphas", &owners.5),
        ] {
            assert_eq!(*actual, bytes(&c["output"][name]), "case{n} {name}");
        }
        total += writes.len();
    }
    assert_eq!(total, 95843);
    assert!(widths.contains(&8) && widths.contains(&16));
}
