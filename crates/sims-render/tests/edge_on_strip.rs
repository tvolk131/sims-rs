//! The strip on a marked edge-on diagonal, drawn where the original draws
//! it: House02's cutaway at far zoom, the camera of the screenshot it was
//! compared with (`docs/decisions/20261120-edge-on-strip-ported.md`), and
//! at medium zoom with the rooms' masks kept from far zoom, as the
//! original's were for its shot (`docs/decisions/20261120-kept-cutaway-masks.md`).
//! Needs the game: set `SIMS_INSTALL` to the install folder.

use sims_render::{Camera, Canvas, Focus, Lot, Walls, Zoom, draw, open_house};
use std::path::PathBuf;

/// The original's mean colour (16-bit, read as 8-bit) across the strip's
/// four columns, 557 to 560, on rows 80 to 111: rows 80 to 100 show the
/// lifted edge-on strip (pattern 252 in shape 9, whose transparent first row
/// hides row 79), and rows 101 to 111 show corner `(27, 24)`'s mid-run
/// standing diagonal trim (`0x50`, frame 29, `0x53df39`–`0x53df5c`) winning
/// the depth test over the strip.
const ORIGINAL: [[i32; 3]; 32] = [
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
    [160, 121, 84],
    [166, 130, 88],
    [162, 132, 92],
    [154, 117, 78],
    [150, 111, 71],
    [153, 119, 82],
    [160, 129, 92],
    [154, 123, 86],
    [147, 113, 76],
    [154, 115, 74],
    [150, 111, 67],
];

/// The same at medium zoom, across the strip's eight columns, 715 to
/// 722, on rows 102 to 122.
const ORIGINAL_MEDIUM: [[i32; 3]; 21] = [
    [138, 108, 76],
    [135, 105, 74],
    [124, 95, 66],
    [128, 100, 71],
    [126, 101, 72],
    [124, 94, 66],
    [129, 101, 70],
    [127, 99, 70],
    [121, 94, 63],
    [124, 94, 64],
    [125, 96, 65],
    [127, 98, 67],
    [124, 96, 66],
    [120, 92, 63],
    [118, 91, 63],
    [112, 87, 60],
    [115, 86, 60],
    [116, 86, 56],
    [116, 88, 58],
    [124, 96, 65],
    [122, 94, 64],
];

/// House02 drawn with the walls cut away for `rooms` (newest first), or
/// None without `SIMS_INSTALL`.
fn render(
    zoom: Zoom,
    origin: [i32; 2],
    rooms: &[i16],
    masks_built_at: Option<Zoom>,
) -> Option<Canvas> {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        eprintln!("skipping: set SIMS_INSTALL to render House02");
        return None;
    };
    let house = root.join("UserData/Houses/House02.iff");
    let mut opened = open_house(&root, &house).unwrap();
    let lot = Lot::from_world(&opened.world).unwrap();
    let mut focus = Focus {
        masks_built_at,
        ..Focus::default()
    };
    for &room in rooms.iter().rev() {
        focus.visit(room);
    }
    let camera = Camera {
        zoom,
        rotation: 0,
        level: 1,
        origin,
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
    Some(canvas)
}

/// The mean colour of `columns` on row `y`.
fn mean(canvas: &Canvas, columns: std::ops::Range<usize>, y: usize) -> [i32; 3] {
    let pixels = canvas.rgb565();
    let mut sum = [0; 3];
    let n = columns.len() as i32;
    for x in columns {
        for (s, c) in sum.iter_mut().zip(pixels[y * 800 + x]) {
            *s += i32::from(c);
        }
    }
    sum.map(|s| s / n)
}

fn near(got: [i32; 3], want: [i32; 3]) -> bool {
    got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 3)
}

#[test]
fn the_edge_on_strip_starts_on_the_originals_row_and_shows_its_colours() {
    let Some(canvas) = render(Zoom::Far, [511, -246], &[2], None) else {
        return;
    };
    let mean = |y: usize| mean(&canvas, 557..561, y);
    // Through row 100 the colours don't show where depth is read: on rows
    // 80 to 100 the strip wins the depth test either way (from row 101 to
    // 111, corner (27, 24)'s standing trim drawn through 0x53e38f–0x53e3b4
    // wins over it). So rows 80, 90, and 100 pin depth directly: read at
    // the row drawn (0x49a5d3), as the decision records. Read at the
    // unlifted row, 24 rows lower, each value would be 347 more.
    for (row, want) in [(80, 44471), (90, 44615), (100, 44760)] {
        for x in 557..561 {
            let got = canvas.depth[row * 800 + x];
            assert_eq!(got, want, "depth at ({x}, {row}): read at the drawn row");
        }
    }
    // Unlifted, the strip would start 24 rows lower; with the mask lined
    // up at the bottom, row 79 would show the pattern's first row.
    let above = mean(79);
    assert!(
        above[0] < 100,
        "row 79 is {above:?}: the strip starts above the original's"
    );
    for (row, want) in (80..).zip(ORIGINAL) {
        let got = mean(row);
        assert!(
            near(got, want),
            "row {row}: {got:?}, the original's {want:?}"
        );
    }
}

/// The original's medium shot was taken after zooming in from far zoom,
/// paused, so its rooms' masks were the ones built at far zoom. Built at
/// medium, room 2's would take one more row of tiles, and the strip would
/// go to another wall where the floor behind hides it.
#[test]
fn at_medium_zoom_masks_kept_from_far_put_the_strip_where_the_original_does() {
    let origin = [623, -550];
    let Some(kept) = render(Zoom::Medium, origin, &[2, 1, 3], Some(Zoom::Far)) else {
        return;
    };
    for (row, want) in (102..).zip(ORIGINAL_MEDIUM) {
        let got = mean(&kept, 715..723, row);
        assert!(
            near(got, want),
            "row {row}: {got:?}, the original's {want:?}"
        );
    }
    let fresh = render(Zoom::Medium, origin, &[2, 1, 3], None).unwrap();
    let got = mean(&fresh, 715..723, 102);
    assert!(
        !near(got, ORIGINAL_MEDIUM[0]),
        "built at medium, row 102 is {got:?}: the masks made no difference"
    );
}
