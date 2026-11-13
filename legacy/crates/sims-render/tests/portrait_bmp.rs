use serde::Deserialize;
use sims_render::portrait::{
    bmp,
    resize::{Format, Source},
};

#[derive(Deserialize)]
struct Input {
    format: u32,
    width: usize,
    height: usize,
    stride: usize,
    pixels: Vec<u16>,
    crop: Option<[i32; 4]>,
    reduce: u32,
    fill: u8,
    present: bool,
    lock: bool,
    allocate: bool,
}
#[derive(Deserialize)]
struct Original {
    bytes: Option<Vec<u8>>,
    events: serde_json::Value,
}
#[derive(Deserialize)]
struct Case {
    input: Input,
    original: Original,
}
#[derive(Deserialize)]
struct Report {
    cases: Vec<Case>,
}

fn check(cases: Vec<Case>) -> usize {
    let mut checked = 0;
    for (n, case) in cases.into_iter().enumerate() {
        let c = case.input;
        if !c.present || !c.lock || !c.allocate {
            // Host allocation/locking is separate from the owned pixel encoder.
            assert!(case.original.bytes.is_none());
            if !c.present {
                assert_eq!(case.original.events, serde_json::json!([]));
            }
            if c.present && !c.lock {
                assert_eq!(case.original.events, serde_json::json!([["lock", 64]]));
            }
            continue;
        }
        let bytes = bmp::encode(
            Format::from_native(c.format).unwrap(),
            Source {
                pixels: &c.pixels,
                width: c.width,
                height: c.height,
                stride: c.stride,
                key: None,
            },
            c.crop,
            c.reduce != 0,
            c.fill,
        )
        .unwrap();
        assert_eq!(
            &bytes,
            case.original.bytes.as_ref().unwrap(),
            "native BMP case {n}"
        );
        let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Bmp)
            .unwrap()
            .to_rgb8();
        let [left, top, right, bottom] = c.crop.unwrap_or([0, 0, c.width as i32, c.height as i32]);
        assert_eq!(
            decoded.dimensions(),
            ((right - left) as u32, (bottom - top) as u32)
        );
        for (x, y, pixel) in decoded.enumerate_pixels() {
            let packed =
                c.pixels[(y as usize + top as usize) * c.stride + x as usize + left as usize];
            let [r, mut g, b] = Format::from_native(c.format).unwrap().decode(packed);
            if c.format == 2 && c.reduce != 0 {
                g &= !4;
            }
            assert_eq!(pixel.0, [r, g, b]);
        }
        checked += 1;
    }
    checked
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_bmp_headers_rows_crops_colors_and_untouched_padding_match() {
    let report: Report = serde_json::from_str(include_str!("fixtures/portrait-bmp.json")).unwrap();
    assert_eq!(report.cases.len(), 83);
    assert_eq!(check(report.cases), 80);
}

#[test]
#[ignore = "requires private source portrait and native BMP evidence"]
fn actual_mashuga_portraits_export_the_same_bytes_as_original() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report: Report = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/portrait-bmp-source.json"))
            .unwrap(),
    )
    .unwrap();
    let count = report.cases.len();
    assert_eq!(count, 4);
    assert_eq!(check(report.cases), count);
    let saved: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-portrait-job-session.json")).unwrap(),
    )
    .unwrap();
    let images: sims_render::portrait::cache::Images =
        serde_json::from_value(saved[2].clone()).unwrap();
    let faces = images.get(3).unwrap();
    assert_eq!([faces.image.width, faces.image.height], [105, 41]);
    std::fs::write(
        root.join("cache/original-portrait-persistable.bmp"),
        faces.bmp().unwrap(),
    )
    .unwrap();
}
