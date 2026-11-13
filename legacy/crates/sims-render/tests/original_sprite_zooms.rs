use anyhow::{Context, Result, ensure};
use sims_content::{
    Index,
    vm::{ChairFixture, ChairScenario, chair_room, load_chair_animations, load_program},
};
use sims_formats::sprite::SpriteSet;
use sims_render::{
    Assets, Camera, graphic_group, original_house::OriginalHouse, room_scene, software,
    sprite_visible,
};
use sims_runtime::original_session::{Options, Session};

fn repo() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn authored_zoom_frames_offsets_and_flips_reach_the_scene() -> Result<()> {
    let index = Index::scan(&repo().join("game-data/cabinet-extracted"))?;
    let assets = Assets::load(&index)?;
    let doc = index.document(index.find("DiningChairs.iff")?)?;
    let obj = &doc
        .objects
        .iter()
        .find(|o| o.id == 16807)
        .context("chair absent")?
        .value;
    let mut program = load_program(&index, "DiningChairs.iff", "Global.iff")?;
    load_chair_animations(&index, &mut program)?;
    let mut checked = 0;
    for facing in [1, 4, 16, 64] {
        let fixture = ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty)?;
        let sim = chair_room(&program, fixture, false, facing)?;
        let before = serde_json::to_vec(&sims_sim::Snapshot::capture(&program, &sim))?;
        let chair = &sim.vm.world.objects[&2];
        let graphic = graphic_group(
            obj.graphic,
            obj.graphics,
            chair.data.get(&0).copied().unwrap_or(0),
        )
        .unwrap();
        let group = &doc.groups.iter().find(|g| g.id == graphic).unwrap().value;
        let base = program.modules[&doc.source].object_definition_words[&16807]
            .get(0x40)
            .copied()
            .unwrap_or(0);
        for rotation in 0..4 {
            let bits = (facing as u32).rotate_left(rotation as u32 * 2);
            let direction = (bits & 255) | (bits >> 8);
            for zoom in 1..=3 {
                let camera = Camera::room(&sim, 550, 380, rotation, (1u32 << zoom) as f32 / 8.);
                let scene = room_scene(&assets, &program, &sim, camera, 0.)?;
                let draws: Vec<_> = scene.draws.iter().filter(|d| d.object == Some(2)).collect();
                let view = group
                    .views
                    .iter()
                    .find(|v| v.zoom == zoom && v.direction == direction)
                    .context("authored view absent")?;
                let mut expected = vec![];
                for s in &view.sprites {
                    if !sprite_visible(s.id as i16, base, &chair.dynamic_sprite_flags) {
                        continue;
                    }
                    let chunk = doc.iff.get(b"SPR2", s.id as u16).unwrap();
                    let raw = doc.iff.data(chunk);
                    let frame = SpriteSet::parse(raw)?.decode(raw, s.frame as usize, &doc.iff)?;
                    if frame.width > 0 && frame.height > 0 {
                        expected.push((s, frame));
                    }
                }
                ensure!(!expected.is_empty() && draws.len() == expected.len());
                for (draw, (s, frame)) in draws.iter().zip(expected) {
                    let texture = &scene.textures[draw.texture];
                    ensure!(texture.width == frame.width && texture.height == frame.height);
                    ensure!(
                        texture.rgba.as_ref() == frame.rgba && texture.depth.as_ref() == frame.depth,
                        "authored zoom pixels changed"
                    );
                    let vertices =
                        &scene.vertices[draw.vertices.start as usize..draw.vertices.end as usize];
                    let body = &sim.bodies[&2];
                    let position = glam::Vec3::new(
                        body.position.x as f32 / 16.,
                        0.,
                        body.position.y as f32 / 16.,
                    );
                    let rotation = glam::Quat::from_rotation_y(
                        -(facing.trailing_zeros() as f32) * std::f32::consts::FRAC_PI_4,
                    );
                    let offset =
                        glam::Vec3::new(s.object[0] / 16., s.object[2] / 5., s.object[1] / 16.);
                    let expected_origin = camera.screen(position + rotation * offset)
                        + glam::Vec2::new(s.x as f32, s.y as f32 - frame.height as f32);
                    let actual_origin = glam::Vec2::new(
                        (vertices[0].position[0] + 1.) * 275.,
                        (1. - vertices[0].position[1]) * 190.,
                    );
                    ensure!(
                        (actual_origin - expected_origin).abs().max_element() < 0.001,
                        "DGRP offset differs at zoom{zoom}"
                    );
                    let width = (vertices[1].position[0] - vertices[0].position[0]) * 550. / 2.;
                    let height = (vertices[1].position[1] - vertices[2].position[1]) * 380. / 2.;
                    ensure!(
                        (width - frame.width as f32).abs() < 0.001
                            && (height - frame.height as f32).abs() < 0.001,
                        "original sprite pixels were rescaled at zoom{zoom}"
                    );
                    ensure!(vertices[0].uv[0] == if s.flags & 1 != 0 { 1. } else { 0. });
                    checked += 1;
                }
            }
        }
        ensure!(
            before == serde_json::to_vec(&sims_sim::Snapshot::capture(&program, &sim))?,
            "rendering changed simulation"
        );
    }
    println!("{checked} authored sprite layers checked across48 camera/facing combinations");
    Ok(())
}

#[test]
#[ignore = "requires private original houses; headless CPU gallery and saved-state equality"]
fn original_houses_render_all_authored_zooms_and_restore() -> Result<()> {
    let repo = repo().canonicalize()?;
    let output = if let Ok(path) = std::env::var("SIMS_SPRITE_ZOOMS_OUTPUT") {
        let output = repo.join(path);
        ensure!(
            output
                .parent()
                .context("gallery parent")?
                .canonicalize()?
                .starts_with(repo.join("cache")),
            "private output required"
        );
        std::fs::create_dir(&output).context("gallery evidence must be fresh")?;
        output
    } else {
        repo.join("cache")
    };
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let mut report = vec![];
    for (name, source) in [
        ("newbie", "UserData/Houses/House07.iff"),
        ("goth", "UserData/Houses/House05.iff"),
        ("mashuga", "UserData2/Houses/House09.iff"),
    ] {
        let session = Session::load(
            &index,
            source,
            Options {
                routing: sims_sim::lot_route::path::Policy::InferredRectangles,
                ..Default::default()
            },
        )?;
        let before = serde_json::to_vec(&session.save())?;
        let restored = Session::restore(&index, serde_json::from_slice(&before)?)?;
        let mut renderer = OriginalHouse::default();
        let mut resumed = OriginalHouse::default();
        let mut gallery = image::RgbaImage::new(1920, 960);
        for level in 1..=2 {
            for zoom in 1..=3 {
                let camera =
                    OriginalHouse::camera(&session, 640, 480, 0, (1u32 << zoom) as f32 / 8., level);
                let scene = renderer.scene(&session, camera)?;
                let frame = software::render(&scene);
                ensure!(
                    frame == software::render(&resumed.scene(&restored, camera)?),
                    "restored view differs: {name}/{level}/{zoom}"
                );
                image::imageops::replace(
                    &mut gallery,
                    &frame,
                    (zoom - 1) as i64 * 640,
                    (level - 1) as i64 * 480,
                );
                report.push(serde_json::json!({"house":name,"source":source,"level":level,"zoom":zoom,"draws":scene.draws.len(),"textures":scene.textures.len(),"restored_pixels_equal":true}));
            }
        }
        ensure!(
            before == serde_json::to_vec(&session.save())?
                && before == serde_json::to_vec(&restored.save())?,
            "rendering changed saved simulation or hosts"
        );
        gallery.save(output.join(format!("native-sprite-zooms-{name}.png")))?;
    }
    std::fs::write(
        output.join("native-sprite-zooms-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
