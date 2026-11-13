//! Offline road/floor sampling evidence. No original guest or simulation ticks.
use anyhow::{Result, ensure};
use serde_json::json;
use sims_content::Index;
use sims_render::{original_house::OriginalHouse, software};
use sims_runtime::original_session::Session;
use std::path::Path;
#[path = "support/offscreen.rs"]
#[allow(dead_code)]
mod offscreen;

#[test]
#[ignore = "requires private original floor resources and managed snapshot"]
fn original_road_source_and_native_zoom_sampling() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("road sampling diagnostic panicked")
}

fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = root.join(std::env::var("SIMS_ROAD_SAMPLING_OUTPUT")?);
    ensure!(output.starts_with(root.join("cache")) && !output.exists());
    std::fs::create_dir_all(&output)?;
    let index =
        Index::scan(&root.join("cache/managed-independent-20261022-third/immutable-assets"))?;
    let saved = serde_json::from_slice(&std::fs::read(
        root.join("cache/managed-independent-20261022-third/newbie.json"),
    )?)?;
    let session = Session::restore(&index, saved)?;
    let before = serde_json::to_vec(&session.save())?;
    let cold = Session::restore(&index, session.save_managed()?)?;
    ensure!(
        before == serde_json::to_vec(&cold.save())?,
        "cold state differs"
    );
    let floors = sims_content::surfaces::load_floors(&index, &session.house)?;
    let lot = session
        .simulation
        .architecture
        .as_ref()
        .unwrap()
        .original
        .as_ref()
        .unwrap();
    let asphalt_cells: Vec<_> = lot.floors[0]
        .source
        .coverings
        .iter()
        .enumerate()
        .filter(|(_, id)| **id == 9)
        .map(|(i, _)| i)
        .collect();
    let center_cell = asphalt_cells[asphalt_cells.len() / 2];
    let road_center = OriginalHouse::ground_point(
        &session,
        [
            (center_cell % lot.size) as i32 * 16 + 8,
            (center_cell / lot.size) as i32 * 16 + 8,
        ],
        1,
    )?;
    let mut materials = vec![];
    for (id, material) in &floors {
        for (zoom, frame) in material.frames.iter().enumerate() {
            let key = format!("source-floor{id}-zoom{}", zoom + 1);
            image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba.clone())
                .expect("source dimensions")
                .save(output.join(format!("{key}.png")))?;
            materials.push(json!({"saved_id":id,"source":material.source,"local":material.local,
                "zoom":zoom+1,"width":frame.width,"height":frame.height,
                "opaque_pixels":frame.rgba.chunks_exact(4).filter(|p|p[3]>0).count(),
                "bright_opaque_pixels":frame.rgba.chunks_exact(4).filter(|p|p[3]>0 && p[0]>128 && p[1]>128 && p[2]>128).count()}));
        }
    }
    let mut renderer = OriginalHouse::default();
    let mut cold_renderer = OriginalHouse::default();
    let mut views = vec![];
    for (width, height) in [(640, 480), (800, 500), (1024, 768)] {
        for zoom in 1..=3 {
            let camera =
                OriginalHouse::camera(&session, width, height, 0, (1u32 << zoom) as f32 / 8., 1);
            let scene = renderer.scene(&session, camera)?;
            let key = format!("newbie-{width}x{height}-zoom{zoom}");
            let frame = software::render(&scene);
            frame.save(output.join(format!("{key}.png")))?;
            ensure!(
                frame == software::render(&scene),
                "{key}: repeat pixels changed"
            );
            let cold_frame = software::render(&cold_renderer.scene(&cold, camera)?);
            if frame != cold_frame {
                cold_frame.save(output.join(format!("{key}-cold-failure.png")))?;
                anyhow::bail!("{key}: cold pixels differ");
            }
            for material in floors.values() {
                let source = &material.frames[(zoom - 1) as usize];
                ensure!(
                    scene.textures.iter().any(|t| matches_authored(source, t)),
                    "authored floor{} zoom{zoom} pixels changed",
                    material.saved_id
                );
            }
            let asphalt = &floors[&9].frames[(zoom - 1) as usize];
            let ids: Vec<_> = scene
                .textures
                .iter()
                .enumerate()
                .filter(|(_, t)| matches_authored(asphalt, t))
                .map(|(i, _)| i)
                .collect();
            ensure!(
                ids.len() == 1,
                "asphalt source did not map uniquely to bound texture"
            );
            let mut road = scene.clone();
            road.draws.retain(|d| ids.contains(&d.texture));
            software::render_with_background(&road, [255, 0, 255, 255])
                .save(output.join(format!("{key}-asphalt-only.png")))?;
            views.push(json!({"key":key,"width":width,"height":height,"zoom":zoom,
                "camera_center":camera.center.to_array(),"road_texture_ids":ids,"road_draws":road.draws.len(),"repeat_pixels_equal":true,"cold_pixels_equal":true,"authored_nontransparent_texels_unchanged":true}));
            if std::env::var_os("SIMS_ROAD_SAMPLING_GPU").is_some() {
                // Keep actual asphalt visible at every native zoom; a zero
                // hole count on an offscreen road would prove nothing.
                let mut gpu_camera = camera;
                gpu_camera.center = road_center;
                let mut road = renderer.scene(&session, gpu_camera)?;
                road.draws.retain(|d| ids.contains(&d.texture));
                let mut texture = road.textures[ids[0]].clone();
                for pixel in std::sync::Arc::make_mut(&mut texture.rgba).chunks_exact_mut(4) {
                    pixel[0..3].fill(255);
                }
                road.textures = std::sync::Arc::new(vec![texture.clone()]);
                for draw in &mut road.draws {
                    draw.texture = 0;
                }
                for scale in [1., 0.6060606] {
                    let target = [
                        (width as f32 * scale).ceil() as u32,
                        (height as f32 * scale).ceil() as u32,
                    ];
                    let viewport = [0., 0., width as f32 * scale, height as f32 * scale];
                    road.textures = std::sync::Arc::new(vec![texture.clone()]);
                    let actual = offscreen::render_at(&road, target, viewport)?;
                    let mut opaque = texture.clone();
                    for pixel in std::sync::Arc::make_mut(&mut opaque.rgba).chunks_exact_mut(4) {
                        pixel[3] = 255;
                    }
                    road.textures = std::sync::Arc::new(vec![opaque]);
                    let expected = offscreen::render_at(&road, target, viewport)?;
                    let holes = expected
                        .pixels()
                        .zip(actual.pixels())
                        .filter(|(e, a)| e[0] > 0 && a[0] == 0)
                        .count();
                    let tested = expected.pixels().filter(|p| p[0] > 0).count();
                    actual.save(output.join(format!("{key}-gpu-alpha-scale{scale}.png")))?;
                    expected.save(output.join(format!("{key}-gpu-solid-scale{scale}.png")))?;
                    ensure!(tested > 0, "{key}: no visible road samples at scale{scale}");
                    views.push(json!({"key":key,"gpu_scale":scale,"target":target,"camera_center":road_center.to_array(),"tested_floor_pixels":tested,"floor_alpha_holes":holes}));
                }
            }
        }
    }
    ensure!(
        before == serde_json::to_vec(&session.save())?,
        "road inspection changed complete retained simulation state"
    );
    ensure!(
        before == serde_json::to_vec(&cold.save())?,
        "drawing changed cold retained simulation state"
    );
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"materials":materials,"views":views,"full_retained_state_unchanged":true,"full_cold_state_equal":true,"simulation_ticks_added":0,"original_pixel_parity_claimed":false}),
        )?,
    )?;
    Ok(())
}

fn matches_authored(source: &sims_formats::sprite::Frame, t: &sims_render::Texture) -> bool {
    t.width == source.width
        && t.height == source.height
        && t.rgba
            .chunks_exact(4)
            .zip(source.rgba.chunks_exact(4))
            .all(|(bound, source)| source[3] == 0 || bound == source)
}
