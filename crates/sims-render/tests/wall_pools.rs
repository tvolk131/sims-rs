//! Lone walls drawn with the game's art: a lamp's pool (with
//! `lightMap.dat`) and the trim's light at night. Needs the game: set
//! `SIMS_INSTALL` to the install folder.

use sims_formats::lot::Array;
use sims_render::lot::{FULL, Lamps, WEST};
use sims_render::{Camera, Canvas, Focus, Lot, Walls, Zoom, draw, open_house};
use sims_sim::World;
use sims_sim::rooms::Room;
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!("skipping: set SIMS_INSTALL to draw with the game's art");
            None
        }
    }
}

/// An empty lot, all one dark indoor room, with one west wall on tile
/// (10, 10).
fn lone_wall() -> World {
    wall_run(1)
}

/// An empty lot, all one dark indoor room, with a straight run of `n`
/// west walls on tiles (10, 10) to (10, 10 + n - 1), and no room 0.
fn wall_run(n: usize) -> World {
    let mut world = World::empty();
    world.size = FULL as i16;
    let full = FULL * FULL;
    for (id, stride) in [(0, 4), (1, 1), (2, 8), (101, 1), (102, 8)] {
        world.arrays.insert(
            id,
            Array {
                width: FULL,
                height: FULL,
                stride,
                encoding: 0,
                data: vec![0; full * stride],
            },
        );
    }
    let walls = world.arrays.get_mut(&2).unwrap();
    for y in 10..10 + n {
        let cell = Lot::cell(10, y) * 8;
        walls.data[cell..cell + 8].copy_from_slice(&[WEST, 0, 1, 1, 2, 2, 2, 2]);
    }
    world.rooms.map = [vec![1; full], vec![0; full]];
    world.rooms.rooms.insert(
        1,
        Room {
            outdoors: false,
            half_tiles: 2,
            light: sims_core::F32::from_f32(0.0),
            ..Room::default()
        },
    );
    world
}

#[test]
fn a_lamp_lights_a_walls_corner_caps_as_well_as_its_face() {
    let Some(root) = install() else {
        return;
    };
    let mut assets = open_house(&root, &root.join("UserData/Houses/House05.iff"))
        .unwrap()
        .assets;
    let world = lone_wall();
    let dark = Lot::from_world(&world).unwrap();
    // A lamp on tile (11, 10), beside the wall on its seen side, marks
    // every tile within four of it with the offset from it, as a lit
    // lamp in the room does (see `Lot::lamps`).
    let mut lit = dark.clone();
    for dx in -4..=4i8 {
        for dy in -4..=4i8 {
            let tile = [(11 + dx) as usize, (10 + dy) as usize];
            lit.lamps[0][Lot::cell(tile[0], tile[1])] = Lamps([Some([dx, dy]), None]);
        }
    }
    let mut camera = Camera {
        zoom: Zoom::Near,
        rotation: 0,
        level: 1,
        origin: [0, 0],
        walls: Walls::Up,
        focus: Focus::default(),
    };
    camera.center_on([10 * 16, 10 * 16 + 8], [200, 150]);
    let frame = |lot: &Lot, assets: &mut _| {
        let mut canvas = Canvas::new(400, 300, [0, 0, 0]);
        let stats = draw(&world, lot, assets, &camera, &mut canvas);
        assert_eq!((stats.walls, stats.missing_walls), (1, 0));
        canvas
    };
    let (before, after) = (frame(&dark, &mut assets), frame(&lit, &mut assets));
    // The face spans the screen columns between the wall's two ends; the
    // caps stand beside them.
    let ends = [[10, 10], [10, 11]].map(|c| camera.project_fine(c.map(|n| n * 16), 1, 0)[0]);
    let face = ends[0].min(ends[1])..=ends[0].max(ends[1]);
    let (mut on_face, mut on_caps) = (0, 0);
    for (i, (a, b)) in before.color.iter().zip(&after.color).enumerate() {
        if a != b {
            let x = (i % before.width) as i32;
            if face.contains(&x) {
                on_face += 1;
            } else {
                on_caps += 1;
            }
        }
    }
    assert!(on_face > 100, "{on_face}");
    assert!(on_caps > 0, "face {on_face}, caps {on_caps}");
}

#[test]
fn at_night_without_room_0_only_a_runs_corner_records_trim_in_full_light() {
    let Some(root) = install() else {
        return;
    };
    let mut assets = open_house(&root, &root.join("UserData/Houses/House05.iff"))
        .unwrap()
        .assets;
    // Three walls in a straight line: corner records stand at the run's
    // two ends, (10, 10) and (10, 13); the two corners between are not
    // records.
    let world = wall_run(3);
    let mut without = Lot::from_world(&world).unwrap();
    without.time_of_day = 2;
    assert_eq!(without.outdoor_light, None);
    // With room 0 in full light, every trim takes 1 through the drawing's
    // setter, held at the night's cap of 0.8. Without it the corner pass
    // resets its trims to full light (0x53c3c6), but the wall pass still
    // passes 1 through the setter for the trim along the run (0x53e2f5).
    // So only the corner records' trims differ.
    let mut with = without.clone();
    with.outdoor_light = Some(1.0);
    let mut camera = Camera {
        zoom: Zoom::Near,
        rotation: 0,
        level: 1,
        origin: [0, 0],
        walls: Walls::Up,
        focus: Focus::default(),
    };
    camera.center_on([10 * 16, 12 * 16], [300, 400]);
    let frame = |lot: &Lot, assets: &mut _| {
        let mut canvas = Canvas::new(600, 600, [0, 0, 0]);
        let stats = draw(&world, lot, assets, &camera, &mut canvas);
        assert_eq!((stats.walls, stats.missing_walls), (3, 0));
        canvas
    };
    let (a, b) = (frame(&without, &mut assets), frame(&with, &mut assets));
    // Each corner's trim covers the halves of the walls meeting there, so
    // a changed pixel goes to the corner nearest it across the screen (a
    // midpoint goes to the corner nearer the run's start).
    let columns: Vec<i32> = (10..=13)
        .map(|y| camera.project_fine([10 * 16, y * 16], 1, 0)[0])
        .collect();
    let mut changed = [0; 4];
    for (i, (p, q)) in a.color.iter().zip(&b.color).enumerate() {
        if p != q {
            let x = (i % a.width) as i32;
            let nearest = (0..4).min_by_key(|&k| (columns[k] - x).abs()).unwrap();
            changed[nearest] += 1;
        }
    }
    let [first, second, third, last] = changed;
    assert!(first > 100 && last > 100, "{changed:?}");
    assert_eq!((second, third), (0, 0), "{changed:?}");
}
