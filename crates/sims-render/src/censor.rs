//! Native 4ac301 mosaic arithmetic on 16-bit surfaces. Projection and the
//! presentation RNG's position are caller inputs, not established by this pass.
use anyhow::{Result, ensure};

/// Renderer-owned projection of a kind-1 appearance. The native bounding-box
/// and mosaic structure is retained; this camera's screen/depth coordinates and
/// deterministic presentation seed are not claimed equivalent to a running EXE.
#[derive(Debug, Clone)]
pub struct Region {
    pub rect: [i32; 4],
    pub depth: f32,
    pub seed: u32,
}

impl Region {
    pub fn projected(points: &[[f32; 3]], zoom: f32, seed: u32) -> Option<Self> {
        if points.is_empty() {
            return None;
        }
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for p in points {
            if p.iter().any(|v| !v.is_finite()) {
                return None;
            }
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        // 4ac707 pads and shifts the projected rectangle. Our free camera
        // scales the large-view (4px, 12px) values; native zoom/projection is a
        // separate unverified adapter, as is the conservative depth plane.
        let pad = (4. * zoom).ceil();
        let shift = 12. * zoom;
        let left = (lo[0] - pad).floor() as i32;
        let top = (lo[1] - pad + shift).floor() as i32;
        let right = (hi[0] + pad).ceil() as i32;
        let bottom = (hi[1] + pad + shift).ceil() as i32;
        Some(Self {
            rect: [left, top, right - left, bottom - top],
            depth: lo[2] - 0.00001,
            seed,
        })
    }
}

pub(crate) fn composite(image: &mut image::RgbaImage, depths: &[f32], regions: &[Region]) {
    if regions.is_empty() {
        return;
    }
    let (w, h) = image.dimensions();
    let mut pixels: Vec<_> = image
        .pixels()
        .map(|p| pack([p[0], p[1], p[2]], 2))
        .collect();
    let depth: Vec<_> = depths
        .iter()
        .map(|z| (z.clamp(0., 1.) * 65535.).round() as u16)
        .collect();
    for region in regions {
        let mut rng = region.seed;
        let threshold = (region.depth.clamp(0., 1.) * 65535.).round() as i32;
        let written = apply(
            &mut pixels,
            &depth,
            w as usize,
            Mosaic {
                rect: region.rect,
                clip: [0, 0, w as i32, h as i32],
                zoom: 3,
                format: 2,
            },
            |_| threshold,
            || sims_vm::rng::next(&mut rng),
        )
        .expect("scene censor projection has valid dimensions");
        for i in written {
            let rgb = unpack(pixels[i], 2);
            let p = image.get_pixel_mut(i as u32 % w, i as u32 / w);
            p.0[..3].copy_from_slice(&rgb);
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Mosaic {
    /// x, y, width, height, before clipping.
    pub rect: [i32; 4],
    /// left, top, right, bottom (exclusive).
    pub clip: [i32; 4],
    pub zoom: i32,
    pub format: i32,
}

pub fn unpack(pixel: u16, format: i32) -> [u8; 3] {
    let p = u32::from(pixel);
    let rgb = match format {
        1 => (((p & 0x7c00) << 3 | p & 0x3e0) << 3 | p & 0x1f) << 3,
        2 => (((p & 0xf800) << 3 | p & 0x7e0) << 2 | p & 0x1f) << 3,
        3 => (((p & 0xf800) << 4 | p & 0x7c0) << 2 | p & 0x3f) << 2,
        _ => 0xffff00,
    };
    [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]
}

pub fn pack(rgb: [u8; 3], format: i32) -> u16 {
    let p = u32::from(rgb[0]) << 16 | u32::from(rgb[1]) << 8 | u32::from(rgb[2]);
    (match format {
        1 => ((p >> 3 & 0x1f0000 | p & 0xf800) >> 3 | p & 0xf8) >> 3,
        2 => ((p >> 3 & 0x1f0000 | p & 0xfc00) >> 2 | p & 0xf8) >> 3,
        3 => ((p >> 3 & 0x1f0000 | p & 0xf800) >> 3 | p & 0xfc) >> 2,
        _ => 0,
    }) as u16
}

/// Mutates only pixels passing the native strict depth comparison, returning
/// their indices so an RGBA adapter need not quantize untouched pixels. Random
/// draws occur R/G/B, three per nonempty block. This does not advance a VM.
pub fn apply(
    pixels: &mut [u16],
    depth: &[u16],
    width: usize,
    options: Mosaic,
    mut threshold: impl FnMut(usize) -> i32,
    mut random: impl FnMut() -> u32,
) -> Result<Vec<usize>> {
    ensure!(
        width > 0 && pixels.len().is_multiple_of(width) && pixels.len() == depth.len(),
        "invalid censor surfaces"
    );
    let height = pixels.len() / width;
    let [l, t, r, b] = options.clip;
    ensure!(
        l >= 0 && t >= 0 && l <= r && t <= b && r as usize <= width && b as usize <= height,
        "invalid censor clip"
    );
    let [x, y, w, h] = options.rect;
    let right = x
        .checked_add(w)
        .ok_or_else(|| anyhow::anyhow!("censor rectangle overflow"))?
        .min(r);
    let bottom = y
        .checked_add(h)
        .ok_or_else(|| anyhow::anyhow!("censor rectangle overflow"))?
        .min(b);
    let (left, top) = (x.max(l), y.max(t));
    if left >= right || top >= bottom {
        return Ok(vec![]);
    }
    let (left, top, right, bottom) = (left as usize, top as usize, right as usize, bottom as usize);
    let cell = match options.zoom {
        1 => 2,
        3 => 8,
        _ => 4,
    };
    let mut written = vec![];
    for by in (top..bottom).step_by(cell) {
        for bx in (left..right).step_by(cell) {
            let (bw, bh) = (cell.min(right - bx), cell.min(bottom - by));
            let mut sum = [0i32; 3];
            let mut count = 0;
            for dy in 0..bh {
                // 4ac4c8 and 4ac649 use clipped top + row-in-block, even for
                // subsequent vertical blocks. Preserve that observed behavior.
                let z = threshold(top + dy);
                for dx in 0..bw {
                    let i = (by + dy) * width + bx + dx;
                    if z < i32::from(depth[i]) {
                        let color = unpack(pixels[i], options.format);
                        for c in 0..3 {
                            sum[c] += i32::from(color[c]);
                        }
                        count += 1;
                    }
                }
            }
            if count == 0 {
                continue;
            }
            let color = sum.map(|s| (s / count + (random() & 15) as i32 - 8).clamp(0, 255) as u8);
            let color = pack(color, options.format);
            for dy in 0..bh {
                let z = threshold(top + dy);
                for dx in 0..bw {
                    let i = (by + dy) * width + bx + dx;
                    if z < i32::from(depth[i]) {
                        pixels[i] = color;
                        written.push(i);
                    }
                }
            }
        }
    }
    Ok(written)
}
