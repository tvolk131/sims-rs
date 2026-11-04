//! Explicit outfit previews, not a claim that live outfit selection is complete.
use anyhow::{Context, Result, ensure};
use glam::{Quat, Vec2, Vec3};
use sims_content::{
    Index,
    saved_avatar::{BoundAvatar, load_bound},
    saves::House,
};
use sims_render::{Camera, Draw, Scene, Texture, Vertex, pose, software};
use sims_sim::pose::appearance::{Execution, Input};
use std::sync::Arc;

fn scene(avatar: &sims_content::avatar::Avatar, rotation: u8) -> Result<Scene> {
    let bones = pose::from_local_bones(
        &avatar.skeleton,
        &avatar
            .skeleton
            .bones
            .iter()
            .map(|b| sims_sim::pose::LocalBone {
                translation: b.translation,
                rotation: b.rotation,
            })
            .collect::<Vec<_>>(),
    )?;
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
    for part in &avatar.parts {
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
#[ignore = "requires extracted original household and text clothing assets"]
fn original_text_outfits_replace_restore_and_render_identically() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let house = House::load(&index, "UserData2/Houses/House09.iff")?;
    let mut gallery = image::RgbaImage::new(1280, 1520);
    for (row, (id, appearance, texture)) in [
        (196, "f100mafat_original", "f100mafatlgt_original"),
        (196, "l100mafat_original", "l100mafatlgt_original"),
        (195, "f100fafit_original", "f100fafitlgt_original"),
        (195, "l100fafit_original", "l100fafitlgt_original"),
    ]
    .into_iter()
    .enumerate()
    {
        let saved = house.objects.instances[&id]
            .person
            .as_ref()
            .context("saved person")?;
        let mut bound = load_bound(&index, "adult", saved, id as u64)?;
        let body = bound.records.body[0];
        ensure!(
            bound.records.entries[body].name.get(4..9) == appearance.get(4..9),
            "preview outfit sex/age/body shape differs from saved resident"
        );
        let old = bound.records.entries[body]
            .binding
            .context("original body binding")?;
        ensure!(
            bound.apply_record(
                &index,
                body,
                Execution::bind(Input::Replace {
                    name: appearance.into(),
                    texture: format!("BODY={texture}")
                })
            )? == vec![id as u64]
        );
        ensure!(!bound.part_ranges.contains_key(&old.id));
        ensure!(
            bound.avatar.parts.iter().any(|p| p
                .mesh
                .name
                .to_ascii_lowercase()
                .contains(appearance))
        );
        ensure!(
            bound
                .avatar
                .sources
                .keys()
                .any(|p| p.to_ascii_lowercase().ends_with(".skn"))
        );
        let state = bound.snapshot();
        let restored = BoundAvatar::restore(
            &index,
            "adult",
            id as u64,
            &serde_json::from_slice(&serde_json::to_vec(&state)?)?,
        )?;
        ensure!(restored.snapshot() == state);
        // The active binding snapshot reconstructs active assets; the source
        // avatar also retains provenance of outfits detached earlier.
        ensure!(
            restored
                .avatar
                .sources
                .iter()
                .all(|(path, hash)| bound.avatar.sources.get(path) == Some(hash))
        );
        for rotation in 0..4 {
            let before = software::render(&scene(&bound.avatar, rotation)?);
            let after = software::render(&scene(&restored.avatar, rotation)?);
            ensure!(
                before == after,
                "text outfit pixels changed after restoration"
            );
            image::imageops::replace(
                &mut gallery,
                &before,
                rotation as i64 * 320,
                row as i64 * 380,
            );
        }
        println!(
            "{id}: {appearance}, {} parts, restored four views identical",
            bound.avatar.parts.len()
        );
    }
    gallery.save(root.join("cache/original-text-outfits.png"))?;
    Ok(())
}

#[test]
#[ignore = "requires original household clothing-profile input report and private assets"]
fn original_selected_pajamas_and_migrated_sleep_outfits_restore_and_render() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-clothing-profile-inputs.json"),
    )?)?;
    let cases = report["cases"].as_array().context("clothing cases")?;
    ensure!(cases.len() == 7);
    let mut gallery = image::RgbaImage::new(cases.len() as u32 * 320, 760);
    for (column, case) in cases.iter().enumerate() {
        let house = House::load(&index, case["house"].as_str().unwrap())?;
        let id = case["object"].as_u64().unwrap() as u16;
        let saved = house.objects.instances[&id]
            .person
            .as_ref()
            .context("saved person")?;
        let skeleton = case["input"]["profile"][0].as_str().unwrap();
        let mut bound = load_bound(&index, skeleton, saved, id as u64)?;
        let first = sims_sim::pose::appearance_selection::Descriptor {
            name: case["expected"]["body"].as_str().unwrap().into(),
            texture: case["expected"]["texture"].as_str().unwrap().into(),
        };
        let next = sims_sim::pose::appearance_selection::descriptor(
            case["expected"]["profile"][32].as_str().unwrap(),
            false,
        );
        for (row, descriptor) in [first, next].into_iter().enumerate() {
            ensure!(!descriptor.name.is_empty());
            bound.apply_record(
                &index,
                bound.records.body[0],
                Execution::bind(Input::Replace {
                    name: descriptor.name.clone(),
                    texture: descriptor.texture,
                }),
            )?;
            let snapshot = serde_json::from_slice(&serde_json::to_vec(&bound.snapshot())?)?;
            let restored = BoundAvatar::restore(&index, skeleton, id as u64, &snapshot)?;
            ensure!(restored.snapshot() == snapshot);
            for rotation in 0..4 {
                let frame = software::render(&scene(&bound.avatar, rotation)?);
                ensure!(
                    frame == software::render(&scene(&restored.avatar, rotation)?),
                    "selected outfit pixels differ after restore"
                );
                if rotation == 0 {
                    image::imageops::replace(
                        &mut gallery,
                        &frame,
                        column as i64 * 320,
                        row as i64 * 380,
                    );
                }
            }
            println!("{id}: {}, four restored views identical", descriptor.name);
        }
    }
    gallery.save(root.join("cache/original-selected-sleep-outfits.png"))?;
    Ok(())
}
