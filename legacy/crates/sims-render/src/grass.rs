//! Read-only ground presentation from the saved signed grass map (Arry 6).
//! Native palette endpoints/32 steps:49c0e3,49c8d3; RGB interpolation:49cc39.
//! The GPU texture's grain placement is a replacement, not the original blade
//! rasterizer. Its private RNG never borrows a simulation random stream.
use crate::Texture;
use anyhow::{Context, Result, ensure};
use sims_formats::lot::Array;

const TEXELS: usize = 32;
const LIGHT_BROWN: [u8; 3] = [157, 117, 65];
const DARK_BROWN: [u8; 3] = [81, 60, 18];
const LIGHT_GREEN: [u8; 3] = [80, 116, 59];
const DARK_GREEN: [u8; 3] = [14, 53, 8];

///49cc39 promotes the f32 factor to x87, truncates each signed result and
///clamps channels. f64 preserves all intermediate values for byte endpoints.
fn interpolate(a: [u8; 3], b: [u8; 3], factor: f32) -> [u8; 3] {
    std::array::from_fn(|i| {
        (f64::from(a[i]) + (f64::from(b[i]) - f64::from(a[i])) * f64::from(factor))
            .trunc()
            .clamp(0., 255.) as u8
    })
}

#[derive(Debug)]
pub(crate) struct Ground {
    pub texture: usize,
    size: usize,
}

#[derive(Clone, Copy)]
struct Input<'a> {
    map: &'a Array,
    size: usize,
    width: usize,
}

impl<'a> Input<'a> {
    fn new(dimensions: [usize; 2], map: Option<&'a Array>) -> Result<Self> {
        let map = map.context("original lot grass map absent")?;
        ensure!(
            map.width == dimensions[0]
                && map.height == dimensions[1]
                && map.stride == 1
                && map.data.len() == dimensions[0] * dimensions[1]
                && dimensions[0] == dimensions[1],
            "invalid original grass map"
        );
        let size = dimensions[0];
        let width = size
            .checked_mul(TEXELS)
            .filter(|&n| n > 0 && n <= 4096)
            .context("ground presentation texture too large")?;
        Ok(Self { map, size, width })
    }
}

#[derive(Default)]
pub(crate) struct Cache {
    entry: Option<Entry>,
}

struct Entry {
    map: Array,
    size: usize,
    pixels: Texture,
}

impl Cache {
    pub(crate) fn bind(
        &mut self,
        house: &sims_content::saves::House,
        texture: usize,
    ) -> Result<(Ground, Texture)> {
        self.bind_input([house.width, house.height], house.arrays.get(&6), texture)
    }

    fn bind_input(
        &mut self,
        dimensions: [usize; 2],
        map: Option<&Array>,
        texture: usize,
    ) -> Result<(Ground, Texture)> {
        let input = Input::new(dimensions, map)?;
        if let Some(entry) = &self.entry
            && entry.size == input.size
            && entry.map == *input.map
        {
            return Ok((
                Ground {
                    texture,
                    size: input.size,
                },
                entry.pixels.clone(),
            ));
        }
        let (ground, pixels) = Ground::from_input(input, texture)?;
        self.entry = Some(Entry {
            map: input.map.clone(),
            size: input.size,
            pixels: pixels.clone(),
        });
        Ok((ground, pixels))
    }
}

impl Ground {
    pub fn bind(house: &sims_content::saves::House, texture: usize) -> Result<(Self, Texture)> {
        Self::from_input(
            Input::new([house.width, house.height], house.arrays.get(&6))?,
            texture,
        )
    }

    fn from_input(input: Input<'_>, texture: usize) -> Result<(Self, Texture)> {
        let Input { map, size, width } = input;
        let light: Vec<_> = (0..32)
            .map(|i| interpolate(LIGHT_BROWN, LIGHT_GREEN, i as f32 / 31.))
            .collect();
        let dark: Vec<_> = (0..32)
            .map(|i| interpolate(DARK_BROWN, DARK_GREEN, i as f32 / 31.))
            .collect();
        let sample = |x: usize, z: usize| {
            f32::from(map.data[x.min(size - 1) * size + z.min(size - 1)] as i8)
        };
        //49cd43 averages four neighboring signed grass bytes at a corner.
        let corner = |x: usize, z: usize| {
            let average = ((sample(x, z)
                + sample(x.saturating_sub(1), z)
                + sample(x, z.saturating_sub(1))
                + sample(x.saturating_sub(1), z.saturating_sub(1)))
                / 4.)
                .trunc();
            ((average + 128.) * 32. / 256.).trunc().clamp(0., 31.)
        };
        let mut rgba = vec![0; width * width * 4];
        let mut state = 0x3ef40de9; // renderer color seed, constructor49c0e3+84
        for x in 0..size {
            for z in 0..size {
                let c = [
                    corner(x, z),
                    corner(x + 1, z),
                    corner(x, z + 1),
                    corner(x + 1, z + 1),
                ];
                for u in 0..TEXELS {
                    for v in 0..TEXELS {
                        let a = (u as f32 + 0.5) / TEXELS as f32;
                        let b = (v as f32 + 0.5) / TEXELS as f32;
                        let density = (c[0] * (1. - a) + c[1] * a) * (1. - b)
                            + (c[2] * (1. - a) + c[3] * a) * b;
                        let row = (density as usize).min(31);
                        let grain = (sims_vm::rng::next(&mut state) % 128) as f32 / 127.;
                        let rgb = interpolate(dark[row], light[row], grain);
                        let offset = ((z * TEXELS + v) * width + x * TEXELS + u) * 4;
                        // Native display color precision, uploaded as sRGB pixels.
                        rgba[offset..offset + 4].copy_from_slice(&[
                            ((u16::from(rgb[0]) >> 3) * 255 / 31) as u8,
                            ((u16::from(rgb[1]) >> 2) * 255 / 63) as u8,
                            ((u16::from(rgb[2]) >> 3) * 255 / 31) as u8,
                            255,
                        ]);
                    }
                }
            }
        }
        Ok((
            Self { texture, size },
            Texture {
                width: width as u32,
                height: width as u32,
                rgba: rgba.into(),
                depth: vec![255; width * width].into(),
            },
        ))
    }
    pub fn uv(&self, x: i32, z: i32, local: glam::Vec3) -> [f32; 2] {
        [
            (x as f32 + local.x) / self.size as f32,
            (z as f32 + local.z) / self.size as f32,
        ]
    }
}

#[cfg(test)]
#[path = "grass/cache_tests.rs"]
mod cache_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_matches_unchanged_native_rgb_interpolation() {
        // Original49cc39, 32 x f32 palette factors, captured by
        // scripts/research/native_terrain_palette.py. No renderer/VM execution
        // state or random seed is supplied to this arithmetic routine.
        let expected = [
            [157, 117, 65],
            [154, 116, 64],
            [152, 116, 64],
            [149, 116, 64],
            [147, 116, 64],
            [144, 116, 64],
            [142, 116, 63],
            [139, 116, 63],
            [137, 116, 63],
            [134, 116, 63],
            [132, 116, 63],
            [129, 116, 62],
            [127, 116, 62],
            [124, 116, 62],
            [122, 116, 62],
            [119, 116, 62],
            [117, 116, 61],
            [114, 116, 61],
            [112, 116, 61],
            [109, 116, 61],
            [107, 116, 61],
            [104, 116, 60],
            [102, 116, 60],
            [99, 116, 60],
            [97, 116, 60],
            [94, 116, 60],
            [92, 116, 59],
            [89, 116, 59],
            [87, 116, 59],
            [84, 116, 59],
            [82, 116, 59],
            [80, 116, 59],
        ];
        for (i, rgb) in expected.into_iter().enumerate() {
            assert_eq!(interpolate(LIGHT_BROWN, LIGHT_GREEN, i as f32 / 31.), rgb);
        }
        let expected = [
            [81, 60, 18],
            [78, 59, 17],
            [76, 59, 17],
            [74, 59, 17],
            [72, 59, 16],
            [70, 58, 16],
            [68, 58, 16],
            [65, 58, 15],
            [63, 58, 15],
            [61, 57, 15],
            [59, 57, 14],
            [57, 57, 14],
            [55, 57, 14],
            [52, 57, 13],
            [50, 56, 13],
            [48, 56, 13],
            [46, 56, 12],
            [44, 56, 12],
            [42, 55, 12],
            [39, 55, 11],
            [37, 55, 11],
            [35, 55, 11],
            [33, 55, 10],
            [31, 54, 10],
            [29, 54, 10],
            [26, 54, 9],
            [24, 54, 9],
            [22, 53, 9],
            [20, 53, 8],
            [18, 53, 8],
            [16, 53, 8],
            [14, 53, 8],
        ];
        for (i, rgb) in expected.into_iter().enumerate() {
            assert_eq!(interpolate(DARK_BROWN, DARK_GREEN, i as f32 / 31.), rgb);
        }
    }
}
