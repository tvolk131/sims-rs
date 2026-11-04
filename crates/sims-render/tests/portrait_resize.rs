use serde::Deserialize;
use sims_render::portrait::resize::{self, Format, Image, Source};

#[derive(Deserialize)]
struct Input {
    format: u32,
    width: usize,
    height: usize,
    padding: usize,
    pixels: Vec<u16>,
    scale: [f32; 2],
    rect: Option<[i32; 4]>,
    key: Option<u16>,
    alpha: bool,
    #[serde(default)]
    fail_allocate: usize,
    fail_create: Option<String>,
    fail_alpha: Option<String>,
    deny_lock: Option<String>,
    #[serde(default)]
    expect_cycle: bool,
}
#[derive(Deserialize)]
struct Original {
    image: Option<Image>,
    reflection_cycle: Option<[u32; 3]>,
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

#[cfg(feature = "private-fixtures")]
#[test]
fn native_two_pass_resize_matches_original_pixels() {
    let report: Report =
        serde_json::from_str(include_str!("fixtures/portrait-resize.json")).unwrap();
    assert_eq!(report.cases.len(), 201);
    let mut successes = 0;
    for (n, case) in report.cases.into_iter().enumerate() {
        let c = case.input;
        if c.expect_cycle {
            let [address, extent, index] = case.original.reflection_cycle.unwrap();
            assert!(matches!(address, 0x4babeb | 0x4bb08c));
            assert_eq!(extent, 1);
            assert!(matches!(index, 1 | u32::MAX));
            assert!(
                resize::resize(
                    Format::from_native(c.format).unwrap(),
                    Source {
                        pixels: &c.pixels,
                        width: c.width,
                        height: c.height,
                        stride: c.width + c.padding,
                        key: c.key
                    },
                    c.scale,
                    c.rect,
                    c.alpha
                )
                .is_err()
            );
            continue;
        }
        // Native allocation/virtual lock failures were also captured, but are
        // not the contract of this owned-buffer pixel implementation.
        if case.original.image.is_none()
            && (c.fail_allocate != 0
                || c.fail_create.is_some()
                || c.fail_alpha.is_some()
                || c.deny_lock.is_some())
        {
            continue;
        }
        let result = Format::from_native(c.format)
            .map(|format| {
                resize::resize(
                    format,
                    Source {
                        pixels: &c.pixels,
                        width: c.width,
                        height: c.height,
                        stride: c.width + c.padding,
                        key: c.key,
                    },
                    c.scale,
                    c.rect,
                    c.alpha,
                )
                .unwrap()
            })
            .flatten();
        assert_eq!(
            result, case.original.image,
            "case {n}: format={} scale={:?} alpha={} key={:?}",
            c.format, c.scale, c.alpha, c.key
        );
        if result.is_some() {
            successes += 1;
        }
    }
    assert_eq!(successes, 169);
}

#[test]
#[ignore = "requires private real-mesh portrait and native resize reports"]
fn real_household_portraits_match_original_resizer() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report: Report = serde_json::from_slice(
        &std::fs::read(root.join("cache/native-original-portrait-resize.json")).unwrap(),
    )
    .unwrap();
    let rust: Vec<Image> = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-portrait-resize-results.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report.cases.len(), 20);
    assert_eq!(rust.len(), report.cases.len());
    for (n, (case, drawn)) in report.cases.into_iter().zip(rust).enumerate() {
        let c = case.input;
        let image = resize::resize(
            Format::from_native(c.format).unwrap(),
            Source {
                pixels: &c.pixels,
                width: c.width,
                height: c.height,
                stride: c.width + c.padding,
                key: c.key,
            },
            c.scale,
            c.rect,
            c.alpha,
        )
        .unwrap();
        assert_eq!(image, Some(drawn), "source render {n}");
        assert_eq!(image, case.original.image, "native resize {n}");
    }
}

#[test]
fn portrait_resize_rejects_unaddressable_or_nonterminating_source() {
    let pixels = [0u16; 16];
    let source = Source {
        pixels: &pixels,
        width: 4,
        height: 4,
        stride: 4,
        key: None,
    };
    for rect in [[-1, 0, 3, 4], [0, 0, 5, 4], [0, 0, 1, 4], [0, 0, 4, 1]] {
        assert!(resize::resize(Format::Rgb565, source, [1.; 2], Some(rect), false).is_err());
    }
    for scale in [[f32::NAN, 1.], [f32::INFINITY, 1.], [f32::MAX, 1.]] {
        assert!(resize::resize(Format::Rgb565, source, scale, None, false).is_err());
    }
    assert!(
        resize::resize(
            Format::Rgb565,
            Source {
                stride: usize::MAX,
                ..source
            },
            [1.; 2],
            None,
            false
        )
        .is_err()
    );
}

#[test]
fn portrait_color_roundtrip_preserves_representable_bits() {
    for p in 0..=u16::MAX {
        assert_eq!(Format::Rgb555.encode(Format::Rgb555.decode(p)), p & 0x7fff);
        assert_eq!(Format::Rgb565.encode(Format::Rgb565.decode(p)), p);
    }
}
