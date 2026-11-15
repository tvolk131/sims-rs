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
    (1, 0, "up", "01dddbd62e4fbbfb"),
    (1, 0, "cutaway", "15a987dbe0c36eb3"),
    (1, 1, "up", "005b9dc20e21e1a4"),
    (1, 1, "cutaway", "6a600727809e6624"),
    (1, 2, "up", "8111b1b224d332ec"),
    (1, 2, "cutaway", "8577367bb53d310a"),
    (1, 3, "up", "789522d2e9e27dbb"),
    (1, 3, "cutaway", "6205926e5826492c"),
    (2, 0, "up", "20deb01575f27968"),
    (2, 0, "cutaway", "989502d652e643b6"),
    (2, 1, "up", "f7adca35161169f1"),
    (2, 1, "cutaway", "474cff3adf1bec02"),
    (2, 2, "up", "0a5aa9c3f0937f07"),
    (2, 2, "cutaway", "ad62e54340174ebc"),
    (2, 3, "up", "7718be12340d5d60"),
    (2, 3, "cutaway", "72557210b98c08a3"),
    (3, 0, "up", "883a809f9ccffab4"),
    (3, 0, "cutaway", "57d16d2b6f8a013e"),
    (3, 1, "up", "6a8c50a608d21a5e"),
    (3, 1, "cutaway", "4a2be1b592fc7c53"),
    (3, 2, "up", "1d9f70bd44aa9b0f"),
    (3, 2, "cutaway", "1d9f70bd44aa9b0f"),
    (3, 3, "up", "e3098ea9898bd36f"),
    (3, 3, "cutaway", "e3098ea9898bd36f"),
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
