//! Renders the Goth house (`UserData/Houses/House05.iff`) at every zoom and
//! rotation and pins a digest of each image, so any change to what is drawn
//! shows up. Needs the game: set `SIMS_INSTALL` to the install folder.
//!
//! The digests pin this renderer's output, not the original's. What was
//! compared with the original is in `docs/decisions.md`.

use sha2::{Digest, Sha256};
use sims_render::{Camera, Canvas, Focus, Lot, Walls, Zoom, draw, open_house};
use std::path::PathBuf;

/// SHA-256 prefixes of the 16-bit-color images, by zoom, rotation and wall
/// mode.
const DIGESTS: &[(u32, u8, &str, &str)] = &[
    (1, 0, "up", "e5b4a97eed4bf891"),
    (1, 0, "cutaway", "4e672034d3f63db9"),
    (1, 1, "up", "d0865335d512bef6"),
    (1, 1, "cutaway", "1aaa51c3705ad702"),
    (1, 2, "up", "000d668755862816"),
    (1, 2, "cutaway", "6b770f8ada3e5a30"),
    (1, 3, "up", "0f80c8b2b60ce996"),
    (1, 3, "cutaway", "f26a014b56dbc9e0"),
    (2, 0, "up", "a7c9d37ef6fbf10c"),
    (2, 0, "cutaway", "d612670b6339dc6e"),
    (2, 1, "up", "c6fc0d9673f9d3cc"),
    (2, 1, "cutaway", "4fb76bd36dc352d6"),
    (2, 2, "up", "376df79a4348558f"),
    (2, 2, "cutaway", "f65c802d0c7750d3"),
    (2, 3, "up", "992affbe6bfd6f00"),
    (2, 3, "cutaway", "e14843672c12d456"),
    (3, 0, "up", "737d2bd955a5a6a6"),
    (3, 0, "cutaway", "d06c73ecf0b4462f"),
    (3, 1, "up", "0626c4e2249d123a"),
    (3, 1, "cutaway", "708f419dbcadcb04"),
    (3, 2, "up", "48e2c6ae88a10133"),
    (3, 2, "cutaway", "48e2c6ae88a10133"),
    (3, 3, "up", "97b65f9cfd71291f"),
    (3, 3, "cutaway", "97b65f9cfd71291f"),
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
