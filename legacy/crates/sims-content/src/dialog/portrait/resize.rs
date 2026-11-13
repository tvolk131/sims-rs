//! Pixel portion of 4ba79c. Two passes quantize through the original 16-bit
//! format. Image allocation/locking are owned Rust services; this module does
//! not pretend to reproduce Windows surface allocation or lock failures.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    Rgb555,
    Rgb565,
}
impl Format {
    pub fn from_native(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::Rgb555),
            2 => Some(Self::Rgb565),
            _ => None,
        }
    }
    /// 438ed1 / 438ef6 retain zero low bits, without bit replication.
    pub fn decode(self, pixel: u16) -> [u8; 3] {
        match self {
            Self::Rgb555 => [
                ((pixel >> 10) as u8 & 31) << 3,
                ((pixel >> 5) as u8 & 31) << 3,
                (pixel as u8 & 31) << 3,
            ],
            Self::Rgb565 => [
                ((pixel >> 11) as u8) << 3,
                ((pixel >> 5) as u8 & 63) << 2,
                (pixel as u8 & 31) << 3,
            ],
        }
    }
    /// 4bb448 / 4bb479.
    pub fn encode(self, rgb: [u8; 3]) -> u16 {
        let [r, g, b] = rgb.map(u16::from);
        match self {
            Self::Rgb555 => ((r & 0xf8) << 7) | ((g & 0xf8) << 2) | (b >> 3),
            Self::Rgb565 => ((r & 0xf8) << 8) | ((g & 0xfc) << 3) | (b >> 3),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u16>,
    pub alpha: Option<Vec<u8>>,
}
#[derive(Clone, Copy)]
pub struct Source<'a> {
    pub pixels: &'a [u16],
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub key: Option<u16>,
}

fn zeros<T: Clone + Default>(count: usize) -> Result<Vec<T>, String> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(count)
        .map_err(|_| "portrait image allocation failed")?;
    result.resize(count, T::default());
    Ok(result)
}

// The original uses the horizontal support's slot spacing for BOTH axes. A
// wider vertical filter may overwrite the end of the preceding row's weights.
// Keep that aliasing, including the order of normalization before later writes.
struct Filters {
    spacing: usize,
    counts: Vec<usize>,
    weights: Vec<(usize, f32)>,
}
impl Filters {
    fn build(extent: usize, output: usize, spacing: usize) -> Result<Self, String> {
        let scale = (output as f64 / f64::from(extent as f32)) as f32;
        let support = if scale < 1. {
            1. / f64::from(scale)
        } else {
            1.
        };
        let reciprocal = (1. / support) as f32;
        let support = f64::from(support as f32);
        let extra = (2. * support + 3.) as usize;
        let size = output
            .checked_mul(spacing)
            .and_then(|v| v.checked_add(extra))
            .ok_or("portrait filter size overflow")?;
        let mut result = Self {
            spacing,
            counts: zeros(output)?,
            weights: zeros(size)?,
        };
        for index in 0..output {
            let center = index as f64 / f64::from(scale);
            let start = (center - support).ceil() as i64;
            // The start uses the unspilled quotient; the end and kernel use
            // the explicitly stored f32 center.
            let center = f64::from(center as f32);
            let end = (center + support).floor() as i64;
            let mut total = 0.;
            for (n, sample) in (start..=end).enumerate() {
                let distance = ((center - sample as f64) * f64::from(reciprocal)).abs();
                let weight = if distance >= 1. {
                    0.
                } else {
                    ((distance + distance) - 3.) * distance * distance + 1.
                } * f64::from(reciprocal);
                total += weight;
                let mut reflected = sample;
                while reflected < 0 || reflected >= extent as i64 {
                    if reflected < 0 {
                        reflected = -reflected;
                    }
                    if reflected >= extent as i64 {
                        reflected = 2 * extent as i64 - reflected - 2;
                    }
                }
                let slot = index * spacing + n;
                *result
                    .weights
                    .get_mut(slot)
                    .ok_or("portrait filter outside workspace")? =
                    (reflected as usize, weight as f32);
                result.counts[index] = n + 1;
            }
            let normalization = if total > 1e-12 { 1. / total } else { total };
            for n in 0..result.counts[index] {
                let weight = &mut result.weights[index * spacing + n].1;
                *weight = (f64::from(*weight) * normalization) as f32;
            }
        }
        Ok(result)
    }
    fn row(&self, index: usize) -> &[(usize, f32)] {
        &self.weights[index * self.spacing..index * self.spacing + self.counts[index]]
    }
}

fn sample(
    format: Format,
    filters: &[(usize, f32)],
    pixel: impl Fn(usize) -> u16,
    key: Option<u16>,
    alpha: bool,
) -> (u16, u8) {
    let decoded_key = key.map(|k| format.decode(k));
    let mut sum = [0f32; 3];
    let mut total = 0f32;
    for &(index, weight) in filters {
        let rgb = format.decode(pixel(index));
        if Some(rgb) == decoded_key {
            continue;
        }
        for channel in 0..3 {
            // x87 accumulates the unrounded product then spills each sum.
            sum[channel] =
                (f64::from(rgb[channel]) * f64::from(weight) + f64::from(sum[channel])) as f32;
        }
        total += weight;
    }
    if (alpha && total == 0.) || (!alpha && total < 0.46875) {
        return (key.unwrap_or(0), 0);
    }
    let reciprocal = 1. / f64::from(total);
    let rgb = sum.map(|n| (f64::from(n) * reciprocal + 0.5) as u8);
    (
        format.encode(rgb),
        (f64::from(total) * 255. + 0.5).clamp(0., 255.) as u8,
    )
}

/// Native pixel results for initialized, addressable surfaces. `None` is the
/// original early rejection (too-small scale or empty rectangle). Nonfinite,
/// out-of-range storage and single-pixel source axes are explicit errors: the
/// original mirror loop does not terminate for an axis of length one.
pub fn resize(
    format: Format,
    source: Source<'_>,
    scale: [f32; 2],
    rect: Option<[i32; 4]>,
    alpha: bool,
) -> Result<Option<Image>, String> {
    if scale.iter().any(|v| !v.is_finite()) {
        return Err("nonfinite portrait scale".into());
    }
    if scale.iter().any(|v| *v < 1e-6) {
        return Ok(None);
    }
    if source.width > i32::MAX as usize || source.height > i32::MAX as usize {
        return Err("portrait source dimensions overflow".into());
    }
    let [x, y, right, bottom] = rect.unwrap_or([0, 0, source.width as i32, source.height as i32]);
    if right <= x || bottom <= y {
        return Ok(None);
    }
    if x < 0
        || y < 0
        || right as usize > source.width
        || bottom as usize > source.height
        || source.width > source.stride
        || (source.height - 1)
            .checked_mul(source.stride)
            .and_then(|n| n.checked_add(source.width))
            .is_none_or(|end| end > source.pixels.len())
    {
        return Err("portrait source pixels outside surface".into());
    }
    let width = (right - x) as usize;
    let height = (bottom - y) as usize;
    if width == 1 || height == 1 {
        return Err(
            "native portrait reflection requires source axes of at least two pixels".into(),
        );
    }
    let dims: [f64; 2] =
        std::array::from_fn(|axis| ([width, height][axis] as f64 * f64::from(scale[axis])).ceil());
    if dims.iter().any(|n| *n > i32::MAX as f64) {
        return Err("portrait output dimensions overflow".into());
    }
    let [out_width, out_height] = [dims[0] as usize, dims[1] as usize];
    if out_width == 0 || out_height == 0 {
        return Ok(None);
    }
    let xscale = (out_width as f64 / f64::from(width as f32)) as f32;
    let support = if xscale < 1. {
        (1. / f64::from(xscale)) as f32
    } else {
        1.
    };
    let spacing = (2. * f64::from(support) + 1.) as usize;
    let horizontal = Filters::build(width, out_width, spacing)?;
    let mut intermediate = zeros::<u16>(
        out_width
            .checked_mul(height)
            .ok_or("portrait intermediate size overflow")?,
    )?;
    for row in 0..height {
        for col in 0..out_width {
            intermediate[row * out_width + col] = sample(
                format,
                horizontal.row(col),
                |i| source.pixels[(y as usize + row) * source.stride + x as usize + i],
                source.key,
                alpha,
            )
            .0;
        }
    }
    let vertical = Filters::build(height, out_height, spacing)?;
    let count = out_width
        .checked_mul(out_height)
        .ok_or("portrait output size overflow")?;
    let mut image = Image {
        width: out_width,
        height: out_height,
        pixels: zeros(count)?,
        alpha: if alpha { Some(zeros(count)?) } else { None },
    };
    // The original does not read the intermediate alpha plane. The vertical
    // pass again tests decoded color-key equality and derives its own alpha.
    for col in 0..out_width {
        for row in 0..out_height {
            let (color, opacity) = sample(
                format,
                vertical.row(row),
                |i| intermediate[i * out_width + col],
                source.key,
                alpha,
            );
            image.pixels[row * out_width + col] = color;
            if let Some(values) = &mut image.alpha {
                values[row * out_width + col] = opacity;
            }
        }
    }
    Ok(Some(image))
}
