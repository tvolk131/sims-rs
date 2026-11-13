//! Headless head-pose diagnostics using original saved resident appearances.
//! This samples the saved base clip at frame zero; it does not activate a lot
//! or claim restoration of its suspended native animator/interaction.
use anyhow::{Context, Result, ensure};
use glam::{Mat4, Quat, Vec3};
use sims_content::{Index, avatar::Avatar, saves::House};
use sims_render::{Camera, Draw, Scene, Texture, Vertex, pose, software};
use sims_sim::head_look::{State, rig::Rig};
use std::sync::Arc;
fn scene(avatar: &Avatar, bones: &[Mat4], rotation: u8) -> Result<Scene> {
    scene_with_zoom(avatar, bones, rotation, 2.)
}
fn scene_with_zoom(avatar: &Avatar, bones: &[Mat4], rotation: u8, zoom: f32) -> Result<Scene> {
    scene_with_vertices(avatar, bones, rotation, zoom, None)
}
fn scene_with_vertices(
    avatar: &Avatar,
    bones: &[Mat4],
    rotation: u8,
    zoom: f32,
    published: Option<&[Vec<(Vec3, Vec3)>]>,
) -> Result<Scene> {
    let camera = Camera {
        level: 1,
        width: 400,
        height: 400,
        rotation,
        zoom,
        center: Vec3::new(0., 1., 0.),
    };
    let textures = avatar
        .parts
        .iter()
        .map(|p| Texture {
            width: p.texture.width,
            height: p.texture.height,
            rgba: p.texture.rgba.clone().into(),
            depth: vec![255; (p.texture.width * p.texture.height) as usize].into(),
        })
        .collect();
    let mut scene = Scene {
        vertices: vec![],
        draws: vec![],
        censors: vec![],
        textures: Arc::new(textures),
        width: 400,
        height: 400,
    };
    let turn = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
    let mut censor_points = vec![];
    for (texture, part) in avatar.parts.iter().enumerate() {
        let verts = if let Some(published) = published {
            published
                .get(texture)
                .context("published vertex part absent")?
                .clone()
        } else {
            pose::skin(&part.mesh, &avatar.skeleton, bones)?
        };
        ensure!(
            verts.len() >= part.mesh.vertices.len(),
            "published vertices incomplete"
        );
        if part.censor {
            for (p, _) in &verts[..part.mesh.vertices.len()] {
                let p = turn * *p / 3.;
                let pixel = camera.screen(p);
                censor_points.push([pixel.x, pixel.y, camera.depth(p)]);
            }
            continue;
        }
        let start = scene.vertices.len() as u32;
        for face in &part.mesh.triangles {
            for &j in face {
                let (p, n) = verts[j as usize];
                let p = turn * p / 3.;
                let pixel = camera.screen(p);
                let shade = 0.72
                    + 0.28
                        * (turn * n)
                            .normalize_or_zero()
                            .dot(Vec3::new(-0.35, 0.8, -0.4).normalize())
                            .max(0.);
                scene.vertices.push(Vertex {
                    position: [pixel.x / 200. - 1., 1. - pixel.y / 200., camera.depth(p)],
                    uv: part.mesh.vertices[j as usize].uv,
                    tint: [shade, shade, shade, 1.],
                    depth_scale: 0.,
                });
            }
        }
        scene.draws.push(Draw {
            vertices: start..scene.vertices.len() as u32,
            texture,
            object: None,
        });
    }
    if let Some(region) = sims_render::censor::Region::projected(&censor_points, zoom, 0) {
        scene.censors.push(region);
    }
    Ok(scene)
}
/// Pose-channel diagnostic, acknowledging events without gameplay callbacks.
/// The original native event/pose test independently checks callback-time poses.
fn layer_progression(index: &Index, program: &mut sims_vm::Program, avatar: &Avatar) -> Result<()> {
    use sims_sim::pose::{
        LocalBone,
        layer::{Layer, Start},
        sampling::BoneInput,
        update::Update,
    };
    let mut gallery = image::RgbaImage::new(1600, 1600);
    for (row, name) in [
        "a2o-sit-front",
        "a2o-stand-front",
        "a2o-walking-loop",
        "a2o-walking-start",
    ]
    .into_iter()
    .enumerate()
    {
        sims_content::assets::load_named_animations(index, program, &[name.to_owned()].into())?;
        let asset = &program.animations[name];
        let mut layer = Layer::bind(
            &avatar.skeleton,
            &asset.clip,
            Start {
                priority: 1,
                speed: 1.,
                weight: 1.,
                now: 0,
            },
        )
        .map_err(anyhow::Error::msg)?;
        layer.clock.mode = i32::from(name == "a2o-walking-loop");
        let mut bones = avatar
            .skeleton
            .bones
            .iter()
            .map(|b| BoneInput {
                local: LocalBone {
                    translation: b.translation,
                    rotation: b.rotation,
                },
                priority: -10000,
                blend_marker: b.flags[2],
            })
            .collect::<Vec<_>>();
        for tick in 0..=25 {
            let mut update =
                Update::begin(&mut layer, tick * 50, true, true).map_err(anyhow::Error::msg)?;
            while update
                .next(&mut layer, &asset.clip, &asset.samples, &mut bones)
                .map_err(anyhow::Error::msg)?
                .is_some()
            {
                (layer, update, bones) =
                    serde_json::from_slice(&serde_json::to_vec(&(&layer, &update, &bones))?)?;
                update.acknowledge().map_err(anyhow::Error::msg)?;
            }
            if let Some(column) = [0, 5, 15, 25].iter().position(|t| *t == tick) {
                let local = bones.iter().map(|b| b.local).collect::<Vec<_>>();
                let matrices = pose::from_local_bones(&avatar.skeleton, &local)?;
                let pixels = software::render(&scene_with_zoom(avatar, &matrices, 0, 1.5)?);
                let restored: Vec<BoneInput> =
                    serde_json::from_slice(&serde_json::to_vec(&bones)?)?;
                let restored = pose::from_local_bones(
                    &avatar.skeleton,
                    &restored.iter().map(|b| b.local).collect::<Vec<_>>(),
                )?;
                ensure!(
                    pixels == software::render(&scene_with_zoom(avatar, &restored, 0, 1.5)?),
                    "layer progression restore changed pixels"
                );
                image::imageops::replace(
                    &mut gallery,
                    &pixels,
                    column as i64 * 400,
                    row as i64 * 400,
                );
            }
        }
    }
    gallery.save("cache/head-look-frames/layer-progression-gallery.png")?;
    Ok(())
}

/// Original ownership/property updates. The diagnostic supplies a hard floor
/// surface and records sound requests; it does not run an audio engine.
#[derive(Clone, Copy)]
enum TransitionProfile {
    Forced,
    Loaded,
    Ordinary,
    Turning,
    Walking,
    RootWalking,
}

/// Measure the original walking rig for this diagnostic. This does not execute
/// the full native appearance constructor or its gameplay/property callbacks.
fn movement_measurements(
    index: &Index,
    program: &mut sims_vm::Program,
    avatar: &Avatar,
    private: &str,
) -> Result<(
    sims_sim::pose::movement::calibration::Distances,
    [f32; 2],
    [f32; 2],
)> {
    use sims_sim::pose::{
        LocalBone,
        layer::{Layer, Start},
        movement::calibration,
        sampling::BoneInput,
    };
    let mut distances = [0.; 5];
    let mut names = vec![];
    for (i, selection) in [19, 21, 20, 22, 23].into_iter().enumerate() {
        let name = sims_vm::animation::posture_name(30, 0, 0, selection, |table, n| {
            let (source, table) = match table {
                sims_vm::animation::NameTable::Private(t) => (private, t),
                sims_vm::animation::NameTable::Global(t) => (program.globals.as_str(), t),
            };
            program
                .modules
                .get(source)?
                .strings
                .get(&table)?
                .get(n)
                .map(String::as_str)
        })
        .map(str::to_ascii_lowercase);
        if let Some(name) = &name {
            sims_content::assets::load_named_animations(index, program, &[name.clone()].into())?;
            distances[i] = program.animations[name].clip.distance;
        }
        names.push(name);
    }
    let asset = &program.animations[names[2]
        .as_ref()
        .context("walking measurement clip absent")?];
    let anchors = [
        calibration::property(&asset.clip, "anchor", "1")
            .map_err(anyhow::Error::msg)?
            .context("anchor 1")?,
        calibration::property(&asset.clip, "anchor", "2")
            .map_err(anyhow::Error::msg)?
            .context("anchor 2")?,
    ];
    let times = anchors.map(|(_, time)| time);
    let phase_window =
        calibration::phases(times, asset.clip.duration_ms).map_err(anyhow::Error::msg)?;
    let anchor_bones: Vec<_> = anchors
        .iter()
        .map(|(motion, _)| {
            avatar
                .skeleton
                .bones
                .iter()
                .position(|b| b.name == asset.clip.motions[*motion].bone)
                .context("measurement anchor bone")
        })
        .collect::<Result<_>>()?;
    let mut layer = Layer::bind(
        &avatar.skeleton,
        &asset.clip,
        Start {
            priority: 1,
            speed: 1.,
            weight: 1.,
            now: 0,
        },
    )
    .map_err(anyhow::Error::msg)?;
    let mut bones: Vec<_> = avatar
        .skeleton
        .bones
        .iter()
        .map(|b| BoneInput {
            local: LocalBone {
                translation: b.translation,
                rotation: b.rotation,
            },
            priority: -10000,
            blend_marker: b.flags[2],
        })
        .collect();
    let mut z = [[0.; 2]; 2];
    for (i, phase) in phase_window.iter().enumerate() {
        layer.clock.phase = *phase;
        layer
            .sample(&asset.clip, &asset.samples, &mut bones, 0.)
            .map_err(anyhow::Error::msg)?;
        let absolute = sims_sim::pose::hierarchy(
            &avatar
                .skeleton
                .bones
                .iter()
                .map(|b| b.parent)
                .collect::<Vec<_>>(),
            &bones.iter().map(|b| b.local).collect::<Vec<_>>(),
            sims_sim::head_look::rig::AbsoluteBone {
                translation: [0.; 3],
                rotation: [0., 0., 0., 1.],
            },
        )
        .map_err(anyhow::Error::msg)?;
        z[i] = [
            absolute[anchor_bones[0]].translation[2],
            absolute[anchor_bones[1]].translation[2],
        ];
    }
    let measured = calibration::Distances::new(distances).map_err(anyhow::Error::msg)?;
    let stop = calibration::stop_distances(z).map_err(anyhow::Error::msg)?;
    std::fs::write(
        "cache/head-look-frames/movement-measurements.json",
        serde_json::to_vec_pretty(
            &serde_json::json!({"clip_names":names,"distances":distances,"times":times,"anchor_bones":anchor_bones,"phase_window":phase_window,"z":z,"stop_distance":stop,"measured":measured}),
        )?,
    )?;
    Ok((measured, phase_window, stop))
}
fn animator_transitions(
    index: &Index,
    program: &mut sims_vm::Program,
    avatar: &Avatar,
    profile: &[String],
    private: &str,
    transition: TransitionProfile,
) -> Result<()> {
    use sims_sim::pose::{
        animator::{Settings, Slot},
        ordinary::{Entry, Prefix, RateSettings, Source, State as PrefixState},
        properties::{Context as PropertyContext, Request as PropertyRequest},
        refresh::{Censor, Character, Request, Response},
    };
    let loaded = matches!(transition, TransitionProfile::Loaded);
    let ordinary = !matches!(
        transition,
        TransitionProfile::Forced | TransitionProfile::Loaded
    );
    let turning = matches!(transition, TransitionProfile::Turning);
    let rooted = matches!(transition, TransitionProfile::RootWalking);
    let walking = matches!(
        transition,
        TransitionProfile::Walking | TransitionProfile::RootWalking
    );
    let locomotion = turning || walking;
    let measurements = if rooted {
        Some(movement_measurements(index, program, avatar, private)?)
    } else {
        None
    };
    let output = match transition {
        TransitionProfile::Forced => "animator",
        TransitionProfile::Loaded => "loaded",
        TransitionProfile::Ordinary => "ordinary",
        TransitionProfile::Turning => "movement",
        TransitionProfile::Walking => "path",
        TransitionProfile::RootWalking => "root-path",
    };
    let mut gallery = image::RgbaImage::new(1600, 800);
    for (row, fade_enabled) in [false, true].into_iter().enumerate() {
        let constructed = sims_sim::pose::construction::State::new(0);
        let settings = Settings {
            fade_enabled,
            fade_duration: 200,
            sampling_config: 1,
        };
        // Explicit diagnostic process seed, separate from saved lot RNG. The
        // real prototype clone now supplies bone/skin state and constructor
        // draw consumption. This still does not run the whole 58831d initializer.
        let mut crt = 0x51a5cafe;
        let mut cloned =
            sims_sim::pose::skeleton::Instance::clone_prototype(&avatar.skeleton, &mut crt)
                .map_err(anyhow::Error::msg)?;
        cloned
            .hierarchy
            .update(&mut cloned.skin, false)
            .map_err(anyhow::Error::msg)?;
        let mut character = Character::new(
            cloned.skin,
            cloned.hierarchy,
            State {
                last_clock: 0,
                basis_heading: 0.,
                target_heading: 2.,
                turn: 0.8,
                target_point: Some([13., 17., 23.]),
            },
            Rig::bind(&avatar.skeleton, profile).map_err(anyhow::Error::msg)?,
            Mat4::IDENTITY.to_cols_array(),
            constructed.placement,
            sims_sim::pose::model::Model {
                placement: sims_sim::pose::model::Matrix::IDENTITY,
                scale: sims_sim::pose::model::Matrix::IDENTITY,
                bounds_transform: sims_sim::pose::model::Matrix::IDENTITY,
                bounds_flags: 0,
            },
            Censor {
                binding: None,
                mask: 0,
            },
            constructed.clock,
        )
        .map_err(anyhow::Error::msg)?;
        let mut person: std::collections::BTreeMap<i16, i16> = [
            (0, 0),
            (22, 0),
            (23, 0),
            (30, 0),
            (41, 2),
            (42, 1),
            (44, 0),
            (45, 60),
            (58, 30),
            (64, 0),
            (65, 0),
            (68, 0),
            (74, 0),
            (84, 0),
        ]
        .into();
        // Explicit portrait initialization. Imported animator initialization is
        // a separate service; these values are not a saved-person fallback.
        let mut prefix = Prefix::new(PrefixState {
            clock_mode: 0,
            base_posture: -1,
            pickup: 0,
            carry: sims_vm::animation::CarrySelector {
                mode: 0,
                selection: -1,
            },
            mode: 0,
            rate: 1.,
            target_rate: 1.,
        });
        let rate = RateSettings {
            base: 1.2,
            positive: 0.2,
            negative: 0.4,
            change: 0.2,
        };
        use sims_sim::pose::movement;
        let mut movement = movement::Movement::new(movement::State {
            status: 0,
            flags: [0; 2],
            start: [160., 320., 2.],
            end: [160., 320., 2.],
            turn_delta: 0.,
            nominal_turn: 0.,
            reason: 0,
            apply_rate: 0,
        });
        let movement_settings = movement::Settings {
            short_distance: 3.1,
            turn_rate: 1.,
            small_turn: 0.5,
            medium_turn: 1.5,
            large_turn: 3.,
            minimum_weight: 0.5,
            snap_distance: 0.5,
            rate,
        };
        let mut facing = 2;
        let mut physical_position = [320, 160];
        let walk_style = if walking { row as i16 } else { 0 };
        if locomotion {
            prefix.state.mode = 3;
            // The first tick exercises the native sentinel-placement service;
            // a turn target is submitted after that cache initialization.
            character.placement.base = [-999.; 3];
            person.insert(41, 0);
            person.insert(42, 0);
        }
        let context = PropertyContext {
            actor: 137,
            sound_visible: true,
            selected: true,
            debug_sound: false,
            position_x: 32.,
            appearance_mode: 0,
            snow: false,
            lot_kind: 1,
        };
        let mut sounds = vec![];
        let mut selections = vec![];
        let mut movement_frames = vec![];
        let mut physical_steps = 0;
        let mut root_corrections = 0;
        let mut endpoint_tick = None;
        let mut geometry_bounds = [0_f32; 6];
        let mut published_bounds = [0_f32; 6];
        for tick in 0..=if walking {
            240
        } else if turning {
            30
        } else {
            15
        } {
            let now = sims_sim::pose::ordinary::clock(tick, 0);
            let mut root_entry = u8::from(rooted);
            if turning && tick == 1 {
                character.placement.previous[2] = if row == 0 { 5. } else { 0. };
            }
            if walking && tick == 1 {
                prefix.state.mode = if walk_style == 0 { 9 } else { 10 };
                movement.state.status = 1;
                // Authored open-floor path and retained per-actor state. Only
                // the Settings constants come from the captured configuration;
                // this is not saved-lot initialization or protected path search.
                movement.path = Some(movement::path::Path {
                    points: vec![[320, 160], [320, 192], [352, 192], [352, 224], [352, 225]],
                    settings: movement::path::Settings {
                        sample_step: 1.,
                        turn_threshold: 2.,
                        turn_step: 0.2,
                        minimum_brake_time: 100,
                        endpoint_distance: [2., 3.],
                    },
                    index: 0,
                    progress: 0.,
                    deadline: 0,
                    phase_window: [0.25, 0.75],
                    stop_distance: [10., 10.],
                    start_distance: [2., 2.],
                    lookahead: [8., 12.],
                    interrupt: 0,
                    guard: 0,
                    delta: [0.; 2],
                    stop_mode: 0,
                });
                if let Some((distances, phases, stop)) = measurements {
                    let path = movement.path.as_mut().unwrap();
                    path.phase_window = phases;
                    path.stop_distance = stop;
                    path.start_distance = distances.start_distance;
                    path.lookahead = distances.lookahead;
                }
            }
            if let Some(name) = match (locomotion, tick) {
                (false, 0) => Some("a2o-sit-front"),
                (false, 9) => Some("a2o-stand-front"),
                (false, 11) => Some("a2o-walking-loop"),
                _ => None,
            } {
                sims_content::assets::load_named_animations(
                    index,
                    program,
                    &[name.to_owned()].into(),
                )?;
                character
                    .skin
                    .animator
                    .start(
                        Slot::Foreground,
                        &avatar.skeleton,
                        Some(&program.animations[name].clip),
                        false,
                        now,
                        settings,
                    )
                    .map_err(anyhow::Error::msg)?;
            }
            if ordinary {
                use sims_sim::pose::ordinary::{
                    Continuation, Request as PrefixRequest, Response as PrefixResponse,
                };
                prefix
                    .begin(
                        &mut character,
                        tick,
                        Entry {
                            root_motion: root_entry,
                            movement_input: person[&64],
                            walk_style,
                        },
                    )
                    .map_err(anyhow::Error::msg)?;
                while let Some(request) = prefix
                    .next(
                        &mut character,
                        Source {
                            posture: person[&0],
                            species: person[&65],
                            definition_flags: 0,
                            walk_style,
                        },
                        settings,
                        rate,
                    )
                    .map_err(anyhow::Error::msg)?
                {
                    (prefix, character, person) = serde_json::from_slice(&serde_json::to_vec(&(
                        &prefix, &character, &person,
                    ))?)?;
                    let response = match request {
                        PrefixRequest::CarriedSlot => PrefixResponse::CarriedSlot(false),
                        // Turning has an authored interior placement; the other
                        // profiles isolate the outside-lot shared tail.
                        PrefixRequest::OutsideLot => PrefixResponse::OutsideLot(!locomotion),
                        PrefixRequest::Animation {
                            index: selection, ..
                        } => {
                            let name = sims_vm::animation::posture_name(
                                person[&58],
                                person[&64],
                                person[&65],
                                selection as usize,
                                |table, index| {
                                    let (source, table) = match table {
                                        sims_vm::animation::NameTable::Private(table) => {
                                            (private, table)
                                        }
                                        sims_vm::animation::NameTable::Global(table) => {
                                            (program.globals.as_str(), table)
                                        }
                                    };
                                    program
                                        .modules
                                        .get(source)?
                                        .strings
                                        .get(&table)?
                                        .get(index)
                                        .map(String::as_str)
                                },
                            )
                            .map(str::to_ascii_lowercase);
                            ensure!(
                                name.is_some(),
                                "original portrait posture animation {selection} is absent"
                            );
                            selections.push(serde_json::json!({"index":selection,"name":name}));
                            if let Some(name) = &name {
                                sims_content::assets::load_named_animations(
                                    index,
                                    program,
                                    &[name.clone()].into(),
                                )?;
                            }
                            PrefixResponse::Animation { name, ready: true }
                        }
                    };
                    prefix
                        .acknowledge(
                            &mut character,
                            program,
                            &avatar.skeleton,
                            settings,
                            response,
                        )
                        .map_err(anyhow::Error::msg)?;
                }
                let continuation = prefix.finish().map_err(anyhow::Error::msg)?;
                if locomotion {
                    movement
                        .begin(tick, continuation)
                        .map_err(anyhow::Error::msg)?;
                    while let Some(request) = movement
                        .next(
                            &mut character.skin.animator,
                            &mut character.placement,
                            &mut prefix.state,
                            &mut facing,
                            movement::Source {
                                object9: 0,
                                level: 1,
                                species: person[&65],
                                person78: 0,
                                walk_style,
                            },
                            movement_settings,
                            settings,
                            program,
                        )
                        .map_err(anyhow::Error::msg)?
                    {
                        (
                            movement,
                            character,
                            prefix,
                            person,
                            facing,
                            physical_position,
                        ) = serde_json::from_slice(&serde_json::to_vec(&(
                            &movement,
                            &character,
                            &prefix,
                            &person,
                            facing,
                            physical_position,
                        ))?)?;
                        if request == movement::Request::Placement {
                            use sims_sim::pose::transform::{Source, Terrain, View};
                            movement
                                .place(
                                    &mut character,
                                    Source {
                                        position: physical_position,
                                        facing,
                                        level: 1,
                                        flags: [0; 2],
                                    },
                                    View {
                                        rotation: 0,
                                        floor_vector: [0.; 3],
                                    },
                                    Terrain {
                                        size: 64,
                                        corners: &vec![[0; 4]; 64 * 64],
                                    },
                                )
                                .map_err(anyhow::Error::msg)?;
                            continue;
                        }
                        if let movement::Request::AcceptStep { level, .. } = request {
                            ensure!(walking && level == 1, "unexpected path acceptance");
                            // This authored diagnostic has a flat, empty floor.
                            movement.accept_step(true).map_err(anyhow::Error::msg)?;
                            continue;
                        }
                        if let movement::Request::MoveStep { position, level } = request {
                            ensure!(walking && level == 1, "unexpected physical move");
                            physical_position = position;
                            physical_steps += 1;
                            movement
                                .moved(
                                    &mut character.skin.animator,
                                    &mut character.placement,
                                    &mut prefix.state,
                                )
                                .map_err(anyhow::Error::msg)?;
                            continue;
                        }
                        let selection = match request {
                            movement::Request::Foreground { index }
                            | movement::Request::StopForeground { index } => index,
                            other => anyhow::bail!(
                                "unexpected movement portrait service {other:?} at tick {tick}"
                            ),
                        };
                        let name = sims_vm::animation::posture_name(
                            person[&58],
                            person[&64],
                            person[&65],
                            selection as usize,
                            |table, index| {
                                let (source, table) = match table {
                                    sims_vm::animation::NameTable::Private(table) => {
                                        (private, table)
                                    }
                                    sims_vm::animation::NameTable::Global(table) => {
                                        (program.globals.as_str(), table)
                                    }
                                };
                                program
                                    .modules
                                    .get(source)?
                                    .strings
                                    .get(&table)?
                                    .get(index)
                                    .map(String::as_str)
                            },
                        )
                        .context("original movement clip name")?
                        .to_ascii_lowercase();
                        selections
                            .push(serde_json::json!({"tick":tick,"index":selection,"name":name}));
                        sims_content::assets::load_named_animations(
                            index,
                            program,
                            &[name.clone()].into(),
                        )?;
                        let selected =
                            if matches!(request, movement::Request::StopForeground { .. }) {
                                movement.stop_selected(
                                    &mut character.skin.animator,
                                    &prefix.state,
                                    &avatar.skeleton,
                                    Some(&program.animations[&name].clip),
                                    settings,
                                )
                            } else {
                                movement.selected(
                                    &mut character.skin.animator,
                                    &prefix.state,
                                    &avatar.skeleton,
                                    Some(&program.animations[&name].clip),
                                    settings,
                                )
                            };
                        selected.map_err(anyhow::Error::msg)?;
                    }
                    let movement::Continuation::Tail { entry, .. } =
                        movement.finish().map_err(anyhow::Error::msg)?
                    else {
                        anyhow::bail!("movement portrait left path following unfinished");
                    };
                    root_entry = entry.root_motion;
                } else {
                    ensure!(
                        continuation == Continuation::Tail,
                        "portrait unexpectedly requested native movement"
                    );
                }
                ensure!(
                    character.skin.animator.slot(Slot::Base).is_some(),
                    "original portrait base layer did not start"
                );
                character
                    .begin_ordinary_tail(now, false, true, character.model.placement)
                    .map_err(anyhow::Error::msg)?;
            } else if loaded {
                character
                    .begin_loaded_tail(true)
                    .map_err(anyhow::Error::msg)?;
            } else {
                character
                    .begin(now, false, true, true)
                    .map_err(anyhow::Error::msg)?;
            }
            while let Some(request) = character
                .next(program, &mut person, context, 1)
                .map_err(anyhow::Error::msg)?
            {
                (character, person) =
                    serde_json::from_slice(&serde_json::to_vec(&(&character, &person))?)?;
                ensure!(
                    character
                        .next(program, &mut person, context, 1)
                        .map_err(anyhow::Error::msg)?
                        == Some(request.clone()),
                    "animator property request changed during restore"
                );
                let response = match request {
                    Request::LoadedAppearance => {
                        // Headless appearance host evaluates the temporary head
                        // pose at this boundary; native mesh-cache flags remain
                        // outside this diagnostic.
                        let matrices = pose::from_absolute_bones(&character.hierarchy.absolute)?;
                        for part in &avatar.parts {
                            pose::skin(&part.mesh, &avatar.skeleton, &matrices)?;
                        }
                        Response::Done
                    }
                    Request::RefreshClock => Response::Clock(now),
                    Request::Property(PropertyRequest::FootstepSurface { .. }) => {
                        Response::PropertySurface(1)
                    }
                    Request::Property(request @ PropertyRequest::Sound { .. }) => {
                        sounds.push(request);
                        Response::Done
                    }
                    Request::Invalidate
                    | Request::CarriedObject
                    | Request::OrdinaryCarriedObject => Response::Done,
                    Request::Viewport => Response::Viewport([0, 0, 400, 400]),
                    // The portrait host supplies visibility and an empty carried
                    // slot. RootWalking additionally executes native correction.
                    Request::Visibility { .. } => Response::Visible(true),
                    Request::RootMotionCorrection => {
                        let before = character.placement.base;
                        character
                            .correct_root(sims_sim::pose::root_motion::Source {
                                enabled: root_entry,
                                movement_status: movement.state.status,
                                excluded_person: false,
                                world_word_30: 760,
                                movement_mode: prefix.state.mode,
                            })
                            .map_err(anyhow::Error::msg)?;
                        root_corrections += usize::from(before != character.placement.base);
                        continue;
                    }
                    Request::ModelBounds => {
                        published_bounds = geometry_bounds;
                        Response::Done
                    }
                    Request::Placement => {
                        use sims_sim::pose::transform::{Source, Terrain, View};
                        let terrain = vec![[0; 4]; 64 * 64];
                        character
                            .place(
                                Source {
                                    position: physical_position,
                                    facing,
                                    level: 1,
                                    flags: [person[&84] as u8, person[&68] as u8],
                                },
                                View {
                                    rotation: 0,
                                    floor_vector: [0.; 3],
                                },
                                Terrain {
                                    size: 64,
                                    corners: &terrain,
                                },
                            )
                            .map_err(anyhow::Error::msg)?;
                        continue;
                    }
                    Request::HeadTarget { object: 2 } => {
                        Response::HeadTarget(Some(sims_sim::head_look::Target {
                            level: 1,
                            point: Some([20., 0., 20.]),
                        }))
                    }
                    Request::Geometry { .. } | Request::ModelGeometry => {
                        // This host really skins the loaded appearance for each
                        // base, temporary-head and restored hierarchy request.
                        let matrices = pose::from_absolute_bones(&character.hierarchy.absolute)?;
                        let mut lo = Vec3::splat(f32::INFINITY);
                        let mut hi = Vec3::splat(f32::NEG_INFINITY);
                        for part in &avatar.parts {
                            for (point, _) in pose::skin(&part.mesh, &avatar.skeleton, &matrices)? {
                                lo = lo.min(point);
                                hi = hi.max(point);
                            }
                        }
                        geometry_bounds = [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z];
                        Response::Done
                    }
                    other => anyhow::bail!("unexpected diagnostic property request {other:?}"),
                };
                character
                    .acknowledge(response)
                    .map_err(anyhow::Error::msg)?;
            }
            if locomotion {
                movement_frames.push(serde_json::json!({"tick":tick,"mode":prefix.state.mode,"movement":movement,"placement":character.placement,"physical_position":physical_position,"facing":facing}));
            }
            let completed = walking && prefix.state.mode == 3 && movement.state.status == 2;
            let frame_ticks = if walking {
                [
                    0,
                    if row == 0 { 12 } else { 6 },
                    if row == 0 { 30 } else { 12 },
                    if completed { tick } else { -1 },
                ]
            } else if turning {
                [0, 10, 20, 30]
            } else {
                [9, 10, 12, 15]
            };
            if let Some(column) = frame_ticks.iter().position(|t| *t == tick) {
                let display = |character: &Character| -> Result<Vec<Mat4>> {
                    let absolute = character.displayed_pose().map_err(anyhow::Error::msg)?;
                    if locomotion {
                        let mut matrix = character.matrix;
                        // Recenter for the portrait, preserving native rotation.
                        matrix[12..15].fill(0.);
                        pose::from_placed_absolute_bones(
                            &absolute,
                            sims_sim::pose::model::Matrix {
                                values: matrix,
                                kind: 0,
                            },
                        )
                    } else {
                        pose::from_absolute_bones(&absolute)
                    }
                };
                let matrices = display(&character)?;
                let pixels = software::render(&scene_with_zoom(avatar, &matrices, 0, 1.5)?);
                let restored: Character = serde_json::from_slice(&serde_json::to_vec(&character)?)?;
                ensure!(character == restored, "animator snapshot differs");
                let restored_pose = display(&restored)?;
                ensure!(
                    pixels == software::render(&scene_with_zoom(avatar, &restored_pose, 0, 1.5)?),
                    "animator restored pixels differ"
                );
                image::imageops::replace(
                    &mut gallery,
                    &pixels,
                    column as i64 * 400,
                    row as i64 * 400,
                );
            }
            if completed {
                endpoint_tick = Some(tick);
                break;
            }
        }
        std::fs::write(
            format!("cache/head-look-frames/{output}-transition-sounds-{row}.json"),
            serde_json::to_vec_pretty(&sounds)?,
        )?;
        if ordinary {
            std::fs::write(
                format!("cache/head-look-frames/{output}-transition-selections-{row}.json"),
                serde_json::to_vec_pretty(&selections)?,
            )?;
        }
        if locomotion {
            std::fs::write(
                format!("cache/head-look-frames/{output}-states-{row}.json"),
                serde_json::to_vec_pretty(&movement_frames)?,
            )?;
        }
        if walking {
            ensure!(
                !rooted || root_corrections > 0,
                "root profile never corrected placement"
            );
            ensure!(
                endpoint_tick.is_some() && physical_steps > 0,
                "path did not complete: row {row}, mode {}, state {:?}",
                prefix.state.mode,
                movement.state
            );
            ensure!(
                character.placement.base == character.placement.previous,
                "path endpoint mismatch"
            );
            ensure!(
                character.placement.base == [224., 352., 2.],
                "unexpected path endpoint {:?}",
                character.placement.base
            );
            println!(
                "Original {output} profile {row}: {} physical steps, {root_corrections} root corrections, settled at tick {}",
                physical_steps,
                endpoint_tick.unwrap()
            );
        } else if turning {
            ensure!(
                prefix.state.mode == 3 && movement.state.status == 0,
                "turn did not settle"
            );
            ensure!(
                character.placement.base == character.placement.previous,
                "turn endpoint mismatch"
            );
        }
        if ordinary {
            ensure!(
                published_bounds.iter().all(|v| v.is_finite()),
                "invalid published portrait bounds"
            );
        }
        if row == 0 && !ordinary {
            model_scale_frames(avatar, &character)?;
        }
    }
    gallery.save(format!(
        "cache/head-look-frames/{output}-transitions-gallery.png"
    ))?;
    Ok(())
}

/// Display a completed pose under explicit native model scales, centered at the
/// portrait origin. This exercises publication/handedness/skinning, not a lot.
fn model_scale_frames(
    avatar: &Avatar,
    character: &sims_sim::pose::refresh::Character,
) -> Result<()> {
    use sims_sim::pose::model::Matrix;
    let mut gallery = image::RgbaImage::new(1200, 400);
    for (column, scale) in [0.75, 1., 1.25].into_iter().enumerate() {
        let mut model = character.model.clone();
        model.set_scale([scale; 3]).map_err(anyhow::Error::msg)?;
        let effective = model
            .publish(Matrix::IDENTITY)
            .map_err(anyhow::Error::msg)?;
        let absolute = character.displayed_pose().map_err(anyhow::Error::msg)?;
        let bones = pose::from_placed_absolute_bones(&absolute, effective)?;
        let frame = software::render(&scene_with_zoom(avatar, &bones, 0, 1.1)?);
        let restored: sims_sim::pose::model::Model =
            serde_json::from_slice(&serde_json::to_vec(&model)?)?;
        let restored_pose = pose::from_placed_absolute_bones(&absolute, restored.bounds_transform)?;
        ensure!(
            frame == software::render(&scene_with_zoom(avatar, &restored_pose, 0, 1.1)?),
            "restored model placement pixels differ"
        );
        image::imageops::replace(&mut gallery, &frame, column as i64 * 400, 0);
    }
    gallery.save("cache/head-look-frames/model-scale-gallery.png")?;
    Ok(())
}

fn owned_saved_pose(
    index: &Index,
    program: &mut sims_vm::Program,
    avatar: &Avatar,
    saved: &sims_formats::saved_objects::Person,
) -> Result<Vec<Mat4>> {
    use sims_sim::pose::{
        LocalBone,
        animator::{Animator, Settings},
        sampling::BoneInput,
    };
    let mut animator = Animator::new(123456, 0, 0, true, -1);
    sims_content::saved_layers::restore(
        index,
        program,
        &avatar.skeleton,
        saved,
        &mut animator,
        123456,
        Settings {
            fade_enabled: true,
            fade_duration: 200,
            sampling_config: 1,
        },
    )?;
    let mut bones = avatar
        .skeleton
        .bones
        .iter()
        .map(|b| BoneInput {
            local: LocalBone {
                translation: b.translation,
                rotation: b.rotation,
            },
            priority: -10000,
            blend_marker: b.flags[2],
        })
        .collect::<Vec<_>>();
    animator
        .begin(123456, true, false, false, &mut bones)
        .map_err(anyhow::Error::msg)?;
    (animator, bones) = serde_json::from_slice(&serde_json::to_vec(&(&animator, &bones))?)?;
    ensure!(
        animator
            .next(program, &mut bones)
            .map_err(anyhow::Error::msg)?
            .is_none(),
        "callback disabled"
    );
    pose::from_local_bones(
        &avatar.skeleton,
        &bones.iter().map(|b| b.local).collect::<Vec<_>>(),
    )
}
/// Render the resource/calibration integration fixture. Its appearance rebuild,
/// geometry and visibility services are supplied; this is not live activation.
fn initializer_calibration_frames(index: &Index) -> Result<()> {
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(
        "cache/original-character-initialization.json",
    )?)?;
    ensure!(rows.len() == 7, "initializer resident coverage changed");
    let mut gallery = image::RgbaImage::new(1600, 2800);
    for (i, row) in rows.iter().enumerate() {
        let house = House::load(index, row["source"].as_str().context("initializer source")?)?;
        let object = row["object"].as_u64().context("initializer object")? as u16;
        let saved = house.objects.instances[&object]
            .person
            .as_ref()
            .context("initializer saved appearance")?;
        let initialized: sims_sim::pose::initialize::Initialization =
            serde_json::from_value(row["initialization"].clone())?;
        ensure!(
            initialized.is_complete(),
            "initializer controller unfinished"
        );
        let avatar = sims_content::saved_avatar::load(index, &initialized.source.name, saved)?;
        ensure!(
            avatar.skeleton == initialized.source,
            "initializer skeleton differs from saved appearance rig"
        );
        let restored: sims_sim::pose::initialize::Initialization =
            serde_json::from_slice(&serde_json::to_vec(&initialized)?)?;
        let matrices = |e: &sims_sim::pose::initialize::Initialization| {
            pose::from_local_bones(
                &e.source,
                &e.bound
                    .skeleton
                    .skin
                    .bones
                    .iter()
                    .map(|b| b.local)
                    .collect::<Vec<_>>(),
            )
        };
        let original_pose = matrices(&initialized)?;
        let restored_pose = matrices(&restored)?;
        for rotation in 0..4 {
            let pixels = software::render(&scene(&avatar, &original_pose, rotation)?);
            ensure!(
                pixels == software::render(&scene(&avatar, &restored_pose, rotation)?),
                "initializer restoration changed pixels"
            );
            image::imageops::replace(
                &mut gallery,
                &pixels,
                i64::from(rotation) * 400,
                i as i64 * 400,
            );
        }
    }
    gallery.save("cache/head-look-frames/initializer-calibration-gallery.png")?;
    println!(
        "Checked 7 original initializer calibration poses in 4 views, with identical restored pixels; bare-model service fixture, not household activation"
    );
    Ok(())
}

/// Reconstruct the freshly selected profile assets and render the existing
/// initializer calibration pose. Tint notifications are recorded by the native
/// selector comparison; this checks mesh/texture selection and record reload.
fn profile_appearance_frames(index: &Index, reload: bool) -> Result<()> {
    use sims_content::saved_avatar::BoundAvatar;
    use sims_sim::pose::appearance::{Execution, Input, Records};
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(if reload {
        "cache/original-appearance-reload.json"
    } else {
        "cache/original-appearance-selection.json"
    })?)?;
    let initialized: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(
        "cache/original-character-initialization.json",
    )?)?;
    ensure!(rows.len() == 7, "profile resident coverage changed");
    let mut gallery = image::RgbaImage::new(1600, 2800);
    for (i, row) in rows.iter().enumerate() {
        let calibration = initialized
            .iter()
            .find(|c| c["source"] == row["source"] && c["object"] == row["object"])
            .context("resident calibration absent")?;
        let calibration: sims_sim::pose::initialize::Initialization =
            serde_json::from_value(calibration["initialization"].clone())?;
        let records: Records = serde_json::from_value(row["records"].clone())?;
        let load = |mut records: Records| -> Result<BoundAvatar> {
            for r in &mut records.entries {
                r.binding = None;
            }
            let mut avatar = BoundAvatar::empty(
                index,
                &calibration.source.name,
                row["object"].as_u64().context("resident ID")?,
            )?;
            avatar.records = records;
            for e in row["events"].as_array().context("profile events")? {
                if let Some(record) = e["bind"]["record"].as_u64() {
                    avatar.apply_record(index, record as usize, Execution::bind(Input::Current))?;
                }
            }
            Ok(avatar)
        };
        let (bound, restored) = if reload {
            let state: sims_content::saved_avatar::BindingState =
                serde_json::from_value(row["asset_state"].clone())?;
            let restored_state = serde_json::from_slice(&serde_json::to_vec(&state)?)?;
            let id = row["object"].as_u64().context("resident ID")?;
            let bound = BoundAvatar::restore(index, &calibration.source.name, id, &state)?;
            let restored =
                BoundAvatar::restore(index, &calibration.source.name, id, &restored_state)?;
            ensure!(bound.snapshot() == state && restored.snapshot() == state);
            (bound, restored)
        } else {
            let bound = load(records.clone())?;
            let restored_records: Records = serde_json::from_slice(&serde_json::to_vec(&records)?)?;
            (bound, load(restored_records)?)
        };
        ensure!(bound.avatar.skeleton == calibration.source);
        ensure!(bound.records == records && restored.records == records);
        let matrices = pose::from_local_bones(
            &calibration.source,
            &calibration
                .bound
                .skeleton
                .skin
                .bones
                .iter()
                .map(|b| b.local)
                .collect::<Vec<_>>(),
        )?;
        for rotation in 0..4 {
            let pixels = software::render(&scene(&bound.avatar, &matrices, rotation)?);
            ensure!(
                pixels == software::render(&scene(&restored.avatar, &matrices, rotation)?),
                "restored appearance records changed pixels"
            );
            image::imageops::replace(
                &mut gallery,
                &pixels,
                i64::from(rotation) * 400,
                i as i64 * 400,
            );
        }
    }
    gallery.save(if reload {
        "cache/head-look-frames/appearance-reload-gallery.png"
    } else {
        "cache/head-look-frames/profile-appearance-gallery.png"
    })?;
    println!(
        "Checked 7 freshly selected original resident appearances in 4 views, identical pixels after record reload; geometry refresh and household activation remain separate"
    );
    Ok(())
}

fn populated_initializer_frames(index: &Index, kind: &str) -> Result<()> {
    use sims_content::{
        character_geometry::{CharacterGeometry, MeshPool},
        saved_avatar::{BindingState, BoundAvatar},
    };
    use sims_sim::pose::initialize::Initialization;
    let groups = if kind == "lifecycle" {
        let reports: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(
            "cache/original-appearance-geometry-lifecycle.json",
        )?)?;
        let mut groups = vec![];
        for phase in ["replace", "clear", "rebind", "reload"] {
            let mut rows = vec![];
            for report in &reports {
                let mut row = report["phases"]
                    .as_array()
                    .context("lifecycle phases")?
                    .iter()
                    .find(|p| p["phase"] == phase)
                    .context("lifecycle phase missing")?
                    .clone();
                row["object"] = report["object"].clone();
                rows.push(row);
            }
            groups.push((format!("appearance-geometry-{phase}"), rows));
        }
        groups
    } else if kind == "saved" {
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(
            "cache/original-saved-appearance-prefix.json",
        )?)?;
        vec![("saved-appearance-prefix".to_owned(), rows)]
    } else {
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(
            "cache/original-character-populated-initialization.json",
        )?)?;
        vec![("populated-initializer".to_owned(), rows)]
    };
    for (name, rows) in groups {
        ensure!(
            rows.len() == 7,
            "populated initializer resident coverage changed"
        );
        let mut gallery = image::RgbaImage::new(1600, 2800);
        for (i, row) in rows.iter().enumerate() {
            let initialized: Initialization =
                serde_json::from_value(row["initialization"].clone())?;
            let geometry: CharacterGeometry = serde_json::from_value(row["geometry"].clone())?;
            let pool: MeshPool = serde_json::from_value(row["mesh_pool"].clone())?;
            let state: BindingState = serde_json::from_value(row["asset_state"].clone())?;
            ensure!(initialized.is_complete());
            let restored: (Initialization, CharacterGeometry, MeshPool, BindingState) =
                serde_json::from_slice(&serde_json::to_vec(&(
                    &initialized,
                    &geometry,
                    &pool,
                    &state,
                ))?)?;
            ensure!(
                (&initialized, &geometry, &pool, &state)
                    == (&restored.0, &restored.1, &restored.2, &restored.3)
            );
            let id = row["object"].as_u64().context("resident ID")?;
            let bound = BoundAvatar::restore(index, &initialized.source.name, id, &state)?;
            let resumed = BoundAvatar::restore(index, &restored.0.source.name, id, &restored.3)?;
            let saved_assets = if kind == "saved" {
                let house = House::load(
                    index,
                    row["source"].as_str().context("saved appearance house")?,
                )?;
                let person = house.objects.instances[&(id as u16)]
                    .person
                    .as_ref()
                    .context("saved appearance person")?;
                Some(sims_content::saved_avatar::load(
                    index,
                    &initialized.source.name,
                    person,
                )?)
            } else {
                None
            };
            ensure!(
                geometry
                    .state
                    .appearances
                    .iter()
                    .map(Vec::len)
                    .sum::<usize>()
                    == bound.avatar.parts.len()
            );
            let matrices =
                pose::from_absolute_bones(&initialized.bound.skeleton.hierarchy.absolute)?;
            let restored_matrices =
                pose::from_absolute_bones(&restored.0.bound.skeleton.hierarchy.absolute)?;
            for rotation in 0..4 {
                let pixels = software::render(&scene(&bound.avatar, &matrices, rotation)?);
                if let Some(saved_assets) = &saved_assets {
                    ensure!(
                        pixels == software::render(&scene(saved_assets, &matrices, rotation)?),
                        "saved-reader appearance differs from original saved asset descriptors"
                    );
                }
                ensure!(
                    pixels
                        == software::render(&scene(&resumed.avatar, &restored_matrices, rotation)?),
                    "populated initializer restore changed pixels"
                );
                image::imageops::replace(
                    &mut gallery,
                    &pixels,
                    i64::from(rotation) * 400,
                    i as i64 * 400,
                );
            }
        }
        gallery.save(format!("cache/head-look-frames/{name}-gallery.png"))?;
        println!("Checked {name}: 7 residents in 4 views, identical restored pixels.");
    }
    println!(
        "Checked appearance assets and geometry snapshots with identical restored pixels. Household activation and native vertex skinning remain separate."
    );
    Ok(())
}

/// Draw completed CPU vertex buffers without re-skinning them in the renderer.
/// These retain the initializer pose; saved-animation tail activation is separate.
fn loaded_vertex_frames(index: &Index) -> Result<()> {
    use serde_json::Value;
    use sims_content::{
        character_geometry::CharacterGeometry,
        saved_avatar::{BindingState, BoundAvatar},
    };
    use sims_sim::pose::initialize::Initialization;
    fn parts(avatar: &BoundAvatar, geometry: &CharacterGeometry) -> Result<Vec<Vec<(Vec3, Vec3)>>> {
        (0..avatar.avatar.parts.len())
            .map(|part| {
                let (&identity, range) = avatar
                    .part_ranges
                    .iter()
                    .find(|(_, range)| range.contains(&part))
                    .context("published appearance")?;
                let binding =
                    geometry.state.appearances[geometry.appearances[&identity]][part - range.start];
                Ok(geometry
                    .vertices
                    .get(&binding)
                    .context("published vertices")?
                    .vertices
                    .iter()
                    .map(|v| {
                        (
                            Vec3::new(-v.position[0], v.position[1], v.position[2]),
                            Vec3::new(-v.normal[0], v.normal[1], v.normal[2]),
                        )
                    })
                    .collect())
            })
            .collect()
    }
    let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(
        "cache/original-saved-appearance-prefix.json",
    )?)?;
    let report: Value = serde_json::from_slice(&std::fs::read(
        "cache/native-original-saved-appearance-vertices.json",
    )?)?;
    let cases = report["cases"].as_array().context("native vertex cases")?;
    ensure!(rows.len() == 7 && cases.len() == 58);
    let mut gallery = image::RgbaImage::new(1600, 2800);
    let mut mesh_count = 0;
    for (i, row) in rows.iter().enumerate() {
        let initialized: Initialization = serde_json::from_value(row["initialization"].clone())?;
        let state: BindingState = serde_json::from_value(row["asset_state"].clone())?;
        let avatar = BoundAvatar::restore(
            index,
            &initialized.source.name,
            row["object"].as_u64().context("vertex object")?,
            &state,
        )?;
        let mut geometry: CharacterGeometry = serde_json::from_value(row["geometry"].clone())?;
        let notifications = geometry.update_loaded_vertices(&avatar, &initialized.bound, false)?;
        ensure!(
            notifications
                == geometry
                    .state
                    .appearances
                    .iter()
                    .flatten()
                    .copied()
                    .collect::<Vec<_>>()
        );
        let restored: CharacterGeometry = serde_json::from_slice(&serde_json::to_vec(&geometry)?)?;
        let rust = parts(&avatar, &geometry)?;
        let resumed = parts(&avatar, &restored)?;
        let mut native = vec![];
        for (part, asset) in avatar.avatar.parts.iter().enumerate() {
            let sample = cases
                .iter()
                .find(|v| {
                    v["name"] == row["name"]
                        && v["object"] == row["object"]
                        && v["part"] == part
                        && v["input"]["rest"] == false
                })
                .context("original vertex sample")?;
            let vertices: Vec<[f32; 8]> =
                serde_json::from_value(sample["original"]["vertices"].clone())?;
            ensure!(vertices.len() == asset.mesh.vertices.len() + asset.mesh.blends.len());
            for (v, source) in vertices.iter().zip(&asset.mesh.vertices) {
                ensure!(v[6..] == source.uv, "native vertex UV changed");
            }
            native.push(
                vertices
                    .iter()
                    .map(|v| (Vec3::new(-v[0], v[1], v[2]), Vec3::new(-v[3], v[4], v[5])))
                    .collect(),
            );
            mesh_count += 1;
        }
        ensure!(rust == native && resumed == rust);
        for rotation in 0..4 {
            let draw = |vertices: &[Vec<(Vec3, Vec3)>]| -> Result<_> {
                Ok(software::render(&scene_with_vertices(
                    &avatar.avatar,
                    &[],
                    rotation,
                    2.,
                    Some(vertices),
                )?))
            };
            let pixels = draw(&rust)?;
            ensure!(
                pixels == draw(&native)? && pixels == draw(&resumed)?,
                "native published vertex pixels differ"
            );
            image::imageops::replace(
                &mut gallery,
                &pixels,
                i64::from(rotation) * 400,
                i as i64 * 400,
            );
        }
    }
    ensure!(mesh_count == 29);
    gallery.save("cache/head-look-frames/loaded-vertices-gallery.png")?;
    println!(
        "Checked 29 original meshes in 28 resident/view combinations: native, Rust and restored published vertices produce identical pixels."
    );
    Ok(())
}

/// Inspect the vertices retained by the completed saved-animation tail,
/// including its temporary head rotation, before post-load Reset.
fn saved_tail_vertex_frames(index: &Index) -> Result<()> {
    use serde_json::Value;
    use sims_content::{
        character_geometry::CharacterGeometry,
        saved_avatar::{BindingState, BoundAvatar},
    };
    use sims_sim::pose::initialize::Initialization;
    let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(
        "cache/original-initialized-saved-animation-tails.json",
    )?)?;
    let prefixes: Vec<Value> = serde_json::from_slice(&std::fs::read(
        "cache/original-saved-appearance-prefix.json",
    )?)?;
    ensure!(rows.len() == 7);
    let mut gallery = image::RgbaImage::new(1600, 2800);
    for (i, row) in rows.iter().enumerate() {
        let prefix = prefixes
            .iter()
            .find(|p| p["source"] == row["source"] && p["object"] == row["object"])
            .context("saved tail source initializer")?;
        let initialized: Initialization = serde_json::from_value(prefix["initialization"].clone())?;
        let state: BindingState = serde_json::from_value(row["asset_state"].clone())?;
        let avatar = BoundAvatar::restore(
            index,
            &initialized.source.name,
            row["object"].as_u64().context("saved tail object")?,
            &state,
        )?;
        let geometry: CharacterGeometry = serde_json::from_value(row["geometry"].clone())?;
        let restored: CharacterGeometry = serde_json::from_slice(&serde_json::to_vec(&geometry)?)?;
        let parts = |g: &CharacterGeometry| -> Result<Vec<Vec<(Vec3, Vec3)>>> {
            (0..avatar.avatar.parts.len())
                .map(|part| {
                    let (&identity, range) = avatar
                        .part_ranges
                        .iter()
                        .find(|(_, r)| r.contains(&part))
                        .context("saved tail appearance")?;
                    let binding = g.state.appearances[g.appearances[&identity]][part - range.start];
                    Ok(g.vertices
                        .get(&binding)
                        .context("saved tail vertices")?
                        .vertices
                        .iter()
                        .map(|v| {
                            (
                                Vec3::new(-v.position[0], v.position[1], v.position[2]),
                                Vec3::new(-v.normal[0], v.normal[1], v.normal[2]),
                            )
                        })
                        .collect())
                })
                .collect()
        };
        let vertices = parts(&geometry)?;
        let resumed = parts(&restored)?;
        for rotation in 0..4 {
            let draw = |v: &[Vec<(Vec3, Vec3)>]| -> Result<_> {
                Ok(software::render(&scene_with_vertices(
                    &avatar.avatar,
                    &[],
                    rotation,
                    2.,
                    Some(v),
                )?))
            };
            let pixels = draw(&vertices)?;
            ensure!(
                pixels == draw(&resumed)?,
                "saved-animation tail restore changed pixels"
            );
            image::imageops::replace(
                &mut gallery,
                &pixels,
                i64::from(rotation) * 400,
                i as i64 * 400,
            );
        }
    }
    gallery.save("cache/head-look-frames/saved-animation-tail-gallery.png")?;
    println!(
        "Checked 28 saved-animation-tail views with populated original meshes and identical restored pixels. Post-load Reset and household scheduling remain separate."
    );
    Ok(())
}

/// Characters produced by the complete original-file read and ordered Reset,
/// without requiring any intermediate initializer/prefix reports as inputs.
fn house_character_frames(
    index: &Index,
    live: bool,
    inferred: bool,
    experiment: Option<&str>,
) -> Result<()> {
    use sims_content::{
        character_geometry::{CharacterGeometry, MeshPool},
        character_model,
        saved_avatar::{BindingState, BoundAvatar},
    };
    use sims_sim::{character::Resident, pose::appearance_reload::Owner};
    let experiment_directory = if let Some(label) = experiment {
        ensure!(
            live && inferred
                && !label.is_empty()
                && label.len() <= 64
                && label
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'),
            "invalid neighborhood experiment label"
        );
        let directory = std::path::Path::new("cache/neighborhood-input-experiments").join(label);
        let report: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(
            directory.join("original-live-character-prefix-inferred.json"),
        )?)?;
        ensure!(
            !report.is_empty()
                && report
                    .iter()
                    .all(|r| r["neighborhood_input_experiment"]["label"] == label),
            "missing/mismatched counterfactual input provenance"
        );
        println!(
            "Rendering counterfactual neighborhood experiment {label}; not verified original-house progress."
        );
        Some(directory)
    } else {
        None
    };
    let input = if let Some(directory) = &experiment_directory {
        directory.join("original-live-character-frames-inferred.json")
    } else if inferred {
        "cache/original-live-character-frames-inferred.json".into()
    } else if live {
        "cache/original-live-character-frames.json".into()
    } else {
        "cache/original-house-character-post-load.json".into()
    };
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(input)?)?;
    ensure!(if live {
        !rows.is_empty()
    } else {
        rows.len() == 7
    });
    let mut gallery = image::RgbaImage::new(1600, rows.len() as u32 * 400);
    for (i, row) in rows.iter().enumerate() {
        let resident: Resident = serde_json::from_value(row["resident"].clone())?;
        let model: character_model::State = serde_json::from_value(row["model"].clone())?;
        let geometry: CharacterGeometry = serde_json::from_value(row["geometry"].clone())?;
        let pool: MeshPool = serde_json::from_value(row["mesh_pool"].clone())?;
        let state: BindingState = serde_json::from_value(row["asset_state"].clone())?;
        let owner: Owner = serde_json::from_value(row["owner"].clone())?;
        let restored: (
            Resident,
            character_model::State,
            CharacterGeometry,
            MeshPool,
            BindingState,
            Owner,
        ) = serde_json::from_slice(&serde_json::to_vec(&(
            &resident, &model, &geometry, &pool, &state, &owner,
        ))?)?;
        ensure!(
            (&resident, &model, &geometry, &pool, &state, &owner)
                == (
                    &restored.0,
                    &restored.1,
                    &restored.2,
                    &restored.3,
                    &restored.4,
                    &restored.5
                )
        );
        let id = row["object"].as_u64().context("post-load object")?;
        let skeleton = row["skeleton"].as_str().context("post-load skeleton")?;
        let avatar = BoundAvatar::restore(index, skeleton, id, &state)?;
        let resumed = BoundAvatar::restore(index, skeleton, id, &restored.4)?;
        let pose = pose::from_absolute_bones(&resident.character.hierarchy.absolute)?;
        let restored_pose = pose::from_absolute_bones(&restored.0.character.hierarchy.absolute)?;
        for rotation in 0..4 {
            let pixels = software::render(&scene(&avatar.avatar, &pose, rotation)?);
            ensure!(
                pixels == software::render(&scene(&resumed.avatar, &restored_pose, rotation)?),
                "original-file post-load restore changed pixels"
            );
            if row["phase"] == "native censor"
                && let Some(directory) = &experiment_directory
            {
                pixels.save(directory.join(format!("censor-{id}-{rotation}.png")))?;
            }
            if row["phase"] == "completed live person Reset"
                && let Some(directory) = &experiment_directory
            {
                pixels.save(directory.join(format!("reset-{id}-{rotation}.png")))?;
            }
            image::imageops::replace(
                &mut gallery,
                &pixels,
                i64::from(rotation) * 400,
                i as i64 * 400,
            );
        }
    }
    if live {
        let suffix = if inferred { "-inferred" } else { "" };
        let output = experiment_directory
            .map(|d| d.join("live-character-gallery.png"))
            .unwrap_or_else(|| {
                format!("cache/head-look-frames/live-character-prefix{suffix}-gallery.png").into()
            });
        gallery.save(output)?;
        println!(
            "Checked {} original-file live-character views with identical restored pixels. These completed poses do not establish a complete household run or rendered native room lighting.",
            rows.len() * 4
        );
    } else {
        gallery.save("cache/head-look-frames/post-load-character-gallery.png")?;
        println!(
            "Checked 28 original-file post-load character views with identical restored pixels. Ordinary household scheduling remains separate."
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    std::fs::create_dir_all("cache/head-look-frames")?;
    let args: Vec<_> = std::env::args().collect();
    if let Some(position) = args.iter().position(|a| a == "--live-character-experiment") {
        let label = args
            .get(position + 1)
            .context("missing neighborhood experiment label")?;
        return house_character_frames(&index, true, true, Some(label));
    }
    if std::env::args().any(|a| a == "--house-characters-only") {
        return house_character_frames(&index, false, false, None);
    }
    if std::env::args().any(|a| a == "--live-characters-only") {
        return house_character_frames(&index, true, false, None);
    }
    if std::env::args().any(|a| a == "--live-characters-inferred") {
        return house_character_frames(&index, true, true, None);
    }
    if std::env::args().any(|a| a == "--saved-tail-only") {
        return saved_tail_vertex_frames(&index);
    }
    if std::env::args().any(|a| a == "--loaded-vertices-only") {
        return loaded_vertex_frames(&index);
    }
    if std::env::args().any(|a| a == "--populated-initializer-only") {
        return populated_initializer_frames(&index, "initializer");
    }
    if std::env::args().any(|a| a == "--appearance-lifecycle-only") {
        return populated_initializer_frames(&index, "lifecycle");
    }
    if std::env::args().any(|a| a == "--saved-appearance-only") {
        return populated_initializer_frames(&index, "saved");
    }
    if std::env::args().any(|a| a == "--reload-only") {
        return profile_appearance_frames(&index, true);
    }
    if std::env::args().any(|a| a == "--appearance-only") {
        return profile_appearance_frames(&index, false);
    }
    if std::env::args().any(|a| a == "--initializer-only") {
        return initializer_calibration_frames(&index);
    }
    let mut program = sims_vm::Program::default();
    let mut report = vec![];
    let mut gallery = image::RgbaImage::new(1200, 2800);
    let mut animation_gallery = image::RgbaImage::new(1600, 2800);
    let mut saved_layer_gallery = image::RgbaImage::new(1600, 2800);
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = House::load(&index, source)?;
        for r in &house.household.as_ref().context("household")?.residents {
            let saved = house.objects.instances[&r.object]
                .person
                .as_ref()
                .context("saved person")?;
            let bound = sims_content::saved_avatar::load_bound(
                &index,
                &r.body_strings[0],
                saved,
                u64::from(r.object),
            )?;
            let appearance_records = bound.records;
            let restored_records: sims_sim::pose::appearance::Records =
                serde_json::from_slice(&serde_json::to_vec(&appearance_records)?)?;
            ensure!(
                appearance_records == restored_records,
                "appearance identity changed during restore"
            );
            let mut covered_parts = 0;
            for record in &appearance_records.entries {
                if let Some(binding) = record.binding {
                    ensure!(
                        binding.owner == u64::from(r.object),
                        "appearance has wrong owner"
                    );
                    covered_parts += bound.part_ranges[&binding.id].len();
                }
            }
            ensure!(
                covered_parts == bound.avatar.parts.len(),
                "unowned saved appearance geometry"
            );
            let avatar = bound.avatar;
            if report.is_empty() {
                let linked =
                    sims_content::vm::load_program(&index, &r.character_source, "Global.iff")?;
                program.globals = linked.globals;
                program.modules.extend(linked.modules);
                layer_progression(&index, &mut program, &avatar)?;
                for transition in [
                    TransitionProfile::Forced,
                    TransitionProfile::Loaded,
                    TransitionProfile::Ordinary,
                    TransitionProfile::Turning,
                    TransitionProfile::Walking,
                    TransitionProfile::RootWalking,
                ] {
                    animator_transitions(
                        &index,
                        &mut program,
                        &avatar,
                        &r.body_strings,
                        &r.character_source,
                        transition,
                    )?;
                }
            }
            let saved_layers = sims_content::saved_layers::load(
                &index,
                &mut program,
                &avatar.skeleton,
                saved,
                123456,
                1,
            )?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            let restored_layers: Vec<sims_sim::pose::layer::Layer> =
                serde_json::from_slice(&serde_json::to_vec(&saved_layers)?)?;
            let saved_pose = pose::evaluate_layers(&avatar.skeleton, &saved_layers, &program)?;
            let owned_pose = owned_saved_pose(&index, &mut program, &avatar, saved)?;
            let restored_pose =
                pose::evaluate_layers(&avatar.skeleton, &restored_layers, &program)?;
            for rotation in 0..4 {
                let pixels = software::render(&scene(&avatar, &saved_pose, rotation)?);
                ensure!(
                    pixels == software::render(&scene(&avatar, &owned_pose, rotation)?),
                    "owned saved animator disagrees with stored pose"
                );
                ensure!(
                    pixels == software::render(&scene(&avatar, &restored_pose, rotation)?),
                    "saved layer restore changed pixels"
                );
                image::imageops::replace(
                    &mut saved_layer_gallery,
                    &pixels,
                    rotation as i64 * 400,
                    report.len() as i64 * 400,
                );
            }
            let rig = Rig::bind(&avatar.skeleton, &r.body_strings).map_err(anyhow::Error::msg)?;
            ensure!(
                rig.head.is_some() && rig.forward.is_some() && rig.turn_bone.is_some(),
                "original head rig incomplete"
            );
            let state = State::from_saved(saved, 0).map_err(anyhow::Error::msg)?;
            let restored: State = serde_json::from_slice(&serde_json::to_vec(&state)?)?;
            ensure!(
                restored == state,
                "saved head state changed during JSON roundtrip"
            );
            ensure!(
                state.target_point.unwrap().map(f32::to_bits)
                    == [saved.floats[3], saved.floats[4], saved.floats[5]].map(f32::to_bits),
                "saved target point changed"
            );
            let name = saved.animations[2]
                .split(';')
                .next()
                .context("saved base clip")?;
            sims_content::assets::load_named_animations(
                &index,
                &mut program,
                &[name.to_owned()].into(),
            )?;
            let base = pose::evaluate(&avatar.skeleton, &program.animations[name], 0., true)?;
            for (column, fraction) in [0.125, 0.375, 0.625, 0.875].into_iter().enumerate() {
                let frame = fraction * program.animations[name].clip.frames as f32;
                let bones =
                    pose::evaluate(&avatar.skeleton, &program.animations[name], frame, true)?;
                let pixels = software::render(&scene(&avatar, &bones, 0)?);
                image::imageops::replace(
                    &mut animation_gallery,
                    &pixels,
                    column as i64 * 400,
                    report.len() as i64 * 400,
                );
            }
            let before = base.clone();
            for (column, turn) in [-1.5, 0., 1.5].into_iter().enumerate() {
                let bones = pose::head_overlay(&avatar.skeleton, &base, turn)?;
                ensure!(base == before, "head overlay mutated its source");
                for (i, b) in avatar.skeleton.bones.iter().enumerate() {
                    let mut parent = Some(i);
                    let mut is_head = false;
                    while let Some(p) = parent {
                        if Some(p) == rig.turn_bone {
                            is_head = true;
                            break;
                        }
                        parent = avatar.skeleton.bones[p].parent;
                    }
                    if !is_head || turn == 0. {
                        ensure!(bones[i] == base[i], "unrelated bone {} changed", b.name);
                    }
                }
                let frame = software::render(&scene(&avatar, &bones, 0)?);
                image::imageops::replace(
                    &mut gallery,
                    &frame,
                    column as i64 * 400,
                    report.len() as i64 * 400,
                );
            }
            for rotation in 0..4 {
                let original = software::render(&scene(
                    &avatar,
                    &pose::head_overlay(&avatar.skeleton, &base, state.turn)?,
                    rotation,
                )?);
                let resumed = software::render(&scene(
                    &avatar,
                    &pose::head_overlay(&avatar.skeleton, &base, restored.turn)?,
                    rotation,
                )?);
                ensure!(original == resumed, "restored head pixels changed");
                original.save(format!(
                    "cache/head-look-frames/resident-{}-view-{rotation}.png",
                    r.object
                ))?;
            }
            report.push(serde_json::json!({"source":source,"object":r.object,"rig":rig,"saved_head":state,"preview_clip":name,"appearance_records":appearance_records,"part_ranges":bound.part_ranges}));
        }
    }
    gallery.save("cache/head-look-frames/gallery.png")?;
    animation_gallery.save("cache/head-look-frames/sampling-gallery.png")?;
    saved_layer_gallery.save("cache/head-look-frames/saved-layers-gallery.png")?;
    std::fs::write(
        "cache/head-look-frames/report.json",
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "Checked {} original resident rigs, three head poses, four restored camera views each",
        report.len()
    );
    Ok(())
}
