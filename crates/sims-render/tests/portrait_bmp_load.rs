use serde_json::Value;
use sims_render::portrait::{images::Raster, resize::Format};

fn check(report: Value, count: usize) {
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), count);
    for (n, case) in cases.iter().enumerate() {
        let input = case["input"]["bmp"].as_str().unwrap();
        let bytes = (0..input.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&input[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>();
        let raster = Raster::from_bmp(
            &bytes,
            Format::from_native(case["input"]["format"].as_u64().unwrap() as u32).unwrap(),
        )
        .unwrap();
        let original = &case["original"];
        assert_eq!(
            raster.image.width,
            original["width"].as_u64().unwrap() as usize
        );
        assert_eq!(
            raster.image.height,
            original["height"].as_u64().unwrap() as usize
        );
        assert_eq!(
            serde_json::to_value(&raster.image.pixels).unwrap(),
            original["pixels"],
            "native BMP load case {n}"
        );
        assert!(raster.key.is_none() && raster.image.alpha.is_none());
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_bmp_load_pixels_match_both_packings_and_row_orientations() {
    check(
        serde_json::from_str(include_str!("fixtures/portrait-bmp-load.json")).unwrap(),
        40,
    );
}

#[test]
#[ignore = "requires private native BMP loading evidence for real portraits"]
fn actual_original_and_regenerated_faces_load_like_the_original() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    check(
        serde_json::from_slice(
            &std::fs::read(
                root.join("cache/research/sims-exe/oracle/portrait-bmp-load-source.json"),
            )
            .unwrap(),
        )
        .unwrap(),
        4,
    );
}
