//! Deterministic reference rasterizer for depth/pose fixtures and PNG diagnostics.
use crate::Scene;
use glam::Vec2;

/// Uploadable complete software frame for the GPU client's censor path.
pub fn composited_scene(scene: &Scene) -> Scene {
    let rgba = render(scene).into_raw();
    rgba_scene(scene.width, scene.height, rgba).expect("rendered frame dimensions")
}

/// Stable presentation surface while a native graphics callback is suspended.
/// Picking is disabled on this image; the command owner also gates input.
pub fn rgba_scene(width: u32, height: u32, rgba: Vec<u8>) -> anyhow::Result<Scene> {
    anyhow::ensure!(
        width > 0
            && height > 0
            && (width as usize)
                .checked_mul(height as usize)
                .and_then(|n| n.checked_mul(4))
                == Some(rgba.len()),
        "invalid retained frame image"
    );
    let v = [
        ([-1., 1., 0.5], [0., 0.]),
        ([1., 1., 0.5], [1., 0.]),
        ([1., -1., 0.5], [1., 1.]),
        ([-1., -1., 0.5], [0., 1.]),
    ];
    Ok(Scene {
        width,
        height,
        censors: vec![],
        textures: std::sync::Arc::new(vec![crate::Texture {
            width,
            height,
            rgba: rgba.into(),
            depth: vec![255; width as usize * height as usize].into(),
        }]),
        vertices: [0, 1, 2, 0, 2, 3]
            .map(|i| crate::Vertex {
                position: v[i].0,
                uv: v[i].1,
                tint: [1.; 4],
                depth_scale: 0.,
            })
            .to_vec(),
        draws: vec![crate::Draw {
            vertices: 0..6,
            texture: 0,
            object: None,
        }],
    })
}

pub fn render(scene: &Scene) -> image::RgbaImage {
    render_with_background(scene, [24, 32, 37, 255])
}

pub fn render_with_background(scene: &Scene, background: [u8; 4]) -> image::RgbaImage {
    static WORKERS: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let workers = if u64::from(scene.width) * u64::from(scene.height) >= 262_144 {
        *WORKERS.get_or_init(|| {
            std::thread::available_parallelism().map_or(1, |count| count.get().min(4))
        })
    } else {
        1
    };
    render_with_workers(scene, background, workers)
}

fn render_with_workers(scene: &Scene, background: [u8; 4], workers: usize) -> image::RgbaImage {
    let (w, h) = (scene.width as usize, scene.height as usize);
    let mut image =
        image::RgbaImage::from_pixel(scene.width, scene.height, image::Rgba(background));
    let mut depths = vec![1f32; w * h];
    let workers = workers.max(1).min(h.max(1));
    if workers == 1 || w == 0 {
        rasterize_rows(scene, image.as_mut(), &mut depths, 0);
    } else {
        let rows = h.div_ceil(workers);
        std::thread::scope(|scope| {
            for (stripe, (pixels, depths)) in image
                .as_mut()
                .chunks_mut(w * rows * 4)
                .zip(depths.chunks_mut(w * rows))
                .enumerate()
            {
                scope.spawn(move || rasterize_rows(scene, pixels, depths, stripe * rows));
            }
        });
    }
    crate::censor::composite(&mut image, &depths, &scene.censors);
    image
}

fn rasterize_rows(scene: &Scene, pixels: &mut [u8], depths: &mut [f32], first_row: usize) {
    if depths.is_empty() {
        return;
    }
    let (w, h) = (scene.width as usize, scene.height as usize);
    let last_row = first_row + depths.len() / w - 1;
    for draw in &scene.draws {
        let texture = &scene.textures[draw.texture];
        for triangle in scene.vertices[draw.vertices.start as usize..draw.vertices.end as usize]
            .as_chunks::<3>()
            .0
        {
            let p: Vec<_> = triangle
                .iter()
                .map(|v| {
                    Vec2::new(
                        (v.position[0] + 1.) * w as f32 / 2.,
                        (1. - v.position[1]) * h as f32 / 2.,
                    )
                })
                .collect();
            let area = (p[1] - p[0]).perp_dot(p[2] - p[0]);
            if area.abs() < 1e-6 {
                continue;
            }
            let lo = p[0].min(p[1]).min(p[2]).floor().max(Vec2::ZERO);
            let hi = p[0]
                .max(p[1])
                .max(p[2])
                .ceil()
                .min(Vec2::new(w as f32 - 1., h as f32 - 1.));
            for y in (lo.y as usize).max(first_row)..=(hi.y as usize).min(last_row) {
                for x in lo.x as usize..=hi.x as usize {
                    let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let b = [
                        (p[1] - q).perp_dot(p[2] - q) / area,
                        (p[2] - q).perp_dot(p[0] - q) / area,
                        (p[0] - q).perp_dot(p[1] - q) / area,
                    ];
                    if b.iter().any(|v| *v < 0.) {
                        continue;
                    }
                    let mut uv = [0.; 2];
                    let mut tint = [0.; 4];
                    for i in 0..3 {
                        for (k, v) in uv.iter_mut().enumerate() {
                            *v += triangle[i].uv[k] * b[i];
                        }
                        for (k, v) in tint.iter_mut().enumerate() {
                            *v += triangle[i].tint[k] * b[i];
                        }
                    }
                    // Interpolate from an anchor so a constant-depth plane
                    // stays exact. Summing z*b rounds some far-plane pixels
                    // above 1 and exposes the clear color behind the backdrop.
                    let mut depth = triangle[0].position[2]
                        + (triangle[1].position[2] - triangle[0].position[2]) * b[1]
                        + (triangle[2].position[2] - triangle[0].position[2]) * b[2];
                    let tx = (uv[0].clamp(0., 1.) * texture.width as f32)
                        .floor()
                        .min(texture.width as f32 - 1.) as usize;
                    let ty = (uv[1].clamp(0., 1.) * texture.height as f32)
                        .floor()
                        .min(texture.height as f32 - 1.) as usize;
                    let texel = ty * texture.width as usize + tx;
                    let rgba = &texture.rgba[texel * 4..texel * 4 + 4];
                    let a = rgba[3] as f32 / 255. * tint[3];
                    if a <= 0. {
                        continue;
                    }
                    depth += (1. - texture.depth[texel] as f32 / 255.) * triangle[0].depth_scale;
                    let dst = (y - first_row) * w + x;
                    if depth > depths[dst] {
                        continue;
                    }
                    let pixel = &mut pixels[dst * 4..dst * 4 + 4];
                    for c in 0..3 {
                        pixel[c] = (rgba[c] as f32 * tint[c] * a + pixel[c] as f32 * (1. - a))
                            .round()
                            .clamp(0., 255.) as u8;
                    }
                    depths[dst] = depth;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    fn layered_scene(width: u32, height: u32) -> crate::Scene {
        let pixels = (0..width as usize * height as usize)
            .flat_map(|index| {
                [
                    (index * 19) as u8,
                    (index * 7) as u8,
                    (index * 37) as u8,
                    (index * 13) as u8,
                ]
            })
            .collect();
        let mut scene = super::rgba_scene(width, height, pixels).unwrap();
        let texture = &mut std::sync::Arc::make_mut(&mut scene.textures)[0];
        texture.depth = (0..width as usize * height as usize)
            .map(|index| (index * 31) as u8)
            .collect();
        for layer in 0..5 {
            let start = scene.vertices.len() as u32;
            for (position, uv) in [
                ([-1.4, 0.9, 0.4], [-0.1, 0.1]),
                ([0.9, 1.3, 0.4], [1.1, -0.3]),
                ([0.7, -1.2, 0.4], [0.6, 1.2]),
            ] {
                scene.vertices.push(crate::Vertex {
                    position: [
                        position[0] - layer as f32 * 0.1,
                        position[1],
                        position[2] - layer as f32 * 0.01,
                    ],
                    uv,
                    tint: [0.7, 0.9, 0.4, 0.8],
                    depth_scale: 0.08,
                });
            }
            scene.draws.push(crate::Draw {
                vertices: start..start + 3,
                texture: 0,
                object: None,
            });
        }
        scene.censors = vec![
            crate::censor::Region {
                rect: [-3, 4, 23, 19],
                depth: 0.1,
                seed: 7,
            },
            crate::censor::Region {
                rect: [7, 11, 33, 27],
                depth: 0.2,
                seed: 19,
            },
        ];
        scene
    }

    #[test]
    fn row_partition_preserves_complete_pixels_for_depth_blend_clip_and_censors() {
        for (width, height) in [(1, 1), (7, 9), (64, 47), (131, 83)] {
            let scene = layered_scene(width, height);
            for background in [[24, 32, 37, 255], [101, 31, 209, 73]] {
                let expected = super::render_with_workers(&scene, background, 1);
                for workers in [2, 3, 4, 7] {
                    assert_eq!(
                        super::render_with_workers(&scene, background, workers),
                        expected,
                        "{width}x{height}, workers {workers}"
                    );
                }
            }
        }
    }

    #[test]
    #[ignore]
    fn row_partition_reports_full_scene_timing_without_native_qualification() {
        let scene = layered_scene(1320, 800);
        let background = [24, 32, 37, 255];
        let expected = super::render_with_workers(&scene, background, 1);
        let mut serial_us = Vec::new();
        let mut parallel_us = Vec::new();
        for _ in 0..16 {
            let start = std::time::Instant::now();
            let actual = super::render_with_workers(&scene, background, 1);
            serial_us.push(start.elapsed().as_micros());
            assert_eq!(actual, expected);
            let start = std::time::Instant::now();
            let actual = super::render_with_workers(&scene, background, 4);
            parallel_us.push(start.elapsed().as_micros());
            assert_eq!(actual, expected);
        }
        serial_us.sort_unstable();
        parallel_us.sort_unstable();
        println!(
            "{}",
            serde_json::json!({
                "scope":"Complete synthetic software scene including overlapping censors; not native controls",
                "samples":16,"width":1320,"height":800,"workers":4,
                "serial_median_us":serial_us[8],"parallel_median_us":parallel_us[8],
                "serial_worst_us":serial_us[15],"parallel_worst_us":parallel_us[15],
                "complete_pixels_equal":true,"native_qualification":false,
            })
        );
    }

    #[test]
    fn far_plane_background_covers_every_pixel() {
        for (width, height) in [(1184, 803), (1320, 750), (37, 29)] {
            let color = [112, 112, 112, 255];
            let mut scene =
                super::rgba_scene(width, height, color.repeat((width * height) as usize)).unwrap();
            for vertex in &mut scene.vertices {
                vertex.position[2] = 1.;
            }
            let frame = super::render(&scene);
            let missing = frame.pixels().filter(|pixel| pixel.0 != color).count();
            assert_eq!(
                missing, 0,
                "far plane leaks clear pixels at {width}x{height}"
            );
        }
    }
}
