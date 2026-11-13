use sims_formats::sprite::{LegacySpriteSet, Palette};

#[cfg(feature = "private-fixtures")]
#[test]
fn legacy_sprite_pixels_match_original_rle_decoder() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/legacy-rle.json")).unwrap();
    let palette = Palette((0..=255).map(|v| [v, v, v]).collect());
    for c in fixture["cases"].as_array().unwrap() {
        for (version, big) in [(502u32, true), (504, false)] {
            let width = c["width"].as_u64().unwrap() as u16;
            let height = c["height"].as_u64().unwrap() as u16;
            let mut bytes = vec![];
            for v in [version, 1, 7, 16] {
                bytes.extend(if big {
                    v.to_be_bytes()
                } else {
                    v.to_le_bytes()
                });
            }
            // Nonzero Rect origin does not change the frame dimensions/runs.
            for v in [3u16, 5, 3 + height, 5 + width, 0] {
                bytes.extend(if big {
                    v.to_be_bytes()
                } else {
                    v.to_le_bytes()
                });
            }
            bytes.extend(
                c["rle"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8),
            );
            let decode = |bytes: &[u8]| {
                LegacySpriteSet::parse(bytes)?.decode_with_palette(bytes, 0, &palette)
            };
            let frame = decode(&bytes).unwrap();
            assert_eq!(
                (frame.x, frame.y, frame.width, frame.height),
                (5, 3, width.into(), height.into())
            );
            for (pixel, original) in frame
                .rgba
                .chunks_exact(4)
                .zip(c["pixels"].as_array().unwrap())
            {
                let original = original.as_u64().unwrap();
                let expected = if original == 65535 {
                    [0, 0, 0, 0]
                } else {
                    [
                        (original - 1) as u8,
                        (original - 1) as u8,
                        (original - 1) as u8,
                        255,
                    ]
                };
                assert_eq!(pixel, expected, "width {width}, version {version}");
            }
            for cut in 0..bytes.len() {
                assert!(decode(&bytes[..cut]).is_err(), "accepted truncation {cut}");
            }
            let mut extra = bytes.clone();
            extra.push(0);
            assert!(decode(&extra).is_err());
        }
    }
}
