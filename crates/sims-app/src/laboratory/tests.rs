use super::*;
use clap::Parser;
use std::path::PathBuf;
#[test]
fn default_house_uses_the_only_gameplay_scheduler_and_labs_are_explicit() {
    let mut args = Args::try_parse_from(["sims-room", "--inferred-routing", "--paused"]).unwrap();
    args.select_default_house();
    assert_eq!(
        args.original.as_deref(),
        Some("UserData/Houses/House07.iff")
    );
    assert!(Args::try_parse_from(["sims-room", "--native-scheduler"]).is_err());
    let mut lab = Args::try_parse_from(["sims-room", "--laboratory"]).unwrap();
    lab.select_default_house();
    assert!(lab.original.is_none());
    assert!(Args::try_parse_from(["sims-room", "--laboratory", "--buy-preview"]).is_err());
}
#[test]
#[ignore = "requires a headless wgpu adapter, no GUI"]
fn censor_compositor_matches_iced_pixels_inside_an_offset_viewport() {
    use sims_render::{Draw, Texture, Vertex, censor::Region};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let mut pixels = vec![];
    for y in 0..16 {
        for x in 0..16 {
            pixels.extend([x * 15, y * 15, 170, 255]);
        }
    }
    let quad = |left, z| {
        let corners = [
            ([left, 1., z], [0., 0.]),
            ([1., 1., z], [1., 0.]),
            ([1., -1., z], [1., 1.]),
            ([left, -1., z], [0., 1.]),
        ];
        [0, 1, 2, 0, 2, 3].map(|i| Vertex {
            position: corners[i].0,
            uv: corners[i].1,
            tint: [1.; 4],
            depth_scale: 0.,
        })
    };
    let mut vertices = quad(-1., 0.6).to_vec();
    vertices.extend(quad(0., 0.2));
    let mut scene = Scene {
        width: 16,
        height: 16,
        vertices,
        censors: vec![Region {
            rect: [0, 0, 16, 16],
            depth: 0.4,
            seed: 71,
        }],
        textures: Arc::new(vec![
            Texture {
                width: 16,
                height: 16,
                rgba: pixels.into(),
                depth: vec![255; 256].into(),
            },
            Texture {
                width: 1,
                height: 1,
                rgba: vec![30, 80, 120, 255].into(),
                depth: vec![255].into(),
            },
        ]),
        draws: vec![
            Draw {
                vertices: 0..6,
                texture: 0,
                object: Some(1),
            },
            Draw {
                vertices: 6..12,
                texture: 1,
                object: Some(2),
            },
        ],
    };
    let expected = sims_render::software::render(&scene);
    assert_eq!(
        sims_render::software::render(&sims_render::software::composited_scene(&scene)),
        expected
    );
    let path = root.join("cache/censor-iced-headless.png");
    let element = container(
        widget::Shader::new(Viewport(Arc::new(scene.clone())))
            .width(256.)
            .height(256.),
    )
    .padding(iced::Padding {
        top: 47.,
        left: 31.,
        right: 0.,
        bottom: 0.,
    });
    render_headless_element(
        &root.join("game-data/cabinet-extracted"),
        &path,
        element.into(),
    )
    .unwrap();
    let actual = image::open(path).unwrap().into_rgba8();
    for y in 0..16 {
        for x in 0..16 {
            let pixel = actual.get_pixel(31 + x * 16 + 8, 47 + y * 16 + 8);
            for c in 0..4 {
                assert!(
                    pixel[c].abs_diff(expected.get_pixel(x, y)[c]) <= 1,
                    "Iced censor pixel mismatch at {x},{y}, channel {c}"
                );
            }
        }
    }
    scene.censors.clear();
    let untouched = sims_render::software::render(&scene);
    assert_ne!(untouched.get_pixel(1, 1), expected.get_pixel(1, 1));
    for y in 0..16 {
        for x in 8..16 {
            assert_eq!(
                untouched.get_pixel(x, y),
                expected.get_pixel(x, y),
                "foreground was censored"
            );
        }
    }
    assert_eq!(scene.pick([12., 8.]), Some(2));
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn two_floor_controls_four_queues_and_save_pixels() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted");
    let save = std::env::temp_dir().join(format!("sims-two-floor-ui-{}.json", std::process::id()));
    let args = Args::parse_from([
        "sims-room",
        "--two-floor",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let loaded = Loaded::load(&args).unwrap();
    let (mut v, _) = Viewer::new(args);
    let _ = v.update(Message::Loaded(Ok(loaded)));
    assert!(v.error.is_none(), "{:?}", v.error);
    for (resident, chair) in [(1, 2), (13, 3), (32, 2), (33, 3)] {
        let _ = v.update(Message::Resident(resident));
        assert_eq!(v.resident, resident);
        let _ = v.update(Message::Queue(chair, 0));
    }
    for resident in [1, 13, 32, 33] {
        assert!(
            !v.sim
                .as_ref()
                .unwrap()
                .pending_interactions(resident)
                .unwrap()
                .is_empty()
        );
    }
    let before = v.sim.clone();
    let _ = v.update(Message::Floor(2));
    assert_eq!(v.floor, 2);
    assert_eq!(before, v.sim, "changing floors must not advance gameplay");
    let upstairs = sims_render::software::render(v.scene.as_ref().unwrap());
    let _ = v.update(Message::Floor(1));
    assert_ne!(
        upstairs,
        sims_render::software::render(v.scene.as_ref().unwrap())
    );
    for _ in 0..180 {
        v.tick().unwrap();
    }
    let _ = v.update(Message::Floor(2));
    let state = v.sim.clone();
    let frame = sims_render::software::render(v.scene.as_ref().unwrap());
    let _ = v.update(Message::Save);
    for _ in 0..30 {
        v.tick().unwrap();
    }
    let _ = v.update(Message::Load);
    assert!(v.error.is_none(), "{:?}", v.error);
    assert_eq!(state, v.sim);
    assert_eq!(
        frame,
        sims_render::software::render(v.scene.as_ref().unwrap())
    );
    std::fs::remove_file(save).unwrap();
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn two_resident_controls_keep_queues_separate_and_restore_pixels() {
    for profile in ["--home", "--domestic"] {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted");
        let save =
            std::env::temp_dir().join(format!("sims-two-home-ui-{}.json", std::process::id()));
        let mut args = Args::parse_from([
            "sims-room",
            profile,
            "--paused",
            "--root",
            root.to_str().unwrap(),
            "--save",
            save.to_str().unwrap(),
        ]);
        args.home |= args.domestic;
        let loaded = Loaded::load(&args).unwrap();
        let (mut v, _) = Viewer::new(args);
        let _ = v.update(Message::Loaded(Ok(loaded)));
        assert!(v.error.is_none(), "{:?}", v.error);
        let _ = v.update(Message::Resident(13));
        assert_eq!(v.resident, 13);
        let _ = v.update(Message::Queue(2, 0));
        let _ = v.update(Message::Queue(3, 0));
        assert_eq!(
            v.sim
                .as_ref()
                .unwrap()
                .pending_interactions(13)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            v.sim
                .as_ref()
                .unwrap()
                .pending_interactions(1)
                .unwrap()
                .len(),
            0
        );
        let cancel = v.sim.as_ref().unwrap().pending_interactions(13).unwrap()[1].id as u64;
        let _ = v.update(Message::Cancel(cancel));
        let _ = v.update(Message::Resident(1));
        let _ = v.update(Message::Queue(3, 0));
        for _ in 0..180 {
            v.tick().unwrap();
        }
        v.refresh();
        assert!(v.error.is_none(), "{:?}", v.error);
        let state = v.sim.clone();
        let frame = sims_render::software::render(v.scene.as_ref().unwrap());
        let _ = v.update(Message::Save);
        for _ in 0..50 {
            v.tick().unwrap();
        }
        let _ = v.update(Message::Load);
        assert!(v.error.is_none(), "{:?}", v.error);
        assert_eq!(state, v.sim);
        assert!(frame == sims_render::software::render(v.scene.as_ref().unwrap()));
        std::fs::remove_file(save).unwrap();
    }
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn household_viewer_menus_queue_and_mid_meal_resume() {
    let mut args = Args::parse_from(["sims-room", "--household", "--paused"]);
    args.root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../game-data/cabinet-extracted")
        .canonicalize()
        .unwrap();
    args.save =
        std::env::temp_dir().join(format!("sims-household-viewer-{}.json", std::process::id()));
    let loaded = Loaded::load(&args).unwrap();
    assert!(
        loaded
            .assets
            .textures
            .iter()
            .all(|t| t.width > 0 && t.height > 0)
    );
    let (mut v, _) = Viewer::new(args);
    let _ = v.update(Message::Loaded(Ok(loaded)));
    assert!(v.error.is_none(), "{:?}", v.error);
    let untouched = v.sim.clone();
    for (target, labels) in [
        (2, vec!["Sit"]),
        (3, vec!["Sit"]),
        (4, vec!["Have A Snack", "Have Lite Meal", "Have A Meal"]),
        (5, vec!["Wash Hands"]),
        (6, vec!["Use"]),
        (7, vec!["Sit", "Nap"]),
        (9, vec!["Take a Shower"]),
        (10, vec!["Read a Book"]),
        (11, vec!["Empty Trash"]),
    ] {
        let choices = v.choices(target).unwrap();
        assert_eq!(
            choices.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            labels
        );
    }
    assert_eq!(v.sim, untouched, "opening menus changed world or RNG");
    let _ = v.update(Message::Select(Some(8)));
    assert_eq!(v.selected, Some(7), "both recliner parts select their head");
    let _ = v.update(Message::Queue(2, 0));
    let _ = v.update(Message::Queue(3, 0));
    let pending = v.sim.as_ref().unwrap().pending_interactions(1).unwrap();
    assert_eq!(pending.len(), 2);
    let id = u64::from(pending[1].id);
    let _ = v.update(Message::Cancel(id));
    assert_eq!(
        v.sim
            .as_ref()
            .unwrap()
            .pending_interactions(1)
            .unwrap()
            .len(),
        1
    );
    let _ = v.update(Message::Restart);
    while v.sim.as_ref().unwrap().vm.tick < 900 {
        v.tick().unwrap();
    }
    v.refresh();
    let saved = v.sim.clone().unwrap();
    let active = saved.interactions[&1].active.as_ref().unwrap();
    let label = interaction_label(
        &v.loaded.as_ref().unwrap().program,
        &saved,
        active.item.target,
        active.item.menu_index,
    );
    assert_eq!(label, "Eat · Food");
    let picture = sims_render::software::render(v.scene.as_ref().unwrap());
    let _ = v.update(Message::Save);
    assert!(v.error.is_none(), "{:?}", v.error);
    while v.sim.as_ref().unwrap().vm.tick < 4400 {
        v.tick().unwrap();
    }
    let continued = v.sim.clone().unwrap();
    let _ = v.update(Message::Load);
    assert!(v.error.is_none(), "{:?}", v.error);
    assert_eq!(v.sim.as_ref().unwrap(), &saved);
    assert_eq!(
        sims_render::software::render(v.scene.as_ref().unwrap()),
        picture
    );
    while v.sim.as_ref().unwrap().vm.tick < 4400 {
        v.tick().unwrap();
    }
    assert_eq!(v.sim.as_ref().unwrap(), &continued);
    // Continue past day two's low-Fun reaction, which the one-day acceptance
    // missed when its original a2o-bored1 asset was absent.
    while v.sim.as_ref().unwrap().vm.tick < 86400 {
        v.tick().unwrap();
    }
    let s = v.sim.as_ref().unwrap();
    assert_eq!(s.household.as_ref().unwrap().clock.elapsed_days, 2);
    assert!(s.vm.sounds.is_empty() && s.household.as_ref().unwrap().clock_events.is_empty());
    v.refresh();
    assert!(v.error.is_none(), "{:?}", v.error);
    std::fs::remove_file(&v.args.save).unwrap();
}
#[test]
fn pointer_mapping_tracks_letterboxing_in_logical_viewport_coordinates() {
    let scene = Scene {
        censors: vec![],
        vertices: vec![],
        draws: vec![],
        textures: Arc::new(vec![]),
        width: 1100,
        height: 760,
    };
    for scale in [0.5, 1., 2.] {
        let bounds = iced::Rectangle {
            x: 31.,
            y: 47.,
            width: 800. * scale,
            height: 600. * scale,
        };
        let middle =
            iced::mouse::Cursor::Available(iced::Point::new(bounds.center_x(), bounds.center_y()));
        let point = scene_cursor(&scene, bounds, middle).unwrap();
        assert!((point[0] - 550.).abs() < 0.001 && (point[1] - 380.).abs() < 0.001);
        let margin =
            iced::mouse::Cursor::Available(iced::Point::new(bounds.center_x(), bounds.y + 1.));
        assert!(scene_cursor(&scene, bounds, margin).is_none());
    }
}
#[test]
fn output_guards_protect_originals_and_atomic_save_replaces_a_complete_file() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("sims-viewer-{stamp}"));
    std::fs::create_dir(&dir).unwrap();
    let root = dir.join("originals");
    std::fs::create_dir(&root).unwrap();
    let out = dir.join("save.json");
    assert!(validate_output(&root, &root.join("new.json")).is_err());
    assert!(validate_output(&root, &out).is_ok());
    #[cfg(unix)]
    {
        let link = dir.join("alias");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        assert!(validate_output(&root, &link.join("new.json")).is_err());
    }
    atomic_save(&out, b"first").unwrap();
    atomic_save(&out, b"second").unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"second");
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn viewer_controls_and_mid_animation_save_load_preserve_scene_and_simulation() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../game-data/cabinet-extracted")
        .canonicalize()
        .unwrap();
    let index = Index::scan(&root).unwrap();
    let assets = Assets::load(&index).unwrap();
    let mut program = load_program(&index, "DiningChairs.iff", "Global.iff").unwrap();
    load_chair_animations(&index, &mut program).unwrap();
    let fixture =
        ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty).unwrap();
    let initial = two_chair_room(&program, fixture, 1).unwrap();
    let loaded = Arc::new(Loaded {
        assets,
        dialog_icons: dialog_view::Icons::load(&index).unwrap(),
        program,
        initial,
    });
    let save =
        std::env::temp_dir().join(format!("sims-viewer-control-{}.json", std::process::id()));
    let args = Args {
        exploration_plan: None,
        exploration_output: None,
        original: None,
        laboratory: true,
        inferred_routing: false,
        neighborhood_observation: None,
        neighborhood_summary: None,
        neighborhood_sample: 0,
        neighborhood_policy: None,

        buy_preview: false,
        root,
        save: save.clone(),
        load: None,
        household: false,
        home: false,
        domestic: false,
        two_floor: false,
        floor: 1,
        resident: 1,
        headless_screenshot: false,
        facing: 1,
        camera: 0,
        paused: true,
        at_tick: 0,
        queue: vec![],
        screenshot: None,
        exit_after_screenshot: false,
    };
    let (mut v, _) = Viewer::new(args);
    let _ = v.update(Message::Loaded(Ok(loaded)));
    assert!(v.error.is_none());
    // Feed actual chair pixels through the shader's pointer handler, including
    // viewport offset and letterboxing, rather than bypassing input with Select.
    let select_chair = |v: &mut Viewer, target| {
        use widget::shader::Program as _;
        let scene = v.scene.as_ref().unwrap();
        let draw = scene
            .draws
            .iter()
            .find(|d| d.object == Some(target))
            .unwrap();
        let vertices = &scene.vertices[draw.vertices.start as usize..draw.vertices.end as usize];
        let pixels: Vec<_> = vertices
            .iter()
            .map(|p| {
                [
                    (p.position[0] + 1.) * scene.width as f32 / 2.,
                    (1. - p.position[1]) * scene.height as f32 / 2.,
                ]
            })
            .collect();
        let min = [0, 1].map(|i| pixels.iter().map(|p| p[i]).fold(f32::INFINITY, f32::min));
        let max = [0, 1].map(|i| {
            pixels
                .iter()
                .map(|p| p[i])
                .fold(f32::NEG_INFINITY, f32::max)
        });
        let point = (0..20)
            .flat_map(|y| {
                (0..20).map(move |x| {
                    [
                        min[0] + (max[0] - min[0]) * (x as f32 + 0.5) / 20.,
                        min[1] + (max[1] - min[1]) * (y as f32 + 0.5) / 20.,
                    ]
                })
            })
            .find(|p| scene.pick(*p) == Some(target))
            .expect("visible original chair pixel");
        let bounds = iced::Rectangle {
            x: 24.,
            y: 150.,
            width: 850.,
            height: 600.,
        };
        let scale = (bounds.width / scene.width as f32).min(bounds.height / scene.height as f32);
        let cursor = iced::mouse::Cursor::Available(iced::Point::new(
            bounds.center_x() + (point[0] - scene.width as f32 / 2.) * scale,
            bounds.center_y() + (point[1] - scene.height as f32 / 2.) * scale,
        ));
        let viewport = Viewport(scene.clone());
        assert_eq!(
            viewport.mouse_interaction(&(), bounds, cursor),
            iced::mouse::Interaction::Pointer
        );
        let action = viewport
            .update(
                &mut (),
                &iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
                bounds,
                cursor,
            )
            .unwrap();
        let (message, _, status) = action.into_inner();
        assert_eq!(status, iced::event::Status::Captured);
        assert!(matches!(message, Some(Message::Select(Some(id))) if id == target));
        let _ = v.update(message.unwrap());
        assert_eq!(v.selected, Some(target));
    };
    select_chair(&mut v, 2);
    let _ = v.update(Message::Queue(2, 0));
    select_chair(&mut v, 3);
    let _ = v.update(Message::Queue(3, 0));
    assert_eq!(
        v.sim
            .as_ref()
            .unwrap()
            .pending_interactions(1)
            .unwrap()
            .len(),
        2 // Original Main is still in its startup Sleep.
    );
    let start = Instant::now();
    let _ = v.update(Message::Frame(start));
    let _ = v.update(Message::Frame(start + std::time::Duration::from_secs(1)));
    assert_eq!(v.sim.as_ref().unwrap().vm.tick, 0);
    let _ = v.update(Message::Step);
    assert_eq!(v.sim.as_ref().unwrap().vm.tick, 1);
    assert_eq!(
        v.sim
            .as_ref()
            .unwrap()
            .pending_interactions(1)
            .unwrap()
            .len(),
        1
    );
    let _ = v.update(Message::Speed(2.));
    let _ = v.update(Message::Toggle);
    let _ = v.update(Message::Frame(start));
    let _ = v.update(Message::Frame(
        start + std::time::Duration::from_millis(101),
    ));
    assert_eq!(v.sim.as_ref().unwrap().vm.tick, 6);
    let _ = v.update(Message::Toggle);
    while v.sim.as_ref().unwrap().vm.tick < 150 {
        v.tick().unwrap();
    }
    v.refresh();
    let saved = v.sim.clone().unwrap();
    let picture = sims_render::software::render(v.scene.as_ref().unwrap());
    let _ = v.update(Message::Save);
    assert!(v.error.is_none(), "{:?}", v.error);
    while v.sim.as_ref().unwrap().vm.tick < 650 {
        v.tick().unwrap();
    }
    let final_state = v.sim.clone().unwrap();
    let _ = v.update(Message::Load);
    assert!(v.error.is_none(), "{:?}", v.error);
    assert!(!v.playing);
    assert_eq!(v.sim.as_ref().unwrap(), &saved);
    assert_eq!(
        sims_render::software::render(v.scene.as_ref().unwrap()),
        picture
    );
    let _ = v.update(Message::Rotate);
    let mut rotated = saved.clone();
    rotated.vm.world.camera_rotation = 1;
    assert_eq!(v.sim.as_ref().unwrap(), &rotated);
    assert_eq!(v.rotation, 1);
    let _ = v.update(Message::Save);
    let _ = v.update(Message::Rotate);
    assert_eq!(v.rotation, 2);
    let _ = v.update(Message::Load);
    assert!(v.error.is_none(), "{:?}", v.error);
    assert_eq!(v.rotation, 1);
    assert_eq!(v.sim.as_ref().unwrap(), &rotated);
    // Return to the saved native camera before comparing the same replay.
    for _ in 0..3 {
        let _ = v.update(Message::Rotate);
    }
    assert_eq!(v.sim.as_ref().unwrap(), &saved);
    while v.sim.as_ref().unwrap().vm.tick < 650 {
        v.tick().unwrap();
    }
    assert_eq!(v.sim.as_ref().unwrap(), &final_state);
    assert_eq!(
        v.sim.as_ref().unwrap().vm.world.objects[&1].container,
        Some((3, 0))
    );
    let id = v.sim.as_ref().unwrap().interactions[&1]
        .active
        .as_ref()
        .unwrap()
        .item
        .id;
    let _ = v.update(Message::Cancel(id));
    assert!(
        v.sim.as_ref().unwrap().interactions[&1]
            .active
            .as_ref()
            .unwrap()
            .cancel_requested
    );
    for _ in 0..200 {
        if v.sim.as_ref().unwrap().interactions[&1]
            .active
            .as_ref()
            .unwrap()
            .phase
            == sims_sim::interaction::Phase::Exiting
        {
            break;
        }
        v.tick().unwrap();
    }
    assert_eq!(
        v.sim.as_ref().unwrap().interactions[&1]
            .active
            .as_ref()
            .unwrap()
            .phase,
        sims_sim::interaction::Phase::Exiting
    );
    let _ = v.update(Message::Restart);
    assert_eq!(v.sim.as_ref().unwrap().vm.tick, 0);
    std::fs::remove_file(save).unwrap();
}
