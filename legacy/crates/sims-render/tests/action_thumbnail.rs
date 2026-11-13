use sims_render::portrait::{
    images::{Raster, thumbnail},
    resize::{Format, Image},
};
fn raster(width: usize, height: usize, pixel: impl Fn(usize, usize) -> u16) -> Raster {
    Raster {
        format: Format::Rgb565,
        key: None,
        image: Image {
            width,
            height,
            pixels: (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .map(|(x, y)| pixel(x, y))
                .collect(),
            alpha: None,
        },
    }
}
#[test]
fn resident_two_view_picture_uses_left_half_and_background_without_stretching() {
    let background = raster(180, 45, |x, y| (x + 180 * y) as u16);
    let mut picture = raster(74, 37, |x, y| 10000 + (x + 74 * y) as u16);
    picture.key = Some(10000);
    for initial in [
        None,
        Some(raster(13, 8, |_, _| 65535)),
        Some(raster(45, 45, |_, _| 65535)),
    ] {
        let result = thumbnail::compose(&background, &picture, initial).unwrap();
        assert!(result.success);
        let output = result.image.unwrap();
        assert_eq!([output.image.width, output.image.height], [45, 45]);
        for y in 0..45 {
            for x in 0..45 {
                let expected = if (4..41).contains(&x) && (4..41).contains(&y) && [x, y] != [4, 4] {
                    10000 + ((x - 4) + 74 * (y - 4)) as u16
                } else {
                    (x + 180 * y) as u16
                };
                assert_eq!(output.image.pixels[y * 45 + x], expected, "{x},{y}");
            }
        }
    }
}
/// Raw captured inputs exercise our CPU service with an explicit opaque/RGB565
/// profile. Native output pixels and source display/key interpretation are NOT
/// qualified, so this must not be reported as a native pixel comparison.
#[test]
#[ignore = "private captured image inputs"]
fn private_captured_resident_input_profile() {
    let path = std::env::var_os("SIMS_THUMBNAIL_EVIDENCE").expect("SIMS_THUMBNAIL_EVIDENCE");
    let path = std::path::PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    };
    let evidence: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(evidence["native_output_pixels_available"], false);
    let decode = |role: &str| {
        let v = &evidence[role];
        let layout = &v["surface_layout"];
        let width = layout["width"].as_u64().unwrap() as usize;
        let height = layout["height"].as_u64().unwrap() as usize;
        let stride = layout["stride"].as_u64().unwrap() as usize;
        let hex = v["pixels"]["raw"].as_str().unwrap();
        let raw: Vec<u8> = hex
            .as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect();
        raster(width, height, |x, y| {
            u16::from_le_bytes(raw[y * stride + 2 * x..][..2].try_into().unwrap())
        })
    };
    let background = decode("background");
    let picture = decode("cached");
    let result = thumbnail::compose(&background, &picture, None).unwrap();
    assert!(result.success);
    let output = result.image.unwrap();
    let mut expected = vec![0u16; 45 * 45];
    // Expected regions are the actual original witness's two blit arguments.
    for (n, call) in evidence["calls"].as_array().unwrap().iter().enumerate() {
        let r = |i: usize| -> Vec<usize> {
            call["rectangles"][i]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_u64().unwrap() as usize)
                .collect()
        };
        let from = r(0);
        let to = r(1);
        let source = if n == 0 { &background } else { &picture };
        assert_eq!(
            [from[2] - from[0], from[3] - from[1]],
            [to[2] - to[0], to[3] - to[1]]
        );
        for y in to[1]..to[3] {
            for x in to[0]..to[2] {
                expected[y * 45 + x] = source.image.pixels
                    [(from[1] + y - to[1]) * source.image.width + from[0] + x - to[0]];
            }
        }
    }
    assert_eq!(output.image.pixels, expected);
}
