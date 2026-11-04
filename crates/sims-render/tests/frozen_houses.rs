//! Original furnishings and resident assets after frozen Load/Reset callbacks.
//! This checks presentation without pretending to resume original animator state.
use glam::Vec3;
use sims_content::{Index, saves::House};
use sims_render::{frozen_house::FrozenHouse, software};

#[test]
#[ignore = "requires original homes and appearance assets; headless frozen preview"]
fn original_furnished_houses_and_residents_render_headlessly() {
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&repo.join("game-data/cabinet-extracted")).unwrap();
    let mut sheet = image::RgbaImage::new(2880, 1440);
    let mut rotations = image::RgbaImage::new(1920, 1440);
    for (column, source) in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ]
    .into_iter()
    .enumerate()
    {
        let house = House::load(&index, source).unwrap();
        let mut frozen = house
            .load_frozen(
                &index,
                sims_vm::loading::HouseContext {
                    simulator: sims_vm::loading::SimulatorContext {
                        game_mode: 0,
                        retained_global25: 0,
                        global30: 1,
                    },
                    installed_expansions: 0xff,
                    performance_class: 2,
                },
            )
            .unwrap();
        frozen.initialize_indicators(&index).unwrap();
        let before = frozen.execution.vm.clone();
        let materials = sims_content::surfaces::load_floors(&index, &house).unwrap();
        let walls = sims_content::surfaces::load_walls(&index, &house).unwrap();
        println!(
            "{source}: wall materials {:?}",
            walls.keys().collect::<Vec<_>>()
        );
        let expected_walls: Vec<u16> = match column {
            0 => vec![2, 4, 5, 8, 248],
            1 => vec![2, 6, 16, 25, 26, 250, 251],
            _ => vec![19, 260, 263, 265, 271],
        };
        assert_eq!(walls.keys().copied().collect::<Vec<_>>(), expected_walls);
        for (&id, m) in &walls {
            assert_eq!(id, m.saved_id);
            assert_eq!(
                m.frames[2].each_ref().map(|f| (f.width, f.height)),
                [(64, 271), (64, 272), (128, 240), (16, 232)]
            );
        }
        if column == 2 {
            assert!(
                walls[&263]
                    .source
                    .ends_with("WallsAndFloors\\GiraffeWallCovering.wll")
            );
            assert_eq!(walls[&263].local, 1);
        }
        if column == 0 {
            let mut sheet = image::RgbaImage::new(360, 300);
            for (i, f) in walls.values().next().unwrap().frames[2].iter().enumerate() {
                let frame = image::RgbaImage::from_raw(f.width, f.height, f.rgba.clone()).unwrap();
                image::imageops::replace(&mut sheet, &frame, [0, 70, 140, 275][i], 0);
            }
            sheet
                .save(repo.join("cache/original-wall-frames.png"))
                .unwrap();
        }
        let expected: Vec<u16> = match column {
            0 => vec![3, 6, 8, 9, 10, 11, 12, 14, 24],
            1 => vec![1, 2, 6, 9, 11, 12, 21, 23, 26, 27],
            _ => vec![9, 10, 11, 12, 261, 263, 265, 279, 287],
        };
        assert_eq!(materials.keys().copied().collect::<Vec<_>>(), expected);
        for (&id, m) in &materials {
            assert_eq!(id, m.saved_id);
            assert_eq!(
                m.frames.each_ref().map(|f| (f.width, f.height)),
                [(31, 16), (63, 32), (127, 64)]
            );
        }
        if column == 2 {
            assert!(
                materials[&261]
                    .source
                    .ends_with("WallsAndFloors\\BoomerangRed.flr")
            );
            assert_eq!(materials[&261].local, 1);
        }
        let preview = FrozenHouse::load(&index, &house, &frozen)
            .unwrap_or_else(|e| panic!("{source}: {e:#}"));
        println!(
            "{source}: cached graphic lookup returned no sprite for {:?}",
            preview.missing_graphics
        );
        let lot = frozen.architecture.original.as_ref().unwrap();
        let inside = lot.floors[0]
            .rooms
            .primary
            .iter()
            .enumerate()
            .filter(|(_, r)| lot.floors[0].metrics.get(r).is_some_and(|m| !m.outdoors))
            .map(|(i, _)| ((i / lot.size) as f32, (i % lot.size) as f32))
            .collect::<Vec<_>>();
        let minx = inside.iter().map(|p| p.0).reduce(f32::min).unwrap();
        let maxx = inside.iter().map(|p| p.0).reduce(f32::max).unwrap() + 1.;
        let miny = inside.iter().map(|p| p.1).reduce(f32::min).unwrap();
        let maxy = inside.iter().map(|p| p.1).reduce(f32::max).unwrap() + 1.;
        for level in 1..=2 {
            let mut camera = preview.camera(960, 720, 0, 0.55, level);
            camera.center = Vec3::new((minx + maxx) / 2., camera.center.y, (miny + maxy) / 2.);
            let scene = preview
                .scene(camera)
                .unwrap_or_else(|e| panic!("{source} floor {level}: {e:#}"));
            for resident in &house.household.as_ref().unwrap().residents {
                let id = resident.object as i16;
                if frozen.execution.vm.world.positions[&id].level <= level {
                    assert!(
                        scene.draws.iter().any(|d| d.object == Some(id)),
                        "resident #{id} omitted"
                    );
                }
            }
            let frame = software::render(&scene);
            // Re-rendering and VM save/restore must not mutate callback or pose state.
            assert_eq!(frame, software::render(&preview.scene(camera).unwrap()));
            image::imageops::replace(
                &mut sheet,
                &frame,
                column as i64 * 960,
                (level as i64 - 1) * 720,
            );
        }
        for rotation in 0..4 {
            let mut camera = preview.camera(480, 480, rotation, 0.30, 1);
            camera.center = Vec3::new((minx + maxx) / 2., camera.center.y, (miny + maxy) / 2.);
            let scene = preview.scene(camera).unwrap();
            let frame = software::render(&scene);
            image::imageops::replace(
                &mut rotations,
                &frame,
                i64::from(rotation) * 480,
                column as i64 * 480,
            );
        }
        assert_eq!(frozen.execution.vm, before);
        assert_eq!(
            sims_vm::Snapshot::capture(&frozen.program, &before)
                .restore(&frozen.program)
                .unwrap(),
            before
        );
        println!(
            "{source}: furnished frozen scenes, {} original residents",
            house.household.as_ref().unwrap().residents.len()
        );
    }
    sheet
        .save(repo.join("cache/original-furnished-house-scenes.png"))
        .unwrap();
    rotations
        .save(repo.join("cache/original-floor-rotations.png"))
        .unwrap();
}
