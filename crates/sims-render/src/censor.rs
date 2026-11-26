//! The censor mosaic: the blocks of averaged colour drawn over a person
//! whose censor word (person data word 30, `+0x5c8`) is set.
//!
//! The original draws it after the people and before the moving objects
//! (`0x4b3510`: `0x4b017b`, then `0x4ac707`, then `0x4aa1d7`). For each
//! person on a level the camera shows (`+0xfe` not above the view's level)
//! with a censor word, `0x4ac707` reads the screen rectangle the person's
//! last draw kept (`+0x230`, [`rect`]), pads it by the zoom and hands it to
//! `0x4ac301` ([`mosaic`]), which averages blocks of 2, 4 or 8 pixels.
//! Only pixels whose depth is farther than the person's upright plane at
//! their row are counted and written, so whatever stands in front of the
//! person keeps its own pixels.
//!
//! The arithmetic is the original's on its 16-bit 5-6-5 surface: a pixel is
//! read with [`unpack`] (channels zero-filled, not widened), each channel's
//! sum is divided by the count (truncating), three random draws add noise
//! (`(draw & 15) - 8`, red, green, then blue: `0x46f457`), and the result is
//! clamped and packed back ([`pack`]).

use crate::camera::{Camera, Zoom};
use crate::canvas::{Canvas, quantize565};
use crate::depth::Depth;
use crate::lot::Lot;

/// A pixel of the lot's 16-bit 5-6-5 surface as `0x4ac301` reads it (its
/// format 2): each channel shifted up, its low bits zero, not widened as
/// [`quantize565`] widens them. The original also reads 5-5-5 and 5-5-6
/// surfaces there; the lot's is 5-6-5, so only that one is ported.
pub fn unpack(pixel: u16) -> [u8; 3] {
    let p = u32::from(pixel);
    let rgb = (((p & 0xf800) << 3 | p & 0x7e0) << 2 | p & 0x1f) << 3;
    [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]
}

/// The inverse of [`unpack`]: the top bits of each channel.
pub fn pack(rgb: [u8; 3]) -> u16 {
    let p = u32::from(rgb[0]) << 16 | u32::from(rgb[1]) << 8 | u32::from(rgb[2]);
    (((p >> 3 & 0x1f0000 | p & 0xfc00) >> 2 | p & 0xf8) >> 3) as u16
}

/// The side of a mosaic block at `zoom`, in pixels (`0x4ac301`).
pub fn block(zoom: Zoom) -> usize {
    match zoom {
        Zoom::Far => 2,
        Zoom::Medium => 4,
        Zoom::Near => 8,
    }
}

/// A person to censor this frame, as the people's draw saw them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Censored {
    /// The person's object ID.
    pub id: u16,
    /// The least and greatest screen x and y of the person's censor boxes'
    /// posed vertices (`[min x, min y, max x, max y]`, canvas pixels), or
    /// None when no box is chosen (the rectangle `+0x230` stays unset,
    /// left `0x7fffffff`).
    pub bounds: Option<[f32; 4]>,
    /// The person's position (`+0x240`), in the lot renderer's units
    /// (`[fine x, fine y, altitude]`), which anchors the depth test.
    pub position: [f32; 3],
    /// The person's level (1 the ground floor).
    pub level: i32,
}

/// The original keeps the person's screen rectangle with an offset
/// (`[0x6f6d74]`, `[0x6d0904]`), each shifted by the zoom less one: 0 and
/// -3. Only `0x591248` reads them, and nothing writes them.
const RECT_OFFSET: [i32; 2] = [0, -3];

/// The rectangle `0x4ac707` mosaics for a person whose box bounds are
/// `bounds`, as `[left, top, right, bottom]` (right and bottom exclusive):
/// the bounds cut to whole pixels and offset (`0x591248`..`0x5912b4`: the
/// least down, the greatest up by `+ 0.999`), then padded by 1, 2 or 4
/// pixels and moved down by 3, 6 or 12 (`0x4ac707`), which the offset
/// cancels. None when the rectangle is empty.
pub fn rect(bounds: [f32; 4], zoom: Zoom) -> Option<[i32; 4]> {
    if bounds.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let shift = zoom.level() - 1;
    let [ox, oy] = RECT_OFFSET.map(|o| o << shift);
    let left = (bounds[0] as i32).saturating_add(ox);
    let top = (bounds[1] as i32).saturating_add(oy);
    let right = ((bounds[2] + 0.999) as i32).saturating_add(ox);
    let bottom = ((bounds[3] + 0.999) as i32).saturating_add(oy);
    if left >= right || top >= bottom {
        return None;
    }
    let pad = 1 << shift;
    let down = 3 << shift;
    Some([
        left.saturating_sub(pad),
        top.saturating_sub(pad).saturating_add(down),
        right.saturating_add(pad),
        bottom.saturating_add(pad).saturating_add(down),
    ])
}

/// Mosaics `rect` (`[left, top, right, bottom]`, cut to `clip`) of `canvas`
/// at `zoom` (`0x4ac301`). A pixel takes part when its depth is greater
/// (farther) than `threshold(row)`. As the original, the row it asks about
/// is the clipped rectangle's top plus the row within the block, for every
/// block row (`0x4ac4c8`, `0x4ac649`). Each block with a pixel that takes
/// part draws three numbers from `random`. Returns the pixels written.
pub fn mosaic(
    canvas: &mut Canvas,
    rect: [i32; 4],
    clip: [i32; 4],
    zoom: Zoom,
    threshold: impl Fn(i32) -> i32,
    mut random: impl FnMut() -> u32,
) -> usize {
    let width = canvas.width as i32;
    let height = canvas.height as i32;
    let left = rect[0].max(clip[0]).max(0);
    let top = rect[1].max(clip[1]).max(0);
    let right = rect[2].min(clip[2]).min(width);
    let bottom = rect[3].min(clip[3]).min(height);
    if left >= right || top >= bottom {
        return 0;
    }
    let (left, top, right, bottom) = (left as usize, top as usize, right as usize, bottom as usize);
    let cell = block(zoom);
    let rows: Vec<i32> = (0..cell).map(|dy| threshold((top + dy) as i32)).collect();
    let mut written = 0;
    for by in (top..bottom).step_by(cell) {
        for bx in (left..right).step_by(cell) {
            let (bw, bh) = (cell.min(right - bx), cell.min(bottom - by));
            let takes_part = |canvas: &Canvas, dx: usize, dy: usize| -> Option<usize> {
                let i = (by + dy) * canvas.width + bx + dx;
                (rows[dy] < i32::from(canvas.depth[i])).then_some(i)
            };
            let mut sum = [0i32; 3];
            let mut count = 0;
            for dy in 0..bh {
                for dx in 0..bw {
                    if let Some(i) = takes_part(canvas, dx, dy) {
                        let rgb = canvas.pixel_rgb(i % canvas.width, i / canvas.width);
                        let rgb = unpack(pack(rgb));
                        for c in 0..3 {
                            sum[c] += i32::from(rgb[c]);
                        }
                        count += 1;
                    }
                }
            }
            if count == 0 {
                continue;
            }
            let colour = sum.map(|s| ((random() & 15) as i32 + s / count - 8).clamp(0, 255) as u8);
            let shown = quantize565(unpack(pack(colour)));
            for dy in 0..bh {
                for dx in 0..bw {
                    if let Some(i) = takes_part(canvas, dx, dy) {
                        canvas.set_pixel_rgb(i % canvas.width, i / canvas.width, shown);
                        written += 1;
                    }
                }
            }
        }
    }
    written
}

/// Draws the censor mosaic over each of `people` that `camera` shows, in
/// their order, into `canvas`, which holds the lot and the people as drawn
/// with `camera`, cut to the house view's `viewport`. `random` draws from
/// the stream the original draws from (`0x46f457`, the global stream).
/// Returns the pixels written, and pushes onto `rects` each mosaicked
/// rectangle (`[x0, y0, x1, y1)`, clipped) that changed a pixel.
pub fn draw_censor(
    lot: &Lot,
    camera: &Camera,
    viewport: (u32, u32),
    people: &[Censored],
    mut random: impl FnMut() -> u32,
    canvas: &mut Canvas,
    rects: &mut Vec<crate::canvas::Rect>,
) -> usize {
    let depth = Depth::new(camera);
    let clip = [
        0,
        0,
        i32::try_from(viewport.0).unwrap_or(i32::MAX),
        i32::try_from(viewport.1).unwrap_or(i32::MAX),
    ];
    let mut written = 0;
    for person in people {
        if person.level > camera.level {
            continue;
        }
        let Some(rect) = person.bounds.and_then(|b| rect(b, camera.zoom)) else {
            continue;
        };
        let [x, y, altitude] = person.position;
        if !(x.is_finite() && y.is_finite() && altitude.is_finite()) {
            continue;
        }
        // 0x4b966b → 0x4b9991: the person's position placed as a moved
        // object, whose upright plane is the threshold.
        let fine = [(x + 0.5).trunc() as i32, (y + 0.5).trunc() as i32];
        let (_, anchor) =
            crate::scene::place_moved(&depth, camera, lot, fine, person.level, altitude);
        let changed = mosaic(
            canvas,
            rect,
            clip,
            camera.zoom,
            |row| depth.stand(anchor, row),
            &mut random,
        );
        if changed > 0 {
            rects.push([
                rect[0].max(0),
                rect[1].max(0),
                rect[2].min(clip[2]).min(canvas.width as i32),
                rect[3].min(clip[3]).min(canvas.height as i32),
            ]);
        }
        written += changed;
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_and_unpack_keep_the_top_bits() {
        assert_eq!(unpack(0xffff), [0xf8, 0xfc, 0xf8]);
        assert_eq!(unpack(0x8410), [0x80, 0x80, 0x80]);
        assert_eq!(unpack(0x07e0), [0, 0xfc, 0]);
        for p in [0u16, 1, 0x1f, 0x7e0, 0xf800, 0x8410, 0xabcd, 0xffff] {
            assert_eq!(pack(unpack(p)), p);
            // The canvas's widened colour packs back to the same pixel.
            assert_eq!(pack(quantize565(unpack(p))), p);
        }
    }

    #[test]
    fn the_rectangle_is_the_bounds_padded_by_the_zoom() {
        let b = [10.2, 20.7, 30.0, 40.5];
        // Far: pad 1; the -3 offset cancels the 3 down.
        assert_eq!(rect(b, Zoom::Far), Some([9, 19, 31, 42]));
        // Medium: pad 2, offset -6, down 6.
        assert_eq!(rect(b, Zoom::Medium), Some([8, 18, 32, 43]));
        // Near: pad 4.
        assert_eq!(rect(b, Zoom::Near), Some([6, 16, 34, 45]));
        // A whole maximum stays whole: 4 + 0.999 cuts to 4.
        assert_eq!(rect([0.0, 0.0, 4.0, 4.0], Zoom::Far), Some([-1, -1, 5, 5]));
        // A rectangle no wider than a point is empty.
        assert_eq!(rect([5.0, 5.0, 5.0, 9.0], Zoom::Far), None);
        assert_eq!(rect([5.0, 5.0, f32::NAN, 9.0], Zoom::Far), None);
    }

    fn canvas(w: usize, h: usize) -> Canvas {
        let mut c = Canvas::new(w, h, [0, 0, 0]);
        for y in 0..h {
            for x in 0..w {
                c.set_pixel_rgb(x, y, quantize565([(x * 16) as u8, (y * 16) as u8, 0x80]));
                c.depth[y * w + x] = 1000;
            }
        }
        c
    }

    #[test]
    fn a_block_averages_the_pixels_behind_the_threshold() {
        let mut c = canvas(4, 4);
        // A nearer pixel keeps its colour and is not counted.
        c.depth[5] = 10;
        let kept = c.pixel_rgb(1, 1);
        let mut draws = vec![8u32, 8, 8 + 16].into_iter();
        let written = mosaic(
            &mut c,
            [0, 0, 4, 4],
            [0, 0, 4, 4],
            Zoom::Medium,
            |_| 500,
            || draws.next().unwrap(),
        );
        assert_eq!(written, 15);
        assert_eq!(c.pixel_rgb(1, 1), kept);
        // Red: x*16 summed over 15 pixels; green: y*16; blue 0x80.
        let mut sum = [0i32; 3];
        for i in (0..16).filter(|&i| i != 5) {
            let (x, y) = (i % 4, i / 4);
            let rgb = unpack(pack(quantize565([(x * 16) as u8, (y * 16) as u8, 0x80])));
            for ch in 0..3 {
                sum[ch] += i32::from(rgb[ch]);
            }
        }
        let want = quantize565(unpack(pack(sum.map(|s| (s / 15) as u8))));
        assert_eq!(c.pixel_rgb(0, 0), want);
        assert_eq!(c.pixel_rgb(3, 3), want);
    }

    #[test]
    fn blocks_draw_three_numbers_each_and_skip_empty_ones() {
        let mut c = canvas(8, 4);
        // The right block is all in front: no draws.
        for y in 0..4 {
            for x in 4..8 {
                c.depth[y * 8 + x] = 0;
            }
        }
        let mut n = 0;
        let written = mosaic(
            &mut c,
            [0, 0, 8, 4],
            [0, 0, 8, 4],
            Zoom::Medium,
            |_| 500,
            || {
                n += 1;
                0
            },
        );
        assert_eq!((written, n), (16, 3));
    }

    #[test]
    fn the_threshold_row_restarts_at_the_clipped_top_for_every_block_row() {
        let mut c = canvas(2, 6);
        let asked = std::cell::RefCell::new(vec![]);
        mosaic(
            &mut c,
            [0, -1, 2, 6],
            [0, 0, 2, 6],
            Zoom::Far,
            |row| {
                asked.borrow_mut().push(row);
                // Only rows 0 and 1 count; rows 2.. would not, but they
                // are never asked.
                if row < 2 { 0 } else { 5000 }
            },
            || 8,
        );
        assert_eq!(*asked.borrow(), vec![0, 1]);
        // Every block row was mosaicked: the bottom block is one colour.
        assert_eq!(c.pixel_rgb(0, 4), c.pixel_rgb(1, 5));
    }

    #[test]
    fn noise_and_clamping() {
        let mut c = Canvas::new(2, 2, [0, 0, 0]);
        for i in 0..4 {
            c.set_pixel_rgb(i % 2, i / 2, [0, 0, 0]);
            c.depth[i] = 9;
        }
        let mut draws = vec![0u32, 15, 0xfff3].into_iter();
        mosaic(
            &mut c,
            [0, 0, 2, 2],
            [0, 0, 2, 2],
            Zoom::Far,
            |_| 0,
            || draws.next().unwrap(),
        );
        // -8 clamps to 0; +7; 3 - 8 clamps to 0.
        assert_eq!(c.pixel_rgb(0, 0), quantize565([0, 4, 0]));
    }
}
