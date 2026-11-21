//! Renders the Goth house (`UserData/Houses/House05.iff`) at every zoom and
//! rotation and pins a digest of each image, so any change to what is drawn
//! shows up. Needs the game: set `SIMS_INSTALL` to the install folder.
//!
//! The digests pin this renderer's output, not the original's. What was
//! compared with the original is in `docs/decisions/`.

use sha2::{Digest, Sha256};
use sims_render::{Camera, Canvas, Focus, Lot, Walls, Zoom, draw, open_house};
use std::path::PathBuf;

/// SHA-256 prefixes of the 16-bit-color images, by zoom, rotation and wall
/// mode.
const DIGESTS: &[(u32, u8, &str, &str)] = &[
    (1, 0, "up", "489e49d35047767d"),
    (1, 0, "cutaway", "20032461d289c46a"),
    (1, 1, "up", "72bddc2febc0bd2e"),
    (1, 1, "cutaway", "83511cc4400a8a8e"),
    (1, 2, "up", "2b0a18029c134301"),
    (1, 2, "cutaway", "5fc3f9fdb1b0f02d"),
    (1, 3, "up", "a3c600e6189168e3"),
    (1, 3, "cutaway", "ebece623c9ccf6a9"),
    (2, 0, "up", "05116b4bdf73c830"),
    (2, 0, "cutaway", "c135b92db338e7bf"),
    (2, 1, "up", "f2ac49d9955f711b"),
    (2, 1, "cutaway", "bb47a2398aaf781f"),
    (2, 2, "up", "8f4c2e2aabd0a0c8"),
    (2, 2, "cutaway", "bb4f2b0c8bed2ce1"),
    (2, 3, "up", "96ba51bf04a1e71e"),
    (2, 3, "cutaway", "e175524ba183b815"),
    (3, 0, "up", "83522815a8355fb6"),
    (3, 0, "cutaway", "e5222a8890e99e5a"),
    (3, 1, "up", "4aa47d188bdab023"),
    (3, 1, "cutaway", "cdd27beba0720a5e"),
    (3, 2, "up", "5028f52b7a2b74a8"),
    (3, 2, "cutaway", "5028f52b7a2b74a8"),
    (3, 3, "up", "ffd2ddb9fe18940e"),
    (3, 3, "cutaway", "ffd2ddb9fe18940e"),
];

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!("skipping: set SIMS_INSTALL to render the Goth house");
            None
        }
    }
}

#[test]
fn goth_house_renders_the_same_at_every_zoom_and_rotation() {
    let Some(root) = install() else {
        return;
    };
    let house = root.join("UserData/Houses/House05.iff");
    let mut opened = open_house(&root, &house).unwrap();
    // The house asks for a lot 52 tiles on a side; the grass's random
    // streams depend on it.
    assert_eq!(opened.world.size, 52);
    let lot = Lot::from_world(&opened.world).unwrap();
    let before = opened.world.hash();
    let upstairs: std::collections::BTreeSet<i16> = lot.levels[1]
        .rooms
        .iter()
        .flatten()
        .copied()
        .filter(|r| *r >= 0 && !lot.outdoor_rooms.contains(r))
        .collect();
    let mut found = Vec::new();
    for zoom in Zoom::ALL {
        for rotation in 0..4u8 {
            for (walls, name) in [(Walls::Up, "up"), (Walls::Cutaway, "cutaway")] {
                let mut camera = Camera {
                    zoom,
                    rotation,
                    level: 2,
                    origin: [0, 0],
                    walls,
                    focus: Focus::default(),
                };
                camera.center_on([22 * 16, 26 * 16], [200, 150]);
                // The cutaway looks at three rooms upstairs.
                for room in upstairs.iter().take(3) {
                    camera.focus.visit(*room);
                }
                let mut canvas = Canvas::new(400, 300, [0, 0, 0]);
                let stats = draw(
                    &opened.world,
                    &lot,
                    &mut opened.assets,
                    &camera,
                    &mut canvas,
                );
                assert_eq!(stats.missing_walls, 0, "zoom {zoom:?} rotation {rotation}");
                assert!(stats.walls > 0 && stats.floors > 0 && stats.objects > 100);
                let bytes: Vec<u8> = canvas.rgb565().into_iter().flatten().collect();
                let digest = Sha256::digest(&bytes);
                let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
                found.push((zoom.level(), rotation, name, hex));
            }
        }
    }
    // Drawing never changes the world.
    assert_eq!(opened.world.hash(), before);
    // The cutaway cuts something in every view that shows the whole house
    // (the near zoom's window shows only part of it).
    for pair in found.chunks(2).filter(|p| p[0].0 < 3) {
        assert_ne!(pair[0].3, pair[1].3, "{pair:?}");
    }
    for (zoom, rotation, name, hex) in &found {
        eprintln!("    ({zoom}, {rotation}, \"{name}\", \"{hex}\"),");
    }
    let expected: Vec<_> = DIGESTS
        .iter()
        .map(|&(z, r, n, h)| (z, r, n, h.to_string()))
        .collect();
    assert_eq!(found, expected);
}

#[test]
fn hidden_objects_are_not_drawn() {
    let Some(root) = install() else {
        return;
    };
    let house = root.join("UserData/Houses/House05.iff");
    let mut opened = open_house(&root, &house).unwrap();
    let lot = Lot::from_world(&opened.world).unwrap();
    let mut camera = Camera {
        zoom: Zoom::Far,
        rotation: 0,
        level: 1,
        origin: [0, 0],
        walls: Walls::Up,
        focus: Focus::default(),
    };
    camera.center_on(lot.built_center(), [200, 150]);
    let mut shown = Canvas::new(400, 300, [0, 0, 0]);
    let before = draw(&opened.world, &lot, &mut opened.assets, &camera, &mut shown);
    assert!(before.objects > 100);
    // Data word 34 hides an object (the simulation's HIDDEN).
    for object in opened.world.objects.values_mut() {
        if object.data.len() > 34 {
            object.data[34] = 1;
        }
    }
    let mut hidden = Canvas::new(400, 300, [0, 0, 0]);
    let after = draw(
        &opened.world,
        &lot,
        &mut opened.assets,
        &camera,
        &mut hidden,
    );
    assert_eq!(after.objects, 0);
    assert_ne!(shown, hidden);
}

#[test]
fn floor_shadows_fit_the_floor_tile_and_every_code_has_a_mask() {
    let Some(root) = install() else {
        return;
    };
    let house = root.join("UserData/Houses/House05.iff");
    let mut opened = open_house(&root, &house).unwrap();
    let guids: std::collections::BTreeSet<u32> =
        opened.world.objects.values().map(|o| o.guid).collect();
    let codes: std::collections::BTreeSet<u8> = guids
        .iter()
        .filter_map(|&g| opened.assets.definition(g)?.shadow)
        .collect();
    // Plants cast the first hand-drawn shadow; the bench, a group of two,
    // casts worked-out ones across its shared side.
    assert!(codes.contains(&0), "{codes:?}");
    assert!(codes.iter().any(|&c| c != 0 && c != 16), "{codes:?}");
    for zoom in 0..3 {
        let tile = opened.assets.floor(0, zoom).unwrap();
        let size = (tile.width, tile.height);
        for &code in &codes {
            for turns in 0..4 {
                let code = sims_render::shadows::rotate(code, turns);
                let mask = opened.assets.shadow_mask(zoom, code);
                let mask = mask.unwrap_or_else(|| panic!("code {code} zoom {zoom}"));
                assert_eq!((mask.width, mask.height), size, "code {code} zoom {zoom}");
            }
        }
    }
}

#[test]
fn off_lot_void_colour_follows_time_of_day_and_house_99() {
    let Some(root) = install() else {
        return;
    };
    let house = root.join("UserData/Houses/House05.iff");
    let mut opened = open_house(&root, &house).unwrap();
    // Camera at the night ratchet's scroll: (10, 10) and (790, 10) are off the lot.
    let camera = Camera {
        zoom: Zoom::Far,
        rotation: 0,
        level: 1,
        origin: [527, -16],
        walls: Walls::Cutaway,
        focus: Focus::default(),
    };
    let cases = [
        (0, 5, [115, 113, 115]),
        (1, 5, [66, 65, 66]),
        (2, 5, [24, 24, 24]),
        (3, 5, [66, 65, 66]),
        (4, 5, [0, 0, 255]),
        (2, 99, [16, 20, 140]),
    ];
    // Build `Lot` once at daytime; `draw` reads live globals 4 and 10 on each
    // clear (0x4aa9bd) so changes to globals take effect without rebuilding `Lot`.
    let lot = Lot::from_world(&opened.world).unwrap();
    for (time_of_day, house_no, want) in cases {
        let mut world = opened.world.clone();
        if world.globals.len() <= 10 {
            world.globals.resize(11, 0);
        }
        world.globals[4] = time_of_day;
        world.globals[10] = house_no;
        let mut canvas = Canvas::new(800, 600, [0, 0, 0]);
        draw(&world, &lot, &mut opened.assets, &camera, &mut canvas);
        // Assert against raw `canvas.color` (not `rgb565()`) so the clear
        // colour used for blending is verified to be 5-6-5 quantized.
        for [x, y] in [[10usize, 10], [790, 10]] {
            assert_eq!(
                canvas.color[y * 800 + x],
                want,
                "time_of_day={time_of_day} house={house_no} at ({x}, {y})"
            );
        }
    }
}

#[test]
fn cut_doorways_draw_no_wall_stub_and_cap_both_flanking_ends() {
    let Some(root) = install() else {
        return;
    };
    let house = root.join("UserData/Houses/House05.iff");
    let mut opened = open_house(&root, &house).unwrap();
    // Clear objects so the test checks only floors, walls and mid-run caps
    // (load scripts that mark style-3 doors hidden have not run here).
    opened.world.objects.clear();
    let lot = Lot::from_world(&opened.world).unwrap();
    // Control lot with every wall removed, giving the bare floor beneath.
    let mut no_walls = lot.clone();
    for cell in &mut no_walls.levels[0].walls {
        cell.segments = 0;
    }
    // Control lot with door-like styles turned into plain walls, giving the
    // stub that used to cover every cut doorway.
    let mut plain_walls = lot.clone();
    for cell in &mut plain_walls.levels[0].walls {
        for style in &mut cell.styles {
            if matches!(*style, 3 | 5 | 6 | 15 | 23) {
                *style = 1;
            }
        }
    }
    // `near-r3-down-0816` camera: bathroom-to-kitchen doorway sits between
    // its two flanking wall ends around (676, 193) and (729, 161), with the
    // open doorway threshold around (706, 177).
    let camera = Camera {
        zoom: Zoom::Near,
        rotation: 3,
        level: 1,
        origin: [1759, -1968],
        walls: Walls::Down,
        focus: Focus::default(),
    };
    let mut actual = Canvas::new(800, 600, [0, 0, 0]);
    let mut bare = Canvas::new(800, 600, [0, 0, 0]);
    let mut stubbed = Canvas::new(800, 600, [0, 0, 0]);
    let actual_stats = draw(
        &opened.world,
        &lot,
        &mut opened.assets,
        &camera,
        &mut actual,
    );
    draw(
        &opened.world,
        &no_walls,
        &mut opened.assets,
        &camera,
        &mut bare,
    );
    let stubbed_stats = draw(
        &opened.world,
        &plain_walls,
        &mut opened.assets,
        &camera,
        &mut stubbed,
    );
    // Skipping cut door-like walls draws strictly fewer wall faces than
    // turning them into plain walls (exact count of level-1 door segments).
    let door_segments = lot.levels[0]
        .walls
        .iter()
        .map(|cell| {
            use sims_render::lot::{ASCENDING, DESCENDING, NORTH, WEST};
            let mut count = 0usize;
            if cell.segments & WEST != 0 && matches!(cell.styles[0], 3 | 5 | 6 | 15 | 23) {
                count += 1;
            }
            for bit in [NORTH, DESCENDING, ASCENDING] {
                if cell.segments & bit != 0 && matches!(cell.styles[1], 3 | 5 | 6 | 15 | 23) {
                    count += 1;
                }
            }
            count
        })
        .sum::<usize>();
    assert!(door_segments > 0);
    assert_eq!(stubbed_stats.walls - actual_stats.walls, door_segments);

    let at = |c: &Canvas, x: usize, y: usize| c.color[y * 800 + x];
    // Open doorway shows bare floor, not a plain-wall stub.
    assert_eq!(at(&actual, 706, 177), at(&bare, 706, 177));
    assert_ne!(at(&actual, 706, 177), at(&stubbed, 706, 177));
    // Both flanking wall ends are closed by cut-height mid-run caps (drawn
    // at cut height, not standing height above them).
    for ([x, y], [above_x, above_y]) in [([676usize, 193], [676, 150]), ([729, 161], [729, 120])] {
        assert_ne!(
            at(&actual, x, y),
            at(&bare, x, y),
            "flanking end ({x}, {y}) should be capped"
        );
        assert_eq!(
            at(&actual, above_x, above_y),
            at(&bare, above_x, above_y),
            "standing height ({above_x}, {above_y}) above cut cap should stay bare"
        );
    }

    // Repainting only door-like wall segments must not change either
    // flanking cap, whereas repainting non-door walls must change both
    // caps — verifying each cap owns the surviving solid wall's slot
    // rather than the cut doorway's opposite slot.
    let mut repainted_doors = lot.clone();
    let mut repainted_solids = lot.clone();
    let size = lot.size;
    for x in 0..size {
        for y in 0..size {
            let idx = sims_render::Lot::cell(x, y);
            let cell = lot.levels[0].walls[idx];
            let repaint = |target: &mut sims_render::Lot, west: bool| {
                if west {
                    target.levels[0].walls[idx].patterns[0] = 10;
                    if x > 0 {
                        target.levels[0].walls[sims_render::Lot::cell(x - 1, y)].patterns[3] = 10;
                    }
                } else {
                    target.levels[0].walls[idx].patterns[1] = 10;
                    if y > 0 {
                        target.levels[0].walls[sims_render::Lot::cell(x, y - 1)].patterns[2] = 10;
                    }
                }
            };
            if cell.segments & 1 != 0 {
                if matches!(cell.styles[0], 3 | 5 | 6 | 15 | 23) {
                    repaint(&mut repainted_doors, true);
                } else {
                    repaint(&mut repainted_solids, true);
                }
            }
            if cell.segments & 2 != 0 {
                if matches!(cell.styles[1], 3 | 5 | 6 | 15 | 23) {
                    repaint(&mut repainted_doors, false);
                } else {
                    repaint(&mut repainted_solids, false);
                }
            }
        }
    }
    let mut with_door_paint = Canvas::new(800, 600, [0, 0, 0]);
    let mut with_solid_paint = Canvas::new(800, 600, [0, 0, 0]);
    draw(
        &opened.world,
        &repainted_doors,
        &mut opened.assets,
        &camera,
        &mut with_door_paint,
    );
    draw(
        &opened.world,
        &repainted_solids,
        &mut opened.assets,
        &camera,
        &mut with_solid_paint,
    );
    for [x, y] in [[676usize, 193], [729, 161]] {
        assert_eq!(
            at(&with_door_paint, x, y),
            at(&actual, x, y),
            "cap ({x}, {y}) must read the surviving wall's pattern, not the cut door's"
        );
        assert_ne!(
            at(&with_solid_paint, x, y),
            at(&actual, x, y),
            "cap ({x}, {y}) should change when its surviving wall is repainted"
        );
    }
}
