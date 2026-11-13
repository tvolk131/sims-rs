//! Read-only model checks for the actual direct native person factory.
//! Diagnostic full-body camera only; this is not the native portrait crop,
//! lighting, pixel renderer or dialog/cache integration.
use anyhow::{Context, Result, ensure};
use glam::{Quat, Vec2, Vec3};
use sims_content::Index;
use sims_render::{Camera, Draw, Scene, Texture, Vertex, pose, software};
use sims_runtime::original_session::{Saved, Session};
use std::sync::Arc;

fn scene(session: &Session, id: i16, rotation: u8) -> Result<Scene> {
    scene_selected(session, id, rotation, false)
}

fn scene_selected(session: &Session, id: i16, rotation: u8, only_head: bool) -> Result<Scene> {
    let assets = &session.host.characters[&id];
    let selected = assets.geometry.portrait_bindings(&assets.avatar)?;
    let avatar = &session.host.characters[&id].avatar.avatar;
    let bones = session.simulation.characters[&id]
        .character
        .displayed_pose()
        .map_err(anyhow::Error::msg)?;
    let bones = pose::from_absolute_bones(&bones)?;
    let camera = Camera {
        level: 1,
        width: 320,
        height: 380,
        rotation,
        zoom: 1.,
        center: Vec3::ZERO,
    };
    let mut textures = Vec::new();
    let mut vertices = Vec::new();
    let mut draws = Vec::new();
    let mut minimum = Vec2::splat(f32::INFINITY);
    let mut maximum = Vec2::splat(f32::NEG_INFINITY);
    for (part_index, part) in avatar.parts.iter().enumerate() {
        if only_head && !selected.iter().any(|(_, part)| *part == part_index) {
            continue;
        }
        let samples = pose::skin(&part.mesh, &avatar.skeleton, &bones)?;
        let start = vertices.len() as u32;
        for &i in part.mesh.triangles.iter().flatten() {
            let (p, normal) = samples[i as usize];
            let p = Quat::from_rotation_y(std::f32::consts::PI) * p / 3.;
            let screen = camera.screen(p);
            minimum = minimum.min(screen);
            maximum = maximum.max(screen);
            let shade = 0.65
                + 0.35
                    * normal
                        .normalize_or_zero()
                        .dot(Vec3::new(0.3, 0.8, 0.5).normalize())
                        .abs();
            vertices.push(Vertex {
                position: [screen.x, screen.y, camera.depth(p)],
                uv: part.mesh.vertices[i as usize].uv,
                tint: [shade, shade, shade, 1.],
                depth_scale: 0.,
            });
        }
        draws.push(Draw {
            object: None,
            vertices: start..vertices.len() as u32,
            texture: textures.len(),
        });
        textures.push(Texture {
            width: part.texture.width,
            height: part.texture.height,
            rgba: part.texture.rgba.clone().into(),
            depth: vec![255; (part.texture.width * part.texture.height) as usize].into(),
        });
    }
    let scale = (280. / (maximum.x - minimum.x)).min(340. / (maximum.y - minimum.y));
    for v in &mut vertices {
        let p = (Vec2::new(v.position[0], v.position[1]) - (minimum + maximum) * 0.5) * scale
            + Vec2::new(160., 190.);
        v.position[0] = p.x / 160. - 1.;
        v.position[1] = 1. - p.y / 190.;
    }
    Ok(Scene {
        censors: vec![],
        width: 320,
        height: 380,
        vertices,
        draws,
        textures: Arc::new(textures),
    })
}

#[test]
#[ignore = "requires original portrait-preparation capture and assets; CPU rendering"]
fn forced_portrait_head_bindings_render_identically_after_restore() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let saved: Saved = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-portrait-prepared-session.json"),
    )?)?;
    let session = Session::restore(&index, saved)?;
    let before = session.save();
    let restored = Session::restore(
        &index,
        serde_json::from_slice(&serde_json::to_vec(&before)?)?,
    )?;
    let mut gallery = image::RgbaImage::new(1280, 380);
    for rotation in 0..4 {
        let frame = software::render(&scene_selected(&session, 196, rotation, true)?);
        ensure!(frame == software::render(&scene_selected(&restored, 196, rotation, true)?));
        ensure!(frame.pixels().any(|p| p.0 != [24, 32, 37, 255]));
        image::imageops::replace(&mut gallery, &frame, i64::from(rotation) * 320, 0);
    }
    ensure!(
        session.simulation == before.simulation.state && session.host.snapshot() == before.assets
    );
    gallery.save(root.join("cache/original-portrait-selected-head.png"))?;
    println!(
        "four views of native-selected HEAD models match after restore; diagnostic framing/lighting, no simulation writes"
    );
    Ok(())
}

#[test]
#[ignore = "requires direct factory source diagnostics and original assets; CPU rendering"]
fn direct_factory_models_restore_pixels_without_advancing_the_household() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut gallery = image::RgbaImage::new(1280, 760);
    for (row, label) in ["initial", "deceased-diagnostic"].into_iter().enumerate() {
        let report: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
            format!("cache/original-direct-portrait-person-{label}.json"),
        ))?)?;
        let saved: Saved = serde_json::from_slice(&std::fs::read(root.join(format!(
            "cache/original-direct-portrait-person-{label}-session.json"
        )))?)?;
        let session = Session::restore(&index, saved)?;
        let before = session.save();
        let restored = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&before)?)?,
        )?;
        let id = i16::try_from(report["object"].as_i64().context("created ID absent")?)?;
        ensure!(session.simulation.person_factories[&id].parent.is_none());
        ensure!(session.simulation.person_factories[&id].finalized);
        ensure!(session.simulation.unplaced.contains(&id));
        ensure!(!session.host.characters[&id].avatar.avatar.parts.is_empty());
        for rotation in 0..4 {
            let frame = software::render(&scene(&session, id, rotation)?);
            ensure!(frame == software::render(&scene(&restored, id, rotation)?));
            ensure!(
                frame.pixels().any(|p| p.0 != [24, 32, 37, 255]),
                "empty preview"
            );
            image::imageops::replace(
                &mut gallery,
                &frame,
                i64::from(rotation) * 320,
                row as i64 * 380,
            );
        }
        ensure!(session.simulation == before.simulation.state);
        ensure!(session.host.snapshot() == before.assets);
        println!(
            "{label}: four original-asset model views match after restore; no simulation writes"
        );
    }
    gallery.save(root.join("cache/original-direct-portrait-models.png"))?;
    Ok(())
}

#[test]
#[ignore = "requires temporary-person lifecycle diagnostics and original assets; CPU rendering"]
fn temporary_person_model_and_cleaned_house_restore_without_visual_side_effects() -> Result<()> {
    use sims_render::original_house::OriginalHouse;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    for label in ["placed", "no-location"] {
        let report: serde_json::Value = serde_json::from_slice(&std::fs::read(
            root.join(format!("cache/original-temporary-person-{label}.json")),
        )?)?;
        let saved: Saved = serde_json::from_slice(&std::fs::read(root.join(format!(
            "cache/original-temporary-person-{label}-session.json"
        )))?)?;
        let session = Session::restore(&index, saved)?;
        let before = session.save();
        let restored = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&before)?)?,
        )?;
        let id = i16::try_from(report["object"].as_i64().context("created ID absent")?)?;
        ensure!(
            session.simulation.vm.tick == 215742 && session.simulation.portrait_person.is_none()
        );
        if label == "placed" {
            ensure!(!session.simulation.unplaced.contains(&id));
            // The original +88 call changes physical placement. Portrait
            // rendering later supplies a separate model matrix; this preview
            // inspects the loaded local pose, not that unimplemented matrix.
            let mut gallery = image::RgbaImage::new(1280, 380);
            for rotation in 0..4 {
                let frame = software::render(&scene(&session, id, rotation)?);
                ensure!(frame == software::render(&scene(&restored, id, rotation)?));
                ensure!(frame.pixels().any(|p| p.0 != [24, 32, 37, 255]));
                image::imageops::replace(&mut gallery, &frame, i64::from(rotation) * 320, 0);
            }
            gallery.save(root.join("cache/original-temporary-person-model.png"))?;
        } else {
            ensure!(
                !session.simulation.vm.world.objects.contains_key(&id)
                    && !session.host.characters.contains_key(&id)
            );
            let camera = OriginalHouse::camera(&session, 960, 720, 0, 0.55, 1);
            let scene = OriginalHouse::default().scene(&session, camera)?;
            ensure!(scene.draws.iter().all(|d| d.object != Some(id)));
            let frame = software::render(&scene);
            ensure!(frame == software::render(&OriginalHouse::default().scene(&restored, camera)?));
            frame.save(root.join("cache/original-temporary-person-cleaned-house.png"))?;
        }
        ensure!(
            session.simulation == before.simulation.state
                && session.host.snapshot() == before.assets
        );
        println!("{label}: headless views match after restore, with no simulation/asset writes");
    }
    Ok(())
}
