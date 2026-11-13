use sha2::{Digest, Sha256};
use sims_sim::lot_presenter::{
    sprite_full,
    sprite_gradient::{self, Input, Tables},
};
use std::io::Read;
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}
fn digest(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_single_gradient_full_and_clipped_ordered_stores() {
    let mut bytes = vec![];
    flate2::read::GzDecoder::new(
        include_bytes!("fixtures/native-sprite-gradient.json.gz").as_slice(),
    )
    .read_to_end(&mut bytes)
    .unwrap();
    let all: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let tables = all["table_reads"].as_array().unwrap();
    let t = tables
        .iter()
        .find(|v| v["label"] == "floor_gradient_tables")
        .unwrap();
    let raw = hex(t["raw"].as_str().unwrap());
    let shift = tables
        .iter()
        .find(|v| v["label"] == "floor_gradient_green_shift")
        .unwrap();
    let shift = u32::from_le_bytes(hex(shift["raw"].as_str().unwrap()).try_into().unwrap());
    let mut stores = 0;
    assert_eq!(all["cases"].as_array().unwrap().len(), 256);
    for (n, c) in all["cases"].as_array().unwrap().iter().enumerate() {
        let i = &c["input"];
        let int = |s: &str| i[s].as_i64().unwrap() as i32;
        let frame = hex(i["frame"].as_str().unwrap());
        let palette = hex(i["palette"].as_str().unwrap());
        let mask = hex(i["mask"].as_str().unwrap());
        let rows: Vec<i32> = serde_json::from_value(i["rows"].clone()).unwrap();
        let clip = i["clipped"]
            .as_bool()
            .unwrap()
            .then(|| serde_json::from_value(i["clip"].clone()).unwrap());
        let writes = sprite_gradient::plan(
            Input {
                sprite: sprite_full::Input {
                    frame: &frame,
                    palette: &palette,
                    rows: &rows,
                    base: int("base"),
                    x: int("x"),
                    y: int("y"),
                    color_pitch: int("color_pitch"),
                    depth_pitch: int("depth_pitch"),
                },
                mask: &mask,
                mask_pitch: int("mask_pitch"),
                light: int("light"),
                tables: Tables {
                    bytes: &raw,
                    green_shift: shift,
                },
            },
            clip,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&writes).unwrap(),
            c["events"],
            "stores case{n}"
        );
        let length = i["plane_bytes"].as_u64().unwrap() as usize;
        let mut color = hex(i["color_pattern"].as_str().unwrap()).repeat(length / 2);
        let mut depth = hex(i["depth_pattern"].as_str().unwrap()).repeat(length / 2);
        for w in &writes {
            sprite_full::apply(w, &mut color, &mut depth).unwrap();
        }
        assert_eq!(digest(&color), c["color_sha256"], "color case{n}");
        assert_eq!(digest(&depth), c["depth_sha256"], "depth case{n}");
        stores += writes.len();
    }
    assert_eq!(stores, 53936);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_resident_kernel_addresses() {
    let all: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-floor-kernel.json")).unwrap();
    let cases = all["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 1024);
    for (n, c) in cases.iter().enumerate() {
        let i = &c["input"];
        let get = |s: &str| i[s].as_i64().unwrap() as i32;
        assert_eq!(
            sprite_gradient::kernel_offset(get("side"), get("pitch"), get("x"), get("y")).unwrap()
                as u64,
            c["offset"].as_u64().unwrap(),
            "kernel case{n}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_gradient_intensity_with_retained_x87_control() {
    let all: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-gradient-intensity.json")).unwrap();
    let cases = all["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 2048);
    for (n, c) in cases.iter().enumerate() {
        let bits = c["input"]["brightness"].as_u64().unwrap() as u32;
        let control = c["input"]["control"].as_u64().unwrap() as u16;
        assert_eq!(
            sprite_gradient::intensity(bits, control).unwrap() as u32,
            c["low"].as_u64().unwrap() as u32,
            "intensity case{n}"
        );
        assert_eq!(c["high"], 0);
        assert_eq!(c["control_after"], control);
    }
}
