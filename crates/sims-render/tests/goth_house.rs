//! Renders the Goth house (`UserData/Houses/House05.iff`) at every zoom and
//! rotation and pins a digest of each image, so any change to what is drawn
//! shows up. Needs the game: set `SIMS_INSTALL` to the install folder.
//!
//! The digests pin this renderer's output, not the original's. What was
//! compared with the original is in `docs/decisions.md`.

use sha2::{Digest, Sha256};
use sims_render::{Camera, Canvas, Lot, Walls, Zoom, draw, open_house};
use std::path::PathBuf;

/// SHA-256 prefixes of the 16-bit-color images, by zoom, rotation and wall
/// mode.
const DIGESTS: &[(u32, u8, &str, &str)] = &[
    (1, 0, "up", "d48d3a69fd383377"),
    (1, 0, "cutaway", "8f98c8eeeeceec58"),
    (1, 1, "up", "b3104baf703cc11b"),
    (1, 1, "cutaway", "28d3597d65464978"),
    (1, 2, "up", "f77ce07b119d7647"),
    (1, 2, "cutaway", "d7dee2bf5dc8a647"),
    (1, 3, "up", "609d6fcf2827b8cb"),
    (1, 3, "cutaway", "b5bc120f96a90f23"),
    (2, 0, "up", "3c29c97799a9c138"),
    (2, 0, "cutaway", "bbba05ed409f82c9"),
    (2, 1, "up", "b4348117d41dec02"),
    (2, 1, "cutaway", "8c0c47fb96cea254"),
    (2, 2, "up", "f0bd44fc8c6757f1"),
    (2, 2, "cutaway", "81ff15caa547d241"),
    (2, 3, "up", "03ab6e56c0cf06e3"),
    (2, 3, "cutaway", "ac8942b9a9a825b0"),
    (3, 0, "up", "737d2bd955a5a6a6"),
    (3, 0, "cutaway", "e498cb0f7788a172"),
    (3, 1, "up", "a4aef4fbf390b2c3"),
    (3, 1, "cutaway", "5b6808251a09fba6"),
    (3, 2, "up", "9322082d59be8530"),
    (3, 2, "cutaway", "fd55f0b71bf37848"),
    (3, 3, "up", "db181145975b7057"),
    (3, 3, "cutaway", "fb5e485a5d79bec9"),
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
                };
                camera.center_on([22 * 16, 26 * 16], [200, 150]);
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
