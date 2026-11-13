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
    let (width, height) = image.dimensions();
    assert!(width > 0 && depths.len() == image.as_raw().len() / 4);
    let clip_right = i32::try_from(width).expect("scene censor width fits clip coordinates");
    let clip_bottom = i32::try_from(height).expect("scene censor height fits clip coordinates");
    for region in regions {
        let [left, top, region_width, region_height] = region.rect;
        let right = left
            .checked_add(region_width)
            .expect("scene censor rectangle does not overflow")
            .min(clip_right);
        let bottom = top
            .checked_add(region_height)
            .expect("scene censor rectangle does not overflow")
            .min(clip_bottom);
        let (left, top) = (left.max(0), top.max(0));
        if left >= right || top >= bottom {
            continue;
        }
        let (crop_width, crop_height) = ((right - left) as usize, (bottom - top) as usize);
        let mut pixels = Vec::with_capacity(crop_width * crop_height);
        let mut depth = Vec::with_capacity(crop_width * crop_height);
        for row in top as u32..bottom as u32 {
            for column in left as u32..right as u32 {
                let pixel = image.get_pixel(column, row);
                pixels.push(pack([pixel[0], pixel[1], pixel[2]], 2));
                let value = depths[row as usize * width as usize + column as usize];
                depth.push((value.clamp(0., 1.) * 65535.).round() as u16);
            }
        }
        let mut rng = region.seed;
        let threshold = (region.depth.clamp(0., 1.) * 65535.).round() as i32;
        let written = apply(
            &mut pixels,
            &depth,
            crop_width,
            Mosaic {
                rect: [0, 0, crop_width as i32, crop_height as i32],
                clip: [0, 0, crop_width as i32, crop_height as i32],
                zoom: 3,
                format: 2,
            },
            |_| threshold,
            || sims_vm::rng::next(&mut rng),
        )
        .expect("scene censor projection has valid dimensions");
        for index in written {
            let rgb = unpack(pixels[index], 2);
            let pixel = image.get_pixel_mut(
                left as u32 + (index % crop_width) as u32,
                top as u32 + (index / crop_width) as u32,
            );
            pixel.0[..3].copy_from_slice(&rgb);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn full_surface_reference(image: &mut image::RgbaImage, depths: &[f32], regions: &[Region]) {
        if regions.is_empty() {
            return;
        }
        let (width, height) = image.dimensions();
        let mut pixels: Vec<_> = image
            .pixels()
            .map(|pixel| pack([pixel[0], pixel[1], pixel[2]], 2))
            .collect();
        let depth: Vec<_> = depths
            .iter()
            .map(|value| (value.clamp(0., 1.) * 65535.).round() as u16)
            .collect();
        for region in regions {
            let mut rng = region.seed;
            let threshold = (region.depth.clamp(0., 1.) * 65535.).round() as i32;
            let written = apply(
                &mut pixels,
                &depth,
                width as usize,
                Mosaic {
                    rect: region.rect,
                    clip: [0, 0, width as i32, height as i32],
                    zoom: 3,
                    format: 2,
                },
                |_| threshold,
                || sims_vm::rng::next(&mut rng),
            )
            .unwrap();
            for index in written {
                let rgb = unpack(pixels[index], 2);
                image
                    .get_pixel_mut(index as u32 % width, index as u32 / width)
                    .0[..3]
                    .copy_from_slice(&rgb);
            }
        }
    }

    fn image(width: u32, height: u32) -> image::RgbaImage {
        image::RgbaImage::from_fn(width, height, |column, row| {
            image::Rgba([
                (column * 19 + row * 11) as u8,
                (column * 7 + row * 31) as u8,
                (column * 37 + row * 3) as u8,
                (column * 13 + row * 23) as u8,
            ])
        })
    }

    #[test]
    fn rgb565_written_pixels_repack_without_loss_between_overlapping_regions() {
        for pixel in u16::MIN..=u16::MAX {
            assert_eq!(pack(unpack(pixel, 2), 2), pixel);
        }
    }

    #[test]
    fn cropped_adapter_matches_full_surface_clipping_depth_alpha_and_overlap() {
        let values = [
            f32::NEG_INFINITY,
            -1.,
            0.,
            0.25,
            0.5,
            1.,
            2.,
            f32::INFINITY,
            f32::NAN,
        ];
        for (width, height) in [(1, 1), (9, 13), (31, 19), (64, 48)] {
            let source = image(width, height);
            let depths: Vec<_> = (0..width as usize * height as usize)
                .map(|index| values[(index * 7) % values.len()])
                .collect();
            for seed in [0, 1, 91, u32::MAX] {
                let mut regions: Vec<_> = [
                    [-5, -3, 14, 18],
                    [3, 5, 27, 17],
                    [5, 7, 11, 15],
                    [0, 0, 1, height as i32],
                    [width as i32 - 3, height as i32 - 4, 20, 20],
                    [width as i32, 0, 10, 10],
                    [-20, -20, 2, 3],
                    [0, 0, 0, 5],
                    [2, 2, -1, -1],
                ]
                .into_iter()
                .enumerate()
                .map(|(index, rect)| Region {
                    rect,
                    depth: values[index % values.len()],
                    seed: seed.wrapping_add(index as u32),
                })
                .collect();
                for _ in 0..2 {
                    let mut expected = source.clone();
                    let mut actual = source.clone();
                    full_surface_reference(&mut expected, &depths, &regions);
                    composite(&mut actual, &depths, &regions);
                    assert_eq!(actual, expected, "{width}x{height}, seed {seed}");
                    for (before, after) in source.pixels().zip(actual.pixels()) {
                        assert_eq!(before[3], after[3]);
                    }
                    regions.reverse();
                }
            }
        }
    }

    #[test]
    fn cropped_adapter_leaves_empty_offscreen_and_occluded_pixels_unquantized() {
        let source = image(19, 23);
        let depths = vec![0.; 19 * 23];
        for regions in [
            vec![],
            vec![Region {
                rect: [30, 40, 7, 9],
                depth: 0.,
                seed: 1,
            }],
            vec![Region {
                rect: [-3, -5, 30, 40],
                depth: 1.,
                seed: 7,
            }],
        ] {
            let mut actual = source.clone();
            composite(&mut actual, &depths, &regions);
            assert_eq!(actual, source);
        }
    }

    #[test]
    #[should_panic]
    fn cropped_adapter_rejects_mismatched_depth_even_for_offscreen_regions() {
        composite(
            &mut image(4, 5),
            &[],
            &[Region {
                rect: [20, 20, 1, 1],
                depth: 0.,
                seed: 0,
            }],
        );
    }

    #[test]
    #[ignore]
    fn cropped_adapter_reports_small_region_timing_without_native_qualification() {
        let source = image(1320, 800);
        let depths = vec![0.9; 1320 * 800];
        let regions = [
            Region {
                rect: [880, 310, 40, 70],
                depth: 0.4,
                seed: 19,
            },
            Region {
                rect: [905, 355, 45, 80],
                depth: 0.5,
                seed: 91,
            },
        ];
        let mut expected = source.clone();
        full_surface_reference(&mut expected, &depths, &regions);
        let mut old_us = Vec::new();
        let mut new_us = Vec::new();
        for _ in 0..32 {
            let mut image = source.clone();
            let start = std::time::Instant::now();
            full_surface_reference(&mut image, &depths, &regions);
            old_us.push(start.elapsed().as_micros());
            assert_eq!(image, expected);
            let mut image = source.clone();
            let start = std::time::Instant::now();
            composite(&mut image, &depths, &regions);
            new_us.push(start.elapsed().as_micros());
            assert_eq!(image, expected);
        }
        old_us.sort_unstable();
        new_us.sort_unstable();
        println!(
            "{}",
            serde_json::json!({
                "scope":"RGBA censor adapter only; excludes world rasterization and native controls",
                "samples":32,"width":1320,"height":800,"regions":2,
                "old_median_us":old_us[16],"new_median_us":new_us[16],
                "old_worst_us":old_us[31],"new_worst_us":new_us[31],
                "complete_pixels_equal":true,"native_qualification":false,
            })
        );
    }
}
