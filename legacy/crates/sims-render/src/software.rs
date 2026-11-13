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
    let (w, h) = (scene.width as usize, scene.height as usize);
    let mut image =
        image::RgbaImage::from_pixel(scene.width, scene.height, image::Rgba(background));
    let mut depths = vec![1f32; w * h];
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
            // Screen Y grows downward. Normalize edge direction for either
            // winding, then give a shared sample to only its top/left owner.
            // Equal-depth independent draws still blend normally below.
            let inclusive_edges = [(p[1], p[2]), (p[2], p[0]), (p[0], p[1])]
                .map(|(a, b)| {
                    let edge = if area > 0. { b - a } else { a - b };
                    edge.y < 0. || (edge.y == 0. && edge.x > 0.)
                });
            let lo = p[0].min(p[1]).min(p[2]).floor().max(Vec2::ZERO);
            let hi = p[0]
                .max(p[1])
                .max(p[2])
                .ceil()
                .min(Vec2::new(w as f32 - 1., h as f32 - 1.));
            for y in lo.y as usize..=hi.y as usize {
                for x in lo.x as usize..=hi.x as usize {
                    let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let b = [
                        (p[1] - q).perp_dot(p[2] - q) / area,
                        (p[2] - q).perp_dot(p[0] - q) / area,
                        (p[0] - q).perp_dot(p[1] - q) / area,
                    ];
                    if b.iter().zip(inclusive_edges).any(|(weight, inclusive)| {
                        *weight < 0. || (*weight == 0. && !inclusive)
                    }) {
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
                    let dst = y * w + x;
                    if depth > depths[dst] {
                        continue;
                    }
                    let pixel = image.get_pixel_mut(x as u32, y as u32);
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
    crate::censor::composite(&mut image, &depths, &scene.censors);
    image
}

#[cfg(test)]
mod tests {
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

#[cfg(test)]
#[path = "software/raster_context_tests.rs"]
mod raster_context_tests;
