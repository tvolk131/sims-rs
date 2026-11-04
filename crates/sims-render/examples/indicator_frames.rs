//! Headless source-mesh inspection. Neutral white material and diagnostic light;
//! this does not claim the original marker's dynamic color, spin or placement.
use anyhow::{Context, Result, ensure};
use glam::Vec3;
use serde_json::Value;
use sims_content::{
    Index,
    character_geometry::CharacterGeometry,
    saved_avatar::{BindingState, BoundAvatar},
};
use sims_render::{Camera, Draw, Scene, Texture, Vertex};
use sims_sim::pose::model::Matrix;

fn frame(
    avatar: &BoundAvatar,
    geometry: &CharacterGeometry,
    matrix: Matrix,
    rotation: u8,
) -> Result<image::RgbaImage> {
    // Portrait selection filters for head/body attachment names. These are
    // ordinary ROOT-bound graphics, so inspect every actual appearance binding.
    let mut pairs = vec![];
    for (identity, &appearance) in &geometry.appearances {
        let range = avatar
            .part_ranges
            .get(identity)
            .context("marker part range absent")?;
        let bindings = &geometry.state.appearances[appearance];
        ensure!(range.len() == bindings.len());
        pairs.extend(bindings.iter().copied().zip(range.clone()));
    }
    let points = pairs
        .iter()
        .flat_map(|(b, _)| geometry.vertices[b].vertices.iter())
        .map(|v| {
            matrix
                .point(v.position)
                .map(|p| Vec3::new(-p[0], p[1], p[2]))
        })
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(anyhow::Error::msg)?;
    ensure!(!points.is_empty());
    let lo = points
        .iter()
        .copied()
        .fold(Vec3::splat(f32::INFINITY), Vec3::min);
    let hi = points
        .iter()
        .copied()
        .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
    let mut camera = Camera {
        level: 1,
        width: 320,
        height: 260,
        rotation,
        zoom: 1.,
        center: (lo + hi) * 0.5,
    };
    let low = points
        .iter()
        .map(|&v| camera.screen(v))
        .fold(glam::Vec2::splat(f32::INFINITY), glam::Vec2::min);
    let high = points
        .iter()
        .map(|&v| camera.screen(v))
        .fold(glam::Vec2::splat(f32::NEG_INFINITY), glam::Vec2::max);
    camera.zoom = (250. / (high.x - low.x)).min(190. / (high.y - low.y));
    let light = Vec3::new(-0.4, 0.8, -0.5).normalize();
    let mut scene = Scene {
        vertices: vec![],
        draws: vec![],
        censors: vec![],
        textures: std::sync::Arc::new(vec![]),
        width: 320,
        height: 260,
    };
    let mut textures = vec![];
    for (binding, part) in pairs {
        let part = &avatar.avatar.parts[part];
        let vertices = &geometry.vertices[&binding].vertices;
        let start = scene.vertices.len() as u32;
        for face in &part.mesh.triangles {
            for &index in face {
                let v = vertices
                    .get(index as usize)
                    .context("mesh face outside vertex buffer")?;
                let p = matrix.point(v.position).map_err(anyhow::Error::msg)?;
                let point = Vec3::new(-p[0], p[1], p[2]);
                let normal = camera
                    .rotated(Vec3::new(-v.normal[0], v.normal[1], v.normal[2]))
                    .normalize_or_zero();
                let shade = 0.3 + 0.7 * normal.dot(light).max(0.);
                let p = camera.screen(point);
                // Center-relative diagnostic depth keeps these isolated assets
                // inside the rasterizer's range, regardless of source units.
                let q = camera.rotated(point - camera.center);
                scene.vertices.push(Vertex {
                    position: [
                        p.x / 160. - 1.,
                        1. - p.y / 130.,
                        0.5 - (q.x + q.z + q.y) * 0.001,
                    ],
                    uv: v.uv,
                    tint: [shade, shade, shade, 1.],
                    depth_scale: 0.,
                });
            }
        }
        scene.draws.push(Draw {
            vertices: start..scene.vertices.len() as u32,
            texture: textures.len(),
            object: None,
        });
        textures.push(Texture {
            width: part.texture.width,
            height: part.texture.height,
            rgba: part.texture.rgba.clone().into(),
            depth: vec![255; part.texture.width as usize * part.texture.height as usize].into(),
        });
    }
    scene.textures = std::sync::Arc::new(textures);
    Ok(sims_render::software::render(&scene))
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    ensure!(
        args.is_empty() || args.len() == 2,
        "usage: indicator_frames [report.json gallery.png]"
    );
    let input = args
        .first()
        .map(String::as_str)
        .unwrap_or("cache/original-indicator-bindings.json");
    let output = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("cache/original-indicator-mesh-gallery.png");
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(input)?)?;
    ensure!(rows.len() == 3);
    let mut gallery = image::RgbaImage::new(960, 1040);
    for (column, row) in rows.iter().enumerate() {
        let owner = row
            .get("owner")
            .unwrap_or(&row["state"]["kind"])
            .as_u64()
            .context("marker owner absent")?;
        let skeleton = row["skeleton"].as_str().context("skeleton absent")?;
        let state: BindingState = serde_json::from_value(row["asset_state"].clone())?;
        let geometry: CharacterGeometry = serde_json::from_value(row["geometry"].clone())?;
        let matrix: Matrix = serde_json::from_value(row["bound"]["matrix"].clone())?;
        let avatar = BoundAvatar::restore(&index, skeleton, owner, &state)?;
        let restored_state = serde_json::from_slice(&serde_json::to_vec(&state)?)?;
        let restored = BoundAvatar::restore(&index, skeleton, owner, &restored_state)?;
        let restored_geometry = serde_json::from_slice(&serde_json::to_vec(&geometry)?)?;
        for rotation in 0..4 {
            let image = frame(&avatar, &geometry, matrix, rotation)?;
            ensure!(
                image == frame(&restored, &restored_geometry, matrix, rotation)?,
                "restored marker pixels differ"
            );
            ensure!(
                image.pixels().any(|p| p.0 != [24, 32, 37, 255]),
                "empty marker frame"
            );
            image::imageops::replace(
                &mut gallery,
                &image,
                column as i64 * 320,
                i64::from(rotation) * 260,
            );
        }
        ensure!(avatar.snapshot() == state && restored.snapshot() == state);
    }
    gallery.save(output)?;
    println!("12 neutral marker views; restored pixels identical");
    Ok(())
}
