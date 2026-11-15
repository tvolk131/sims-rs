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
    (1, 0, "up", "a8b2d39adfd498a5"),
    (1, 0, "cutaway", "6b318d27f1e75cfd"),
    (1, 1, "up", "f724e9cf542a7945"),
    (1, 1, "cutaway", "8649d591ea660c16"),
    (1, 2, "up", "06d59d582bf9ca5a"),
    (1, 2, "cutaway", "709b18c5e0ae3f3d"),
    (1, 3, "up", "c650a512054d73b9"),
    (1, 3, "cutaway", "7abfdc409c27ac4f"),
    (2, 0, "up", "d976a54c7a6ccbf1"),
    (2, 0, "cutaway", "4f626a9e307a4201"),
    (2, 1, "up", "a5a6b05673d92486"),
    (2, 1, "cutaway", "9afb0f7bc8d502ca"),
    (2, 2, "up", "a08f208153492f12"),
    (2, 2, "cutaway", "6c88362a2568df0c"),
    (2, 3, "up", "50661229d3fcc832"),
    (2, 3, "cutaway", "d3bb817333a5c86d"),
    (3, 0, "up", "ee27738e79627d30"),
    (3, 0, "cutaway", "7e00a434ec1318d5"),
    (3, 1, "up", "9dbeeaf9cf4ad84e"),
    (3, 1, "cutaway", "cb2af648026d2261"),
    (3, 2, "up", "f1029ce0059aea36"),
    (3, 2, "cutaway", "f1029ce0059aea36"),
    (3, 3, "up", "c383656350044d6b"),
    (3, 3, "cutaway", "c383656350044d6b"),
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
