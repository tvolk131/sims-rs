use glam::{Quat, Vec3};
use sims_content::{
    Index,
    vm::{ChairFixture, ChairScenario, chair_room, load_chair_animations, load_program},
};
use sims_render::{Assets, Camera, pose, room_scene};
use sims_sim::{Snapshot, StepStop};
fn world_bone(
    assets: &Assets,
    program: &sims_vm::Program,
    sim: &sims_sim::Simulation,
    name: &str,
) -> Vec3 {
    let p = &sim.animations[&1];
    let bones = pose::evaluate(
        &assets.avatar.skeleton,
        &program.animations[&p.clip],
        p.frame_fifths as f32 / 5.,
        p.looping,
    )
    .unwrap();
    let i = assets
        .avatar
        .skeleton
        .bones
        .iter()
        .position(|b| b.name == name)
        .unwrap();
    let actor = &sim.bodies[&1];
    let angle = match actor.facing {
        1 => 0.,
        4 => std::f32::consts::FRAC_PI_2,
        16 => std::f32::consts::PI,
        _ => std::f32::consts::FRAC_PI_2 * 3.,
    };
    Vec3::new(
        actor.position.x as f32 / 16.,
        0.,
        actor.position.y as f32 / 16.,
    ) + Quat::from_rotation_y(std::f32::consts::PI - angle) * bones[i].transform_point3(Vec3::ZERO)
        / 3.
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_avatar_views_resume_and_sit_to_idle_are_consistent() {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../game-data/cabinet-extracted")
        });
    let index = Index::scan(&root).unwrap();
    let assets = Assets::load(&index).unwrap();
    assert_eq!(assets.avatar.skeleton.bones.len(), 29);
    assert_eq!(assets.avatar.parts.len(), 4);
    let mut program = load_program(&index, "DiningChairs.iff", "Global.iff").unwrap();
    load_chair_animations(&index, &mut program).unwrap();
    for facing in [1, 4, 16, 64] {
        let fixture =
            ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty).unwrap();
        let mut sim = chair_room(&program, fixture, false, facing).unwrap();
        let mut previous = None;
        loop {
            let step = sim.run_fixture_tick(&program, 1000).unwrap();
            if sim.vm.tick == 150 {
                let restored = Snapshot::from_json(
                    &serde_json::to_vec(&Snapshot::capture(&program, &sim)).unwrap(),
                    &program,
                )
                .unwrap();
                for rotation in 0..4 {
                    let camera = Camera::room(&sim, 550, 380, rotation, 0.5);
                    for fraction in [0., 0.5] {
                        let a = room_scene(&assets, &program, &sim, camera, fraction).unwrap();
                        let b = room_scene(&assets, &program, &restored, camera, fraction).unwrap();
                        assert_eq!(
                            bytemuck::cast_slice::<_, u8>(&a.vertices),
                            bytemuck::cast_slice::<_, u8>(&b.vertices)
                        );
                        assert!(
                            a.vertices.iter().all(|v| v
                                .position
                                .iter()
                                .chain(v.uv.iter())
                                .all(|x| x.is_finite()))
                        );
                        assert_eq!(
                            sims_render::software::render(&a),
                            sims_render::software::render(&b)
                        );
                    }
                }
            }
            if step.stop == StepStop::Complete {
                let head = world_bone(&assets, &program, &sim, "HEAD");
                assert!(
                    head.distance(previous.unwrap()) < 0.12,
                    "facing {facing}: discontinuous seated head"
                );
                break;
            }
            if sim.animations.contains_key(&1) {
                previous = Some(world_bone(&assets, &program, &sim, "HEAD"));
            }
            assert!(sim.vm.tick < 512);
            sim.advance_fixture_tick().unwrap();
        }
    }
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_locomotion_blends_continuously_and_resumes_during_every_phase() {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../game-data/cabinet-extracted")
        });
    let index = Index::scan(&root).unwrap();
    let assets = Assets::load(&index).unwrap();
    let mut program = load_program(&index, "DiningChairs.iff", "Global.iff").unwrap();
    load_chair_animations(&index, &mut program).unwrap();
    let fixture =
        ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty).unwrap();
    let mut sim = chair_room(&program, fixture, false, 1).unwrap();
    // Exercise the planted initial turn as well as the walking turns around the obstacle.
    sim.bodies.get_mut(&1).unwrap().facing = 64;
    // Body facing is a bit mask; native data word 1 stores the direction index.
    sim.vm.world.objects.get_mut(&1).unwrap().data.insert(1, 6);
    sim.run_fixture_tick(&program, 1000).unwrap();
    let phases = sim.routes[&2].locomotion.phases.clone();
    let end = sim.routes[&2].locomotion.duration();
    let midpoints: Vec<_> = phases
        .iter()
        .map(|p| u64::from(p.start_tick + p.ticks / 2))
        .collect();
    let boundaries: Vec<_> = phases
        .iter()
        .map(|p| u64::from(p.start_tick + p.ticks))
        .collect();
    let mut before_boundary = None;
    for tick in 0..=u64::from(end) {
        if tick > 0 {
            sim.advance_fixture_tick().unwrap();
            sim.run_fixture_tick(&program, 1000).unwrap();
        }
        let camera = Camera::room(&sim, 550, 380, 0, 0.5);
        if let Some(previous) = before_boundary.take() {
            let current = room_scene(&assets, &program, &sim, camera, 0.).unwrap();
            let previous: sims_render::Scene = previous;
            let max_pixels = previous
                .vertices
                .iter()
                .zip(&current.vertices)
                .map(|(a, b)| {
                    let dx = (a.position[0] - b.position[0]) * 275.;
                    let dy = (a.position[1] - b.position[1]) * 190.;
                    (dx * dx + dy * dy).sqrt()
                })
                .fold(0f32, f32::max);
            let tolerance = if tick == u64::from(end) { 8. } else { 0.1 };
            assert!(
                max_pixels < tolerance,
                "pose pop at {tick}: {max_pixels} pixels"
            );
        }
        if boundaries.contains(&(tick + 1)) {
            before_boundary = Some(room_scene(&assets, &program, &sim, camera, 0.9999).unwrap());
        }
        if midpoints.contains(&tick) {
            let saved = serde_json::to_vec(&Snapshot::capture(&program, &sim)).unwrap();
            let restored = Snapshot::from_json(&saved, &program).unwrap();
            for rotation in 0..4 {
                for fraction in [0., 0.5] {
                    let camera = Camera::room(&sim, 550, 380, rotation, 0.5);
                    let a = room_scene(&assets, &program, &sim, camera, fraction).unwrap();
                    let b = room_scene(&assets, &program, &restored, camera, fraction).unwrap();
                    assert_eq!(
                        bytemuck::cast_slice::<_, u8>(&a.vertices),
                        bytemuck::cast_slice::<_, u8>(&b.vertices)
                    );
                    if fraction == 0.5 {
                        assert_eq!(
                            sims_render::software::render(&a),
                            sims_render::software::render(&b)
                        );
                    }
                }
            }
            // Resuming gameplay also preserves all subsequent VM events and the seated state.
            let mut reference = sim.clone();
            let mut replay = restored;
            loop {
                reference.advance_fixture_tick().unwrap();
                replay.advance_fixture_tick().unwrap();
                let a = reference.run_fixture_tick(&program, 1000).unwrap();
                let b = replay.run_fixture_tick(&program, 1000).unwrap();
                assert_eq!(a, b);
                assert_eq!(reference, replay);
                if a.stop == StepStop::Complete {
                    break;
                }
                assert!(reference.vm.tick < 512);
            }
        }
    }
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn start_to_walk_does_not_rush_the_pose_change_in_a_short_path_segment() {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../game-data/cabinet-extracted")
        });
    let index = Index::scan(&root).unwrap();
    let assets = Assets::load(&index).unwrap();
    let mut program = load_program(&index, "DiningChairs.iff", "Global.iff").unwrap();
    load_chair_animations(&index, &mut program).unwrap();
    let fixture =
        ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty).unwrap();
    let mut sim = chair_room(&program, fixture, false, 1).unwrap();
    sim.run_fixture_tick(&program, 1000).unwrap();
    let route = &sim.routes[&2];
    let mut movement = route.locomotion.clone();
    // A three-tick straight at tick 6 is followed by a walking turn at tick 9.
    // A blend confined to half that straight made a toe jump 1.085 model units
    // (over a third of a tile) in one 40 ms simulation tick.
    assert_eq!(movement.phases[1].start_tick, 6);
    assert_eq!(movement.phases[1].ticks, 3);
    let mut previous: Option<Vec<glam::Mat4>> = None;
    for tick in 0..=18 {
        movement.elapsed = tick;
        let bones = pose::locomotion(
            &assets.avatar.skeleton,
            &program,
            &movement,
            movement.sample(&route.path, 0.),
        )
        .unwrap();
        if let Some(previous) = previous {
            let displacement = bones
                .iter()
                .zip(previous)
                .map(|(a, b)| {
                    a.transform_point3(Vec3::ZERO)
                        .distance(b.transform_point3(Vec3::ZERO))
                })
                .fold(0f32, f32::max);
            assert!(
                displacement < 0.65,
                "rapid pose change into tick {tick}: {displacement}"
            );
            if tick == 7 {
                assert!(displacement < 0.25, "start-to-walk jump: {displacement}");
            }
        }
        previous = Some(bones);
    }
    // The blend must continue across tick 9 instead of dropping its remaining weight.
    movement.elapsed = 8;
    let before = pose::locomotion(
        &assets.avatar.skeleton,
        &program,
        &movement,
        movement.sample(&route.path, 0.9999),
    )
    .unwrap();
    movement.elapsed = 9;
    let after = pose::locomotion(
        &assets.avatar.skeleton,
        &program,
        &movement,
        movement.sample(&route.path, 0.),
    )
    .unwrap();
    for (a, b) in before.iter().zip(after) {
        assert!(
            a.transform_point3(Vec3::ZERO)
                .distance(b.transform_point3(Vec3::ZERO))
                < 0.001
        );
    }
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn first_swing_finishes_on_the_same_foot_across_the_start_to_walk_blend() {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../game-data/cabinet-extracted")
        });
    let index = Index::scan(&root).unwrap();
    let assets = Assets::load(&index).unwrap();
    let skeleton = &assets.avatar.skeleton;
    let mut program = load_program(&index, "DiningChairs.iff", "Global.iff").unwrap();
    load_chair_animations(&index, &mut program).unwrap();
    let fixture =
        ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty).unwrap();
    let mut sim = chair_room(&program, fixture, false, 1).unwrap();
    sim.run_fixture_tick(&program, 1000).unwrap();
    let route = &sim.routes[&2];
    let mut movement = route.locomotion.clone();
    let left = skeleton
        .bones
        .iter()
        .position(|b| b.name == "L_TOE02")
        .unwrap();
    let right = skeleton
        .bones
        .iter()
        .position(|b| b.name == "R_TOE02")
        .unwrap();
    let mut previous = None;
    // The source start lifts the left foot. It must continue forward and land before
    // the right foot starts its swing, including through the turn at tick 9.
    // A smooth blend into an unsynchronized loop instead pulled it backwards at 6–7.
    for quarter in 24..=40 {
        movement.elapsed = quarter / 4;
        let sample = movement.sample(&route.path, (quarter % 4) as f32 / 4.);
        let bones = pose::locomotion(skeleton, &program, &movement, sample).unwrap();
        let l = bones[left].transform_point3(Vec3::ZERO);
        let r = bones[right].transform_point3(Vec3::ZERO);
        if let Some(z) = previous {
            assert!(
                l.z >= z - 0.002,
                "left swing reversed at tick {}",
                quarter as f32 / 4.
            );
        }
        assert!(
            r.y.abs() < 0.04,
            "support foot lifted before the first step landed"
        );
        previous = Some(l.z);
    }
    assert!(previous.unwrap() > 1.4, "first step was cut short");
    movement.elapsed = 12;
    let bones = pose::locomotion(
        skeleton,
        &program,
        &movement,
        movement.sample(&route.path, 0.),
    )
    .unwrap();
    assert!(
        bones[left].transform_point3(Vec3::ZERO).y.abs() < 0.03,
        "first step never landed"
    );

    // Marker alignment must also preserve continuity with a partial standing blend,
    // an initial planted turn, and a start long enough to use the entire source clip.
    for length in [3, 8, 64] {
        let path: Vec<_> = (0..=length)
            .map(|y| sims_sim::geometry::Point { x: 0, y: -y })
            .collect();
        for facing in [1, 64] {
            let mut m = sims_sim::locomotion::Locomotion::plan(&program, &path, facing, 1).unwrap();
            let start = m
                .phases
                .iter()
                .find(|p| p.kind == sims_sim::locomotion::Kind::Start)
                .unwrap();
            let boundary = start.start_tick + start.ticks;
            m.elapsed = boundary - 1;
            let before = pose::locomotion(skeleton, &program, &m, m.sample(&path, 0.9999)).unwrap();
            m.elapsed = boundary;
            let after = pose::locomotion(skeleton, &program, &m, m.sample(&path, 0.)).unwrap();
            for (a, b) in before.iter().zip(after) {
                assert!(
                    a.transform_point3(Vec3::ZERO)
                        .distance(b.transform_point3(Vec3::ZERO))
                        < 0.001,
                    "start boundary popped: length {length}, facing {facing}"
                );
            }
        }
    }
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_exit_poses_join_seated_and_standing_and_resume_in_all_views() {
    use sims_sim::interaction::Phase;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../game-data/cabinet-extracted");
    let index = Index::scan(&root).unwrap();
    let assets = Assets::load(&index).unwrap();
    let mut program = load_program(&index, "DiningChairs.iff", "Global.iff").unwrap();
    load_chair_animations(&index, &mut program).unwrap();
    for (facing, blocked_sides) in [(1, 0), (4, 0), (16, 0), (64, 0), (1, 1), (1, 2), (1, 3)] {
        let fixture =
            ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty).unwrap();
        let mut s = sims_content::vm::two_chair_room(&program, fixture, facing).unwrap();
        let id = s.enqueue(&program, 1, 2, 0).unwrap();
        s.run_fixture_tick(&program, 1000).unwrap();
        while s.interactions[&1].active.as_ref().unwrap().phase != Phase::Seated {
            s.advance_fixture_tick().unwrap();
            s.run_fixture_tick(&program, 1000).unwrap();
            assert!(s.vm.tick < 400);
        }
        let head = world_bone(&assets, &program, &s, "HEAD");
        // Force each fallback animation in the north-facing case; clear chairs
        // must use the front animation in every orientation.
        for (x, y) in [(88, 40), (104, 56), (88, 72)]
            .into_iter()
            .take(blocked_sides)
        {
            s.room.obstacles.push(sims_sim::Rect {
                min: sims_sim::Point::new(x - 4, y - 4),
                max: sims_sim::Point::new(x + 4, y + 4),
            });
        }
        s.cancel_interaction(&program, 1, id).unwrap();
        s.run_fixture_tick(&program, 1000).unwrap();
        assert_eq!(
            s.animations[&1].clip,
            [
                "a2o-stand-front",
                "a2o-stand-right",
                "a2o-stand-behind",
                "a2o-stand-left"
            ][blocked_sides]
        );
        assert!(
            head.distance(world_bone(&assets, &program, &s, "HEAD")) < 0.12,
            "exit start pop for facing {facing}"
        );
        let start = s.vm.tick;
        while s.interactions[&1].active.is_some() {
            let previous = world_bone(&assets, &program, &s, "HEAD");
            s.advance_fixture_tick().unwrap();
            s.run_fixture_tick(&program, 1000).unwrap();
            if s.vm.tick == start + 40 {
                let resumed = Snapshot::capture(&program, &s).restore(&program).unwrap();
                for rotation in 0..4 {
                    for fraction in [0., 0.5] {
                        let camera = Camera::room(&s, 550, 380, rotation, 0.5);
                        let a = room_scene(&assets, &program, &s, camera, fraction).unwrap();
                        let b = room_scene(&assets, &program, &resumed, camera, fraction).unwrap();
                        assert_eq!(
                            bytemuck::cast_slice::<_, u8>(&a.vertices),
                            bytemuck::cast_slice::<_, u8>(&b.vertices)
                        );
                        assert_eq!(
                            sims_render::software::render(&a),
                            sims_render::software::render(&b)
                        );
                    }
                }
            }
            if s.interactions[&1].active.is_none() {
                let actor = &s.bodies[&1];
                let bones = pose::evaluate(
                    &assets.avatar.skeleton,
                    &program.animations[sims_sim::locomotion::STAND],
                    s.vm.tick as f32 * 1.2,
                    true,
                )
                .unwrap();
                let head = assets
                    .avatar
                    .skeleton
                    .bones
                    .iter()
                    .position(|b| b.name == "HEAD")
                    .unwrap();
                let world = Vec3::new(
                    actor.position.x as f32 / 16.,
                    0.,
                    actor.position.y as f32 / 16.,
                ) + Quat::from_rotation_y(
                    std::f32::consts::PI
                        - actor.facing.trailing_zeros() as f32 * std::f32::consts::FRAC_PI_4,
                ) * bones[head].transform_point3(Vec3::ZERO)
                    / 3.;
                assert!(
                    world.distance(previous) < 0.15,
                    "exit end pop for facing {facing}: {}",
                    world.distance(previous)
                );
            }
            assert!(s.vm.tick < start + 200);
        }
    }
}
