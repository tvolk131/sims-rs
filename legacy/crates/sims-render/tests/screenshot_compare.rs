#[path = "../examples/support/screenshot_compare.rs"]
mod comparison;
use comparison::*;
use image::{Rgba, RgbaImage};
use std::path::PathBuf;

fn metadata(engine: Engine) -> Metadata {
    Metadata {
        engine,
        profile: "complete-collection".into(),
        source: "House05.iff".into(),
        source_state_schema: Some("synthetic-scene-v1".into()),
        source_state_sha256: Some("a".repeat(64)),
        assets_sha256: Some("b".repeat(64)),
        tick: Some(20),
        phase: "paused".into(),
        camera: Camera {
            projection: "native-iso".into(),
            center: CameraCenter::NativeScreen([300, 20]),
            rotation: 0,
            zoom: 2,
            world_scale: None,
        },
        floor: 1,
        wall_mode: "walls_up".into(),
        cutaway_mask_sha256: None,
        viewport: Rect {
            x: 0,
            y: 0,
            width: 8,
            height: 4,
        },
    }
}
fn capture(engine: Engine) -> Capture {
    Capture {
        image: PathBuf::from("image.png"),
        metadata: metadata(engine),
        geometry_mask: None,
        geometry_schema: None,
    }
}
fn loaded(engine: Engine) -> Loaded {
    Loaded {
        capture: capture(engine),
        pixels: RgbaImage::from_pixel(8, 4, Rgba([250, 250, 250, 255])),
        geometry: None,
    }
}
fn regions() -> Vec<Region> {
    [("left", 0), ("right", 4)]
        .map(|(name, x)| Region {
            name: name.into(),
            rect: Rect {
                x,
                y: 0,
                width: 4,
                height: 4,
            },
            limits: None,
            tolerance_basis: None,
        })
        .to_vec()
}
fn job(a: &Loaded, b: &Loaded) -> Job {
    Job {
        schema: 1,
        mode: Mode::Strict,
        reference: a.capture.clone(),
        candidate: b.capture.clone(),
        regions: regions(),
    }
}
fn allow(job: &mut Job, limit: u8) {
    for r in &mut job.regions {
        r.limits = Some(Limits {
            max_different_pixels: 1,
            max_channel_delta: limit,
            max_absolute_error_sum: u64::from(limit),
        });
        r.tolerance_basis = Some("deliberately supplied synthetic threshold".into());
    }
}
fn masks(a: &mut Loaded, b: &mut Loaded) {
    for c in [a, b] {
        c.geometry = Some(RgbaImage::from_pixel(8, 4, Rgba([1, 0, 0, 255])));
        c.capture.geometry_schema = Some("visible-geometry-identity-v1".into());
    }
}

#[test]
fn identical_decoded_pixels_require_zero_difference() {
    let a = loaded(Engine::Rust);
    let b = loaded(Engine::Rust);
    let result = compare(&job(&a, &b), &a, &b).unwrap();
    assert_eq!(result.report.status, Status::Passed);
    assert!(
        result
            .report
            .regions
            .iter()
            .all(|r| r.metrics.different_pixels == 0)
    );
    assert!(
        result
            .difference
            .unwrap()
            .pixels()
            .all(|p| p.0 == [0, 0, 0, 255])
    );
}
#[test]
fn region_metrics_locate_meaningful_changes_without_diluting_them() {
    let a = loaded(Engine::Rust);
    let mut b = loaded(Engine::Rust);
    b.pixels.put_pixel(6, 2, Rgba([20, 40, 80, 255]));
    let result = compare(&job(&a, &b), &a, &b).unwrap();
    assert_eq!(result.report.status, Status::Failed);
    assert_eq!(result.report.regions[0].metrics.different_pixels, 0);
    let m = &result.report.regions[1].metrics;
    assert_eq!(
        (
            m.different_pixels,
            m.max_channel_delta,
            m.absolute_error_sum
        ),
        (1, 230, 610)
    );
    assert_eq!(
        m.difference_bounds,
        Some(Rect {
            x: 6,
            y: 2,
            width: 1,
            height: 1
        })
    );
    assert_eq!(
        result.overlay.unwrap().get_pixel(6, 2).0,
        [135, 145, 165, 255]
    );
}
#[test]
fn metadata_mismatch_cannot_be_labelled_strict_parity() {
    let a = loaded(Engine::Original);
    let mut b = loaded(Engine::Rust);
    b.capture.metadata.camera.zoom = 1;
    b.capture.metadata.floor = 2;
    b.capture.metadata.tick = Some(21);
    b.capture.metadata.source_state_sha256 = None;
    let mut j = job(&a, &b);
    let result = compare(&j, &a, &b).unwrap();
    assert_eq!(result.report.status, Status::NotComparable);
    assert!(
        result
            .report
            .metadata_issues
            .iter()
            .any(|s| s == "camera differs")
    );
    assert!(
        result
            .report
            .metadata_issues
            .iter()
            .any(|s| s == "tick differs")
    );
    j.mode = Mode::Illustrative;
    assert_eq!(
        compare(&j, &a, &b).unwrap().report.status,
        Status::Illustrative
    );
}
#[test]
fn thresholds_are_explicit_and_violations_fail() {
    let mut a = loaded(Engine::Original);
    let mut b = loaded(Engine::Rust);
    masks(&mut a, &mut b);
    b.pixels.put_pixel(6, 2, Rgba([244, 250, 250, 255]));
    let mut j = job(&a, &b);
    allow(&mut j, 5);
    assert_eq!(compare(&j, &a, &b).unwrap().report.status, Status::Failed);
    allow(&mut j, 6);
    assert_eq!(compare(&j, &a, &b).unwrap().report.status, Status::Passed);
    j.regions[0].tolerance_basis = None;
    assert_eq!(
        compare(&j, &a, &b).unwrap().report.status,
        Status::NotComparable
    );
    a.capture.metadata.engine = Engine::Rust;
    assert_eq!(
        compare(&job(&a, &b), &a, &b).unwrap().report.status,
        Status::Failed
    );
    let mut j = job(&a, &b);
    allow(&mut j, 6);
    assert_eq!(
        compare(&j, &a, &b).unwrap().report.status,
        Status::NotComparable
    );
}
#[test]
fn appearance_tolerances_cannot_hide_missing_geometry() {
    let mut a = loaded(Engine::Original);
    let mut b = loaded(Engine::Rust);
    let mut j = job(&a, &b);
    allow(&mut j, 255);
    assert_eq!(
        compare(&j, &a, &b).unwrap().report.status,
        Status::NotComparable
    );
    masks(&mut a, &mut b);
    b.geometry
        .as_mut()
        .unwrap()
        .put_pixel(2, 1, Rgba([0, 0, 0, 255]));
    let result = compare(&j, &a, &b).unwrap();
    assert!(result.report.regions.iter().all(|r| r.within_limits));
    assert_eq!(result.report.geometry.unwrap().different_pixels, 1);
    assert_eq!(result.report.status, Status::Failed);
}
#[test]
fn regions_must_cover_every_pixel_once() {
    let a = loaded(Engine::Rust);
    let b = loaded(Engine::Rust);
    let mut j = job(&a, &b);
    j.regions.pop();
    assert!(compare(&j, &a, &b).is_err());
    j.regions = regions();
    j.regions[1].rect.x = 3;
    assert!(compare(&j, &a, &b).is_err());
}

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "sims-screenshot-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn file_decoding_and_artifacts_are_independent_of_encoding() {
    let tmp = Temp::new();
    let image = loaded(Engine::Rust).pixels;
    image.save(tmp.0.join("a.png")).unwrap();
    image.save(tmp.0.join("b.bmp")).unwrap();
    assert_ne!(
        std::fs::read(tmp.0.join("a.png")).unwrap(),
        std::fs::read(tmp.0.join("b.bmp")).unwrap()
    );
    let mut a = capture(Engine::Rust);
    a.image = "a.png".into();
    let mut b = capture(Engine::Rust);
    b.image = "b.bmp".into();
    let j = Job {
        schema: 1,
        mode: Mode::Strict,
        reference: a,
        candidate: b,
        regions: regions(),
    };
    assert_eq!(
        write_comparison(&j, &tmp.0, &tmp.0.join("report")).unwrap(),
        Status::Passed
    );
    for file in ["difference.png", "overlay.png", "report.json"] {
        assert!(tmp.0.join("report").join(file).exists());
    }
}
#[test]
fn repeated_original_variance_is_measured_but_not_auto_applied() {
    let tmp = Temp::new();
    let a = loaded(Engine::Original).pixels;
    let mut b = a.clone();
    b.put_pixel(6, 2, Rgba([247, 250, 250, 255]));
    a.save(tmp.0.join("a.png")).unwrap();
    b.save(tmp.0.join("b.png")).unwrap();
    let mut c1 = capture(Engine::Original);
    c1.image = "a.png".into();
    let mut c2 = c1.clone();
    c2.image = "b.png".into();
    let mut j = VarianceJob {
        schema: 1,
        samples: vec![c1, c2],
        regions: regions(),
    };
    let report = variance(&j, &tmp.0).unwrap();
    assert_eq!(
        report["regions"][0]["observed_pairwise_limits"]["max_different_pixels"],
        0
    );
    assert_eq!(
        report["regions"][1]["observed_pairwise_limits"]["max_channel_delta"],
        3
    );
    j.samples[1].metadata.tick = Some(21);
    assert!(variance(&j, &tmp.0).is_err());
}

#[test]
fn higher_bit_depth_is_rejected_instead_of_silently_quantized() {
    let tmp = Temp::new();
    let image = image::ImageBuffer::<Rgba<u16>, Vec<u16>>::from_pixel(
        8,
        4,
        Rgba([1000, 1000, 1000, 65535]),
    );
    image.save(tmp.0.join("image.png")).unwrap();
    assert!(load(capture(Engine::Rust), &tmp.0).is_err());
}

#[test]
fn different_viewport_sizes_are_reported_without_implicit_alignment() {
    let a = loaded(Engine::Original);
    let mut b = loaded(Engine::Rust);
    b.pixels = RgbaImage::new(4, 4);
    b.capture.metadata.viewport.width = 4;
    let result = compare(&job(&a, &b), &a, &b).unwrap();
    assert_eq!(result.report.status, Status::NotComparable);
    assert!(result.difference.is_none());
    assert!(result.report.regions.is_empty());
}

#[test]
fn native_and_world_camera_coordinates_are_explicit_and_never_converted() {
    let a = loaded(Engine::Rust);
    let mut b = loaded(Engine::Rust);
    let json = serde_json::to_value(&a.capture.metadata.camera).unwrap();
    assert_eq!(json["center"], serde_json::json!([300, 20]));
    assert!(json.get("world_scale").is_none());
    b.capture.metadata.camera.center = CameraCenter::WorldTiles {
        world_tiles: [300., 0., 20.],
    };
    b.capture.metadata.camera.world_scale = Some(0.5);
    assert_eq!(
        compare(&job(&a, &b), &a, &b).unwrap().report.status,
        Status::NotComparable
    );
    let a = Loaded {
        capture: b.capture.clone(),
        pixels: b.pixels.clone(),
        geometry: None,
    };
    assert_eq!(
        compare(&job(&a, &b), &a, &b).unwrap().report.status,
        Status::Passed
    );
    b.capture.metadata.camera.world_scale = None;
    assert_eq!(
        compare(&job(&a, &b), &a, &b).unwrap().report.status,
        Status::NotComparable
    );
}
