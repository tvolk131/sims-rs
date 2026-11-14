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
    (1, 0, "up", "93bce391fd3aba3e"),
    (1, 0, "cutaway", "af4518fd26d8da09"),
    (1, 1, "up", "de8485022c613dc1"),
    (1, 1, "cutaway", "1c8d3bc258fa2e7c"),
    (1, 2, "up", "ca9406dfa9aa91ba"),
    (1, 2, "cutaway", "a15892ba0a113d27"),
    (1, 3, "up", "fda86285ef217859"),
    (1, 3, "cutaway", "4af6c7271568b36d"),
    (2, 0, "up", "c37ae4bcbbb45197"),
    (2, 0, "cutaway", "9f69933461d2d625"),
    (2, 1, "up", "ad99288af97c53c7"),
    (2, 1, "cutaway", "d2490aac37b6cfbb"),
    (2, 2, "up", "085dda85fd040db3"),
    (2, 2, "cutaway", "65969f38866de469"),
    (2, 3, "up", "a0ae7030e486feb8"),
    (2, 3, "cutaway", "ca8ec890e26f3ad5"),
    (3, 0, "up", "a8d663bd2526bf7a"),
    (3, 0, "cutaway", "d945a633c971540c"),
    (3, 1, "up", "0a1df0437d4eb206"),
    (3, 1, "cutaway", "f37fef8b3f530081"),
    (3, 2, "up", "7492355b9c174957"),
    (3, 2, "cutaway", "033f8357fc111c20"),
    (3, 3, "up", "2bf2ec7956e02fc0"),
    (3, 3, "cutaway", "7b69d88981865e1f"),
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
