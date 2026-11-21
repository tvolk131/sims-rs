//! The strip on a marked edge-on diagonal, drawn where the original draws
//! it: House02's cutaway at far zoom, the camera of the screenshot it was
//! compared with (`docs/decisions/20261120-edge-on-strip-ported.md`).
//! Needs the game: set `SIMS_INSTALL` to the install folder.

use sims_render::{Camera, Canvas, Focus, Lot, Walls, Zoom, draw, open_house};
use std::path::PathBuf;

/// The original's mean colour (16-bit, read as 8-bit) across the strip's
/// four columns, 557 to 560, on rows 80 to 100: the strip's top row is
/// 80, and the pattern's first row, hidden by shape 9's transparent first
/// row, is not drawn.
const ORIGINAL: [[i32; 3]; 21] = [
    [153, 123, 86],
    [139, 109, 77],
    [143, 115, 84],
    [137, 107, 73],
    [143, 107, 71],
    [145, 109, 71],
    [139, 103, 65],
    [148, 109, 71],
    [152, 115, 78],
    [143, 111, 75],
    [131, 99, 63],
    [129, 95, 61],
    [139, 107, 69],
    [143, 109, 71],
    [143, 109, 73],
    [150, 115, 76],
    [139, 105, 69],
    [135, 101, 65],
    [135, 101, 61],
    [137, 103, 63],
    [133, 99, 61],
];

#[test]
fn the_edge_on_strip_starts_on_the_originals_row_and_shows_its_colours() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        eprintln!("skipping: set SIMS_INSTALL to render House02");
        return;
    };
    let house = root.join("UserData/Houses/House02.iff");
    let mut opened = open_house(&root, &house).unwrap();
    let lot = Lot::from_world(&opened.world).unwrap();
    let mut focus = Focus::default();
    focus.visit(2);
    let camera = Camera {
        zoom: Zoom::Far,
        rotation: 0,
        level: 1,
        origin: [511, -246],
        walls: Walls::Cutaway,
        focus,
    };
    let mut canvas = Canvas::new(800, 600, [0, 0, 0]);
    draw(
        &opened.world,
        &lot,
        &mut opened.assets,
        &camera,
        &mut canvas,
    );
    let pixels = canvas.rgb565();
    let mean = |y: usize| -> [i32; 3] {
        let mut sum = [0; 3];
        for x in 557..561 {
            for (s, c) in sum.iter_mut().zip(pixels[y * 800 + x]) {
                *s += i32::from(c);
            }
        }
        sum.map(|s| s / 4)
    };
    // Unlifted, the strip would start 24 rows lower; with its depth
    // wrong, the floor behind would cover it; with the mask lined up at
    // the bottom, row 79 would show the pattern's first row.
    let above = mean(79);
    assert!(
        above[0] < 100,
        "row 79 is {above:?}: the strip starts above the original's"
    );
    for (row, want) in (80..).zip(ORIGINAL) {
        let got = mean(row);
        assert!(
            got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 3),
            "row {row}: {got:?}, the original's {want:?}"
        );
    }
}
