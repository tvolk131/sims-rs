//! Private original-resource regression; no guest, input, ticks or GPU window.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::json;
use sims_content::Index;
use sims_formats::{
    iff::Iff,
    sprite::{Frame, SpriteSet},
};
use sims_render::{Camera, Scene, Texture, original_house::OriginalHouse, software};
use sims_runtime::original_session::{Saved, Session};
use std::{collections::BTreeMap, path::Path};

#[derive(Deserialize)]
struct Player {
    session: Saved,
}
#[derive(Deserialize)]
struct Native {
    actual: Actual,
}
#[derive(Deserialize)]
struct Actual {
    grid_sha256: String,
    selections: Vec<Selection>,
}
#[derive(Deserialize)]
struct Selection {
    position: [i32; 2],
    kind: u8,
    original: [u8; 4],
}

#[test]
#[ignore = "requires original immutable assets, Mashuga checkpoint and private native selectors"]
fn mashuga_pool_edges_and_road_views_preserve_warm_cold_state() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("pool/road regression panicked"))?
}

fn source(iff: &Iff, id: u16) -> Result<Frame> {
    let chunk = iff
        .get(b"SPR2", id)
        .with_context(|| format!("source SPR2{id}"))?;
    let raw = iff.data(chunk);
    let set = SpriteSet::parse(raw)?;
    ensure!(
        set.frames.len() == 1,
        "source SPR2{id} is not a single frame"
    );
    Ok(set.decode(raw, 0, iff)?)
}

fn authored(frame: &Frame, texture: &Texture) -> bool {
    let surface = matches!((frame.width, frame.height), (31, 16) | (63, 32) | (127, 64));
    if surface && (texture.width, texture.height) != (frame.width, frame.height) {
        return false;
    }
    if !surface {
        for y in 0..texture.height as usize {
            for x in 0..texture.width as usize {
                let actual = &texture.rgba[(y * texture.width as usize + x) * 4
                    ..(y * texture.width as usize + x) * 4 + 4];
                let sx = x as i32 - frame.x;
                let sy = y as i32 - frame.y;
                let expected =
                    if sx >= 0 && sy >= 0 && sx < frame.width as i32 && sy < frame.height as i32 {
                        &frame.rgba[(sy as usize * frame.width as usize + sx as usize) * 4
                            ..(sy as usize * frame.width as usize + sx as usize) * 4 + 4]
                    } else {
                        &[0; 4]
                    };
                if actual != expected {
                    return false;
                }
            }
        }
        return true;
    }
    if frame.x < 0
        || frame.y < 0
        || frame.x as u32 + frame.width > texture.width
        || frame.y as u32 + frame.height > texture.height
    {
        return false;
    }
    for y in 0..frame.height as usize {
        for x in 0..frame.width as usize {
            let i = y * frame.width as usize + x;
            if frame.rgba[i * 4 + 3] == 0 {
                continue;
            }
            let j = (y + frame.y as usize) * texture.width as usize + x + frame.x as usize;
            if frame.rgba[i * 4..i * 4 + 4] != texture.rgba[j * 4..j * 4 + 4] {
                return false;
            }
        }
    }
    true
}

fn triangle(session: &Session, camera: Camera, cell: [i32; 2]) -> Result<[[f32; 3]; 3]> {
    let mut result = [[0.; 3]; 3];
    for (i, [dx, dz]) in [[0, 0], [1, 0], [1, 1]].into_iter().enumerate() {
        let p =
            OriginalHouse::ground_point(session, [(cell[1] + dz) * 16, (cell[0] + dx) * 16], 1)?;
        let screen = camera.screen(p);
        result[i] = [
            screen.x / camera.width as f32 * 2. - 1.,
            1. - screen.y / camera.height as f32 * 2.,
            camera.depth(p),
        ];
    }
    Ok(result)
}

fn matching_draws<'a>(scene: &'a Scene, points: &[[f32; 3]; 3]) -> Vec<(&'a Texture, [f32; 3])> {
    scene
        .draws
        .iter()
        .filter(|d| d.object.is_none() && d.vertices.len() == 3)
        .filter(|d| {
            scene.vertices[d.vertices.start as usize..d.vertices.end as usize]
                .iter()
                .zip(points)
                .all(|(v, p)| {
                    v.position
                        .iter()
                        .zip(p)
                        // The complete scene applies one affine depth remap.
                        // Locate by projected geometry, then compare actual
                        // surface/corner depths in that normalized space.
                        .take(2)
                        .all(|(a, b)| (a - b).abs() < 0.00001)
                })
        })
        .map(|d| {
            (
                &scene.textures[d.texture],
                std::array::from_fn(|i| scene.vertices[d.vertices.start as usize + i].position[2]),
            )
        })
        .collect()
}

fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = root.join(std::env::var("SIMS_POOL_ROAD_OUTPUT")?);
    ensure!(output.starts_with(root.join("cache")) && !output.exists());
    std::fs::create_dir_all(&output)?;
    let checkpoint = root.join("cache/post-buy-mashuga-4096-observed-20261022-first/midpoint.json");
    let bytes = std::fs::read(&checkpoint)?;
    let index =
        Index::scan(&root.join("cache/managed-independent-20261022-third/immutable-assets"))?;
    let player: Player = serde_json::from_slice(&bytes)?;
    let session = Session::restore(&index, player.session)?;
    let before = serde_json::to_vec(&session.save())?;
    let cold = Session::restore(&index, session.save_managed()?)?;
    ensure!(
        before == serde_json::to_vec(&cold.save())?,
        "cold retained state differs"
    );
    let lot = session
        .simulation
        .architecture
        .as_ref()
        .context("architecture")?
        .original
        .as_ref()
        .context("lot")?;
    ensure!(
        lot.pool == session.house.pool_cells()?,
        "retained pool differs from imported source"
    );
    ensure!(
        lot.water.iter().all(|v| *v == 255),
        "Mashuga pond map changed"
    );
    let native: Native = serde_json::from_slice(&std::fs::read(
        root.join("cache/pool-road-20261022-first/native-selectors-actual.json"),
    )?)?;
    let mut original_pool = vec![255; lot.size * lot.size];
    for selection in &native.actual.selections {
        original_pool[selection.position[0] as usize * lot.size + selection.position[1] as usize] =
            selection.kind;
    }
    ensure!(original_pool == lot.pool, "native pool evidence differs");
    ensure!(
        native
            .actual
            .selections
            .iter()
            .filter(|s| s.kind < 16)
            .count()
            == 75
    );
    ensure!(
        native
            .actual
            .selections
            .iter()
            .filter(|s| s.kind == 16)
            .count()
            == 44
    );
    let floor_source = index.find("floors.iff")?;
    let floor_bytes = index.resources[floor_source].read()?;
    let iff = Iff::parse(floor_bytes.clone())?;
    let mut sources = BTreeMap::new();
    for zoom in 1..=3u16 {
        let ids = (0..16)
            .map(|n| 1024 + (zoom - 1) * 16 + n)
            .chain((0..4).map(|n| 1072 + (zoom - 1) * 4 + n))
            .chain([9, 10, 11, 12].map(|n| n + (zoom - 1) * 256));
        for id in ids {
            let frame = source(&iff, id)?;
            image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba.clone())
                .context("source dimensions")?
                .save(output.join(format!("source-SPR2-{id}.png")))?;
            sources.insert(id, frame);
        }
    }
    let selected = native
        .actual
        .selections
        .iter()
        .filter(|s| s.kind < 16)
        .collect::<Vec<_>>();
    let center_cell = selected[selected.len() / 2].position;
    let center = OriginalHouse::ground_point(
        &session,
        [center_cell[1] * 16 + 8, center_cell[0] * 16 + 8],
        1,
    )?;
    let mut renderer = OriginalHouse::default();
    let mut cold_renderer = OriginalHouse::default();
    let mut reports = vec![];
    for level in 1..=2 {
        for rotation in 0..4u8 {
            for zoom in 1..=3u16 {
                let mut camera = OriginalHouse::camera(
                    &session,
                    800,
                    600,
                    rotation,
                    (1u32 << zoom) as f32 / 8.,
                    level,
                );
                camera.center = center;
                let scene = renderer.scene(&session, camera)?;
                let key = format!("pool-floor{level}-rotation{rotation}-zoom{zoom}");
                let frame = software::render(&scene);
                frame.save(output.join(format!("{key}.png")))?;
                ensure!(frame == software::render(&scene), "{key}: repeat differs");
                let restored = software::render(&cold_renderer.scene(&cold, camera)?);
                if frame != restored {
                    restored.save(output.join(format!("{key}-cold-failure.png")))?;
                    anyhow::bail!("{key}: cold frame differs");
                }
                let mut checked_corners = 0;
                for selection in &native.actual.selections {
                    let pool_kind = selection.kind;
                    let mut rotated = pool_kind;
                    if pool_kind < 16 {
                        for _ in 0..rotation {
                            rotated = (rotated >> 1) | ((rotated & 1) << 3);
                        }
                    }
                    let id = if pool_kind < 16 {
                        1024 + (zoom - 1) * 16 + u16::from(rotated)
                    } else {
                        12 + (zoom - 1) * 256
                    };
                    let draws =
                        matching_draws(&scene, &triangle(&session, camera, selection.position)?);
                    let surface = draws
                        .iter()
                        .find(|(texture, _)| authored(&sources[&id], texture));
                    ensure!(
                        surface.is_some(),
                        "{key}: cell{:?} pool{pool_kind} missing SPR2{id}",
                        selection.position
                    );
                    let surface = surface.unwrap();
                    ensure!(
                        surface
                            .1
                            .iter()
                            .all(|v| v.is_finite() && (0. ..1.).contains(v)),
                        "{key}: pool depth outside scene range"
                    );
                    for corner in 0..4 {
                        if selection.original[rotation as usize] & (1 << corner) == 0 {
                            continue;
                        }
                        let id = 1072 + (zoom - 1) * 4 + corner;
                        let cap = draws
                            .iter()
                            .find(|(texture, _)| authored(&sources[&id], texture));
                        ensure!(
                            cap.is_some(),
                            "{key}: corner SPR2{id} missing at{:?}",
                            selection.position
                        );
                        ensure!(
                            cap.unwrap().1 == surface.1,
                            "{key}: cap/surface depth plane differs"
                        );
                        checked_corners += 1;
                    }
                }
                reports.push(json!({"key":key,"floor":level,"rotation":rotation,"zoom":zoom,
                    "camera_center":camera.center.to_array(),"pool_surfaces_checked":75,"rim_cells_checked":44,
                    "native_corner_caps_checked":checked_corners,"repeat_pixels_equal":true,"cold_pixels_equal":true}));
            }
        }
    }
    let saved_rotation = (session.house.view.camera_direction as u16 & 3) as u8;
    for rotation in 0..4u8 {
        for zoom in 1..=3u16 {
            let cell = lot.floors[0]
                .source
                .coverings
                .iter()
                .position(|id| *id == 10)
                .context("road10")?;
            let center = OriginalHouse::ground_point(
                &session,
                [
                    (cell % lot.size) as i32 * 16 + 8,
                    (cell / lot.size) as i32 * 16 + 8,
                ],
                1,
            )?;
            let mut camera =
                OriginalHouse::camera(&session, 640, 480, rotation, (1u32 << zoom) as f32 / 8., 1);
            camera.center = center;
            let scene = renderer.scene(&session, camera)?;
            let key = format!("road-rotation{rotation}-zoom{zoom}");
            let frame = software::render(&scene);
            frame.save(output.join(format!("{key}.png")))?;
            ensure!(frame == software::render(&scene), "{key}: repeat differs");
            ensure!(
                frame == software::render(&cold_renderer.scene(&cold, camera)?),
                "{key}: cold differs"
            );
            for authored_id in [10, 11] {
                let cell = lot.floors[0]
                    .source
                    .coverings
                    .iter()
                    .position(|id| *id == authored_id)
                    .context("road cell")?;
                let position = [(cell / lot.size) as i32, (cell % lot.size) as i32];
                let viewed = if (saved_rotation ^ rotation) & 1 != 0 {
                    authored_id ^ 1
                } else {
                    authored_id
                };
                let id = viewed + (zoom - 1) * 256;
                let draws = matching_draws(&scene, &triangle(&session, camera, position)?);
                ensure!(
                    draws.len() == 1 && authored(&sources[&id], draws[0].0),
                    "{key}: road{authored_id} did not select SPR2{id}"
                );
            }
            reports.push(json!({"key":key,"rotation":rotation,"zoom":zoom,"saved_artwork_rotation":saved_rotation,
                "road_materials_checked":[10,11],"repeat_pixels_equal":true,"cold_pixels_equal":true}));
        }
    }
    ensure!(
        before == serde_json::to_vec(&session.save())?,
        "render changed complete warm retained state"
    );
    ensure!(
        before == serde_json::to_vec(&cold.save())?,
        "render changed complete cold retained state"
    );
    let retained_camera = OriginalHouse::native_camera(&session)?;
    software::render(&renderer.scene(&session, retained_camera)?)
        .save(output.join("retained-view.png"))?;
    ensure!(before == serde_json::to_vec(&session.save())?);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
        "source_checkpoint":checkpoint,
        "floors_source":index.resources[floor_source].locator,
            "source_house_hashes":session.house.sources,"tick":session.simulation.vm.tick,
            "pool_grid_sha256":native.actual.grid_sha256,"cases":reports,"full_warm_state_unchanged":true,"full_cold_state_equal":true,
            "simulation_ticks_added":0,"matched_original_mashuga_reference":false,
            "reference_policy":"Original selector semantics and original authored resources; screenshots are Rust regression evidence, not matched original pixel parity."
        }))?,
    )?;
    println!("24 pool/floor/rotation/zoom cases,12 road cases; complete warm/cold state preserved");
    Ok(())
}
