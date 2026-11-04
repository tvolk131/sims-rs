//! Screenshot diagnostics. Metadata and geometry provenance remain the capture owner's responsibility.
use anyhow::{Result, ensure};
use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    Rust,
    Original,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CameraCenter {
    /// Existing native screen-space capture metadata remains unchanged.
    NativeScreen([i32; 2]),
    WorldTiles {
        world_tiles: [f64; 3],
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub projection: String,
    pub center: CameraCenter,
    pub rotation: u8,
    pub zoom: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world_scale: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    pub engine: Engine,
    pub profile: String,
    pub source: String,
    pub source_state_schema: Option<String>,
    pub source_state_sha256: Option<String>,
    pub assets_sha256: Option<String>,
    pub tick: Option<u64>,
    pub phase: String,
    pub camera: Camera,
    pub floor: u8,
    pub wall_mode: String,
    pub cutaway_mask_sha256: Option<String>,
    pub viewport: Rect,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capture {
    pub image: PathBuf,
    pub metadata: Metadata,
    pub geometry_mask: Option<PathBuf>,
    pub geometry_schema: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub max_different_pixels: u64,
    pub max_channel_delta: u8,
    pub max_absolute_error_sum: u64,
}
impl Limits {
    pub const EXACT: Self = Self {
        max_different_pixels: 0,
        max_channel_delta: 0,
        max_absolute_error_sum: 0,
    };
    fn permits(self, m: &Metrics) -> bool {
        m.different_pixels <= self.max_different_pixels
            && m.max_channel_delta <= self.max_channel_delta
            && m.absolute_error_sum <= self.max_absolute_error_sum
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Region {
    pub name: String,
    /// Coordinates within the declared viewport, not the complete screenshot.
    pub rect: Rect,
    /// None means exact. Nonzero limits must be deliberately supplied for this region.
    pub limits: Option<Limits>,
    pub tolerance_basis: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Strict,
    Illustrative,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub schema: u32,
    pub mode: Mode,
    pub reference: Capture,
    pub candidate: Capture,
    pub regions: Vec<Region>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub pixels: u64,
    pub different_pixels: u64,
    pub max_channel_delta: u8,
    pub absolute_error_sum: u64,
    pub mean_absolute_channel_error: f64,
    pub difference_bounds: Option<Rect>,
}
#[derive(Debug, Serialize)]
pub struct RegionReport {
    pub name: String,
    pub metrics: Metrics,
    pub limits: Limits,
    pub within_limits: bool,
}
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Passed,
    Failed,
    NotComparable,
    Illustrative,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema: u32,
    pub input_directory: Option<PathBuf>,
    pub status: Status,
    pub mode: Mode,
    pub reference: Capture,
    pub candidate: Capture,
    pub metadata_issues: Vec<String>,
    pub policy_issues: Vec<String>,
    pub regions: Vec<RegionReport>,
    pub geometry: Option<Metrics>,
    pub note: String,
}
pub struct Loaded {
    pub capture: Capture,
    pub pixels: RgbaImage,
    pub geometry: Option<RgbaImage>,
}
pub struct Compared {
    pub report: Report,
    pub difference: Option<RgbaImage>,
    pub overlay: Option<RgbaImage>,
    pub geometry_difference: Option<RgbaImage>,
}

fn decode(path: &Path) -> Result<RgbaImage> {
    let image = image::open(path)?;
    ensure!(
        matches!(
            image.color(),
            image::ColorType::L8
                | image::ColorType::La8
                | image::ColorType::Rgb8
                | image::ColorType::Rgba8
        ),
        "comparison requires 8-bit image channels; refusing lossy conversion"
    );
    Ok(image.to_rgba8())
}

pub fn load(capture: Capture, base: &Path) -> Result<Loaded> {
    let image = decode(&base.join(&capture.image))?;
    let dimensions = image.dimensions();
    let pixels = crop(&image, capture.metadata.viewport)?;
    let geometry = capture
        .geometry_mask
        .as_ref()
        .map(|path| -> Result<_> {
            let image = decode(&base.join(path))?;
            ensure!(
                image.dimensions() == dimensions,
                "geometry mask must align with the complete capture"
            );
            ensure!(
                capture
                    .geometry_schema
                    .as_ref()
                    .is_some_and(|s| !s.trim().is_empty()),
                "geometry mask schema missing"
            );
            crop(&image, capture.metadata.viewport)
        })
        .transpose()?;
    Ok(Loaded {
        capture,
        pixels,
        geometry,
    })
}
fn crop(image: &RgbaImage, r: Rect) -> Result<RgbaImage> {
    ensure!(
        r.width > 0
            && r.height > 0
            && r.x.checked_add(r.width).is_some_and(|n| n <= image.width())
            && r.y
                .checked_add(r.height)
                .is_some_and(|n| n <= image.height()),
        "viewport outside image"
    );
    Ok(image::imageops::crop_imm(image, r.x, r.y, r.width, r.height).to_image())
}
fn hash_available(value: &Option<String>) -> bool {
    value
        .as_ref()
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn metadata_issues(a: &Metadata, b: &Metadata) -> Vec<String> {
    let mut issues = vec![];
    for (side, m) in [("reference", a), ("candidate", b)] {
        for (name, known) in [
            ("profile", !m.profile.trim().is_empty()),
            ("source", !m.source.trim().is_empty()),
            (
                "source_state_schema",
                m.source_state_schema
                    .as_ref()
                    .is_some_and(|s| !s.trim().is_empty()),
            ),
            (
                "source_state_sha256",
                hash_available(&m.source_state_sha256),
            ),
            ("assets_sha256", hash_available(&m.assets_sha256)),
            ("tick", m.tick.is_some()),
            ("phase", !m.phase.trim().is_empty()),
            ("camera.projection", !m.camera.projection.trim().is_empty()),
            ("camera.rotation", m.camera.rotation < 4),
            ("camera.zoom", (1..=3).contains(&m.camera.zoom)),
            (
                "camera.center/scale",
                match &m.camera.center {
                    CameraCenter::NativeScreen(_) => m.camera.world_scale.is_none(),
                    CameraCenter::WorldTiles { world_tiles } => {
                        world_tiles.iter().all(|v| v.is_finite())
                            && m.camera
                                .world_scale
                                .is_some_and(|s| s.is_finite() && s > 0.)
                    }
                },
            ),
            ("floor", (1..=2).contains(&m.floor)),
            (
                "wall_mode",
                ["walls_up", "walls_down", "cutaway", "roof"].contains(&m.wall_mode.as_str()),
            ),
            (
                "cutaway_mask_sha256",
                m.wall_mode != "cutaway" || hash_available(&m.cutaway_mask_sha256),
            ),
        ] {
            if !known {
                issues.push(format!("{side}.{name} unavailable or invalid"));
            }
        }
    }
    let a = serde_json::to_value(a).unwrap();
    let b = serde_json::to_value(b).unwrap();
    for (field, value) in a.as_object().unwrap() {
        // Engine identity differs by definition in a cross-engine comparison.
        if field != "engine" && Some(value) != b.get(field) {
            issues.push(format!("{field} differs"));
        }
    }
    issues
}
fn validate_regions(regions: &[Region], width: u32, height: u32) -> Result<()> {
    ensure!(
        !regions.is_empty(),
        "regions must explicitly cover the entire viewport"
    );
    let count = u64::from(width) * u64::from(height);
    ensure!(
        count > 0 && count <= 16_777_216,
        "comparison viewport exceeds pixel budget"
    );
    let mut coverage = vec![false; count as usize];
    let mut names = BTreeSet::new();
    for region in regions {
        ensure!(
            !region.name.trim().is_empty() && names.insert(&region.name),
            "region names must be nonempty and unique"
        );
        let r = region.rect;
        ensure!(
            r.width > 0
                && r.height > 0
                && r.x.checked_add(r.width).is_some_and(|n| n <= width)
                && r.y.checked_add(r.height).is_some_and(|n| n <= height),
            "region outside viewport"
        );
        for y in r.y..r.y + r.height {
            for x in r.x..r.x + r.width {
                let pixel = &mut coverage[(y * width + x) as usize];
                ensure!(!*pixel, "regions overlap");
                *pixel = true;
            }
        }
    }
    ensure!(
        coverage.iter().all(|v| *v),
        "regions leave uncovered pixels"
    );
    Ok(())
}
pub fn metrics(a: &RgbaImage, b: &RgbaImage, r: Rect) -> Metrics {
    let mut different = 0;
    let mut maximum = 0;
    let mut sum = 0u64;
    let mut bounds = [u32::MAX, u32::MAX, 0, 0];
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            let p = a.get_pixel(x, y);
            let q = b.get_pixel(x, y);
            if p != q {
                different += 1;
                bounds[0] = bounds[0].min(x);
                bounds[1] = bounds[1].min(y);
                bounds[2] = bounds[2].max(x);
                bounds[3] = bounds[3].max(y);
            }
            for c in 0..4 {
                let d = p[c].abs_diff(q[c]);
                maximum = maximum.max(d);
                sum += u64::from(d);
            }
        }
    }
    let pixels = u64::from(r.width) * u64::from(r.height);
    Metrics {
        pixels,
        different_pixels: different,
        max_channel_delta: maximum,
        absolute_error_sum: sum,
        mean_absolute_channel_error: sum as f64 / (pixels * 4) as f64,
        difference_bounds: (different != 0).then(|| Rect {
            x: bounds[0],
            y: bounds[1],
            width: bounds[2] - bounds[0] + 1,
            height: bounds[3] - bounds[1] + 1,
        }),
    }
}
fn images(a: &RgbaImage, b: &RgbaImage) -> (RgbaImage, RgbaImage) {
    let mut difference = RgbaImage::new(a.width(), a.height());
    let mut overlay = difference.clone();
    for (x, y, p) in a.enumerate_pixels() {
        let q = b.get_pixel(x, y);
        let delta = std::array::from_fn::<_, 4, _>(|c| p[c].abs_diff(q[c]));
        // Gain is only for visibility; numeric comparisons use unscaled RGBA.
        difference.put_pixel(
            x,
            y,
            Rgba([
                delta[0].max(delta[3]).saturating_mul(4),
                delta[1].saturating_mul(4),
                delta[2].max(delta[3]).saturating_mul(4),
                255,
            ]),
        );
        overlay.put_pixel(
            x,
            y,
            Rgba([
                ((u16::from(p[0]) + u16::from(q[0])) / 2) as u8,
                ((u16::from(p[1]) + u16::from(q[1])) / 2) as u8,
                ((u16::from(p[2]) + u16::from(q[2])) / 2) as u8,
                255,
            ]),
        );
    }
    (difference, overlay)
}
pub fn compare(job: &Job, a: &Loaded, b: &Loaded) -> Result<Compared> {
    ensure!(job.schema == 1, "unsupported comparison schema");
    let mut issues = metadata_issues(&a.capture.metadata, &b.capture.metadata);
    let mut policy = vec![];
    let rust_only =
        a.capture.metadata.engine == Engine::Rust && b.capture.metadata.engine == Engine::Rust;
    let allows_difference = job
        .regions
        .iter()
        .any(|r| r.limits.unwrap_or(Limits::EXACT) != Limits::EXACT);
    if rust_only && allows_difference {
        policy.push("Rust-versus-Rust requires exact pixels; nonzero limits are forbidden".into());
    }
    for r in &job.regions {
        if r.limits.unwrap_or(Limits::EXACT) != Limits::EXACT
            && !r
                .tolerance_basis
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty())
        {
            policy.push(format!(
                "{}: nonzero tolerance has no explicit basis",
                r.name
            ));
        }
    }
    let sizes_match = a.pixels.dimensions() == b.pixels.dimensions();
    if !sizes_match {
        issues.push(
            "decoded viewport dimensions differ; no implicit resizing/alignment is performed"
                .into(),
        );
    }
    let mut reports = vec![];
    let mut difference = None;
    let mut overlay = None;
    let mut geometry = None;
    let mut geometry_difference = None;
    if sizes_match {
        validate_regions(&job.regions, a.pixels.width(), a.pixels.height())?;
        for r in &job.regions {
            let m = metrics(&a.pixels, &b.pixels, r.rect);
            let limits = r.limits.unwrap_or(Limits::EXACT);
            let within_limits = limits.permits(&m);
            reports.push(RegionReport {
                name: r.name.clone(),
                metrics: m,
                limits,
                within_limits,
            });
        }
        let (d, o) = images(&a.pixels, &b.pixels);
        difference = Some(d);
        overlay = Some(o);
        if let (Some(x), Some(y)) = (&a.geometry, &b.geometry) {
            ensure!(
                x.dimensions() == a.pixels.dimensions() && y.dimensions() == b.pixels.dimensions(),
                "geometry mask dimensions differ"
            );
            if a.capture.geometry_schema != b.capture.geometry_schema {
                policy.push("geometry schemas differ".into());
            }
            if !a
                .capture
                .geometry_schema
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty())
                || !b
                    .capture
                    .geometry_schema
                    .as_ref()
                    .is_some_and(|s| !s.trim().is_empty())
            {
                policy.push("geometry schema missing".into());
            }
            geometry = Some(metrics(
                x,
                y,
                Rect {
                    x: 0,
                    y: 0,
                    width: x.width(),
                    height: x.height(),
                },
            ));
            geometry_difference = Some(images(x, y).0);
        }
    }
    if !rust_only && allows_difference && geometry.is_none() {
        policy.push("nonzero cross-engine tolerances require aligned full-viewport geometry masks from both captures".into());
    }
    let status = if job.mode == Mode::Illustrative {
        Status::Illustrative
    } else if !issues.is_empty() || !policy.is_empty() {
        Status::NotComparable
    } else if reports.iter().all(|r| r.within_limits)
        && geometry.as_ref().is_none_or(|m| m.different_pixels == 0)
    {
        Status::Passed
    } else {
        Status::Failed
    };
    Ok(Compared {
        report: Report {
            schema: 1,
            input_directory: None,
            status,
            mode: job.mode,
            reference: a.capture.clone(),
            candidate: b.capture.clone(),
            metadata_issues: issues,
            policy_issues: policy,
            regions: reports,
            geometry,
            note: "Pixel comparison does not establish gameplay parity or authenticate supplied metadata/masks. Difference PNG uses 4x channel gain; overlay is a 50/50 RGB blend.".into(),
        },
        difference,
        overlay,
        geometry_difference,
    })
}

#[derive(Debug, Deserialize)]
pub struct VarianceJob {
    pub schema: u32,
    pub samples: Vec<Capture>,
    pub regions: Vec<Region>,
}
pub fn variance(job: &VarianceJob, base: &Path) -> Result<serde_json::Value> {
    ensure!(
        job.schema == 1 && job.samples.len() >= 2,
        "variance requires at least two repeated original captures"
    );
    let paths = job
        .samples
        .iter()
        .map(|c| base.join(&c.image).canonicalize())
        .collect::<std::io::Result<BTreeSet<_>>>()?;
    ensure!(
        paths.len() == job.samples.len(),
        "variance samples must be distinct capture files"
    );
    let samples = job
        .samples
        .iter()
        .cloned()
        .map(|c| load(c, base))
        .collect::<Result<Vec<_>>>()?;
    let first = &samples[0];
    ensure!(
        samples
            .iter()
            .all(|s| s.capture.metadata.engine == Engine::Original
                && metadata_issues(&first.capture.metadata, &s.capture.metadata).is_empty()),
        "variance samples need complete identical original capture metadata"
    );
    ensure!(
        samples
            .iter()
            .all(|s| s.pixels.dimensions() == first.pixels.dimensions()),
        "variance dimensions differ"
    );
    validate_regions(&job.regions, first.pixels.width(), first.pixels.height())?;
    let mut rows = vec![];
    for r in &job.regions {
        let mut limits = Limits::EXACT;
        let mut pairs = vec![];
        for i in 0..samples.len() {
            for j in i + 1..samples.len() {
                let m = metrics(&samples[i].pixels, &samples[j].pixels, r.rect);
                limits.max_different_pixels = limits.max_different_pixels.max(m.different_pixels);
                limits.max_channel_delta = limits.max_channel_delta.max(m.max_channel_delta);
                limits.max_absolute_error_sum =
                    limits.max_absolute_error_sum.max(m.absolute_error_sum);
                pairs.push(serde_json::json!({"samples":[i,j],"metrics":m}));
            }
        }
        rows.push(serde_json::json!({"name":r.name,"rect":r.rect,"observed_pairwise_limits":limits,"pairs":pairs}));
    }
    Ok(
        serde_json::json!({"schema":1,"input_directory":base.canonicalize()?,"scope":"Observed repeated-original pairwise variance; proposed bounds only, never automatically applied to comparison policy.","samples":job.samples,"regions":rows}),
    )
}
pub fn write_comparison(job: &Job, base: &Path, out: &Path) -> Result<Status> {
    ensure!(!out.exists(), "output directory must be fresh");
    let a = load(job.reference.clone(), base)?;
    let b = load(job.candidate.clone(), base)?;
    let mut result = compare(job, &a, &b)?;
    result.report.input_directory = Some(base.canonicalize()?);
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("input-job.json"), serde_json::to_vec_pretty(job)?)?;
    if let Some(image) = result.difference {
        image.save(out.join("difference.png"))?;
    }
    if let Some(image) = result.overlay {
        image.save(out.join("overlay.png"))?;
    }
    if let Some(image) = result.geometry_difference {
        image.save(out.join("geometry-difference.png"))?;
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&result.report)?,
    )?;
    Ok(result.report.status)
}
