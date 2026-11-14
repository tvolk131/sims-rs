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
    (1, 0, "up", "3a6a3cfce2739636"),
    (1, 0, "cutaway", "fdf3f67bbe2244a3"),
    (1, 1, "up", "b728247b0dfd6395"),
    (1, 1, "cutaway", "b6f1d3dc3ca55bf0"),
    (1, 2, "up", "4bc004f0090e6e6f"),
    (1, 2, "cutaway", "16d1f0e323df8326"),
    (1, 3, "up", "bbea6b34b10baee5"),
    (1, 3, "cutaway", "a48edc4061bbe183"),
    (2, 0, "up", "82042a233516e5ae"),
    (2, 0, "cutaway", "511f4caaff6e0a1e"),
    (2, 1, "up", "c16263bfe21b452f"),
    (2, 1, "cutaway", "011a83b7ecca5dff"),
    (2, 2, "up", "26d119e8ad484883"),
    (2, 2, "cutaway", "b7de14db63e3cdcb"),
    (2, 3, "up", "76b46ee7ab728f05"),
    (2, 3, "cutaway", "09c7e73f38cf4bf7"),
    (3, 0, "up", "64657df6585f54db"),
    (3, 0, "cutaway", "309650467603989b"),
    (3, 1, "up", "0f78c7c527e4f1e1"),
    (3, 1, "cutaway", "23415eb9a68707d1"),
    (3, 2, "up", "705ebabefdc8c327"),
    (3, 2, "cutaway", "74a3f68983e814d2"),
    (3, 3, "up", "cd26782769c9a9b4"),
    (3, 3, "cutaway", "74e1b97cbc263bb2"),
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
