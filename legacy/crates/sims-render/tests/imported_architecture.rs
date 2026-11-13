//! Headless scene integration for original geometry, before residents, objects,
//! floor/wall textures and lifecycle callbacks are activated.
use glam::Vec3;
use sims_render::{Assets, Camera, room_scene, software};
use sims_sim::{Room, Simulation, Snapshot};
use sims_vm::{Program, Vm};

#[test]
#[ignore = "requires extracted original houses and avatar resources"]
fn original_house_geometry_renders_and_restores_headlessly() {
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = sims_content::Index::scan(&repo.join("game-data/cabinet-extracted")).unwrap();
    let assets = Assets::load_objects(&index, &[]).unwrap();
    let p = Program::default();
    let mut sheet = image::RgbaImage::new(2880, 1440);
    for (column, source) in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ]
    .into_iter()
    .enumerate()
    {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        let heads = house
            .floors
            .iter()
            .map(|f| f.object_heads.clone())
            .collect::<Vec<_>>();
        let lot = house.lot_geometry(&heads).unwrap();
        let interior = lot.floors[0]
            .rooms
            .primary
            .iter()
            .enumerate()
            .filter(|(_, r)| lot.floors[0].metrics.get(r).is_some_and(|m| !m.outdoors))
            .map(|(i, _)| ((i / lot.size) as f32, (i % lot.size) as f32))
            .collect::<Vec<_>>();
        let minx = interior.iter().map(|p| p.0).reduce(f32::min).unwrap();
        let maxx = interior.iter().map(|p| p.0).reduce(f32::max).unwrap() + 1.;
        let miny = interior.iter().map(|p| p.1).reduce(f32::min).unwrap();
        let maxy = interior.iter().map(|p| p.1).reduce(f32::max).unwrap() + 1.;
        let mut sim = Simulation::new(
            Vm::new(Default::default(), 1),
            Room {
                width: 1024,
                height: 1024,
                obstacles: vec![],
            },
            Default::default(),
        )
        .unwrap();
        sim.set_original_lot(&p, lot).unwrap();
        let restored = Snapshot::capture(&p, &sim).restore(&p).unwrap();
        for level in 1..=2 {
            let mut camera = Camera::room(&sim, 960, 720, 0, 0.48).floor(level);
            camera.center = Vec3::new((minx + maxx) / 2., camera.center.y, (miny + maxy) / 2.);
            let scene = room_scene(&assets, &p, &sim, camera, 0.).unwrap();
            let frame = software::render(&scene);
            let resumed =
                software::render(&room_scene(&assets, &p, &restored, camera, 0.).unwrap());
            assert_eq!(frame, resumed, "{source} floor {level}");
            image::imageops::replace(
                &mut sheet,
                &frame,
                column as i64 * 960,
                (level as i64 - 1) * 720,
            );
        }
    }
    sheet
        .save(repo.join("cache/original-house-architecture-scenes.png"))
        .unwrap();
}
