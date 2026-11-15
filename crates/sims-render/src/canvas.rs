//! An image with a depth buffer, and the one way sprites are drawn into it.
//!
//! Depth follows the original's sense: a smaller value is closer to the
//! camera, and a pixel that ties with what is there replaces it, so a later
//! draw wins (the `<=` test of the old prototype's reading of the sprite
//! blitters, `legacy/crates/sims-sim/src/lot_presenter/sprite2_clipped.rs`).
//!
//! Depth is the original's 16-bit depth (see [`crate::depth`]): what a
//! pixel stores is held to 0 to 65535, where the original keeps the low
//! 16 bits; the two differ only off the original's tables.
//!
//! # Which object drew a pixel
//!
//! The canvas also keeps, for each pixel, the ID of the object or person
//! that drew it last, for finding what a click hit:
//! [`Canvas::id_at`]. Whatever draws sets the pen first with
//! [`Canvas::set_id`]; every pixel the canvas takes after that (one that
//! passes the depth test and is not fainter than [`FAINT`], blended or
//! not) takes the pen's ID with it. [`crate::draw`] gives each object
//! sprite its object ID (the key in `World::objects`) and terrain, floors,
//! walls and trims none; `sims_people::People::draw_on_lot` gives each
//! person's pixels the person's object ID. So after the lot and its people
//! are drawn, `id_at` names the object or person shown at a pixel, and a
//! person or object hidden behind a wall is named nowhere. Each tile of a
//! multi-tile object is its own object and names itself.

pub use crate::depth::EMPTY;
use crate::light::Light;

/// The faintest alpha drawn: 8 of the files' 31 steps (8 * 255 / 31,
/// rounded down). The original leaves out a sprite's fainter edge pixels,
/// colour and depth both; the threshold is fitted to its frame buffer and
/// depth buffer at three cameras, where it lifts the depth
/// match from 98.0% to 99.1% (medium zoom) and 98.9% to 99.7% (far zoom),
/// and puts the door's left jamb back. Not read from the code.
pub const FAINT: u8 = 65;

/// A decoded sprite frame: straight-alpha RGBA and, if it has one, a depth
/// byte per pixel (255 where nothing is drawn).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    pub depth: Option<Vec<u8>>,
}

impl Image {
    /// An image with nothing drawn.
    pub fn empty() -> Self {
        Self {
            width: 0,
            height: 0,
            rgba: Vec::new(),
            depth: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The alpha of a pixel.
    pub fn alpha(&self, x: usize, y: usize) -> u8 {
        self.rgba[(y * self.width + x) * 4 + 3]
    }

    /// This image with each pixel's alpha scaled by `mask`'s alpha. The
    /// two must be the same size; the original's wall shapes and wall
    /// patterns always are.
    pub fn masked(&self, mask: &Image) -> Option<Self> {
        if (self.width, self.height) != (mask.width, mask.height) {
            return None;
        }
        let mut rgba = self.rgba.clone();
        for (p, m) in rgba
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(mask.rgba.as_chunks::<4>().0)
        {
            p[3] = ((u16::from(p[3]) * u16::from(m[3])) / 255) as u8;
        }
        Some(Self {
            rgba,
            ..self.clone()
        })
    }
}

impl From<sims_formats::sprite::Frame> for Image {
    fn from(frame: sims_formats::sprite::Frame) -> Self {
        // SPR2 flags bit 1 (value 2) says the frame has a depth channel.
        let depth = (frame.flags & 2 != 0 && !frame.depth.is_empty()).then_some(frame.depth);
        Self {
            width: frame.width as usize,
            height: frame.height as usize,
            rgba: frame.rgba,
            depth,
        }
    }
}

/// An RGB image with a depth per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub color: Vec<[u8; 3]>,
    pub depth: Vec<i32>,
    /// For each pixel, the ID of what drew it last ([`Canvas::id_at`]).
    ids: Vec<Option<u16>>,
    /// The ID the pixels drawn now take ([`Canvas::set_id`]).
    pen: Option<u16>,
}

impl Canvas {
    pub fn new(width: usize, height: usize, background: [u8; 3]) -> Self {
        Self {
            width,
            height,
            color: vec![background; width * height],
            depth: vec![EMPTY; width * height],
            ids: vec![None; width * height],
            pen: None,
        }
    }

    /// Clears to `background`, empty depth and no IDs, keeping the size,
    /// and puts the pen down (no ID).
    pub fn clear(&mut self, background: [u8; 3]) {
        self.color.fill(background);
        self.depth.fill(EMPTY);
        self.ids.fill(None);
        self.pen = None;
    }

    /// Sets the ID the pixels drawn from now on take, until it is set
    /// again: an object's or a person's object ID, or None for what is
    /// nobody's (terrain, floors, walls). See [the module
    /// docs](self#which-object-drew-a-pixel).
    pub fn set_id(&mut self, id: Option<u16>) {
        self.pen = id;
    }

    /// The ID of the object or person that drew the pixel at (`x`, `y`)
    /// last, if any did and the pixel is on the canvas: what a click there
    /// hit.
    pub fn id_at(&self, x: i32, y: i32) -> Option<u16> {
        self.ids[self.index(x, y)?]
    }

    /// Draws `image` with its top-left corner at `at`, mirrored left to
    /// right if `flip`. `depth` gives each pixel's depth from its screen
    /// position and its depth byte (255 for an image without depth).
    ///
    /// A pixel that is not fully transparent and passes the depth test
    /// writes its depth. A fully opaque one also writes its color; a partly
    /// transparent one blends its color over what is there. So a farther
    /// pixel drawn later never shows through a nearer soft edge, as in the
    /// old prototype's blitter (`legacy/crates/sims-render`).
    ///
    /// `at` may be anywhere: the image is clipped to the canvas with wide
    /// arithmetic, so far-off positions draw nothing rather than overflow.
    pub fn draw(
        &mut self,
        image: &Image,
        at: [i32; 2],
        flip: bool,
        depth: impl Fn(i32, i32, u8) -> i32,
    ) {
        self.draw_lit(image, at, flip, Light::FULL, |x, y, b| Some(depth(x, y, b)));
    }

    /// [`Canvas::draw`] under a light: each pixel's color is darkened by
    /// `light` before it is written or blended. A pixel whose depth is
    /// None is not drawn, nor is one fainter than [`FAINT`].
    pub fn draw_lit(
        &mut self,
        image: &Image,
        at: [i32; 2],
        flip: bool,
        light: Light,
        depth: impl Fn(i32, i32, u8) -> Option<i32>,
    ) {
        self.blit(image, at, flip, |_, _| light, 0, depth);
    }

    /// Draws an image whose light changes from pixel to pixel: `light`
    /// gives it for each image pixel (column, row), as a lamp's pool does
    /// on a floor. Otherwise as [`Canvas::draw_lit`], unflipped.
    pub fn draw_shaded(
        &mut self,
        image: &Image,
        at: [i32; 2],
        light: impl Fn(usize, usize) -> Light,
        depth: impl Fn(i32, i32, u8) -> Option<i32>,
    ) {
        self.blit(image, at, false, light, 0, depth);
    }

    /// Draws a wall: as [`Canvas::draw_lit`], but a pixel up to one unit
    /// farther than what is there still wins, as in the original's wall
    /// blitter (0x4c4907).
    pub fn draw_wall(
        &mut self,
        image: &Image,
        at: [i32; 2],
        light: Light,
        depth: impl Fn(i32, i32) -> i32,
    ) {
        self.blit(
            image,
            at,
            false,
            |_, _| light,
            1,
            |x, y, _| Some(depth(x, y)),
        );
    }

    fn blit(
        &mut self,
        image: &Image,
        at: [i32; 2],
        flip: bool,
        light: impl Fn(usize, usize) -> Light,
        slack: i32,
        depth: impl Fn(i32, i32, u8) -> Option<i32>,
    ) {
        // Clip in i64: the canvas and image sides fit, and so does any i32
        // position plus a side. Inside the canvas everything fits in i32.
        let pixels = image.width.checked_mul(image.height);
        if pixels.and_then(|n| n.checked_mul(4)) != Some(image.rgba.len())
            || pixels == Some(0)
            || self.color.is_empty()
        {
            return;
        }
        let wide = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
        let span = |at: i32, size: usize, limit: usize| {
            let (at, limit) = (i64::from(at), wide(limit));
            let low = at.clamp(0, limit);
            let high = at.saturating_add(wide(size)).clamp(low, limit);
            (low as usize, high as usize, at)
        };
        let (x0, x1, ax) = span(at[0], image.width, self.width);
        let (y0, y1, ay) = span(at[1], image.height, self.height);
        for py in y0..y1 {
            let sy = py as i32;
            let iy = (py as i64 - ay) as usize;
            for px in x0..x1 {
                let sx = px as i32;
                let mut ix = (px as i64 - ax) as usize;
                if flip {
                    ix = image.width - 1 - ix;
                }
                let source = iy * image.width + ix;
                let a = image.rgba[source * 4 + 3];
                if a < FAINT {
                    continue;
                }
                let byte = image
                    .depth
                    .as_ref()
                    .map_or(255, |d| d.get(source).copied().unwrap_or(255));
                let Some(d) = depth(sx, sy, byte) else {
                    continue;
                };
                let target = py * self.width + px;
                if d > self.depth[target].saturating_add(slack) {
                    continue;
                }
                let s = &image.rgba[source * 4..source * 4 + 3];
                let rgb = light(ix, iy).apply([s[0], s[1], s[2]]);
                self.write(target, rgb, a, d);
            }
        }
    }

    /// Writes a pixel that passed its depth test: its depth, and its
    /// colour, blended by `a` if it is not opaque.
    fn write(&mut self, target: usize, rgb: [u8; 3], a: u8, d: i32) {
        let pixel = &mut self.color[target];
        self.depth[target] = d.clamp(0, EMPTY);
        self.ids[target] = self.pen;
        if a == 255 {
            *pixel = rgb;
        } else {
            let a = u32::from(a);
            for (c, &s) in pixel.iter_mut().zip(&rgb) {
                *c = ((u32::from(s) * a + u32::from(*c) * (255 - a) + 127) / 255) as u8;
            }
        }
    }

    /// The index of pixel (`x`, `y`), if it is on the canvas.
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
        (x < self.width && y < self.height).then(|| y * self.width + x)
    }

    /// Whether a sprite pixel at depth `d` on (`x`, `y`) would be drawn:
    /// it is on the canvas and no farther than what is there.
    pub fn passes(&self, x: i32, y: i32, d: i32) -> bool {
        self.index(x, y).is_some_and(|i| d <= self.depth[i])
    }

    /// Draws one straight-alpha pixel at depth `d` by the sprite rule, for
    /// what is drawn over the finished lot a pixel at a time (a person's
    /// mesh, with [`crate::depth::Depth::point`]): left out if fainter
    /// than [`FAINT`] or farther than what is there, otherwise blended and
    /// its depth written, so a nearer part of the same mesh drawn later
    /// covers it. Returns whether it was drawn.
    pub fn put(&mut self, x: i32, y: i32, rgba: [u8; 4], d: i32) -> bool {
        let [r, g, b, a] = rgba;
        if a < FAINT || !self.passes(x, y, d) {
            return false;
        }
        let Some(target) = self.index(x, y) else {
            return false;
        };
        self.write(target, [r, g, b], a, d);
        true
    }

    /// Fills one pixel if `d` passes the depth test.
    pub fn plot(&mut self, x: i32, y: i32, d: i32, rgb: [u8; 3]) {
        if let Some(target) = self.index(x, y)
            && d <= self.depth[target]
        {
            self.write(target, rgb, 255, d);
        }
    }

    /// The image as the original shows it: 16-bit color (5 bits red, 6
    /// green, 5 blue), widened back to 8 bits a channel by repeating the
    /// high bits. The original runs its display in that mode, so a fair
    /// comparison quantizes both sides this way.
    pub fn rgb565(&self) -> Vec<[u8; 3]> {
        self.color.iter().map(|&c| quantize565(c)).collect()
    }

    /// RGBA bytes, row by row, for a window or an image file.
    pub fn rgba(&self) -> Vec<u8> {
        self.color
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect()
    }
}

/// One color through the original's 16-bit display mode.
pub fn quantize565([r, g, b]: [u8; 3]) -> [u8; 3] {
    let (r, g, b) = (r >> 3, g >> 2, b >> 3);
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: usize, height: usize, rgba: [u8; 4], depth: Option<u8>) -> Image {
        Image {
            width,
            height,
            rgba: rgba.repeat(width * height),
            depth: depth.map(|d| vec![d; width * height]),
        }
    }

    #[test]
    fn closer_pixels_win_and_ties_go_to_the_later_draw() {
        let mut c = Canvas::new(4, 4, [0, 0, 0]);
        c.draw(
            &image(2, 2, [255, 0, 0, 255], None),
            [0, 0],
            false,
            |_, _, _| 10,
        );
        c.draw(
            &image(2, 2, [0, 255, 0, 255], None),
            [1, 1],
            false,
            |_, _, _| 20,
        );
        assert_eq!(c.color[5], [255, 0, 0]);
        assert_eq!(c.color[10], [0, 255, 0]);
        c.draw(
            &image(1, 1, [0, 0, 255, 255], None),
            [1, 1],
            false,
            |_, _, _| 10,
        );
        assert_eq!(c.color[5], [0, 0, 255]);
        assert_eq!(c.depth[5], 10);
    }

    #[test]
    fn partial_alpha_blends_and_hides_what_is_drawn_behind_later() {
        let mut c = Canvas::new(1, 1, [0, 0, 0]);
        c.draw(
            &image(1, 1, [255, 255, 255, 128], None),
            [0, 0],
            false,
            |_, _, _| 5,
        );
        assert_eq!(c.color[0], [128, 128, 128]);
        assert_eq!(c.depth[0], 5);
        // Farther and opaque, drawn later: hidden by the soft edge.
        c.draw(
            &image(1, 1, [0, 255, 0, 255], None),
            [0, 0],
            false,
            |_, _, _| 10,
        );
        assert_eq!(c.color[0], [128, 128, 128]);
        // Nearer: drawn over it.
        c.draw(
            &image(1, 1, [0, 255, 0, 255], None),
            [0, 0],
            false,
            |_, _, _| 1,
        );
        assert_eq!(c.color[0], [0, 255, 0]);
    }

    #[test]
    fn far_off_positions_draw_nothing_and_do_not_overflow() {
        let mut c = Canvas::new(2, 2, [0, 0, 0]);
        let img = image(3, 3, [1, 2, 3, 255], None);
        for at in [
            [i32::MAX, 0],
            [0, i32::MAX],
            [i32::MIN, i32::MIN],
            [i32::MAX, i32::MIN],
            [i32::MAX - 1, i32::MAX - 2],
        ] {
            c.draw(&img, at, false, |_, _, _| 0);
            c.draw(&img, at, true, |_, _, _| 0);
        }
        assert_eq!(c, Canvas::new(2, 2, [0, 0, 0]));
        // An image whose pixels are fewer than its size says draws nothing,
        // and so does one whose size overflows.
        let mut short = img.clone();
        short.rgba.truncate(4);
        c.draw(&short, [0, 0], false, |_, _, _| 0);
        let huge = Image {
            width: 1 << (usize::BITS - 2),
            height: 1,
            rgba: Vec::new(),
            depth: None,
        };
        c.draw(&huge, [0, 0], false, |_, _, _| 0);
        assert_eq!(c, Canvas::new(2, 2, [0, 0, 0]));
        // A canvas with no pixels takes any image.
        let mut empty = Canvas::new(usize::MAX, 0, [0, 0, 0]);
        empty.draw(&img, [0, 0], false, |_, _, _| 0);
        empty.draw(&huge, [0, 0], false, |_, _, _| 0);
    }

    #[test]
    fn pixels_fainter_than_the_cutoff_are_not_drawn() {
        let mut c = Canvas::new(1, 1, [0, 0, 0]);
        c.draw(
            &image(1, 1, [255, 255, 255, FAINT - 1], None),
            [0, 0],
            false,
            |_, _, _| 5,
        );
        assert_eq!((c.color[0], c.depth[0]), ([0, 0, 0], EMPTY));
        c.draw(
            &image(1, 1, [255, 255, 255, FAINT], None),
            [0, 0],
            false,
            |_, _, _| 5,
        );
        assert_ne!(c.color[0], [0, 0, 0]);
        assert_eq!(c.depth[0], 5);
        // File alpha 8 of 31 (8 * 255 / 31 = 65) is the faintest drawn.
        assert_eq!(FAINT, (8 * 255 / 31) as u8);
    }

    #[test]
    fn walls_win_up_to_one_unit_behind_and_sprites_only_at_ties() {
        let wall = image(1, 1, [0, 255, 0, 255], None);
        for (d, wins) in [(9, true), (10, true), (11, true), (12, false)] {
            let mut c = Canvas::new(1, 1, [0, 0, 0]);
            c.draw(
                &image(1, 1, [255, 0, 0, 255], None),
                [0, 0],
                false,
                |_, _, _| 10,
            );
            c.draw_wall(&wall, [0, 0], Light::FULL, |_, _| d);
            let expected = if wins {
                ([0, 255, 0], d)
            } else {
                ([255, 0, 0], 10)
            };
            assert_eq!((c.color[0], c.depth[0]), expected, "wall at {d}");
        }
        for (d, wins) in [(9, true), (10, true), (11, false)] {
            let mut c = Canvas::new(1, 1, [0, 0, 0]);
            c.draw(
                &image(1, 1, [255, 0, 0, 255], None),
                [0, 0],
                false,
                |_, _, _| 10,
            );
            c.draw(&wall, [0, 0], false, |_, _, _| d);
            let expected = if wins {
                ([0, 255, 0], d)
            } else {
                ([255, 0, 0], 10)
            };
            assert_eq!((c.color[0], c.depth[0]), expected, "sprite at {d}");
        }
        // A pixel whose depth function gives None is not drawn.
        let mut c = Canvas::new(1, 1, [0, 0, 0]);
        c.draw_lit(&wall, [0, 0], false, Light::FULL, |_, _, _| None);
        assert_eq!(c, Canvas::new(1, 1, [0, 0, 0]));
    }

    #[test]
    fn put_draws_one_pixel_by_the_sprite_rule() {
        let mut c = Canvas::new(2, 2, [0, 0, 0]);
        c.plot(1, 0, 100, [10, 20, 30]);
        // Farther than what is there, too faint, or off the canvas: nothing.
        assert!(!c.put(1, 0, [255, 255, 255, 255], 101));
        assert!(!c.put(0, 0, [255, 255, 255, FAINT - 1], 5));
        for (x, y) in [(-1, 0), (0, -1), (2, 0), (0, 2), (i32::MAX, i32::MIN)] {
            assert!(!c.passes(x, y, 0));
            assert!(!c.put(x, y, [255, 255, 255, 255], 0));
        }
        assert_eq!(c.color, vec![[0, 0, 0], [10, 20, 30], [0, 0, 0], [0, 0, 0]]);
        assert_eq!(c.depth, vec![EMPTY, 100, EMPTY, EMPTY]);
        // A tie wins; a soft pixel blends and writes its depth.
        assert!(c.passes(1, 0, 100) && !c.passes(1, 0, 101));
        assert!(c.put(1, 0, [255, 255, 255, 255], 100));
        assert!(c.put(0, 1, [255, 255, 255, 128], -7));
        assert_eq!(c.color[1], [255, 255, 255]);
        assert_eq!((c.color[2], c.depth[2]), ([128, 128, 128], 0));
    }

    #[test]
    fn depth_bytes_reach_the_depth_function_and_flips_mirror() {
        let mut c = Canvas::new(3, 1, [0, 0, 0]);
        let mut img = image(3, 1, [9, 9, 9, 255], Some(0));
        img.rgba[0] = 200;
        img.depth.as_mut().unwrap()[0] = 7;
        c.draw(&img, [0, 0], true, |x, _, b| x * 100 + i32::from(b));
        assert_eq!(c.color[2], [200, 9, 9]);
        assert_eq!(c.depth, vec![0, 100, 207]);
    }

    #[test]
    fn drawing_clips_at_the_edges() {
        let mut c = Canvas::new(2, 2, [0, 0, 0]);
        c.draw(
            &image(3, 3, [1, 2, 3, 255], None),
            [-1, -2],
            false,
            |_, _, _| 0,
        );
        assert_eq!(c.color, vec![[1, 2, 3], [1, 2, 3], [0, 0, 0], [0, 0, 0]]);
    }

    /// Each pixel's light comes from its place in the image, also where
    /// the image is clipped.
    #[test]
    fn shaded_pixels_take_the_light_of_their_place_in_the_image() {
        let mut c = Canvas::new(2, 1, [0, 0, 0]);
        let dark = Light::from_byte(0);
        c.draw_shaded(
            &image(3, 1, [200, 200, 200, 255], None),
            [-1, 0],
            |x, y| if (x, y) == (2, 0) { Light::FULL } else { dark },
            |_, _, _| Some(0),
        );
        assert_eq!(c.color[0], dark.apply([200; 3]));
        assert_eq!(c.color[1], [200; 3]);
        assert_ne!(c.color[0], c.color[1]);
    }

    #[test]
    fn each_pixel_names_what_drew_it_last() {
        let mut c = Canvas::new(3, 1, [0, 0, 0]);
        // Nobody's floor, then object 7 over two pixels.
        c.draw(
            &image(3, 1, [1, 1, 1, 255], None),
            [0, 0],
            false,
            |_, _, _| 50,
        );
        c.set_id(Some(7));
        c.draw(
            &image(2, 1, [2, 2, 2, 255], None),
            [0, 0],
            false,
            |_, _, _| 40,
        );
        // Person 9 behind object 7 on pixel 0, soft in front on pixel 1,
        // and too faint to count on pixel 2.
        c.set_id(Some(9));
        assert!(!c.put(0, 0, [3, 3, 3, 255], 45));
        assert!(c.put(1, 0, [3, 3, 3, 128], 30));
        assert!(!c.put(2, 0, [3, 3, 3, FAINT - 1], 0));
        assert_eq!(
            [c.id_at(0, 0), c.id_at(1, 0), c.id_at(2, 0)],
            [Some(7), Some(9), None]
        );
        // Off the canvas names nothing; a pen set to None draws nobody's
        // pixels; clearing forgets every ID and the pen.
        assert_eq!([c.id_at(-1, 0), c.id_at(3, 0), c.id_at(0, 1)], [None; 3]);
        c.set_id(None);
        c.plot(1, 0, 0, [4, 4, 4]);
        assert_eq!(c.id_at(1, 0), None);
        c.set_id(Some(7));
        c.clear([0, 0, 0]);
        c.plot(2, 0, 0, [4, 4, 4]);
        assert_eq!([c.id_at(0, 0), c.id_at(2, 0)], [None, None]);
    }

    #[test]
    fn quantizing_keeps_the_extremes() {
        assert_eq!(quantize565([255, 255, 255]), [255, 255, 255]);
        assert_eq!(quantize565([0, 0, 0]), [0, 0, 0]);
        assert_eq!(quantize565([0x84, 0x82, 0x84]), [0x84, 0x82, 0x84]);
    }
}
