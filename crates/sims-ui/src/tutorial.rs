//! The tutorial highlight marker: the one animated arrow the UI Effect
//! primitive (`0x44acff`) puts over a control.
//!
//! The game builds a single `cTSWinTutorialHighlight` window (vtable
//! `0x6747b8`, created at `0x4d089d`) and keeps it at `[[0x6fe944] +
//! 0x1f4]`. Its bitmap is UI graphic [`MARKER`], keyed on `[0x6f2ab4]`
//! (`0xf81f`, the magenta [`crate::art::KEY`]) and cut into [`FRAMES`]
//! cells by `0x5e999a(bitmap, 3, 6.0f)`, played at six a second.
//!
//! A control's slot `+0x160` (`0x5de13f`) shows the marker over it or hides
//! it: a highlight on a visible control moves the marker to [`origin`] and
//! shows it; a clear, or a highlight on a hidden control, hides it,
//! whichever control it was over.

use crate::art::Art;
use crate::frame::{Frame, Rect};

/// The marker's bitmap (150×50: three 50×50 cells side by side).
pub const MARKER: i32 = 4980;

/// The cells the bitmap is cut into (`0x5e999a(bitmap, 3, 6.0f)`).
pub const FRAMES: u32 = 3;

/// How long each cell shows: six cells a second.
pub const FRAME_US: u64 = 1_000_000 / 6;

/// The cells in the order they play: up and back down.
const SEQUENCE: [u32; 4] = [0, 1, 2, 1];

/// Where `0x5de13f` puts a `marker`-sized marker over `target`: centred
/// across it (`(target.w - marker.w) / 2`, `cdq; sub; sar` truncating
/// toward zero) and 3 pixels above its top.
///
/// The original's lot buttons on the neighborhood (`0x54f192`) override
/// the slot and move it 50 pixels further; no lot button takes a UI Effect
/// here.
pub fn origin(target: Rect, marker: [i32; 2]) -> [i32; 2] {
    let [width, height] = marker;
    [
        target.x.wrapping_add(target.width.wrapping_sub(width) / 2),
        target.y.wrapping_sub(height).wrapping_sub(3),
    ]
}

/// The cell the marker shows `elapsed_us` after it was shown.
pub fn cell_at(elapsed_us: u64) -> u32 {
    SEQUENCE[((elapsed_us / FRAME_US) % SEQUENCE.len() as u64) as usize]
}

/// Microseconds from `elapsed_us` until the marker shows its next cell.
pub fn next_cell_us(elapsed_us: u64) -> u64 {
    FRAME_US - elapsed_us % FRAME_US
}

/// Draws the marker over `target`, `elapsed_us` after it was shown.
/// Draws nothing without the bitmap, and returns whether it drew.
pub fn draw(frame: &mut Frame, art: &Art, target: Rect, elapsed_us: u64) -> bool {
    let Some(image) = art.image(MARKER) else {
        return false;
    };
    let width = image.width / FRAMES;
    let size = [
        i32::try_from(width).unwrap_or(i32::MAX),
        i32::try_from(image.height).unwrap_or(i32::MAX),
    ];
    let cell = cell_at(elapsed_us);
    let source = Rect::new(
        i32::try_from(width * cell).unwrap_or(i32::MAX),
        0,
        size[0],
        size[1],
    );
    frame.draw(image, source, origin(target, size));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_formats::bitmap::Image;

    #[test]
    fn the_marker_sits_centred_three_pixels_above_its_control() {
        // The 800×600 Zoom In button (26, 522), 27×27, under a 50×50 marker.
        assert_eq!(origin(Rect::new(26, 522, 27, 27), [50, 50]), [15, 469]);
        // A wider control centres the other way; odd differences truncate
        // toward zero, as `cdq; sub eax, edx; sar eax, 1` does.
        assert_eq!(origin(Rect::new(100, 200, 81, 20), [50, 50]), [115, 147]);
        assert_eq!(origin(Rect::new(0, 60, 1, 1), [50, 50]), [-24, 7]);
    }

    #[test]
    fn the_marker_plays_its_cells_up_and_back_down_six_a_second() {
        let cells: Vec<u32> = (0..9).map(|n| cell_at(n * FRAME_US)).collect();
        assert_eq!(cells, [0, 1, 2, 1, 0, 1, 2, 1, 0]);
        assert_eq!(cell_at(FRAME_US - 1), 0);
        assert_eq!(next_cell_us(0), FRAME_US);
        assert_eq!(next_cell_us(FRAME_US - 1), 1);
        assert_eq!(next_cell_us(FRAME_US), FRAME_US);
    }

    #[test]
    fn the_marker_draws_its_cell_keyed_over_the_frame() {
        // Three 2×2 cells: red, green, blue, each with one keyed pixel.
        let mut rgba = Vec::new();
        for _ in 0..2 {
            for color in [[255, 0, 0], [0, 255, 0], [0, 0, 255]] {
                rgba.extend_from_slice(&[color[0], color[1], color[2], 255]);
                rgba.extend_from_slice(&[255, 0, 255, 255]);
            }
        }
        let mut image = Image {
            width: 6,
            height: 2,
            rgba,
        };
        image.key(crate::art::KEY);
        let mut art = Art::default();
        art.insert(MARKER, image);
        let target = Rect::new(4, 10, 2, 2);
        let at = origin(target, [2, 2]);
        assert_eq!(at, [4, 5]);
        for (elapsed, color) in [
            (0, [255, 0, 0]),
            (FRAME_US, [0, 255, 0]),
            (2 * FRAME_US, [0, 0, 255]),
            (3 * FRAME_US, [0, 255, 0]),
        ] {
            let mut frame = Frame::new(12, 12, [7, 7, 7]);
            assert!(draw(&mut frame, &art, target, elapsed));
            assert_eq!(frame.pixel(at[0], at[1]), Some(color));
            assert_eq!(frame.pixel(at[0] + 1, at[1]), Some([7, 7, 7]), "keyed");
        }
        // No bitmap, nothing drawn.
        let mut frame = Frame::new(12, 12, [7, 7, 7]);
        assert!(!draw(&mut frame, &Art::default(), target, 0));
        assert!(frame.rgb().iter().all(|&p| p == [7, 7, 7]));
    }
}
