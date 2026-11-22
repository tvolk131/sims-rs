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
use crate::light::{self, Light};
use std::sync::OnceLock;

/// The faintest alpha drawn: 8 of the files' 31 steps (8 * 255 / 31,
/// rounded down). The original's SPR2 object blitters (`0x4bbab8` at
/// `0x4bbbe4`, `0x4bc080` at `0x4bc13f`, and their clipped/flipped
/// variants) skip pixels whose 5-bit alpha is `<= 7`, leaving both colour
/// and depth untouched.
pub const FAINT: u8 = 65;

/// Empty 16-bit depth stored in [`Canvas::depth`] (`65535`, matching [`EMPTY`]).
pub const EMPTY_DEPTH: u16 = u16::MAX;

/// Sentinel stored in [`Canvas::ids`] when no object or person drew a pixel.
/// Object IDs (`0`..`< 65535`) and neighborhood lot IDs (`< 100`) never equal
/// `u16::MAX`.
pub const ID_NONE: u16 = u16::MAX;

/// Packs 8-bit `[r, g, b]` into `0x00RRGGBB` (matching `softbuffer` directly).
#[inline(always)]
pub const fn pack_rgb([r, g, b]: [u8; 3]) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Unpacks `0x00RRGGBB` back into 8-bit `[r, g, b]`.
#[inline(always)]
pub const fn unpack_rgb(pixel: u32) -> [u8; 3] {
    [(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8]
}

/// Clamps an `i32` depth into the canvas's 16-bit `0..=65535` range.
#[inline(always)]
pub const fn clamp_depth(d: i32) -> u16 {
    if d <= 0 {
        0
    } else if d >= EMPTY {
        EMPTY_DEPTH
    } else {
        d as u16
    }
}

#[inline(always)]
pub(crate) fn encode_id(id: Option<u16>) -> u16 {
    match id {
        Some(v) => {
            assert_ne!(v, ID_NONE, "Canvas pick ID u16::MAX is reserved as ID_NONE");
            v
        }
        None => ID_NONE,
    }
}

#[inline(always)]
pub(crate) const fn decode_id(raw: u16) -> Option<u16> {
    if raw == ID_NONE { None } else { Some(raw) }
}

/// A contiguous horizontal span of active (non-transparent) columns in one
/// sprite row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowSpan {
    /// Inclusive start column in `0..width`.
    pub start: u32,
    /// Exclusive end column in `start..=width`.
    pub end: u32,
    /// True iff every pixel in `start..end` meets the opaque alpha threshold.
    pub all_opaque: bool,
}

/// Precompiled active column spans per row for a decoded [`Image`], allowing
/// blitters to skip transparent margins and interior cutouts (50–80% of
/// isometric floor diamonds, wall masks, and furniture bounding boxes) and
/// specialize opaque runs without per-pixel alpha checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RowSpans {
    pub width: usize,
    pub height: usize,
    runs: Vec<RowSpan>,
    row_offsets: Vec<u32>,
}

impl RowSpans {
    /// Precomputes active runs where `alpha >= min_alpha`, marking runs where
    /// every pixel has `alpha >= opaque_alpha` as `all_opaque`.
    pub(crate) fn compute(image: &Image, min_alpha: u8, opaque_alpha: u8) -> Self {
        let width = image.width;
        let height = image.height;
        let valid =
            width.checked_mul(height).and_then(|n| n.checked_mul(4)) == Some(image.rgba.len());
        if !valid || width == 0 || height == 0 || width > u32::MAX as usize {
            return Self {
                width,
                height,
                runs: Vec::new(),
                row_offsets: vec![0; height.saturating_add(1)],
            };
        }
        let mut runs = Vec::with_capacity(height);
        let mut row_offsets = Vec::with_capacity(height + 1);
        row_offsets.push(0);
        for iy in 0..height {
            let row = &image.rgba[iy * width * 4..(iy + 1) * width * 4];
            let pixels = row.as_chunks::<4>().0;
            let mut x = 0usize;
            while x < width {
                while x < width && pixels[x][3] < min_alpha {
                    x += 1;
                }
                if x >= width {
                    break;
                }
                let start = x;
                let mut all_opaque = true;
                while x < width && pixels[x][3] >= min_alpha {
                    if pixels[x][3] < opaque_alpha {
                        all_opaque = false;
                    }
                    x += 1;
                }
                runs.push(RowSpan {
                    start: start as u32,
                    end: x as u32,
                    all_opaque,
                });
            }
            row_offsets.push(runs.len() as u32);
        }
        Self {
            width,
            height,
            runs,
            row_offsets,
        }
    }

    /// Active spans for standard sprites (`alpha >= FAINT`, opaque at `255`).
    pub(crate) fn for_sprite(image: &Image) -> Self {
        Self::compute(image, FAINT, 255)
    }

    /// Active spans for pond/water overlays (`alpha >= 8`, opaque at `248`).
    pub(crate) fn for_water(image: &Image) -> Self {
        Self::compute(image, 8, 248)
    }

    #[inline]
    pub(crate) fn row(&self, iy: usize) -> &[RowSpan] {
        let start = self.row_offsets[iy] as usize;
        let end = self.row_offsets[iy + 1] as usize;
        &self.runs[start..end]
    }

    #[inline]
    pub(crate) fn row_bounds(&self, iy: usize) -> Option<(usize, usize)> {
        let row = self.row(iy);
        let first = row.first()?;
        let last = row.last()?;
        Some((first.start as usize, last.end as usize))
    }
}

/// Disjoint horizontal row band of a [`Canvas`], enabling 100% safe parallel
/// multi-threaded band rasterization (`std::thread::scope` + `chunks_mut`).
pub(crate) struct CanvasBand<'a> {
    pub width: usize,
    pub height: usize,
    pub y0: usize,
    pub y1: usize,
    pub color: &'a mut [u32],
    pub depth: &'a mut [u16],
    pub ids: &'a mut [u16],
}

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
    /// patterns always are, except on an edge-on diagonal's strip (see
    /// [`Image::masked_top_left`]).
    pub fn masked(&self, mask: &Image) -> Option<Self> {
        if (self.width, self.height) != (mask.width, mask.height) {
            return None;
        }
        self.masked_top_left(mask)
    }

    /// This image masked by a mask at least as large, lined up at their
    /// top-left corners, as the original draws the strip on an edge-on
    /// diagonal: pattern 252 in shape 9, whose edge-on frame is taller.
    /// The blit starts both at their first row (0x4c76d1, 0x4c7fdb), and
    /// the mask's transparent first rows hide the pattern's, as House02's
    /// screenshots show at every zoom. Only this image's rectangle is
    /// drawn, so the mask's rows below it are left out. `None` when the
    /// mask is smaller or either image's pixels do not fill its size.
    pub fn masked_top_left(&self, mask: &Image) -> Option<Self> {
        // A size too large to count gives none rather than overflowing.
        let fills = |image: &Image| {
            image
                .width
                .checked_mul(image.height)
                .and_then(|n| n.checked_mul(4))
                == Some(image.rgba.len())
        };
        if mask.width < self.width || mask.height < self.height || !fills(self) || !fills(mask) {
            return None;
        }
        let mut rgba = self.rgba.clone();
        if !self.is_empty() {
            for (y, row) in rgba.chunks_exact_mut(self.width * 4).enumerate() {
                let start = y * mask.width * 4;
                let mask_row = &mask.rgba[start..start + self.width * 4];
                for (p, m) in row
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(mask_row.as_chunks::<4>().0)
                {
                    p[3] = ((u16::from(p[3]) * u16::from(m[3])) / 255) as u8;
                }
            }
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
    /// Packed `0x00RRGGBB` pixels (matching `softbuffer` directly, 4-byte aligned).
    pub color: Vec<u32>,
    /// 16-bit depth per pixel (`0..=65535`, [`EMPTY_DEPTH`] = empty).
    pub depth: Vec<u16>,
    /// For each pixel, the ID of what drew it last ([`Canvas::id_at`]), with
    /// [`ID_NONE`] (`u16::MAX`) denoting none.
    ids: Vec<u16>,
    /// The ID the pixels drawn now take ([`Canvas::set_id`]), or [`ID_NONE`].
    pen: u16,
    /// For each ID, the screen rectangle its images were drawn into since
    /// the canvas was cleared ([`Canvas::drawn`]).
    rects: std::collections::BTreeMap<u16, Rect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IdRect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    offset: usize,
}

/// Which object or person drew each pixel of a canvas, kept apart from
/// it: what [`Canvas::id_at`] read when it was taken ([`Canvas::ids`],
/// [`Ids::take`], [`Ids::take_rects`]). A frame drawn in passes keeps the IDs
/// of a pass this way before the next draws over them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ids {
    rects: Vec<IdRect>,
    pool: Vec<u16>,
}

impl Ids {
    /// The ID that had drawn the pixel at (`x`, `y`) last when these were
    /// taken, if any had and the pixel is within a captured rectangle.
    pub fn id_at(&self, x: i32, y: i32) -> Option<u16> {
        for r in &self.rects {
            if x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1 {
                let w = (r.x1 - r.x0) as usize;
                let idx = r.offset + (y - r.y0) as usize * w + (x - r.x0) as usize;
                return decode_id(self.pool[idx]);
            }
        }
        None
    }

    /// Takes `canvas`'s full-screen IDs as they are now, reusing this one's storage.
    pub fn take(&mut self, canvas: &Canvas) {
        self.rects.clear();
        self.pool.clear();
        if canvas.width == 0 || canvas.height == 0 || !canvas.has_pixel_storage() {
            return;
        }
        let Ok(w) = i32::try_from(canvas.width) else {
            return;
        };
        let Ok(h) = i32::try_from(canvas.height) else {
            return;
        };
        let pixels = canvas.width * canvas.height;
        self.pool.extend_from_slice(&canvas.ids[..pixels]);
        self.rects.push(IdRect {
            x0: 0,
            y0: 0,
            x1: w,
            y1: h,
            offset: 0,
        });
    }

    /// Takes `canvas`'s IDs only inside `rects` (clipped to the canvas),
    /// reusing this one's backing allocations. For overlapping rects captured
    /// from the same `canvas` snapshot, every rect stores identical pixel IDs
    /// at their intersection so [`Ids::id_at`] agrees bit-for-bit with
    /// [`Ids::take`] on every pixel inside the union of `rects`.
    pub fn take_rects(&mut self, canvas: &Canvas, rects: &[Rect]) {
        self.rects.clear();
        self.pool.clear();
        if rects.is_empty()
            || canvas.width == 0
            || canvas.height == 0
            || !canvas.has_pixel_storage()
        {
            return;
        }
        let cw = i64::try_from(canvas.width).unwrap_or(i64::MAX);
        let ch = i64::try_from(canvas.height).unwrap_or(i64::MAX);
        for &[rx0, ry0, rx1, ry1] in rects {
            let x0 = i64::from(rx0).clamp(0, cw);
            let y0 = i64::from(ry0).clamp(0, ch);
            let x1 = i64::from(rx1).clamp(x0, cw);
            let y1 = i64::from(ry1).clamp(y0, ch);
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            let (Ok(x0_i), Ok(y0_i), Ok(x1_i), Ok(y1_i)) = (
                i32::try_from(x0),
                i32::try_from(y0),
                i32::try_from(x1),
                i32::try_from(y1),
            ) else {
                continue;
            };
            let (x0_u, x1_u) = (x0 as usize, x1 as usize);
            let (y0_u, y1_u) = (y0 as usize, y1 as usize);
            let offset = self.pool.len();
            self.pool.reserve((x1_u - x0_u) * (y1_u - y0_u));
            for y in y0_u..y1_u {
                let row = y * canvas.width;
                self.pool
                    .extend_from_slice(&canvas.ids[row + x0_u..row + x1_u]);
            }
            self.rects.push(IdRect {
                x0: x0_i,
                y0: y0_i,
                x1: x1_i,
                y1: y1_i,
                offset,
            });
        }
    }

    /// Number of `u16` pixel ID entries currently stored (for diagnostics & tests).
    pub fn stored_pixels(&self) -> usize {
        self.pool.len()
    }
}

/// A screen rectangle: left, top, right and bottom, the right and bottom
/// edges outside it.
pub type Rect = [i32; 4];

impl Canvas {
    pub fn new(width: usize, height: usize, background: [u8; 3]) -> Self {
        Self {
            width,
            height,
            color: vec![pack_rgb(background); width * height],
            depth: vec![EMPTY_DEPTH; width * height],
            ids: vec![ID_NONE; width * height],
            pen: ID_NONE,
            rects: std::collections::BTreeMap::new(),
        }
    }

    /// Unpacks the RGB color of pixel `(x, y)`.
    #[inline]
    pub fn pixel_rgb(&self, x: usize, y: usize) -> [u8; 3] {
        unpack_rgb(self.color[y * self.width + x])
    }

    /// Packs and writes the RGB color of pixel `(x, y)`.
    #[inline]
    pub fn set_pixel_rgb(&mut self, x: usize, y: usize, rgb: [u8; 3]) {
        self.color[y * self.width + x] = pack_rgb(rgb);
    }

    /// Unpacks all pixels into `[r, g, b]` triples.
    pub fn color_rgb(&self) -> Vec<[u8; 3]> {
        self.color.iter().map(|&c| unpack_rgb(c)).collect()
    }

    /// Clears to `background`, empty depth and no IDs, keeping the size,
    /// and puts the pen down (no ID).
    pub fn clear(&mut self, background: [u8; 3]) {
        self.color.fill(pack_rgb(background));
        self.depth.fill(EMPTY_DEPTH);
        self.ids.fill(ID_NONE);
        self.pen = ID_NONE;
        self.rects.clear();
    }

    /// Whether `color`, `depth`, and `ids` each hold at least `width * height`
    /// entries.
    pub(crate) fn has_pixel_storage(&self) -> bool {
        self.width.checked_mul(self.height).is_some_and(|pixels| {
            self.color.len() >= pixels && self.depth.len() >= pixels && self.ids.len() >= pixels
        })
    }

    /// Snapshots `other`'s logical `width * height` surface (`color`, `depth`,
    /// `ids`, `pen`, `rects`) into `self`, reusing backing allocations when
    /// possible. Caller must ensure `other.has_pixel_storage()`.
    pub(crate) fn snapshot_pixels_from(&mut self, other: &Canvas) {
        let pixels = other.width * other.height;
        self.width = other.width;
        self.height = other.height;
        self.color.clear();
        self.color.extend_from_slice(&other.color[..pixels]);
        self.depth.clear();
        self.depth.extend_from_slice(&other.depth[..pixels]);
        self.ids.clear();
        self.ids.extend_from_slice(&other.ids[..pixels]);
        self.pen = other.pen;
        self.rects.clone_from(&other.rects);
    }

    /// Restores a cached `width * height` snapshot from `other` into `self`,
    /// preserving any caller-supplied extra trailing elements in `color`,
    /// `depth`, and `ids` and clearing them identically to [`Canvas::clear`].
    /// Caller must ensure `self.width == other.width`,
    /// `self.height == other.height`, and `self.has_pixel_storage()`.
    pub(crate) fn restore_pixels_from(&mut self, other: &Canvas, background: [u8; 3]) {
        let pixels = other.width * other.height;
        debug_assert_eq!(self.width, other.width);
        debug_assert_eq!(self.height, other.height);
        debug_assert_eq!(other.color.len(), pixels);
        debug_assert_eq!(other.depth.len(), pixels);
        debug_assert_eq!(other.ids.len(), pixels);
        self.color[..pixels].copy_from_slice(&other.color);
        if self.color.len() > pixels {
            self.color[pixels..].fill(pack_rgb(background));
        }
        self.depth[..pixels].copy_from_slice(&other.depth);
        if self.depth.len() > pixels {
            self.depth[pixels..].fill(EMPTY_DEPTH);
        }
        self.ids[..pixels].copy_from_slice(&other.ids);
        if self.ids.len() > pixels {
            self.ids[pixels..].fill(ID_NONE);
        }
        self.pen = other.pen;
        self.rects.clone_from(&other.rects);
    }

    #[inline]
    fn record_rect(&mut self, x: i32, y: i32, width: usize, height: usize) {
        if let Some(id) = decode_id(self.pen) {
            self.record_rect_for(id, x, y, width, height);
        }
    }

    #[inline]
    pub(crate) fn record_rect_for(&mut self, id: u16, x: i32, y: i32, width: usize, height: usize) {
        let side = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
        let rect = [
            x,
            y,
            x.saturating_add(side(width)),
            y.saturating_add(side(height)),
        ];
        self.rects
            .entry(id)
            .and_modify(|r| {
                *r = [
                    r[0].min(rect[0]),
                    r[1].min(rect[1]),
                    r[2].max(rect[2]),
                    r[3].max(rect[3]),
                ]
            })
            .or_insert(rect);
    }

    /// Splits this canvas into up to `num_bands` disjoint horizontal row bands
    /// covering all fully-backed rows (`0..backed_rows`, which equals
    /// `0..self.height` whenever `self.has_pixel_storage()` holds) for safe
    /// parallel rasterization.
    pub(crate) fn split_bands(&mut self, num_bands: usize) -> Vec<CanvasBand<'_>> {
        if self.width == 0 || self.height == 0 {
            return Vec::new();
        }
        let min_len = self.color.len().min(self.depth.len()).min(self.ids.len());
        let backed_rows = (min_len / self.width).min(self.height);
        if backed_rows == 0 {
            return Vec::new();
        }
        let pixels = backed_rows * self.width;
        let bands = num_bands.clamp(1, backed_rows);
        let band_rows = backed_rows.div_ceil(bands);
        let band_pixels = band_rows * self.width;
        let width = self.width;
        let height = self.height;
        let color = &mut self.color[..pixels];
        let depth = &mut self.depth[..pixels];
        let ids = &mut self.ids[..pixels];
        color
            .chunks_mut(band_pixels)
            .zip(depth.chunks_mut(band_pixels))
            .zip(ids.chunks_mut(band_pixels))
            .enumerate()
            .map(|(i, ((c, d), id))| {
                let y0 = i * band_rows;
                let rows = c.len() / width;
                CanvasBand {
                    width,
                    height,
                    y0,
                    y1: y0 + rows,
                    color: c,
                    depth: d,
                    ids: id,
                }
            })
            .collect()
    }

    /// If a manually truncated canvas ends with a partial backed row
    /// (`0 < min_len % width < width`), stages that row into a 1-row
    /// [`CanvasBand`], invokes `f`, and copies the backed prefix back.
    /// No-op for normally allocated canvases (`self.has_pixel_storage()`).
    pub(crate) fn rasterize_partial_tail_row(&mut self, f: impl FnOnce(&mut CanvasBand<'_>)) {
        if self.width == 0 || self.height == 0 || self.has_pixel_storage() {
            return;
        }
        let min_len = self.color.len().min(self.depth.len()).min(self.ids.len());
        let partial_y = min_len / self.width;
        let partial_cols = min_len % self.width;
        if partial_cols == 0 || partial_y >= self.height {
            return;
        }
        let row_start = partial_y * self.width;
        let mut color = vec![0u32; self.width];
        let mut depth = vec![EMPTY_DEPTH; self.width];
        let mut ids = vec![ID_NONE; self.width];
        color[..partial_cols].copy_from_slice(&self.color[row_start..min_len]);
        depth[..partial_cols].copy_from_slice(&self.depth[row_start..min_len]);
        ids[..partial_cols].copy_from_slice(&self.ids[row_start..min_len]);
        let mut band = CanvasBand {
            width: self.width,
            height: self.height,
            y0: partial_y,
            y1: partial_y + 1,
            color: &mut color,
            depth: &mut depth,
            ids: &mut ids,
        };
        f(&mut band);
        self.color[row_start..min_len].copy_from_slice(&color[..partial_cols]);
        self.depth[row_start..min_len].copy_from_slice(&depth[..partial_cols]);
        self.ids[row_start..min_len].copy_from_slice(&ids[..partial_cols]);
    }

    /// Mutable row slices for `(color, depth, ids)` on row `y`, columns `x0..x1`.
    /// Caller must ensure `y < self.height`, `x0 <= x1 <= self.width`, and
    /// `self.has_pixel_storage()`.
    #[inline]
    pub(crate) fn row_slices_mut(
        &mut self,
        y: usize,
        x0: usize,
        x1: usize,
    ) -> (&mut [u32], &mut [u16], &mut [u16]) {
        let start = y * self.width + x0;
        let end = y * self.width + x1;
        (
            &mut self.color[start..end],
            &mut self.depth[start..end],
            &mut self.ids[start..end],
        )
    }

    /// Sets the ID the pixels drawn from now on take, until it is set
    /// again: an object's or a person's object ID, or None for what is
    /// nobody's (terrain, floors, walls). See [the module
    /// docs](self#which-object-drew-a-pixel).
    pub fn set_id(&mut self, id: Option<u16>) {
        self.pen = encode_id(id);
    }

    /// The ID of the object or person that drew the pixel at (`x`, `y`)
    /// last, if any did and the pixel is on the canvas: what a click there
    /// hit.
    pub fn id_at(&self, x: i32, y: i32) -> Option<u16> {
        decode_id(self.ids[self.index(x, y)?])
    }

    /// The IDs of every pixel as they are now ([`Ids`]).
    pub fn ids(&self) -> Ids {
        let mut ids = Ids::default();
        ids.take(self);
        ids
    }

    /// The rectangle the images drawn under `id` cover, each whole, drawn
    /// pixels or not and on the canvas or not, since the canvas was
    /// cleared: what the original keeps as an object's drawn rectangles
    /// (object +0x160 and +0x170, 0x438f71).
    pub fn drawn(&self, id: u16) -> Option<Rect> {
        self.rects.get(&id).copied()
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
        let tables = light::tables();
        self.blit(
            image,
            at,
            flip,
            |_, _, rgb| light.apply_with(tables, rgb),
            0,
            depth,
        );
    }

    /// Draws an image through the original's red-tinted blocked-placement
    /// palette (`0x42242f` → `0x4b44eb(1)` → `0x4c76d1` → `0x4b9425(3, 0.9)`
    /// → `0x49bd78`), used for invalid Buy Mode previews.
    pub fn draw_tinted(
        &mut self,
        image: &Image,
        at: [i32; 2],
        flip: bool,
        depth: impl Fn(i32, i32, u8) -> Option<i32>,
    ) {
        self.blit(image, at, flip, |_, _, rgb| tint_blocked(rgb), 0, depth);
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
        let tables = light::tables();
        self.blit(
            image,
            at,
            false,
            |ix, iy, rgb| light(ix, iy).apply_with(tables, rgb),
            0,
            depth,
        );
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
        self.draw_wall_shaded(image, at, |_, _| light, depth);
    }

    /// [`Canvas::draw_wall`] with each pixel's light given by its place
    /// in the image (column, row), as a lamp's pool on a wall gives it.
    pub fn draw_wall_shaded(
        &mut self,
        image: &Image,
        at: [i32; 2],
        light: impl Fn(usize, usize) -> Light,
        depth: impl Fn(i32, i32) -> i32,
    ) {
        let tables = light::tables();
        self.blit(
            image,
            at,
            false,
            |ix, iy, rgb| light(ix, iy).apply_with(tables, rgb),
            1,
            |x, y, _| Some(depth(x, y)),
        );
    }

    /// Draws an image whose pixels each take their depth from `depth`
    /// (screen column, row, the image's depth byte there), under `light`,
    /// winning up to one unit farther as [`Canvas::draw_wall`] does: the
    /// original's blitter for a cut wall's trim (0x49ecbc).
    pub fn draw_layered(
        &mut self,
        image: &Image,
        at: [i32; 2],
        light: Light,
        depth: impl Fn(i32, i32, u8) -> Option<i32>,
    ) {
        let tables = light::tables();
        self.blit(
            image,
            at,
            false,
            |_, _, rgb| light.apply_with(tables, rgb),
            1,
            depth,
        );
    }

    /// Like [`Canvas::draw_layered`], clipped horizontally to screen columns
    /// `columns` (`[from, to)`) without copying `image`.
    #[cfg(test)]
    pub(crate) fn draw_layered_clipped(
        &mut self,
        image: &Image,
        at: [i32; 2],
        light: Light,
        columns: Option<[i32; 2]>,
        depth: impl Fn(i32, i32, u8) -> Option<i32>,
    ) {
        let tables = light::tables();
        self.blit_clipped(
            image,
            at,
            false,
            |_, _, rgb| light.apply_with(tables, rgb),
            1,
            columns,
            depth,
        );
    }

    /// Draws a floor tile (whole or split-tile sub-rectangle `sub_rect =
    /// Some([ix0, iy0, ix1, iy1])`) with row-only depth computed once per row,
    /// avoiding `Image::clone()` for both whole and split floors.
    #[cfg(test)]
    pub(crate) fn draw_floor_shaded(
        &mut self,
        image: &Image,
        at: [i32; 2],
        sub_rect: Option<[usize; 4]>,
        light: impl Fn(usize, usize) -> Light,
        row_depth: impl Fn(i32) -> i32,
    ) {
        self.blit_row_depth(image, at, light, None, sub_rect, None, row_depth);
    }

    /// Like [`Canvas::draw_floor_shaded`], additionally clipping output rows to
    /// `[0, clip_bottom)` (matching `Sims.exe`'s active house viewport clip at
    /// `0x4c81b1`–`0x4c81c0`).
    #[cfg(test)]
    pub(crate) fn draw_floor_shaded_clipped(
        &mut self,
        image: &Image,
        at: [i32; 2],
        sub_rect: Option<[usize; 4]>,
        clip_bottom: i32,
        light: impl Fn(usize, usize) -> Light,
        row_depth: impl Fn(i32) -> i32,
    ) {
        self.blit_row_depth(
            image,
            at,
            light,
            None,
            sub_rect,
            Some(clip_bottom),
            row_depth,
        );
    }

    /// Draws a pond/natural water overlay tile (`Arry 10`, `0x4c1eea`–`0x4c2059`
    /// → `0x4c6594`), where each pixel's alpha channel holds the raw 8-bit mask
    /// from the SPR2 depth channel:
    /// - `a >> 3 == 0` (`a < 8`): skipped completely;
    /// - `1 <= (a >> 3) < 31`: blended over the canvas via the 5-bit `0x49a60b`
    ///   table (`blend565_with`) and unconditionally writes row depth `row_depth(y)`;
    /// - `a >> 3 >= 31` (`a >= 248`): opaque copy and unconditionally writes
    ///   row depth `row_depth(y)`.
    ///
    /// Output rows are clipped to `[0, clip_bottom)` (`0x4c1a4a`–`0x4c1a8c`).
    #[cfg(test)]
    pub(crate) fn draw_water_overlay(
        &mut self,
        image: &Image,
        at: [i32; 2],
        clip_bottom: i32,
        light: Light,
        row_depth: impl Fn(i32) -> i32,
    ) {
        let Some(pixels) = image.width.checked_mul(image.height) else {
            return;
        };
        if image.is_empty()
            || !self.has_pixel_storage()
            || pixels.checked_mul(4) != Some(image.rgba.len())
        {
            return;
        }
        let max_h = self.height.min(clip_bottom.max(0) as usize);
        let (x0, x1, ax) = clip_span(at[0], image.width, self.width);
        let (y0, y1, ay) = clip_span(at[1], image.height, max_h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        self.record_rect(x0 as i32, y0 as i32, x1 - x0, y1 - y0);
        let table = blend_table();
        let light_tables = light::tables();
        let pen = self.pen;
        let width = self.width;
        let img_w = image.width;
        let rgba = &image.rgba;
        for py in y0..y1 {
            let d = clamp_depth(row_depth(py as i32));
            let iy = (py as i64 - ay) as usize;
            let src_row = iy * img_w;
            let dst_row = py * width;
            let ix_start = (x0 as i64 - ax) as usize;
            for (offset, px) in (x0..x1).enumerate() {
                let pi = (src_row + ix_start + offset) * 4;
                let a = rgba[pi + 3];
                let a5 = a >> 3;
                if a5 == 0 {
                    continue;
                }
                let target = dst_row + px;
                self.depth[target] = d;
                self.ids[target] = pen;
                let rgb = light.apply_with(light_tables, [rgba[pi], rgba[pi + 1], rgba[pi + 2]]);
                if a5 >= 31 {
                    self.color[target] = pack_rgb(rgb);
                } else {
                    self.color[target] = blend565_packed(table, rgb, self.color[target], a);
                }
            }
        }
    }

    fn blit(
        &mut self,
        image: &Image,
        at: [i32; 2],
        flip: bool,
        color: impl Fn(usize, usize, [u8; 3]) -> [u8; 3],
        slack: i32,
        depth: impl Fn(i32, i32, u8) -> Option<i32>,
    ) {
        self.blit_clipped(image, at, flip, color, slack, None, depth);
    }

    #[allow(clippy::too_many_arguments)]
    fn blit_clipped(
        &mut self,
        image: &Image,
        at: [i32; 2],
        flip: bool,
        color: impl Fn(usize, usize, [u8; 3]) -> [u8; 3],
        slack: i32,
        columns: Option<[i32; 2]>,
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
        let Some((x0, x1, ax, left_skip, rect_x, rect_w)) =
            clip_x(at[0], image.width, self.width, columns)
        else {
            return;
        };
        self.record_rect(rect_x, at[1], rect_w, image.height);
        let (y0, y1, ay) = clip_span(at[1], image.height, self.height);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let pen = self.pen;
        let width = self.width;
        let img_w = image.width;
        let rgba = &image.rgba;
        let depth_bytes = image.depth.as_deref();
        let table = blend_table();
        let slice_ok = (y1 - 1)
            .checked_mul(width)
            .and_then(|row| row.checked_add(x1))
            .is_some_and(|max_target| {
                max_target <= self.color.len()
                    && max_target <= self.depth.len()
                    && max_target <= self.ids.len()
            });
        for py in y0..y1 {
            let sy = py as i32;
            let iy = (py as i64 - ay) as usize;
            let src_row = iy * img_w;
            let ix_base = (x0 as i64 - ax) as usize + left_skip;
            if slice_ok {
                let dst_row = py * width;
                let dst_color = &mut self.color[dst_row + x0..dst_row + x1];
                let dst_depth = &mut self.depth[dst_row + x0..dst_row + x1];
                let dst_ids = &mut self.ids[dst_row + x0..dst_row + x1];
                for (offset, px) in (x0..x1).enumerate() {
                    let ix = if flip {
                        img_w - 1 - (ix_base + offset)
                    } else {
                        ix_base + offset
                    };
                    let source = src_row + ix;
                    let si = source * 4;
                    let a = rgba[si + 3];
                    if a < FAINT {
                        continue;
                    }
                    let byte = depth_bytes.map_or(255, |d| d.get(source).copied().unwrap_or(255));
                    let Some(d) = depth(px as i32, sy, byte) else {
                        continue;
                    };
                    if d > i32::from(dst_depth[offset]).saturating_add(slack) {
                        continue;
                    }
                    let rgb = color(ix, iy, [rgba[si], rgba[si + 1], rgba[si + 2]]);
                    dst_depth[offset] = clamp_depth(d);
                    dst_ids[offset] = pen;
                    if a == 255 {
                        dst_color[offset] = pack_rgb(rgb);
                    } else {
                        dst_color[offset] = blend565_packed(table, rgb, dst_color[offset], a);
                    }
                }
            } else {
                for (offset, px) in (x0..x1).enumerate() {
                    let ix = if flip {
                        img_w - 1 - (ix_base + offset)
                    } else {
                        ix_base + offset
                    };
                    let source = src_row + ix;
                    let si = source * 4;
                    let a = rgba[si + 3];
                    if a < FAINT {
                        continue;
                    }
                    let byte = depth_bytes.map_or(255, |d| d.get(source).copied().unwrap_or(255));
                    let Some(d) = depth(px as i32, sy, byte) else {
                        continue;
                    };
                    let target = py * width + px;
                    if d > i32::from(self.depth[target]).saturating_add(slack) {
                        continue;
                    }
                    let rgb = color(ix, iy, [rgba[si], rgba[si + 1], rgba[si + 2]]);
                    let pixel = &mut self.color[target];
                    self.depth[target] = clamp_depth(d);
                    self.ids[target] = pen;
                    if a == 255 {
                        *pixel = pack_rgb(rgb);
                    } else {
                        *pixel = blend565_packed(table, rgb, *pixel, a);
                    }
                }
            }
        }
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn blit_row_depth(
        &mut self,
        image: &Image,
        at: [i32; 2],
        light: impl Fn(usize, usize) -> Light,
        columns: Option<[i32; 2]>,
        sub_rect: Option<[usize; 4]>,
        clip_bottom: Option<i32>,
        row_depth: impl Fn(i32) -> i32,
    ) {
        let pixels = image.width.checked_mul(image.height);
        if pixels.and_then(|n| n.checked_mul(4)) != Some(image.rgba.len())
            || pixels == Some(0)
            || self.color.is_empty()
        {
            return;
        }
        let Some((mut x0, mut x1, ax, left_skip, rect_x, rect_w)) =
            clip_x(at[0], image.width, self.width, columns)
        else {
            return;
        };
        let max_h = match clip_bottom {
            Some(cb) => self.height.min(cb.max(0) as usize),
            None => self.height,
        };
        let (mut y0, mut y1, ay) = clip_span(at[1], image.height, max_h);
        if let Some([ix0, iy0, ix1, iy1]) = sub_rect {
            if ix0 >= ix1 || iy0 >= iy1 {
                return;
            }
            let wide = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
            let base_x = i64::from(at[0]);
            let lim_x = wide(self.width);
            let lim_y = wide(max_h);
            let sub_x0 = base_x.saturating_add(wide(ix0)).clamp(0, lim_x) as usize;
            let sub_x1 = base_x.saturating_add(wide(ix1)).clamp(0, lim_x) as usize;
            let sub_y0 = ay.saturating_add(wide(iy0)).clamp(0, lim_y) as usize;
            let sub_y1 = ay.saturating_add(wide(iy1)).clamp(0, lim_y) as usize;
            x0 = x0.max(sub_x0);
            x1 = x1.min(sub_x1);
            y0 = y0.max(sub_y0);
            y1 = y1.min(sub_y1);
        }
        if clip_bottom.is_some() {
            if x0 >= x1 || y0 >= y1 {
                return;
            }
            self.record_rect(x0 as i32, y0 as i32, x1 - x0, y1 - y0);
        } else {
            self.record_rect(rect_x, at[1], rect_w, image.height);
            if x0 >= x1 || y0 >= y1 {
                return;
            }
        }
        let pen = self.pen;
        let width = self.width;
        let img_w = image.width;
        let rgba = &image.rgba;
        let table = blend_table();
        let light_tables = light::tables();
        let slice_ok = (y1 - 1)
            .checked_mul(width)
            .and_then(|row| row.checked_add(x1))
            .is_some_and(|max_target| {
                max_target <= self.color.len()
                    && max_target <= self.depth.len()
                    && max_target <= self.ids.len()
            });
        for py in y0..y1 {
            let sy = py as i32;
            let d = row_depth(sy);
            let d_clamped = clamp_depth(d);
            let iy = (py as i64 - ay) as usize;
            let src_row = iy * img_w;
            let ix_base = (x0 as i64 - ax) as usize + left_skip;
            if slice_ok {
                let dst_row = py * width;
                let dst_color = &mut self.color[dst_row + x0..dst_row + x1];
                let dst_depth = &mut self.depth[dst_row + x0..dst_row + x1];
                let dst_ids = &mut self.ids[dst_row + x0..dst_row + x1];
                for offset in 0..(x1 - x0) {
                    let ix = ix_base + offset;
                    let si = (src_row + ix) * 4;
                    let a = rgba[si + 3];
                    if a < FAINT {
                        continue;
                    }
                    if d > i32::from(dst_depth[offset]) {
                        continue;
                    }
                    let rgb = light(ix, iy)
                        .apply_with(light_tables, [rgba[si], rgba[si + 1], rgba[si + 2]]);
                    dst_depth[offset] = d_clamped;
                    dst_ids[offset] = pen;
                    if a == 255 {
                        dst_color[offset] = pack_rgb(rgb);
                    } else {
                        dst_color[offset] = blend565_packed(table, rgb, dst_color[offset], a);
                    }
                }
            } else {
                for (offset, px) in (x0..x1).enumerate() {
                    let ix = ix_base + offset;
                    let si = (src_row + ix) * 4;
                    let a = rgba[si + 3];
                    if a < FAINT {
                        continue;
                    }
                    let target = py * width + px;
                    if d > i32::from(self.depth[target]) {
                        continue;
                    }
                    let rgb = light(ix, iy)
                        .apply_with(light_tables, [rgba[si], rgba[si + 1], rgba[si + 2]]);
                    let pixel = &mut self.color[target];
                    self.depth[target] = d_clamped;
                    self.ids[target] = pen;
                    if a == 255 {
                        *pixel = pack_rgb(rgb);
                    } else {
                        *pixel = blend565_packed(table, rgb, *pixel, a);
                    }
                }
            }
        }
    }

    /// Writes a 4-pixel horizontal grass span starting at `(x, y)` with
    /// constant `depth` (already clamped to `[0, EMPTY]` in `depth_clamped`).
    #[cfg(test)]
    #[inline]
    pub(crate) fn plot_grass_span(
        &mut self,
        x: i32,
        y: i32,
        depth: i32,
        depth_clamped: u16,
        pixels: [[u8; 3]; 4],
    ) {
        let Ok(py) = usize::try_from(y) else {
            return;
        };
        if py >= self.height {
            return;
        }
        let pen = self.pen;
        if x >= 0
            && (x as usize).saturating_add(4) <= self.width
            && let Some(start) = py
                .checked_mul(self.width)
                .and_then(|row| row.checked_add(x as usize))
            && let Some(end) = start.checked_add(4)
            && end <= self.depth.len()
            && end <= self.color.len()
            && end <= self.ids.len()
        {
            let dst_depth = &mut self.depth[start..end];
            let dst_color = &mut self.color[start..end];
            let dst_ids = &mut self.ids[start..end];
            for i in 0..4 {
                if depth <= i32::from(dst_depth[i]) {
                    dst_depth[i] = depth_clamped;
                    dst_color[i] = pack_rgb(pixels[i]);
                    dst_ids[i] = pen;
                }
            }
            return;
        }
        for (i, &rgb) in pixels.iter().enumerate() {
            let Some(px) = x
                .checked_add(i as i32)
                .and_then(|v| usize::try_from(v).ok())
            else {
                continue;
            };
            if px < self.width {
                let target = py * self.width + px;
                if depth <= i32::from(self.depth[target]) {
                    let pixel = &mut self.color[target];
                    self.depth[target] = depth_clamped;
                    self.ids[target] = pen;
                    *pixel = pack_rgb(rgb);
                }
            }
        }
    }

    /// Plots a vertical grass blade at column `x` for rows `top..=bottom`
    /// (`top >= 0`, `bottom < height`), testing `depth` against each row.
    #[cfg(test)]
    #[inline]
    pub(crate) fn plot_grass_blade(
        &mut self,
        x: i32,
        top: i32,
        bottom: i32,
        depth: i32,
        depth_clamped: u16,
        color: [u8; 3],
    ) {
        if top > bottom || self.height == 0 {
            return;
        }
        let Ok(px) = usize::try_from(x) else {
            return;
        };
        if px >= self.width {
            return;
        }
        let Ok(bottom_usize) = usize::try_from(bottom) else {
            return;
        };
        let y0 = top.max(0) as usize;
        let y1 = bottom_usize.min(self.height - 1);
        if y0 > y1 {
            return;
        }
        let width = self.width;
        let pen = self.pen;
        let packed = pack_rgb(color);
        for py in y0..=y1 {
            let idx = py * width + px;
            if depth <= i32::from(self.depth[idx]) {
                let pixel = &mut self.color[idx];
                self.depth[idx] = depth_clamped;
                self.ids[idx] = pen;
                *pixel = packed;
            }
        }
    }

    /// Writes a pixel that passed its depth test: its depth, and its
    /// colour, straight-alpha blended by `a` if it is not opaque. Used for
    /// 3D mesh pixels ([`Canvas::put`]), which do not go through the SPR2
    /// 5-5-5 lookup table.
    fn write(&mut self, target: usize, rgb: [u8; 3], a: u8, d: i32) {
        let pixel = &mut self.color[target];
        self.depth[target] = clamp_depth(d);
        self.ids[target] = self.pen;
        if a == 255 {
            *pixel = pack_rgb(rgb);
        } else {
            let a = u32::from(a);
            let inv = 255 - a;
            let [dr, dg, db] = unpack_rgb(*pixel);
            let r = ((u32::from(rgb[0]) * a + u32::from(dr) * inv + 127) / 255) as u8;
            let g = ((u32::from(rgb[1]) * a + u32::from(dg) * inv + 127) / 255) as u8;
            let b = ((u32::from(rgb[2]) * a + u32::from(db) * inv + 127) / 255) as u8;
            *pixel = pack_rgb([r, g, b]);
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
        self.index(x, y)
            .is_some_and(|i| d <= i32::from(self.depth[i]))
    }

    /// Row-indexed depth check for an in-bounds pixel at `row_start + x`,
    /// avoiding repeated `i32 -> usize` conversions and row multiplies inside
    /// scanline loops.
    #[inline]
    pub fn depth_passes_at(&self, row_start: usize, x: usize, d: i32) -> bool {
        x < self.width
            && self
                .depth
                .get(row_start.saturating_add(x))
                .is_some_and(|&cur| d <= i32::from(cur))
    }

    /// Writes a straight-alpha pixel at precomputed index `idx = y * width + x`
    /// whose depth `d` already passed [`Canvas::depth_passes_at`]. Returns
    /// `false` when fainter than [`FAINT`] or out of bounds.
    #[inline]
    pub fn put_passed_at(&mut self, idx: usize, rgba: [u8; 4], d: i32) -> bool {
        let [r, g, b, a] = rgba;
        if a < FAINT || idx >= self.color.len() || idx >= self.depth.len() || idx >= self.ids.len()
        {
            return false;
        }
        self.write(idx, [r, g, b], a, d);
        true
    }

    /// Draws one straight-alpha pixel at depth `d` by the sprite rule, for
    /// what is drawn over the finished lot a pixel at a time (a person's
    /// mesh, with [`crate::depth::Depth::point`]): left out if fainter
    /// than [`FAINT`] or farther than what is there, otherwise blended and
    /// its depth written, so a nearer part of the same mesh drawn later
    /// covers it. Returns whether it was drawn.
    pub fn put(&mut self, x: i32, y: i32, rgba: [u8; 4], d: i32) -> bool {
        let [r, g, b, a] = rgba;
        if a < FAINT {
            return false;
        }
        let Some(target) = self.index(x, y) else {
            return false;
        };
        if d > i32::from(self.depth[target]) {
            return false;
        }
        self.write(target, [r, g, b], a, d);
        true
    }

    /// Fills one pixel if `d` passes the depth test.
    pub fn plot(&mut self, x: i32, y: i32, d: i32, rgb: [u8; 3]) {
        if let Some(target) = self.index(x, y)
            && d <= i32::from(self.depth[target])
        {
            self.write(target, rgb, 255, d);
        }
    }

    /// Writes one pixel's color without modifying depth or pick ID, matching
    /// the original's terrain grid drawers (`0x4c37f2`, `0x4c391d`) which
    /// write directly to the color surface.
    pub fn plot_color(&mut self, x: i32, y: i32, rgb: [u8; 3]) {
        if let Some(target) = self.index(x, y) {
            self.color[target] = pack_rgb(rgb);
        }
    }

    /// Darkens what is at (`x`, `y`) by `light`, if it is at depth
    /// `at_least` or farther, keeping its depth and ID: a shadow cast on
    /// what is drawn there (0x4b10c4).
    pub fn shade(&mut self, x: i32, y: i32, at_least: i32, light: Light) {
        if let Some(i) = self.index(x, y)
            && i32::from(self.depth[i]) >= at_least
        {
            self.color[i] = pack_rgb(light.apply(unpack_rgb(self.color[i])));
        }
    }

    /// The image as the original shows it: 16-bit color (5 bits red, 6
    /// green, 5 blue), widened back to 8 bits a channel by repeating the
    /// high bits. The original runs its display in that mode, so a fair
    /// comparison quantizes both sides this way.
    pub fn rgb565(&self) -> Vec<[u8; 3]> {
        self.color
            .iter()
            .map(|&c| quantize565(unpack_rgb(c)))
            .collect()
    }

    /// RGBA bytes, row by row, for a window or an image file.
    pub fn rgba(&self) -> Vec<u8> {
        self.color
            .iter()
            .flat_map(|&c| {
                let [r, g, b] = unpack_rgb(c);
                [r, g, b, 255]
            })
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

/// Red-tinted blocked-placement palette transform (`0x42242f` → `0x4b44eb(1)`
/// → `0x4c76d1` → `0x4b9425(3, 0.9)` → `0x49bd78`): converts `rgb` to a
/// 0.9× grayscale luminance `gray`, boosts the red channel by `+0x30`, and
/// packs the result through the display's 16-bit RGB565 palette (`0x4b9425`).
pub fn tint_blocked([r, g, b]: [u8; 3]) -> [u8; 3] {
    let sum = f32::from(r) + f32::from(g) + f32::from(b);
    let gray = ((sum * (1.0_f32 / 3.0) * 0.9_f32 + 0.5_f32).trunc() as i32).clamp(0, 255);
    let red = (gray + 0x30).clamp(0, 255);
    quantize565([red as u8, gray as u8, gray as u8])
}

/// The original's 32 × 32 × 32 5-bit alpha-blend table (`0x6e5eac`),
/// filled once at start-up by `0x49a315..0x49a41c`.
///
/// For each 5-bit `(alpha5, src5, dst5)`, it averages over the 8 × 8 × 8
/// 8-bit triples in `[dst5<<3..+8) × [src5<<3..+8) × [alpha5<<3..+8)`:
/// `blended = (d + (s - d) * (a * (1.0f32 / 255.0f32)) + 0.5) as u8`,
/// sums the 512 bytes, multiplies by `1/512`, adds `0.5`, truncates and
/// shifts right by 3.
pub(crate) fn blend_table() -> &'static [u8; 32 * 32 * 32] {
    static TABLE: OnceLock<Box<[u8; 32 * 32 * 32]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let inv255 = f64::from(f32::from_bits(0x3b80_8081));
        let mut table = Box::new([0u8; 32 * 32 * 32]);
        for dst5 in 0..32usize {
            let dst_base = dst5 << 3;
            for src5 in 0..32usize {
                let src_base = src5 << 3;
                for alpha5 in 0..32usize {
                    let alpha_base = alpha5 << 3;
                    let mut sum = 0.0f64;
                    for i in 0..8 {
                        let d = (dst_base + i) as f64;
                        for j in 0..8 {
                            let s = (src_base + j) as f64;
                            let diff = s - d;
                            for k in 0..8 {
                                let a = (alpha_base + k) as f64;
                                let blended = (d + diff * (a * inv255) + 0.5) as i32;
                                sum += f64::from(blended as u8);
                            }
                        }
                    }
                    let avg = (sum * (1.0 / 512.0) + 0.5) as i32;
                    table[(alpha5 << 10) | (src5 << 5) | dst5] = (avg >> 3) as u8;
                }
            }
        }
        table
    })
}

/// Clips a `[at, at + size)` span in `i64` to `[0, limit)`.
#[inline]
pub(crate) fn clip_span(at: i32, size: usize, limit: usize) -> (usize, usize, i64) {
    let wide = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
    let (at, limit) = (i64::from(at), wide(limit));
    let low = at.clamp(0, limit);
    let high = at.saturating_add(wide(size)).clamp(low, limit);
    (low as usize, high as usize, at)
}

/// Clips a horizontal image span `[at, at + size)` to `[0, limit)` and
/// optional screen column bounds `[from, to)`, matching `scene::within`
/// without allocating a cropped image. Returns
/// `(x0, x1, ax, left_skip, rect_x, rect_w)`.
#[inline]
pub(crate) fn clip_x(
    at: i32,
    size: usize,
    limit: usize,
    columns: Option<[i32; 2]>,
) -> Option<(usize, usize, i64, usize, i32, usize)> {
    match columns {
        None => {
            let (x0, x1, ax) = clip_span(at, size, limit);
            Some((x0, x1, ax, 0, at, size))
        }
        Some([from, to]) => {
            let width = i64::try_from(size).ok()?;
            let left = (i64::from(from) - i64::from(at)).clamp(0, width);
            let right = (i64::from(to) - i64::from(at)).clamp(left, width);
            if left == right {
                return None;
            }
            let left_usize = usize::try_from(left).ok()?;
            let w = usize::try_from(right - left).ok()?;
            let shifted = at.saturating_add(i32::try_from(left).ok()?);
            let (x0, x1, ax) = clip_span(shifted, w, limit);
            Some((x0, x1, ax, left_usize, shifted, w))
        }
    }
}

#[inline]
fn blend565_with(table: &[u8; 32 * 32 * 32], src: [u8; 3], dst: [u8; 3], a: u8) -> [u8; 3] {
    let row = usize::from(a >> 3) << 10;
    let look = |s: u8, d: u8| table[row | (usize::from(s >> 3) << 5) | usize::from(d >> 3)];
    let r5 = look(src[0], dst[0]);
    let g5 = look(src[1], dst[1]);
    let b5 = look(src[2], dst[2]);
    let g6 = g5 << 1;
    [
        (r5 << 3) | (r5 >> 2),
        (g6 << 2) | (g6 >> 4),
        (b5 << 3) | (b5 >> 2),
    ]
}

#[inline(always)]
fn blend565_packed(table: &[u8; 32 * 32 * 32], src: [u8; 3], dst_packed: u32, a: u8) -> u32 {
    let row = usize::from(a >> 3) << 10;
    let dr = ((dst_packed >> 19) & 31) as usize;
    let dg = ((dst_packed >> 11) & 31) as usize;
    let db = ((dst_packed >> 3) & 31) as usize;
    let r5 = table[row | (usize::from(src[0] >> 3) << 5) | dr];
    let g5 = table[row | (usize::from(src[1] >> 3) << 5) | dg];
    let b5 = table[row | (usize::from(src[2] >> 3) << 5) | db];
    let g6 = g5 << 1;
    pack_rgb([
        (r5 << 3) | (r5 >> 2),
        (g6 << 2) | (g6 >> 4),
        (b5 << 3) | (b5 >> 2),
    ])
}

/// Blends `src` over `dst` at 8-bit alpha `a` as the original's 16-bit
/// 5-6-5 blitters do (`0x4bbab8` at `0x4bbc04..0x4bbc7d`, `0x4bc2eb`,
/// `0x49a60b`): each channel takes its top 5 bits (green drops its lowest
/// 6th bit: `g6 >> 1`), looks up `0x6e5eac[a >> 3][src5][dst5]`, and packs
/// `(r5, g5 << 1, b5)` back into 5-6-5 (green's low bit is always 0),
/// widened to 8 bits a channel.
///
/// For every SPR2 5-bit alpha `k` in `0..=31`, `((k * 255) / 31) >> 3 == k`,
/// so `a >> 3` recovers the original's exact 5-bit alpha.
pub fn blend565(src: [u8; 3], dst: [u8; 3], a: u8) -> [u8; 3] {
    blend565_with(blend_table(), src, dst, a)
}

impl CanvasBand<'_> {
    #[inline]
    pub(crate) fn plot_color(&mut self, x: i32, y: i32, rgb: [u8; 3]) {
        let (Ok(px), Ok(py)) = (usize::try_from(x), usize::try_from(y)) else {
            return;
        };
        if px < self.width && py >= self.y0 && py < self.y1 {
            self.color[(py - self.y0) * self.width + px] = pack_rgb(rgb);
        }
    }

    #[inline]
    pub(crate) fn plot_grass_span(
        &mut self,
        x: i32,
        y: i32,
        depth: i32,
        depth_clamped: u16,
        pixels: [[u8; 3]; 4],
    ) {
        let Ok(py) = usize::try_from(y) else {
            return;
        };
        if py < self.y0 || py >= self.y1 {
            return;
        }
        let local_row = (py - self.y0) * self.width;
        if x >= 0 && (x as usize).saturating_add(4) <= self.width {
            let start = local_row + x as usize;
            let end = start + 4;
            let dst_depth = &mut self.depth[start..end];
            let dst_color = &mut self.color[start..end];
            let dst_ids = &mut self.ids[start..end];
            for i in 0..4 {
                if depth <= i32::from(dst_depth[i]) {
                    dst_depth[i] = depth_clamped;
                    dst_color[i] = pack_rgb(pixels[i]);
                    dst_ids[i] = ID_NONE;
                }
            }
            return;
        }
        for (i, &rgb) in pixels.iter().enumerate() {
            let Some(px) = x
                .checked_add(i as i32)
                .and_then(|v| usize::try_from(v).ok())
            else {
                continue;
            };
            if px < self.width {
                let target = local_row + px;
                if depth <= i32::from(self.depth[target]) {
                    self.depth[target] = depth_clamped;
                    self.ids[target] = ID_NONE;
                    self.color[target] = pack_rgb(rgb);
                }
            }
        }
    }

    #[inline]
    pub(crate) fn plot_grass_blade(
        &mut self,
        x: i32,
        top: i32,
        bottom: i32,
        depth: i32,
        depth_clamped: u16,
        color: [u8; 3],
    ) {
        if top > bottom || self.y0 >= self.y1 {
            return;
        }
        let Ok(px) = usize::try_from(x) else {
            return;
        };
        if px >= self.width {
            return;
        }
        let Ok(bottom_usize) = usize::try_from(bottom) else {
            return;
        };
        let y0 = (top.max(0) as usize).max(self.y0);
        let y1 = bottom_usize.min(self.y1 - 1);
        if y0 > y1 {
            return;
        }
        let width = self.width;
        let packed = pack_rgb(color);
        for py in y0..=y1 {
            let idx = (py - self.y0) * width + px;
            if depth <= i32::from(self.depth[idx]) {
                self.depth[idx] = depth_clamped;
                self.ids[idx] = ID_NONE;
                self.color[idx] = packed;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blit_row_depth_flat(
        &mut self,
        image: &Image,
        spans: &RowSpans,
        at: [i32; 2],
        light: Light,
        columns: Option<[i32; 2]>,
        sub_rect: Option<[usize; 4]>,
        clip_bottom: Option<i32>,
        pen: Option<u16>,
        row_depth: impl Fn(i32) -> i32,
    ) {
        let Some((mut x0, mut x1, ax, left_skip, _, _)) =
            clip_x(at[0], image.width, self.width, columns)
        else {
            return;
        };
        let max_h = match clip_bottom {
            Some(cb) => self.height.min(cb.max(0) as usize),
            None => self.height,
        };
        let (mut y0, mut y1, ay) = clip_span(at[1], image.height, max_h);
        if let Some([ix0, iy0, ix1, iy1]) = sub_rect {
            if ix0 >= ix1 || iy0 >= iy1 {
                return;
            }
            let wide = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
            let base_x = i64::from(at[0]);
            let lim_x = wide(self.width);
            let lim_y = wide(max_h);
            x0 = x0.max(base_x.saturating_add(wide(ix0)).clamp(0, lim_x) as usize);
            x1 = x1.min(base_x.saturating_add(wide(ix1)).clamp(0, lim_x) as usize);
            y0 = y0.max(ay.saturating_add(wide(iy0)).clamp(0, lim_y) as usize);
            y1 = y1.min(ay.saturating_add(wide(iy1)).clamp(0, lim_y) as usize);
        }
        y0 = y0.max(self.y0);
        y1 = y1.min(self.y1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let width = self.width;
        let img_w = image.width;
        let rgba = image.rgba.as_chunks::<4>().0;
        let table = blend_table();
        let pen = encode_id(pen);
        let ix_min = (x0 as i64 - ax) as usize + left_skip;
        let ix_max = ix_min + (x1 - x0);
        match light.level() {
            None => {
                for py in y0..y1 {
                    let iy = (py as i64 - ay) as usize;
                    let row_spans = spans.row(iy);
                    if row_spans.is_empty() {
                        continue;
                    }
                    let d = row_depth(py as i32);
                    let d_clamped = clamp_depth(d);
                    let src_row = iy * img_w;
                    let dst_base = (py - self.y0) * width + x0;
                    for span in row_spans {
                        let s = (span.start as usize).max(ix_min);
                        let e = (span.end as usize).min(ix_max);
                        if s >= e {
                            continue;
                        }
                        let dst_off = dst_base + (s - ix_min);
                        let len = e - s;
                        let src_px = &rgba[src_row + s..src_row + e];
                        let dc = &mut self.color[dst_off..dst_off + len];
                        let dd = &mut self.depth[dst_off..dst_off + len];
                        let di = &mut self.ids[dst_off..dst_off + len];
                        if span.all_opaque {
                            for i in 0..len {
                                if d <= i32::from(dd[i]) {
                                    let px = src_px[i];
                                    dd[i] = d_clamped;
                                    di[i] = pen;
                                    dc[i] = pack_rgb([px[0], px[1], px[2]]);
                                }
                            }
                        } else {
                            for i in 0..len {
                                let px = src_px[i];
                                let a = px[3];
                                if a < FAINT || d > i32::from(dd[i]) {
                                    continue;
                                }
                                dd[i] = d_clamped;
                                di[i] = pen;
                                let rgb = [px[0], px[1], px[2]];
                                if a == 255 {
                                    dc[i] = pack_rgb(rgb);
                                } else {
                                    dc[i] = blend565_packed(table, rgb, dc[i], a);
                                }
                            }
                        }
                    }
                }
            }
            Some(l) => {
                let lt = light::tables();
                let (tr, tg, tb) = (&lt.r[l], &lt.g[l], &lt.b[l]);
                for py in y0..y1 {
                    let iy = (py as i64 - ay) as usize;
                    let row_spans = spans.row(iy);
                    if row_spans.is_empty() {
                        continue;
                    }
                    let d = row_depth(py as i32);
                    let d_clamped = clamp_depth(d);
                    let src_row = iy * img_w;
                    let dst_base = (py - self.y0) * width + x0;
                    for span in row_spans {
                        let s = (span.start as usize).max(ix_min);
                        let e = (span.end as usize).min(ix_max);
                        if s >= e {
                            continue;
                        }
                        let dst_off = dst_base + (s - ix_min);
                        let len = e - s;
                        let src_px = &rgba[src_row + s..src_row + e];
                        let dc = &mut self.color[dst_off..dst_off + len];
                        let dd = &mut self.depth[dst_off..dst_off + len];
                        let di = &mut self.ids[dst_off..dst_off + len];
                        if span.all_opaque {
                            for i in 0..len {
                                if d <= i32::from(dd[i]) {
                                    let px = src_px[i];
                                    dd[i] = d_clamped;
                                    di[i] = pen;
                                    dc[i] = pack_rgb([
                                        tr[usize::from(px[0] >> 3)],
                                        tg[usize::from(px[1] >> 2)],
                                        tb[usize::from(px[2] >> 3)],
                                    ]);
                                }
                            }
                        } else {
                            for i in 0..len {
                                let px = src_px[i];
                                let a = px[3];
                                if a < FAINT || d > i32::from(dd[i]) {
                                    continue;
                                }
                                dd[i] = d_clamped;
                                di[i] = pen;
                                let rgb = [
                                    tr[usize::from(px[0] >> 3)],
                                    tg[usize::from(px[1] >> 2)],
                                    tb[usize::from(px[2] >> 3)],
                                ];
                                if a == 255 {
                                    dc[i] = pack_rgb(rgb);
                                } else {
                                    dc[i] = blend565_packed(table, rgb, dc[i], a);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blit_row_depth_shaded(
        &mut self,
        image: &Image,
        spans: &RowSpans,
        at: [i32; 2],
        light: impl Fn(usize, usize) -> Light,
        columns: Option<[i32; 2]>,
        sub_rect: Option<[usize; 4]>,
        clip_bottom: Option<i32>,
        pen: Option<u16>,
        row_depth: impl Fn(i32) -> i32,
    ) {
        let Some((mut x0, mut x1, ax, left_skip, _, _)) =
            clip_x(at[0], image.width, self.width, columns)
        else {
            return;
        };
        let max_h = match clip_bottom {
            Some(cb) => self.height.min(cb.max(0) as usize),
            None => self.height,
        };
        let (mut y0, mut y1, ay) = clip_span(at[1], image.height, max_h);
        if let Some([ix0, iy0, ix1, iy1]) = sub_rect {
            if ix0 >= ix1 || iy0 >= iy1 {
                return;
            }
            let wide = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
            let base_x = i64::from(at[0]);
            let lim_x = wide(self.width);
            let lim_y = wide(max_h);
            x0 = x0.max(base_x.saturating_add(wide(ix0)).clamp(0, lim_x) as usize);
            x1 = x1.min(base_x.saturating_add(wide(ix1)).clamp(0, lim_x) as usize);
            y0 = y0.max(ay.saturating_add(wide(iy0)).clamp(0, lim_y) as usize);
            y1 = y1.min(ay.saturating_add(wide(iy1)).clamp(0, lim_y) as usize);
        }
        y0 = y0.max(self.y0);
        y1 = y1.min(self.y1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let width = self.width;
        let img_w = image.width;
        let rgba = image.rgba.as_chunks::<4>().0;
        let table = blend_table();
        let lt = light::tables();
        let pen = encode_id(pen);
        let ix_min = (x0 as i64 - ax) as usize + left_skip;
        let ix_max = ix_min + (x1 - x0);
        for py in y0..y1 {
            let iy = (py as i64 - ay) as usize;
            let row_spans = spans.row(iy);
            if row_spans.is_empty() {
                continue;
            }
            let d = row_depth(py as i32);
            let d_clamped = clamp_depth(d);
            let src_row = iy * img_w;
            let dst_base = (py - self.y0) * width + x0;
            for span in row_spans {
                let s = (span.start as usize).max(ix_min);
                let e = (span.end as usize).min(ix_max);
                if s >= e {
                    continue;
                }
                let dst_off = dst_base + (s - ix_min);
                let len = e - s;
                let src_px = &rgba[src_row + s..src_row + e];
                let dc = &mut self.color[dst_off..dst_off + len];
                let dd = &mut self.depth[dst_off..dst_off + len];
                let di = &mut self.ids[dst_off..dst_off + len];
                if span.all_opaque {
                    for i in 0..len {
                        if d <= i32::from(dd[i]) {
                            let px = src_px[i];
                            dd[i] = d_clamped;
                            di[i] = pen;
                            dc[i] =
                                pack_rgb(light(s + i, iy).apply_with(lt, [px[0], px[1], px[2]]));
                        }
                    }
                } else {
                    for i in 0..len {
                        let px = src_px[i];
                        let a = px[3];
                        if a < FAINT || d > i32::from(dd[i]) {
                            continue;
                        }
                        dd[i] = d_clamped;
                        di[i] = pen;
                        let rgb = light(s + i, iy).apply_with(lt, [px[0], px[1], px[2]]);
                        if a == 255 {
                            dc[i] = pack_rgb(rgb);
                        } else {
                            dc[i] = blend565_packed(table, rgb, dc[i], a);
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_water_overlay(
        &mut self,
        image: &Image,
        spans: &RowSpans,
        at: [i32; 2],
        clip_bottom: i32,
        light: Light,
        pen: Option<u16>,
        row_depth: impl Fn(i32) -> i32,
    ) {
        let max_h = self.height.min(clip_bottom.max(0) as usize);
        let (x0, x1, ax) = clip_span(at[0], image.width, self.width);
        let (mut y0, mut y1, ay) = clip_span(at[1], image.height, max_h);
        y0 = y0.max(self.y0);
        y1 = y1.min(self.y1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let table = blend_table();
        let lt = light::tables();
        let pen = encode_id(pen);
        let width = self.width;
        let img_w = image.width;
        let rgba = image.rgba.as_chunks::<4>().0;
        let ix_min = (x0 as i64 - ax) as usize;
        let ix_max = ix_min + (x1 - x0);
        let light_rows = light.level().map(|l| (&lt.r[l], &lt.g[l], &lt.b[l]));
        for py in y0..y1 {
            let iy = (py as i64 - ay) as usize;
            let row_spans = spans.row(iy);
            if row_spans.is_empty() {
                continue;
            }
            let d = clamp_depth(row_depth(py as i32));
            let src_row = iy * img_w;
            let dst_base = (py - self.y0) * width + x0;
            for span in row_spans {
                let s = (span.start as usize).max(ix_min);
                let e = (span.end as usize).min(ix_max);
                if s >= e {
                    continue;
                }
                let dst_off = dst_base + (s - ix_min);
                let len = e - s;
                let src_px = &rgba[src_row + s..src_row + e];
                let dc = &mut self.color[dst_off..dst_off + len];
                let dd = &mut self.depth[dst_off..dst_off + len];
                let di = &mut self.ids[dst_off..dst_off + len];
                for i in 0..len {
                    let px = src_px[i];
                    let a = px[3];
                    let a5 = a >> 3;
                    if a5 == 0 {
                        continue;
                    }
                    dd[i] = d;
                    di[i] = pen;
                    let rgb = match light_rows {
                        None => [px[0], px[1], px[2]],
                        Some((tr, tg, tb)) => [
                            tr[usize::from(px[0] >> 3)],
                            tg[usize::from(px[1] >> 2)],
                            tb[usize::from(px[2] >> 3)],
                        ],
                    };
                    if a5 >= 31 {
                        dc[i] = pack_rgb(rgb);
                    } else {
                        dc[i] = blend565_packed(table, rgb, dc[i], a);
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_wall_masked_flat(
        &mut self,
        pattern: &Image,
        pat_spans: &RowSpans,
        mask: &Image,
        mask_spans: &RowSpans,
        at: [i32; 2],
        columns: Option<[i32; 2]>,
        light: Light,
        pen: Option<u16>,
        depth: impl Fn(i32, i32) -> i32,
    ) {
        let Some((x0, x1, ax, left_skip, _, _)) = clip_x(at[0], pattern.width, self.width, columns)
        else {
            return;
        };
        let (mut y0, mut y1, ay) = clip_span(at[1], pattern.height, self.height);
        y0 = y0.max(self.y0);
        y1 = y1.min(self.y1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let width = self.width;
        let pat_w = pattern.width;
        let mask_w = mask.width;
        let pat_px = pattern.rgba.as_chunks::<4>().0;
        let mask_px = mask.rgba.as_chunks::<4>().0;
        let table = blend_table();
        let lt = light::tables();
        let pen = encode_id(pen);
        let light_rows = light.level().map(|l| (&lt.r[l], &lt.g[l], &lt.b[l]));
        let ix_min = (x0 as i64 - ax) as usize + left_skip;
        let ix_max = ix_min + (x1 - x0);
        for py in y0..y1 {
            let sy = py as i32;
            let iy = (py as i64 - ay) as usize;
            let Some((ps, pe)) = pat_spans.row_bounds(iy) else {
                continue;
            };
            let p_lo = ps.max(ix_min);
            let p_hi = pe.min(ix_max);
            if p_lo >= p_hi {
                continue;
            }
            let pat_row = iy * pat_w;
            let mask_row = iy * mask_w;
            let dst_base = (py - self.y0) * width + x0;
            for span in mask_spans.row(iy) {
                let s = (span.start as usize).max(p_lo);
                let e = (span.end as usize).min(p_hi);
                if s >= e {
                    continue;
                }
                let dst_off = dst_base + (s - ix_min);
                let len = e - s;
                let px_start = (x0 + (s - ix_min)) as i32;
                let pat_slice = &pat_px[pat_row + s..pat_row + e];
                let dc = &mut self.color[dst_off..dst_off + len];
                let dd = &mut self.depth[dst_off..dst_off + len];
                let di = &mut self.ids[dst_off..dst_off + len];
                if span.all_opaque {
                    // ma == 255 for every pixel in this span => (pa * 255) / 255 == pa.
                    for i in 0..len {
                        let p = pat_slice[i];
                        let a = p[3];
                        if a < FAINT {
                            continue;
                        }
                        let d = depth(px_start + i as i32, sy);
                        if d > i32::from(dd[i]) + 1 {
                            continue;
                        }
                        let rgb = match light_rows {
                            None => [p[0], p[1], p[2]],
                            Some((tr, tg, tb)) => [
                                tr[usize::from(p[0] >> 3)],
                                tg[usize::from(p[1] >> 2)],
                                tb[usize::from(p[2] >> 3)],
                            ],
                        };
                        dd[i] = clamp_depth(d);
                        di[i] = pen;
                        if a == 255 {
                            dc[i] = pack_rgb(rgb);
                        } else {
                            dc[i] = blend565_packed(table, rgb, dc[i], a);
                        }
                    }
                } else {
                    let mask_slice = &mask_px[mask_row + s..mask_row + e];
                    for i in 0..len {
                        let p = pat_slice[i];
                        let pa = p[3];
                        if pa < FAINT {
                            continue;
                        }
                        let ma = mask_slice[i][3];
                        let a = ((u16::from(pa) * u16::from(ma)) / 255) as u8;
                        if a < FAINT {
                            continue;
                        }
                        let d = depth(px_start + i as i32, sy);
                        if d > i32::from(dd[i]) + 1 {
                            continue;
                        }
                        let rgb = match light_rows {
                            None => [p[0], p[1], p[2]],
                            Some((tr, tg, tb)) => [
                                tr[usize::from(p[0] >> 3)],
                                tg[usize::from(p[1] >> 2)],
                                tb[usize::from(p[2] >> 3)],
                            ],
                        };
                        dd[i] = clamp_depth(d);
                        di[i] = pen;
                        if a == 255 {
                            dc[i] = pack_rgb(rgb);
                        } else {
                            dc[i] = blend565_packed(table, rgb, dc[i], a);
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_wall_masked_shaded(
        &mut self,
        pattern: &Image,
        pat_spans: &RowSpans,
        mask: &Image,
        mask_spans: &RowSpans,
        at: [i32; 2],
        columns: Option<[i32; 2]>,
        light: impl Fn(usize, usize) -> Light,
        pen: Option<u16>,
        depth: impl Fn(i32, i32) -> i32,
    ) {
        let Some((x0, x1, ax, left_skip, _, _)) = clip_x(at[0], pattern.width, self.width, columns)
        else {
            return;
        };
        let (mut y0, mut y1, ay) = clip_span(at[1], pattern.height, self.height);
        y0 = y0.max(self.y0);
        y1 = y1.min(self.y1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let width = self.width;
        let pat_w = pattern.width;
        let mask_w = mask.width;
        let pat_px = pattern.rgba.as_chunks::<4>().0;
        let mask_px = mask.rgba.as_chunks::<4>().0;
        let table = blend_table();
        let lt = light::tables();
        let pen = encode_id(pen);
        let ix_min = (x0 as i64 - ax) as usize + left_skip;
        let ix_max = ix_min + (x1 - x0);
        for py in y0..y1 {
            let sy = py as i32;
            let iy = (py as i64 - ay) as usize;
            let Some((ps, pe)) = pat_spans.row_bounds(iy) else {
                continue;
            };
            let p_lo = ps.max(ix_min);
            let p_hi = pe.min(ix_max);
            if p_lo >= p_hi {
                continue;
            }
            let pat_row = iy * pat_w;
            let mask_row = iy * mask_w;
            let dst_base = (py - self.y0) * width + x0;
            for span in mask_spans.row(iy) {
                let s = (span.start as usize).max(p_lo);
                let e = (span.end as usize).min(p_hi);
                if s >= e {
                    continue;
                }
                let dst_off = dst_base + (s - ix_min);
                let len = e - s;
                let px_start = (x0 + (s - ix_min)) as i32;
                let pat_slice = &pat_px[pat_row + s..pat_row + e];
                let mask_slice = &mask_px[mask_row + s..mask_row + e];
                let dc = &mut self.color[dst_off..dst_off + len];
                let dd = &mut self.depth[dst_off..dst_off + len];
                let di = &mut self.ids[dst_off..dst_off + len];
                for i in 0..len {
                    let p = pat_slice[i];
                    let pa = p[3];
                    if pa < FAINT {
                        continue;
                    }
                    let a = if span.all_opaque {
                        pa
                    } else {
                        ((u16::from(pa) * u16::from(mask_slice[i][3])) / 255) as u8
                    };
                    if a < FAINT {
                        continue;
                    }
                    let d = depth(px_start + i as i32, sy);
                    if d > i32::from(dd[i]) + 1 {
                        continue;
                    }
                    let rgb = light(s + i, iy).apply_with(lt, [p[0], p[1], p[2]]);
                    dd[i] = clamp_depth(d);
                    di[i] = pen;
                    if a == 255 {
                        dc[i] = pack_rgb(rgb);
                    } else {
                        dc[i] = blend565_packed(table, rgb, dc[i], a);
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blit_depth_sprite_flat(
        &mut self,
        image: &Image,
        spans: &RowSpans,
        at: [i32; 2],
        flip: bool,
        light: Light,
        slack: i32,
        columns: Option<[i32; 2]>,
        pen: Option<u16>,
        depth: impl Fn(i32, i32, u8) -> Option<i32>,
    ) {
        let Some((x0, x1, ax, left_skip, _, _)) = clip_x(at[0], image.width, self.width, columns)
        else {
            return;
        };
        let (mut y0, mut y1, ay) = clip_span(at[1], image.height, self.height);
        y0 = y0.max(self.y0);
        y1 = y1.min(self.y1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let width = self.width;
        let img_w = image.width;
        let rgba = image.rgba.as_chunks::<4>().0;
        let depth_bytes = image.depth.as_deref();
        let table = blend_table();
        let lt = light::tables();
        let pen = encode_id(pen);
        let light_rows = light.level().map(|l| (&lt.r[l], &lt.g[l], &lt.b[l]));
        let ix_base = (x0 as i64 - ax) as usize + left_skip;
        let ix_end = ix_base + (x1 - x0);
        for py in y0..y1 {
            let sy = py as i32;
            let iy = (py as i64 - ay) as usize;
            let row_spans = spans.row(iy);
            if row_spans.is_empty() {
                continue;
            }
            let src_row = iy * img_w;
            let dst_base = (py - self.y0) * width + x0;
            if !flip {
                for span in row_spans {
                    let s = (span.start as usize).max(ix_base);
                    let e = (span.end as usize).min(ix_end);
                    if s >= e {
                        continue;
                    }
                    let dst_off = dst_base + (s - ix_base);
                    let px_start = (x0 + (s - ix_base)) as i32;
                    let len = e - s;
                    let src_px = &rgba[src_row + s..src_row + e];
                    let dc = &mut self.color[dst_off..dst_off + len];
                    let dd = &mut self.depth[dst_off..dst_off + len];
                    let di = &mut self.ids[dst_off..dst_off + len];
                    if span.all_opaque {
                        match light_rows {
                            None => {
                                for i in 0..len {
                                    let byte = depth_bytes.map_or(255, |db| {
                                        db.get(src_row + s + i).copied().unwrap_or(255)
                                    });
                                    let Some(d) = depth(px_start + i as i32, sy, byte) else {
                                        continue;
                                    };
                                    if d > i32::from(dd[i]).saturating_add(slack) {
                                        continue;
                                    }
                                    let px = src_px[i];
                                    dd[i] = clamp_depth(d);
                                    di[i] = pen;
                                    dc[i] = pack_rgb([px[0], px[1], px[2]]);
                                }
                            }
                            Some((tr, tg, tb)) => {
                                for i in 0..len {
                                    let byte = depth_bytes.map_or(255, |db| {
                                        db.get(src_row + s + i).copied().unwrap_or(255)
                                    });
                                    let Some(d) = depth(px_start + i as i32, sy, byte) else {
                                        continue;
                                    };
                                    if d > i32::from(dd[i]).saturating_add(slack) {
                                        continue;
                                    }
                                    let px = src_px[i];
                                    dd[i] = clamp_depth(d);
                                    di[i] = pen;
                                    dc[i] = pack_rgb([
                                        tr[usize::from(px[0] >> 3)],
                                        tg[usize::from(px[1] >> 2)],
                                        tb[usize::from(px[2] >> 3)],
                                    ]);
                                }
                            }
                        }
                    } else {
                        for i in 0..len {
                            let px = src_px[i];
                            let a = px[3];
                            if a < FAINT {
                                continue;
                            }
                            let byte = depth_bytes
                                .map_or(255, |db| db.get(src_row + s + i).copied().unwrap_or(255));
                            let Some(d) = depth(px_start + i as i32, sy, byte) else {
                                continue;
                            };
                            if d > i32::from(dd[i]).saturating_add(slack) {
                                continue;
                            }
                            let rgb = match light_rows {
                                None => [px[0], px[1], px[2]],
                                Some((tr, tg, tb)) => [
                                    tr[usize::from(px[0] >> 3)],
                                    tg[usize::from(px[1] >> 2)],
                                    tb[usize::from(px[2] >> 3)],
                                ],
                            };
                            dd[i] = clamp_depth(d);
                            di[i] = pen;
                            if a == 255 {
                                dc[i] = pack_rgb(rgb);
                            } else {
                                dc[i] = blend565_packed(table, rgb, dc[i], a);
                            }
                        }
                    }
                }
            } else {
                for span in row_spans.iter().rev() {
                    let fs = (img_w - span.end as usize).max(ix_base);
                    let fe = (img_w - span.start as usize).min(ix_end);
                    if fs >= fe {
                        continue;
                    }
                    let dst_off = dst_base + (fs - ix_base);
                    let px_start = (x0 + (fs - ix_base)) as i32;
                    let len = fe - fs;
                    let dc = &mut self.color[dst_off..dst_off + len];
                    let dd = &mut self.depth[dst_off..dst_off + len];
                    let di = &mut self.ids[dst_off..dst_off + len];
                    for i in 0..len {
                        let ix = img_w - 1 - (fs + i);
                        let source = src_row + ix;
                        let px = rgba[source];
                        let a = px[3];
                        if !span.all_opaque && a < FAINT {
                            continue;
                        }
                        let byte =
                            depth_bytes.map_or(255, |db| db.get(source).copied().unwrap_or(255));
                        let Some(d) = depth(px_start + i as i32, sy, byte) else {
                            continue;
                        };
                        if d > i32::from(dd[i]).saturating_add(slack) {
                            continue;
                        }
                        let rgb = match light_rows {
                            None => [px[0], px[1], px[2]],
                            Some((tr, tg, tb)) => [
                                tr[usize::from(px[0] >> 3)],
                                tg[usize::from(px[1] >> 2)],
                                tb[usize::from(px[2] >> 3)],
                            ],
                        };
                        dd[i] = clamp_depth(d);
                        di[i] = pen;
                        if span.all_opaque || a == 255 {
                            dc[i] = pack_rgb(rgb);
                        } else {
                            dc[i] = blend565_packed(table, rgb, dc[i], a);
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blit_depth_sprite_tinted(
        &mut self,
        image: &Image,
        spans: &RowSpans,
        at: [i32; 2],
        flip: bool,
        slack: i32,
        columns: Option<[i32; 2]>,
        pen: Option<u16>,
        depth: impl Fn(i32, i32, u8) -> Option<i32>,
    ) {
        let Some((x0, x1, ax, left_skip, _, _)) = clip_x(at[0], image.width, self.width, columns)
        else {
            return;
        };
        let (mut y0, mut y1, ay) = clip_span(at[1], image.height, self.height);
        y0 = y0.max(self.y0);
        y1 = y1.min(self.y1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let width = self.width;
        let img_w = image.width;
        let rgba = image.rgba.as_chunks::<4>().0;
        let depth_bytes = image.depth.as_deref();
        let table = blend_table();
        let pen = encode_id(pen);
        let ix_base = (x0 as i64 - ax) as usize + left_skip;
        let ix_end = ix_base + (x1 - x0);
        for py in y0..y1 {
            let sy = py as i32;
            let iy = (py as i64 - ay) as usize;
            let row_spans = spans.row(iy);
            if row_spans.is_empty() {
                continue;
            }
            let src_row = iy * img_w;
            let dst_base = (py - self.y0) * width + x0;
            for span in row_spans {
                let (fs, fe) = if flip {
                    (
                        (img_w - span.end as usize).max(ix_base),
                        (img_w - span.start as usize).min(ix_end),
                    )
                } else {
                    (
                        (span.start as usize).max(ix_base),
                        (span.end as usize).min(ix_end),
                    )
                };
                if fs >= fe {
                    continue;
                }
                let dst_off = dst_base + (fs - ix_base);
                let px_start = (x0 + (fs - ix_base)) as i32;
                let len = fe - fs;
                let dc = &mut self.color[dst_off..dst_off + len];
                let dd = &mut self.depth[dst_off..dst_off + len];
                let di = &mut self.ids[dst_off..dst_off + len];
                for i in 0..len {
                    let ix = if flip { img_w - 1 - (fs + i) } else { fs + i };
                    let source = src_row + ix;
                    let px = rgba[source];
                    let a = px[3];
                    if !span.all_opaque && a < FAINT {
                        continue;
                    }
                    let byte = depth_bytes.map_or(255, |db| db.get(source).copied().unwrap_or(255));
                    let Some(d) = depth(px_start + i as i32, sy, byte) else {
                        continue;
                    };
                    if d > i32::from(dd[i]).saturating_add(slack) {
                        continue;
                    }
                    let rgb = tint_blocked([px[0], px[1], px[2]]);
                    dd[i] = clamp_depth(d);
                    di[i] = pen;
                    if span.all_opaque || a == 255 {
                        dc[i] = pack_rgb(rgb);
                    } else {
                        dc[i] = blend565_packed(table, rgb, dc[i], a);
                    }
                }
            }
        }
    }

    pub(crate) fn shade_mask(
        &mut self,
        mask: &crate::shadows::Mask,
        at: [i32; 2],
        strength: i32,
        row_at_least: impl Fn(i32) -> i32,
    ) {
        if mask.width == 0 || mask.height == 0 {
            return;
        }
        let (x0, x1, ax) = clip_span(at[0], mask.width, self.width);
        let (mut y0, mut y1, ay) = clip_span(at[1], mask.height, self.height);
        y0 = y0.max(self.y0);
        y1 = y1.min(self.y1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let width = self.width;
        let lt = light::tables();
        let mx_base = (x0 as i64 - ax) as usize;
        let len = x1 - x0;
        for py in y0..y1 {
            let my = (py as i64 - ay) as usize;
            let at_least = row_at_least(py as i32);
            let dst_off = (py - self.y0) * width + x0;
            let dst_depth = &self.depth[dst_off..dst_off + len];
            let dst_color = &mut self.color[dst_off..dst_off + len];
            for i in 0..len {
                if let Some(s) = mask.shade(mx_base + i, my, strength)
                    && i32::from(dst_depth[i]) >= at_least
                {
                    dst_color[i] =
                        pack_rgb(Light::from_sum(s).apply_with(lt, unpack_rgb(dst_color[i])));
                }
            }
        }
    }
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
    fn a_taller_or_wider_mask_lines_up_at_the_top_left() {
        let pattern = image(2, 2, [9, 9, 9, 200], None);
        // A 3 by 4 mask whose alpha is its row times 10 plus its column,
        // times 5.
        let mut mask = image(3, 4, [0, 0, 0, 0], None);
        for y in 0..4 {
            for x in 0..3 {
                mask.rgba[(y * 3 + x) * 4 + 3] = (y * 10 + x) as u8 * 5;
            }
        }
        let masked = pattern.masked_top_left(&mask).unwrap();
        let alphas: Vec<u8> = masked.rgba.chunks(4).map(|p| p[3]).collect();
        let want = |row: usize, column: usize| ((200 * (row * 10 + column) * 5) / 255) as u8;
        assert_eq!(alphas, [want(0, 0), want(0, 1), want(1, 0), want(1, 1)]);
        assert_eq!((masked.width, masked.height), (2, 2));
        // `masked` keeps to masks of the same size.
        assert!(pattern.masked(&mask).is_none());
        let same = image(2, 2, [0, 0, 0, 51], None);
        for masked in [pattern.masked(&same), pattern.masked_top_left(&same)] {
            assert!(masked.unwrap().rgba.chunks(4).all(|p| p[3] == 40));
        }
        // A smaller mask, or one whose pixels fall short of its size,
        // gives none; an empty image stays empty.
        assert!(
            pattern
                .masked_top_left(&image(2, 1, [0, 0, 0, 255], None))
                .is_none()
        );
        assert!(
            pattern
                .masked_top_left(&image(1, 2, [0, 0, 0, 255], None))
                .is_none()
        );
        let mut short = image(2, 2, [0, 0, 0, 255], None);
        short.rgba.truncate(12);
        assert!(pattern.masked_top_left(&short).is_none());
        assert!(pattern.masked(&short).is_none());
        let empty = Image::empty();
        assert!(empty.masked_top_left(&mask).unwrap().is_empty());
        // A size whose byte count overflows gives none, on either side.
        let mut huge = image(1, 1, [0, 0, 0, 255], None);
        huge.width = usize::MAX / 4 + 1;
        huge.height = 2;
        huge.rgba.clear();
        assert!(huge.masked_top_left(&huge).is_none());
        assert!(pattern.masked_top_left(&huge).is_none());
        assert!(huge.masked(&huge).is_none());
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
        assert_eq!(unpack_rgb(c.color[5]), [255, 0, 0]);
        assert_eq!(unpack_rgb(c.color[10]), [0, 255, 0]);
        c.draw(
            &image(1, 1, [0, 0, 255, 255], None),
            [1, 1],
            false,
            |_, _, _| 10,
        );
        assert_eq!(unpack_rgb(c.color[5]), [0, 0, 255]);
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
        assert_eq!(
            unpack_rgb(c.color[0]),
            blend565([255, 255, 255], [0, 0, 0], 128)
        );
        assert_eq!(unpack_rgb(c.color[0]), [132, 130, 132]);
        assert_eq!(c.depth[0], 5);
        // Farther and opaque, drawn later: hidden by the soft edge.
        c.draw(
            &image(1, 1, [0, 255, 0, 255], None),
            [0, 0],
            false,
            |_, _, _| 10,
        );
        assert_eq!(unpack_rgb(c.color[0]), [132, 130, 132]);
        // Nearer: drawn over it.
        c.draw(
            &image(1, 1, [0, 255, 0, 255], None),
            [0, 0],
            false,
            |_, _, _| 1,
        );
        assert_eq!(unpack_rgb(c.color[0]), [0, 255, 0]);
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
        assert_eq!(
            (unpack_rgb(c.color[0]), c.depth[0]),
            ([0, 0, 0], EMPTY_DEPTH)
        );
        c.draw(
            &image(1, 1, [255, 255, 255, FAINT], None),
            [0, 0],
            false,
            |_, _, _| 5,
        );
        assert_ne!(unpack_rgb(c.color[0]), [0, 0, 0]);
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
                ([0, 255, 0], d as u16)
            } else {
                ([255, 0, 0], 10u16)
            };
            assert_eq!(
                (unpack_rgb(c.color[0]), c.depth[0]),
                expected,
                "wall at {d}"
            );
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
                ([0, 255, 0], d as u16)
            } else {
                ([255, 0, 0], 10u16)
            };
            assert_eq!(
                (unpack_rgb(c.color[0]), c.depth[0]),
                expected,
                "sprite at {d}"
            );
        }
        // A pixel whose depth function gives None is not drawn.
        let mut c = Canvas::new(1, 1, [0, 0, 0]);
        c.draw_lit(&wall, [0, 0], false, Light::FULL, |_, _, _| None);
        assert_eq!(c, Canvas::new(1, 1, [0, 0, 0]));
    }

    #[test]
    fn blocked_preview_tint_matches_native_0x49bd78() {
        // Exact `0x49bd78` formula with intensity 0.9 packed through native
        // `0x4b9425` RGB565 palette storage:
        // gray = ((r + g + b) * (1/3) * 0.9 + 0.5).trunc(), red = min(255, gray + 0x30).
        assert_eq!(tint_blocked([0, 0, 0]), quantize565([0x30, 0, 0]));
        assert_eq!(tint_blocked([255, 255, 255]), quantize565([255, 230, 230]));
        assert_eq!(tint_blocked([90, 120, 150]), quantize565([156, 108, 108]));
        let mut c = Canvas::new(1, 1, [0, 0, 0]);
        c.draw_tinted(
            &image(1, 1, [90, 120, 150, 255], None),
            [0, 0],
            false,
            |_, _, _| Some(5),
        );
        assert_eq!(unpack_rgb(c.color[0]), quantize565([156, 108, 108]));
        assert_eq!(c.depth[0], 5);
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
        assert_eq!(
            c.color_rgb(),
            vec![[0, 0, 0], [10, 20, 30], [0, 0, 0], [0, 0, 0]]
        );
        assert_eq!(c.depth, vec![EMPTY_DEPTH, 100, EMPTY_DEPTH, EMPTY_DEPTH]);
        // A tie wins; a soft pixel straight-alpha blends and writes its depth
        // without passing through the SPR2 5-5-5 lookup table.
        assert!(c.passes(1, 0, 100) && !c.passes(1, 0, 101));
        assert!(c.put(1, 0, [255, 255, 255, 255], 100));
        assert!(c.put(0, 1, [255, 255, 255, 128], -7));
        assert_eq!(unpack_rgb(c.color[1]), [255, 255, 255]);
        assert_eq!((unpack_rgb(c.color[2]), c.depth[2]), ([128, 128, 128], 0));
        // Odd 6-bit green is preserved on mesh pixels (whereas sprite blits
        // drop green's low bit through `blend565`).
        let mut g = Canvas::new(1, 1, [0, 4, 0]);
        assert!(g.put(0, 0, [0, 4, 0, 128], 10));
        assert_eq!(unpack_rgb(g.color[0]), [0, 4, 0]);
    }

    #[test]
    fn depth_bytes_reach_the_depth_function_and_flips_mirror() {
        let mut c = Canvas::new(3, 1, [0, 0, 0]);
        let mut img = image(3, 1, [9, 9, 9, 255], Some(0));
        img.rgba[0] = 200;
        img.depth.as_mut().unwrap()[0] = 7;
        c.draw(&img, [0, 0], true, |x, _, b| x * 100 + i32::from(b));
        assert_eq!(unpack_rgb(c.color[2]), [200, 9, 9]);
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
        assert_eq!(
            c.color_rgb(),
            vec![[1, 2, 3], [1, 2, 3], [0, 0, 0], [0, 0, 0]]
        );
    }

    /// Each pixel's light comes from its place in the image, also where
    /// the image is clipped.
    #[test]
    fn shaded_pixels_take_the_light_of_their_place_in_the_image() {
        let mut c = Canvas::new(2, 1, [0, 0, 0]);
        let dark = Light::from_byte(0, 1.0);
        c.draw_shaded(
            &image(3, 1, [200, 200, 200, 255], None),
            [-1, 0],
            |x, y| if (x, y) == (2, 0) { Light::FULL } else { dark },
            |_, _, _| Some(0),
        );
        assert_eq!(unpack_rgb(c.color[0]), dark.apply([200; 3]));
        assert_eq!(unpack_rgb(c.color[1]), [200; 3]);
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
        // The IDs taken keep what the canvas named then, not what it
        // names after a later draw.
        c.set_id(Some(7));
        c.plot(2, 0, 0, [4, 4, 4]);
        let taken = c.ids();
        c.set_id(Some(9));
        c.plot(1, 0, 0, [4, 4, 4]);
        c.plot(2, 0, 0, [4, 4, 4]);
        let then = [0, 1, 2, 3].map(|x| taken.id_at(x, 0));
        assert_eq!(then, [None, None, Some(7), None]);
        assert_eq!([c.id_at(1, 0), c.id_at(2, 0)], [Some(9), Some(9)]);
    }

    #[test]
    fn quantizing_keeps_the_extremes() {
        assert_eq!(quantize565([255, 255, 255]), [255, 255, 255]);
        assert_eq!(quantize565([0, 0, 0]), [0, 0, 0]);
        assert_eq!(quantize565([0x84, 0x82, 0x84]), [0x84, 0x82, 0x84]);
    }

    #[test]
    fn blend565_matches_the_originals_table_and_drops_greens_low_bit() {
        // Every 5-bit SPR2 alpha round-trips through `((k * 255) / 31) >> 3`,
        // and the `<= 7` cutoff (`0x4bbbe4`) aligns with `FAINT`.
        for k in 0..=31u16 {
            let a = ((k * 255) / 31) as u8;
            assert_eq!(u16::from(a >> 3), k, "k={k} a={a}");
            assert_eq!(a >= FAINT, k >= 8, "k={k} a={a}");
        }
        // Endpoints, midpoint, and a rounding-sensitive interior point from
        // `0x49a315..0x49a41c` (`[0; 3]` over `[104; 3]` at alpha 65 pins
        // both `+ 0.5` rounding steps in `blend_table`).
        assert_eq!(blend565([255, 255, 255], [0, 0, 0], 0), [0, 0, 0]);
        assert_eq!(blend565([255, 255, 255], [0, 0, 0], 65), [66, 65, 66]);
        assert_eq!(blend565([255, 255, 255], [0, 0, 0], 131), [132, 130, 132]);
        assert_eq!(blend565([255, 255, 255], [0, 0, 0], 255), [255, 251, 255]);
        assert_eq!(blend565([0, 0, 0], [104, 104, 104], 65), [82, 81, 82]);
        // Blended pixels always clear bit 0 of 6-bit green (`g5 << 1`) and
        // are already 5-6-5 quantized.
        for a in [65u8, 128, 197, 246] {
            for src in [[41, 77, 24], [100, 142, 60], [255, 255, 255]] {
                for dst in [[0, 4, 0], [57, 89, 32], [115, 113, 115]] {
                    let out = blend565(src, dst, a);
                    assert_eq!(quantize565(out), out, "src={src:?} dst={dst:?} a={a}");
                    assert_eq!((out[1] >> 2) & 1, 0, "src={src:?} dst={dst:?} a={a}");
                }
            }
        }
        // An opaque draw (`a == 255`) preserves odd 6-bit green, whereas a
        // blended draw (`a == 246`, SPR2 alpha 30) drops green's low bit.
        let mut c = Canvas::new(2, 1, [0, 0, 0]);
        c.draw(
            &image(1, 1, [0, 4, 0, 255], None),
            [0, 0],
            false,
            |_, _, _| 5,
        );
        c.draw(
            &image(1, 1, [0, 4, 0, 246], None),
            [1, 0],
            false,
            |_, _, _| 5,
        );
        assert_eq!(unpack_rgb(c.color[0])[1] >> 2, 1);
        assert_eq!(unpack_rgb(c.color[1])[1] >> 2, 0);
    }

    #[test]
    fn draw_layered_clipped_records_only_the_clipped_rect_and_skips_empty_spans() {
        let sprite = image(10, 4, [200, 100, 50, 255], Some(10));
        let mut c = Canvas::new(20, 10, [0, 0, 0]);
        c.set_id(Some(7));
        // Empty column intersection records no rectangle.
        c.draw_layered_clipped(&sprite, [2, 3], Light::FULL, Some([12, 18]), |_, _, _| {
            Some(5)
        });
        assert_eq!(c.drawn(7), None);
        // Partial intersection records only the clipped horizontal span.
        c.draw_layered_clipped(&sprite, [2, 3], Light::FULL, Some([5, 9]), |_, _, _| {
            Some(5)
        });
        assert_eq!(c.drawn(7), Some([5, 3, 9, 7]));
    }

    #[test]
    fn undersized_canvas_storage_blits_in_bounds_pixels_without_panicking() {
        let mut c = Canvas::new(4, 1, [0, 0, 0]);
        c.color.truncate(1);
        c.depth.truncate(1);
        c.ids.truncate(1);
        c.set_id(Some(3));
        // 1x1 opaque sprite at (0, 0) fits inside the 1-pixel backing store.
        let one = image(1, 1, [200, 100, 50, 255], None);
        c.draw(&one, [0, 0], false, |_, _, _| 5);
        assert_eq!(unpack_rgb(c.color[0]), [200, 100, 50]);
        assert_eq!(c.depth[0], 5);
        assert_eq!(c.id_at(0, 0), Some(3));
        // 2x1 sprite whose second pixel is transparent must also avoid
        // touching index 1 (matching origin/main's per-pixel alpha skip).
        let two = Image {
            width: 2,
            height: 1,
            rgba: vec![10, 20, 30, 255, 99, 99, 99, 0],
            depth: None,
        };
        c.draw(&two, [0, 0], false, |_, _, _| 4);
        assert_eq!(unpack_rgb(c.color[0]), [10, 20, 30]);
        assert_eq!(c.depth[0], 4);

        // Wrapping `width = usize::MAX, height = 2` must skip transparent and
        // `depth == None` pixels at `(0, 1)` without panicking in release mode.
        let mut wide = Canvas::new(1, 1, [0, 0, 0]);
        wide.width = usize::MAX;
        wide.height = 2;
        let transparent = image(1, 1, [1, 2, 3, 0], None);
        wide.draw(&transparent, [0, 1], false, |_, _, _| 5);
        wide.draw_lit(&one, [0, 1], false, Light::FULL, |_, _, _| None);

        // Short `color` buffer checks `self.color[target]` before writing depth
        // or pick ID, leaving both untouched when indexing panics.
        let mut short_color = Canvas::new(2, 1, [0, 0, 0]);
        short_color.color.truncate(1);
        short_color.set_id(Some(11));
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            short_color.draw(&one, [1, 0], false, |_, _, _| 5);
        }))
        .is_err();
        assert!(panicked);
        assert_eq!(short_color.depth[1], EMPTY_DEPTH);
        assert_eq!(short_color.id_at(1, 0), None);

        // `width = usize::MAX, height = 2` with 2 backing pixels must draw
        // split floors at (0, 0), grass spans clipped across x = 0, and
        // single-row grass blades at (1, 0) without panicking or overflowing.
        let mut huge = Canvas::new(2, 1, [0, 0, 0]);
        huge.width = usize::MAX;
        huge.height = 2;
        huge.draw_floor_shaded(&one, [0, 0], Some([0, 0, 1, 1]), |_, _| Light::FULL, |_| 10);
        assert_eq!(unpack_rgb(huge.color[0]), [200, 100, 50]);
        assert_eq!(huge.depth[0], 10);
        huge.plot_grass_span(-3, 0, 8, 8, [[1, 1, 1], [2, 2, 2], [3, 3, 3], [40, 50, 60]]);
        assert_eq!(unpack_rgb(huge.color[0]), [40, 50, 60]);
        assert_eq!(huge.depth[0], 8);
        huge.plot_grass_blade(1, 0, 0, 7, 7, [70, 80, 90]);
        assert_eq!(unpack_rgb(huge.color[1]), [70, 80, 90]);
        assert_eq!(huge.depth[1], 7);
        huge.plot_grass_blade(0, -3, -1, 1, 1, [9, 9, 9]);
        assert_eq!(unpack_rgb(huge.color[0]), [40, 50, 60]);
    }

    #[test]
    fn draw_floor_shaded_records_full_rect_and_clips_combined_columns_and_sub_rect() {
        let sprite = image(8, 2, [200, 100, 50, 255], None);
        let mut c = Canvas::new(10, 4, [0, 0, 0]);
        c.set_id(Some(9));
        // Split-floor sub-rect with active pen records the full sprite rect,
        // matching origin/main's alpha-masked floor half.
        c.draw_floor_shaded(
            &sprite,
            [1, 1],
            Some([2, 0, 4, 2]),
            |_, _| Light::FULL,
            |_| 5,
        );
        assert_eq!(c.drawn(9), Some([1, 1, 9, 3]));
        assert_eq!(unpack_rgb(c.color[12]), [0, 0, 0]);
        assert_eq!(unpack_rgb(c.color[13]), [200, 100, 50]);
        assert_eq!(unpack_rgb(c.color[14]), [200, 100, 50]);
        assert_eq!(unpack_rgb(c.color[15]), [0, 0, 0]);

        // Combined `columns` [3, 5) and source `sub_rect` [1, 3) at x = 0 have
        // empty intersection and must draw no pixels.
        let mut c2 = Canvas::new(10, 4, [0, 0, 0]);
        c2.blit_row_depth(
            &sprite,
            [0, 0],
            |_, _| Light::FULL,
            Some([3, 5]),
            Some([1, 0, 3, 2]),
            None,
            |_| 5,
        );
        assert!(c2.color.iter().all(|&p| p == 0));
    }

    #[test]
    fn draw_water_overlay_blends_five_bit_alpha_and_writes_depth_unconditionally() {
        let water = Image {
            width: 3,
            height: 1,
            // Pixel 0: a=7 (7>>3 == 0, transparent -> skip).
            // Pixel 1: a=128 (128>>3 == 16, 5-bit blended over bg).
            // Pixel 2: a=254 (254>>3 == 31, opaque copy).
            rgba: vec![
                255, 0, 220, 7, // skipped chroma key
                40, 80, 160, 128, // blended shoreline
                33, 57, 82, 254, // opaque water
            ],
            depth: None,
        };
        let bg = [80, 120, 40];
        let mut c = Canvas::new(3, 1, bg);
        // Existing depth is nearer (100) than water depth (500): 0x4c6650 still
        // writes depth 500 on every drawn pixel without a Z-test.
        c.depth.fill(100);
        c.draw_water_overlay(&water, [0, 0], 1, Light::FULL, |_| 500);
        assert_eq!(unpack_rgb(c.color[0]), bg);
        assert_eq!(c.depth[0], 100);
        assert_eq!(unpack_rgb(c.color[1]), blend565([40, 80, 160], bg, 128));
        assert_eq!(c.depth[1], 500);
        assert_eq!(unpack_rgb(c.color[2]), [33, 57, 82]);
        assert_eq!(c.depth[2], 500);

        // Both draw_water_overlay (0x4c1a4a) and draw_floor_shaded_clipped (0x4c81b1)
        // must leave color, depth, and ids untouched at rows >= clip_bottom.
        let tall_water = Image {
            width: 2,
            height: 4,
            rgba: [33, 57, 82, 254].repeat(8),
            depth: None,
        };
        let mut clipped = Canvas::new(2, 4, bg);
        clipped.set_id(Some(42));
        clipped.draw_water_overlay(&tall_water, [0, 0], 2, Light::FULL, |_| 500);
        for row in 0..2 {
            for col in 0..2 {
                let idx = row * 2 + col;
                assert_eq!(unpack_rgb(clipped.color[idx]), [33, 57, 82]);
                assert_eq!(clipped.depth[idx], 500);
                assert_eq!(clipped.ids[idx], 42);
            }
        }
        for row in 2..4 {
            for col in 0..2 {
                let idx = row * 2 + col;
                assert_eq!(unpack_rgb(clipped.color[idx]), bg);
                assert_eq!(clipped.depth[idx], EMPTY_DEPTH);
                assert_eq!(clipped.ids[idx], ID_NONE);
            }
        }

        let tall_pool = Image {
            width: 2,
            height: 4,
            rgba: [90, 174, 189, 255].repeat(8),
            depth: None,
        };
        let mut clipped_pool = Canvas::new(2, 4, bg);
        clipped_pool.set_id(Some(77));
        clipped_pool.draw_floor_shaded_clipped(
            &tall_pool,
            [0, 0],
            None,
            2,
            |_, _| Light::FULL,
            |_| 400,
        );
        for row in 0..2 {
            for col in 0..2 {
                let idx = row * 2 + col;
                assert_eq!(unpack_rgb(clipped_pool.color[idx]), [90, 174, 189]);
                assert_eq!(clipped_pool.depth[idx], 400);
                assert_eq!(clipped_pool.ids[idx], 77);
            }
        }
        for row in 2..4 {
            for col in 0..2 {
                let idx = row * 2 + col;
                assert_eq!(unpack_rgb(clipped_pool.color[idx]), bg);
                assert_eq!(clipped_pool.depth[idx], EMPTY_DEPTH);
                assert_eq!(clipped_pool.ids[idx], ID_NONE);
            }
        }
    }

    #[test]
    fn compact_canvas_helpers_depth_clamping_and_sparse_ids_parity() {
        // Pack / unpack roundtrip across extremes and arbitrary RGB values.
        for rgb in [
            [0, 0, 0],
            [255, 255, 255],
            [0x12, 0x34, 0x56],
            [255, 0, 128],
        ] {
            assert_eq!(unpack_rgb(pack_rgb(rgb)), rgb);
        }
        // Depth clamping covers negative, in-range, and >65535 identically.
        assert_eq!(clamp_depth(i32::MIN), 0);
        assert_eq!(clamp_depth(-1), 0);
        assert_eq!(clamp_depth(0), 0);
        assert_eq!(clamp_depth(12345), 12345);
        assert_eq!(clamp_depth(65535), 65535);
        assert_eq!(clamp_depth(65536), 65535);
        assert_eq!(clamp_depth(i32::MAX), 65535);

        // Object ID 0 is valid (distinct from ID_NONE = u16::MAX).
        let mut c = Canvas::new(8, 6, [0, 0, 0]);
        c.set_id(Some(0));
        c.plot(1, 1, -50, [10, 20, 30]);
        assert_eq!(c.id_at(1, 1), Some(0));
        assert_eq!(c.depth[8 + 1], 0);
        // Depth > 65535 fails standard depth test against EMPTY_DEPTH (65535).
        c.set_id(Some(5));
        c.plot(2, 1, 65536, [40, 50, 60]);
        assert_eq!(c.id_at(2, 1), None);
        assert_eq!(c.depth[8 + 2], EMPTY_DEPTH);
        // Wall with slack=1 wins at 65536 against EMPTY_DEPTH and clamps to 65535.
        let one = image(1, 1, [40, 50, 60, 255], None);
        c.draw_wall(&one, [2, 1], Light::FULL, |_, _| 65536);
        assert_eq!(c.id_at(2, 1), Some(5));
        assert_eq!(c.depth[8 + 2], EMPTY_DEPTH);

        // Passing ID_NONE (u16::MAX) to set_id panics rather than silently aliasing None.
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut bad = Canvas::new(1, 1, [0, 0, 0]);
            bad.set_id(Some(ID_NONE));
        }))
        .is_err();
        assert!(panicked);

        // Sparse Ids::take_rects matches full Ids::take on every pixel inside the
        // captured rects (including overlapping rects) and returns None outside.
        c.set_id(Some(10));
        for y in 1..4 {
            for x in 2..5 {
                c.plot(x, y, 100, [1, 2, 3]);
            }
        }
        c.set_id(Some(20));
        for y in 2..5 {
            for x in 3..6 {
                c.plot(x, y, 50, [4, 5, 6]);
            }
        }
        let mut full = Ids::default();
        full.take(&c);
        let mut sparse = Ids::default();
        sparse.take_rects(&c, &[[2, 1, 5, 4], [3, 2, 6, 5]]);
        assert_eq!(sparse.stored_pixels(), 9 + 9);
        assert_eq!(full.stored_pixels(), 8 * 6);
        for y in -1..7 {
            for x in -1..9 {
                let in_rect = (2..5).contains(&x) && (1..4).contains(&y)
                    || (3..6).contains(&x) && (2..5).contains(&y);
                if in_rect {
                    assert_eq!(sparse.id_at(x, y), full.id_at(x, y), "({x}, {y})");
                } else {
                    assert_eq!(sparse.id_at(x, y), None, "outside ({x}, {y})");
                }
            }
        }
    }
}
