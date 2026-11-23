//! Objects' catalog pictures (thumbnails), as the action queue shows them.
//!
//! **What the original does** (read from its code):
//!
//! - **Which objects have one** (0x4cd830 to 0x4cd8a6): when an object
//!   file is read, each definition gets a catalog item (0x4cbd44, kept at
//!   definition +0x84, 0x457edf), except a tile of a multi-tile object
//!   that is not its lead (0x4575b8) and a person (type 2, 0x4575d2). The
//!   definition needs graphics (0x457588: a master ID or a graphics
//!   count) or an interaction table with entries (0x4575a1, 0x457cb0):
//!   the `TTAB` named by the definition's word at byte 0x0e (0x45726a;
//!   -1 takes the lead's, other negative numbers none), looked up in the
//!   definition's own file and then in its semi-global file (0x4569a3),
//!   with entries when its word +4 is not 0, which is taken here as the
//!   table's count not being 0.
//! - **Whose item an object shows** (0x45728d, 0x43ae0a): its own, or for
//!   a tile of a multi-tile object, the item of the first definition in
//!   the same file with the same master ID and sub-index -1 (0x43adf7).
//!   None found, no item.
//! - **The thumbnail** (0x4cbdc3): the item's file's `BMP_` whose ID is
//!   the definition's catalog strings ID (word at byte 0x52). For a
//!   definition with graphics, it is kept only if it is 74 by 37: two 37
//!   by 37 cells, the object on the catalog's normal background and on
//!   its highlighted one. Global 0x6f2a3c (0 in our capture) would drop
//!   it. With no thumbnail kept, a definition with graphics gets one
//!   rendered (0x4cbf90, 0x4e57a6; see [`rendered`]): the object drawn on
//!   each of the template's first two cells' centre colours (0x4cbfee),
//!   cropped to a square around it and shrunk to 37 by 37, side by side.
//! - **The icon** (0x4ccaa0): a 45 by 45 picture, the first cell of
//!   `kCatalogTemplate` (`CPanel/Catalog/ThumbTemplate.bmp`, four 45 by 45
//!   cells, loaded by 0x4ccfb0), then the thumbnail over it, its left
//!   half when it is wider than 45, centred (rounding toward zero). An
//!   item with no thumbnail leaves the template alone, and the queue
//!   shows that.
//!
//! **Unverified:** that word +4 of the table 0x457cb0 finds is its
//! entries (an empty table's is taken to be 0). No capture shows a queue
//! icon.

use crate::art::Art;
use crate::{Error, Result};
use sims_content::{Catalog, Index};
use sims_formats::bitmap::{self, Image};
use sims_formats::iff::Iff;
use sims_formats::sprite::{DrawGroup, LegacySpriteSet, SpriteSet};
use std::collections::BTreeMap;

/// `kCatalogTemplate` (`UIGraphics/Res_CPanel.h`).
pub const TEMPLATE: i32 = 4300;

/// Every resource a thumbnail's icon draws, for [`Art::load`].
pub const RESOURCES: [i32; 1] = [TEMPLATE];

/// The size the icon is made at, and the template's cell size.
pub const SIZE: u32 = 45;

/// The size of a kept thumbnail of a definition with graphics.
pub const THUMBNAIL: [u32; 2] = [74, 37];

/// Definition words, by byte offset into an `OBJD`.
const BASE_GRAPHIC: usize = 0x06;
const GRAPHICS: usize = 0x08;
const TABLE: usize = 0x0e;
const KIND: usize = 0x12;
const MASTER: usize = 0x14;
const SUB_INDEX: usize = 0x16;
const CATALOG_STRINGS: usize = 0x52;
const DEFAULT_DIRECTION: usize = 0x7a;
/// A person's type.
const PERSON: i16 = 2;
/// Maximum isometric balloon picture side to guard against malformed sprites.
const MAX_PICTURE_SIDE: i64 = 2048;

/// An object type's catalog item, as far as the queue uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The thumbnail kept, if any.
    pub thumbnail: Option<Image>,
}

/// The catalog items and rendered isometric help-balloon pictures of object
/// types, read once each, by GUID.
#[derive(Debug, Clone, Default)]
pub struct Thumbnails {
    items: BTreeMap<u32, Option<Item>>,
    /// The backgrounds a rendered thumbnail is drawn on (0x4cbfee): the
    /// centre pixels of the template's first two cells. Transparent when
    /// the template is not loaded.
    backgrounds: [[u8; 4]; 2],
    /// Isometric sprites for help-balloon object slots (`0x4e5306 -> 0x4384c8`),
    /// indexed `[zoom 1 (far), zoom 2 (medium), zoom 3 (near)]`.
    balloon_pictures: BTreeMap<u32, Option<[Option<Image>; 3]>>,
}

impl Thumbnails {
    /// No items yet, rendering missing thumbnails on `art`'s template
    /// colours (0x4cbfee: the pixel at the middle of cells 0 and 1).
    pub fn with_template(art: &Art) -> Self {
        let centre = SIZE / 2;
        let background = |cell: u32| {
            art.cell(TEMPLATE, 4, cell)
                .and_then(|image| image.pixel(centre, centre))
                .unwrap_or([0; 4])
        };
        Self {
            backgrounds: [background(0), background(1)],
            ..Self::default()
        }
    }

    /// Reads the catalog item `guid`'s objects show, if it was not read.
    /// A type with no file, no definition or no item has none. A file
    /// that cannot be read is an error, and the type is kept with no
    /// item; a thumbnail that does not decode is an error too, but the
    /// item is kept, without its thumbnail, as when it has none. Either
    /// way the type is not read again.
    pub fn read(&mut self, index: &Index, catalog: &Catalog, guid: u32) -> Result<()> {
        if self.items.contains_key(&guid) {
            return Ok(());
        }
        match item(index, catalog, guid, self.backgrounds) {
            Ok((item, problem)) => {
                self.items.insert(guid, item);
                problem.map_or(Ok(()), Err)
            }
            Err(e) => {
                self.items.insert(guid, None);
                Err(e)
            }
        }
    }

    /// Renders and caches the isometric balloon pictures (zooms 1..=3) for
    /// `guid` (`0x4e5306 -> 0x4384c8`), if not already cached.
    pub fn read_balloon(&mut self, index: &Index, catalog: &Catalog, guid: u32) -> Result<()> {
        if self.balloon_pictures.contains_key(&guid) {
            return Ok(());
        }
        match balloon_object_pictures(index, catalog, guid) {
            Ok(pictures) => {
                self.balloon_pictures.insert(guid, pictures);
                Ok(())
            }
            Err(e) => {
                self.balloon_pictures.insert(guid, None);
                Err(e)
            }
        }
    }

    /// Sets `guid`'s item, as tests and hosts with their own art do.
    pub fn insert(&mut self, guid: u32, item: Option<Item>) {
        self.items.insert(guid, item);
    }

    /// Sets `guid`'s rendered isometric balloon pictures `[zoom 1, zoom 2, zoom 3]`,
    /// for synthetic tests.
    pub fn insert_balloon(&mut self, guid: u32, pictures: Option<[Option<Image>; 3]>) {
        self.balloon_pictures.insert(guid, pictures);
    }

    /// `guid`'s item, if it was read and there is one.
    pub fn item(&self, guid: u32) -> Option<&Item> {
        self.items.get(&guid)?.as_ref()
    }

    /// The isometric picture of `guid` chosen for a balloon slot of size
    /// `(slot_width, slot_height)` (`0x4384c8`):
    /// - when `zoom` is `Some(z)` (`1..=3`), uses zoom `z` directly
    ///   (`0x4385cf..0x438659`, e.g. Hunger right-slot override `2` from
    ///   `0x6f3994`/`0x6f3914`);
    /// - otherwise tries zooms `3`, `2`, `1` in order and takes the first
    ///   whose horizontal and vertical half-margins are each at least `3`
    ///   pixels, falling back to zoom `1` (`0x438660..0x438702`).
    pub fn balloon_picture(
        &self,
        guid: u32,
        zoom: Option<u32>,
        slot_width: i32,
        slot_height: i32,
    ) -> Option<&Image> {
        let zooms = self.balloon_pictures.get(&guid)?.as_ref()?;
        if let Some(z) = zoom {
            return (1..=3)
                .contains(&z)
                .then(|| zooms[(z - 1) as usize].as_ref())
                .flatten();
        }
        for z in [3usize, 2, 1] {
            if let Some(image) = &zooms[z - 1] {
                let w = i32::try_from(image.width).unwrap_or(0);
                let h = i32::try_from(image.height).unwrap_or(0);
                if (slot_width - w) / 2 >= 3 && (slot_height - h) / 2 >= 3 {
                    return Some(image);
                }
            }
        }
        zooms[0].as_ref()
    }

    /// The 45 by 45 picture of `guid`'s objects (0x4ccaa0), or None when
    /// they have no catalog item (or it was not read), or the template is
    /// not loaded.
    pub fn icon(&self, art: &Art, guid: u32) -> Option<Image> {
        let item = self.item(guid)?;
        let mut icon = art.cell(TEMPLATE, 4, 0)?;
        if let Some(thumbnail) = &item.thumbnail {
            paste(&mut icon, thumbnail);
        }
        Some(icon)
    }
}

/// Puts `thumbnail` over `icon` as 0x4ccaa0 blits it: its left half when
/// it is wider than the icon, centred, offsets rounding toward zero.
fn paste(icon: &mut Image, thumbnail: &Image) {
    let size = i64::from(SIZE);
    let width = i64::from(thumbnail.width);
    let width = if width > size { width / 2 } else { width };
    let height = i64::from(thumbnail.height);
    let left = (size - width) / 2;
    let top = (size - height) / 2;
    for y in 0..height {
        for x in 0..width {
            let (tx, ty) = (left + x, top + y);
            if !(0..i64::from(icon.width)).contains(&tx)
                || !(0..i64::from(icon.height)).contains(&ty)
            {
                continue;
            }
            let (Ok(sx), Ok(sy), Ok(tx), Ok(ty)) = (
                u32::try_from(x),
                u32::try_from(y),
                usize::try_from(tx),
                usize::try_from(ty),
            ) else {
                continue;
            };
            let Some(pixel) = thumbnail.pixel(sx, sy) else {
                continue;
            };
            let at = (ty * icon.width as usize + tx) * 4;
            if let Some(target) = icon.rgba.get_mut(at..at + 4) {
                target.copy_from_slice(&pixel);
            }
        }
    }
}

/// The catalog item `guid`'s objects show (see the module's notes), and
/// the error its thumbnail gave if it did not decode.
fn item(
    index: &Index,
    catalog: &Catalog,
    guid: u32,
    backgrounds: [[u8; 4]; 2],
) -> Result<(Option<Item>, Option<Error>)> {
    let Some(object) = catalog.object(guid) else {
        return Ok((None, None));
    };
    let read = |resource: usize| -> Result<Option<Iff>> {
        let Some(resource) = index.resources.get(resource) else {
            return Ok(None);
        };
        let bytes = resource.read().map_err(Error::Content)?;
        Iff::parse(bytes).map(Some).map_err(Error::Format)
    };
    let Some(iff) = read(object.resource)? else {
        return Ok((None, None));
    };
    let Some(definition) = iff.get(b"OBJD", object.chunk).map(|c| iff.data(c)) else {
        return Ok((None, None));
    };
    let Some(lead) = lead(&iff, definition) else {
        return Ok((None, None));
    };
    if word(lead, KIND) == Some(PERSON) {
        return Ok((None, None));
    }
    let graphics = word(lead, MASTER).unwrap_or(0) != 0 || word(lead, GRAPHICS).unwrap_or(0) != 0;
    if !graphics {
        // The semi-global file is read only when the table is not in the
        // definition's own.
        let semi = match table(lead) {
            Some(id) if iff.get(b"TTAB", id).is_none() => {
                match catalog
                    .semi_globals
                    .get(&object.resource)
                    .copied()
                    .flatten()
                {
                    Some(resource) => read(resource)?,
                    None => None,
                }
            }
            _ => None,
        };
        if !interactions(&iff, semi.as_ref(), lead) {
            return Ok((None, None));
        }
    }
    let (mut thumbnail, problem) = thumbnail(&iff, lead, graphics);
    if thumbnail.is_none() && graphics {
        thumbnail = rendered(&iff, lead, backgrounds);
    }
    Ok((Some(Item { thumbnail }), problem))
}

/// The thumbnail kept for `definition`'s item (0x4cbdc3), and the error
/// its `BMP_` gave if it did not decode, which keeps none.
fn thumbnail(iff: &Iff, definition: &[u8], graphics: bool) -> (Option<Image>, Option<Error>) {
    let id = word(definition, CATALOG_STRINGS).unwrap_or(0) as u16;
    match iff.get(b"BMP_", id).map(|c| bitmap::bmp(iff.data(c))) {
        Some(Ok(image)) => {
            let kept = !graphics || [image.width, image.height] == THUMBNAIL;
            (kept.then_some(image), None)
        }
        Some(Err(e)) => (None, Some(Error::Format(e))),
        None => (None, None),
    }
}

/// The side of the square surface 0x4e57a6 draws an object on (`0xc8`).
const SURFACE: i64 = 200;

/// The thumbnail 0x4e57a6 renders for a definition with graphics and no
/// kept `BMP_` (0x4cbf90), one 37 by 37 cell per background:
///
/// - 0x4cbfcf: the cell is the template's cell size less 8.
/// - 0x438660..0x438702: the object at the nearest zoom whose picture
///   leaves at least 3 pixels each side in the 200 by 200 surface, else
///   the far zoom, centred (halves rounding toward zero).
/// - 0x4e5aee: the picture's rectangle grown to the cell's size where it
///   is smaller, then squared by growing its short side about its centre
///   (an odd extra pixel to the right or bottom), clipped to the surface
///   (its right and bottom to 199), and shrunk by `37 / width`, never
///   enlarged (`0x666748` = 1.0), through 0x4ba778.
/// - 0x4cc0eb: the two results side by side, the second at the first's
///   width.
///
/// **Unverified:** 0x4ba778's filter, taken here as an area average; its
/// output size, taken as the nearest whole pixel (it floors through
/// 0x636b98 and adds 0.5); and the flag 0x4e5886 passes for the second
/// cell (0x4384c8 `[ebp-0x67]`), taken as changing nothing but the
/// background.
fn rendered(iff: &Iff, definition: &[u8], backgrounds: [[u8; 4]; 2]) -> Option<Image> {
    let fits = |image: &Image| {
        (SURFACE - i64::from(image.width)) / 2 >= 3 && (SURFACE - i64::from(image.height)) / 2 >= 3
    };
    let picture = [3, 2]
        .into_iter()
        .filter_map(|zoom| render_object_zoom(iff, definition, zoom))
        .find(fits)
        .or_else(|| render_object_zoom(iff, definition, 1))?;
    let (w, h) = (i64::from(picture.width), i64::from(picture.height));
    let (x, y) = ((SURFACE - w) / 2, (SURFACE - h) / 2);
    let ([left, top, right, bottom], scale) = fit(x, y, w, h)?;
    let out = |size: i64| ((size as f64 * f64::from(scale)).round() as i64).max(1);
    let (out_w, out_h) = (out(right - left), out(bottom - top));
    let width = usize::try_from(out_w * 2).ok()?;
    let mut rgba = vec![0u8; width * usize::try_from(out_h).ok()? * 4];
    for (i, background) in backgrounds.into_iter().enumerate() {
        let surface = draw_on_surface(&picture, x, y, background);
        let step = f64::from(scale).recip();
        let area = step * step;
        for oy in 0..out_h {
            for ox in 0..out_w {
                let (x0, y0) = (
                    left as f64 + ox as f64 * step,
                    top as f64 + oy as f64 * step,
                );
                let mut sum = [0f64; 4];
                let mut sy = y0.floor() as i64;
                while (sy as f64) < y0 + step && sy < bottom {
                    let cover_y = (y0 + step).min(sy as f64 + 1.0) - y0.max(sy as f64);
                    let mut sx = x0.floor() as i64;
                    while (sx as f64) < x0 + step && sx < right {
                        let cover =
                            cover_y * ((x0 + step).min(sx as f64 + 1.0) - x0.max(sx as f64));
                        let at = ((sy * SURFACE + sx) * 4) as usize;
                        let a = f64::from(surface[at + 3]);
                        for c in 0..3 {
                            sum[c] += f64::from(surface[at + c]) * a * cover;
                        }
                        sum[3] += a * cover;
                        sx += 1;
                    }
                    sy += 1;
                }
                let at = ((oy as usize * width) + (i as i64 * out_w + ox) as usize) * 4;
                if sum[3] > 0.0 {
                    for c in 0..3 {
                        rgba[at + c] = (sum[c] / sum[3]).round().clamp(0.0, 255.0) as u8;
                    }
                    rgba[at + 3] = (sum[3] / area).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    Some(Image {
        width: u32::try_from(width).ok()?,
        height: u32::try_from(out_h).ok()?,
        rgba,
    })
}

/// The part of the 200 by 200 surface 0x4e5aee shrinks into a cell, for
/// a picture drawn at `(x, y)` with size `w` by `h`, and the scale:
/// `[left, top, right, bottom]`.
fn fit(x: i64, y: i64, w: i64, h: i64) -> Option<([i64; 4], f32)> {
    let cell = i64::from(SIZE) - 8;
    let [mut left, mut top, mut right, mut bottom] = [x, y, x + w, y + h];
    // 0x4e5b05..0x4e5bb8: grow a side shorter than the cell to the cell.
    let grow = |low: &mut i64, high: &mut i64| {
        let size = *high - *low;
        if size < cell {
            let half = (cell - size) / 2;
            *low -= half;
            *high += cell - size - half;
        }
    };
    if h < w {
        grow(&mut left, &mut right);
    } else if h > w {
        grow(&mut top, &mut bottom);
    } else {
        grow(&mut left, &mut right);
        grow(&mut top, &mut bottom);
    }
    // 0x4e5bbb..0x4e5bf9: square it.
    let excess = (right - left) - (bottom - top);
    let half = excess / 2;
    if excess > 0 {
        top -= half;
        bottom += excess - half;
    } else if excess < 0 {
        left += half;
        right -= excess - half;
    }
    // 0x4e5bfc..0x4e5c21.
    left = left.max(0);
    top = top.max(0);
    right = right.min(SURFACE - 1);
    bottom = bottom.min(SURFACE - 1);
    if left >= right || top >= bottom {
        return None;
    }
    let scale = (cell as f32 / (right - left) as f32).min(1.0);
    Some(([left, top, right, bottom], scale))
}

/// The 200 by 200 surface filled with `background`, `picture` drawn over
/// it at `(x, y)`.
fn draw_on_surface(picture: &Image, x: i64, y: i64, background: [u8; 4]) -> Vec<u8> {
    let side = SURFACE as usize;
    let mut surface = background.repeat(side * side);
    for py in 0..i64::from(picture.height) {
        for px in 0..i64::from(picture.width) {
            let (sx, sy) = (x + px, y + py);
            if !(0..SURFACE).contains(&sx) || !(0..SURFACE).contains(&sy) {
                continue;
            }
            let Some(src) = picture.pixel(px as u32, py as u32) else {
                continue;
            };
            let at = ((sy * SURFACE + sx) * 4) as usize;
            let a = u32::from(src[3]);
            let da = u32::from(surface[at + 3]);
            let alpha = a + da * (255 - a) / 255;
            if alpha == 0 {
                continue;
            }
            for c in 0..3 {
                surface[at + c] = ((u32::from(src[c]) * a
                    + u32::from(surface[at + c]) * da * (255 - a) / 255)
                    / alpha) as u8;
            }
            surface[at + 3] = alpha as u8;
        }
    }
    surface
}

/// The ID of the interaction table `definition` names (0x45726a): its
/// word at byte 0x0e when that is above 0. Here `definition` is a lead,
/// so -1 (the lead's own) names none either.
fn table(definition: &[u8]) -> Option<u16> {
    word(definition, TABLE)
        .filter(|&id| id > 0)
        .map(|id| id as u16)
}

/// Whether `definition`'s interaction table has entries (0x4575a1): the
/// table it names, from its own file or else the semi-global one
/// (0x457cb0, 0x4569a3), whose count (its first word) is not 0.
fn interactions(iff: &Iff, semi: Option<&Iff>, definition: &[u8]) -> bool {
    let Some(id) = table(definition) else {
        return false;
    };
    let chunk = iff
        .get(b"TTAB", id)
        .map(|c| iff.data(c))
        .or_else(|| semi.and_then(|semi| semi.get(b"TTAB", id).map(|c| semi.data(c))));
    chunk
        .and_then(|data| word(data, 0))
        .is_some_and(|count| count != 0)
}

/// The definition whose item `definition`'s objects show: itself, or for
/// a tile of a multi-tile object, its file's lead tile (0x43ae0a).
fn lead<'a>(iff: &'a Iff, definition: &'a [u8]) -> Option<&'a [u8]> {
    let master = word(definition, MASTER)?;
    if master == 0 {
        return Some(definition);
    }
    iff.all(b"OBJD")
        .map(|chunk| iff.data(chunk))
        .find(|d| word(d, MASTER) == Some(master) && word(d, SUB_INDEX) == Some(-1))
}

fn word(definition: &[u8], at: usize) -> Option<i16> {
    Some(i16::from_le_bytes(
        definition.get(at..at + 2)?.try_into().ok()?,
    ))
}

/// Renders the isometric balloon pictures for `guid` at zooms 1, 2, and 3
/// (`0x4e5306 -> 0x4384c8 -> 0x43878f`, multi-tile `0x438bbc`).
fn balloon_object_pictures(
    index: &Index,
    catalog: &Catalog,
    guid: u32,
) -> Result<Option<[Option<Image>; 3]>> {
    let Some(object) = catalog.object(guid) else {
        return Ok(None);
    };
    let Some(resource) = index.resources.get(object.resource) else {
        return Ok(None);
    };
    let bytes = resource.read().map_err(Error::Content)?;
    let iff = Iff::parse(bytes).map_err(Error::Format)?;
    let Some(definition) = iff.get(b"OBJD", object.chunk).map(|c| iff.data(c)) else {
        return Ok(None);
    };
    let zooms = [
        render_object_zoom(&iff, definition, 1),
        render_object_zoom(&iff, definition, 2),
        render_object_zoom(&iff, definition, 3),
    ];
    if zooms.iter().all(Option::is_none) {
        return Ok(None);
    }
    Ok(Some(zooms))
}

/// Renders `definition` (single-tile or multi-tile lead) at `zoom` (`1..=3`),
/// matching `0x4385a2..0x438cc2`:
/// - rejects non-lead sub-tiles (`master != 0 && sub_index != -1`);
/// - chooses direction `(word(0x7a) + 4) mod 8` (`0x4385b7`);
/// - enumerates multi-tile pieces (`0x438bbc`, `0x43183c`), rotates `(tx, ty)`,
///   projects `tile_x = (rx - ry) * (8 << zoom)`, `tile_y = (rx + ry) * (4 << zoom)`,
///   and sorts by `(tile_y, tile_x)` ascending (`0x438cc3`);
/// - looks up each tile's base `DGRP` (`0x407847` bitmask `1 << dir` and `zoom`),
///   decodes `SPR2` (falling back to `SPR#`), places each sprite at
///   `(tile_x + sprite.x, tile_y + sprite.y - frame.height - rise)` (`0x438ae4`),
///   and composites with straight alpha and the original's isometric depth space.
fn render_object_zoom(iff: &Iff, definition: &[u8], zoom: u32) -> Option<Image> {
    let master = word(definition, MASTER).unwrap_or(0);
    let sub_index = word(definition, SUB_INDEX).unwrap_or(0);
    if master != 0 && sub_index != -1 {
        return None;
    }
    let dir = (i32::from(word(definition, DEFAULT_DIRECTION).unwrap_or(0)) + 4).rem_euclid(8);
    let dir_bit = 1u32 << (dir as u32);
    let mut tiles: Vec<(&[u8], i32, i32)> = if master == 0 {
        vec![(definition, 0, 0)]
    } else {
        let mut collected = Vec::new();
        for chunk in iff.all(b"OBJD") {
            let d = iff.data(chunk);
            if word(d, MASTER) != Some(master) {
                continue;
            }
            let sub = word(d, SUB_INDEX).unwrap_or(-1);
            if sub == -1 {
                continue;
            }
            // 0x43183c: tx = (sub as i16) >> 8, ty = sub & 0xff.
            let tx = i32::from(sub >> 8);
            let ty = i32::from(sub & 0xff);
            // 0x438c0b..0x438c49: rotate (tx, ty) by dir & 7.
            let (rx, ry) = match dir & 7 {
                0 => (tx, ty),
                2 => (-ty, tx),
                4 => (-tx, -ty),
                6 => (ty, -tx),
                _ => (0, 0),
            };
            let tile_x = (rx - ry).saturating_mul(8 << zoom);
            let tile_y = (rx + ry).saturating_mul(4 << zoom);
            collected.push((d, tile_x, tile_y));
        }
        // 0x438cc3: sort back-to-front by tile_y ascending, then tile_x ascending.
        collected.sort_by_key(|&(_, tx, ty)| (ty, tx));
        collected
    };
    if tiles.is_empty() {
        return None;
    }

    // Original's sprite depth byte table (0x4ca5af) and ground row step (0x4ca2f8).
    let scale = f32::from_bits(0x43AD_9D3F);
    let r = 1.0 / f64::from(scale);
    let a = f64::from((r * 12.0) as f32);
    let root2 = f64::from(2f64.sqrt() as f32);
    let root3 = f64::from(3f64.sqrt() as f32);
    let d = (3.0 * root2 * r * root3 + a) * 16383.75;
    let step = f64::from((d * f64::from((1.0f64 / 127.0) as f32)) as f32);
    let base = a * -16383.75 - d;
    let depth_bytes: [i32; 255] =
        std::array::from_fn(|i| (step * (i as f64) + base + 0.5).trunc() as i32);
    let row_step =
        3f64.sqrt() * f64::from((2f64.sqrt() * 3.0 / f64::from(8i32 << zoom)) as f32) * 0.5;
    let top_depth = f64::from(scale - 12.5);

    let mut layers = Vec::new();
    for (tile_def, tile_x, tile_y) in tiles.drain(..) {
        if word(tile_def, GRAPHICS).unwrap_or(0) <= 0 {
            continue;
        }
        let dgrp_id = word(tile_def, BASE_GRAPHIC).unwrap_or(0) as u16;
        let Some(chunk) = iff.get(b"DGRP", dgrp_id) else {
            continue;
        };
        let Ok(dgrp) = DrawGroup::parse(iff.data(chunk)) else {
            continue;
        };
        let Some(view) = dgrp
            .views
            .iter()
            .find(|v| (v.direction & dir_bit) != 0 && v.zoom == zoom)
        else {
            continue;
        };
        let anchor = ((top_depth - f64::from(tile_y) * row_step) * 65535.0 / f64::from(scale))
            .trunc() as i32;
        for sprite in &view.sprites {
            let Ok(id) = u16::try_from(sprite.id) else {
                continue;
            };
            let frame_index = sprite.frame as usize;
            let frame = if let Some(c) = iff.get(b"SPR2", id) {
                let raw = iff.data(c);
                SpriteSet::parse(raw)
                    .and_then(|set| set.decode(raw, frame_index, iff))
                    .ok()
            } else if let Some(c) = iff.get(b"SPR#", id) {
                let raw = iff.data(c);
                LegacySpriteSet::parse(raw)
                    .and_then(|set| set.decode(raw, frame_index, iff))
                    .ok()
            } else {
                None
            };
            let Some(frame) = frame else {
                continue;
            };
            if frame.width == 0 || frame.height == 0 {
                continue;
            }
            let z = sprite.object[2];
            let rise = if z != 0.0 && z.is_finite() {
                (z * (2i32 << zoom) as f32 + 0.5).trunc() as i32
            } else {
                0
            };
            let left = i64::from(tile_x) + i64::from(sprite.x);
            let top =
                i64::from(tile_y) + i64::from(sprite.y) - i64::from(frame.height) - i64::from(rise);
            layers.push((sprite.flags & 1 != 0, frame, left, top, anchor));
        }
    }
    if layers.is_empty() {
        return None;
    }
    let min_x = layers.iter().map(|l| l.2).min()?;
    let min_y = layers.iter().map(|l| l.3).min()?;
    let max_x = layers.iter().map(|l| l.2 + i64::from(l.1.width)).max()?;
    let max_y = layers.iter().map(|l| l.3 + i64::from(l.1.height)).max()?;
    let w = max_x - min_x;
    let h = max_y - min_y;
    if w <= 0 || h <= 0 || w > MAX_PICTURE_SIDE || h > MAX_PICTURE_SIDE {
        return None;
    }
    let width = w as usize;
    let height = h as usize;
    let mut rgba = vec![0u8; width * height * 4];
    let mut depth = vec![i32::MAX; width * height];
    for (flip, frame, left, top, anchor) in layers {
        let ox = (left - min_x) as usize;
        let oy = (top - min_y) as usize;
        let fw = frame.width as usize;
        let fh = frame.height as usize;
        let has_depth = frame.flags & 2 != 0;
        for fy in 0..fh {
            for fx in 0..fw {
                let sx = if flip { fw - 1 - fx } else { fx };
                let src = fy * fw + sx;
                let a = u32::from(frame.rgba[src * 4 + 3]);
                if a == 0 {
                    continue;
                }
                let dst = (oy + fy) * width + ox + fx;
                let px_depth = if has_depth {
                    let b = frame.depth[src];
                    if b < 255 {
                        anchor.saturating_add(depth_bytes[usize::from(b)])
                    } else {
                        i32::MAX - 1
                    }
                } else {
                    anchor
                };
                if px_depth > depth[dst] {
                    continue;
                }
                let da = u32::from(rgba[dst * 4 + 3]);
                let alpha = a + da * (255 - a) / 255;
                if alpha == 0 {
                    continue;
                }
                for c in 0..3 {
                    rgba[dst * 4 + c] = ((u32::from(frame.rgba[src * 4 + c]) * a
                        + u32::from(rgba[dst * 4 + c]) * da * (255 - a) / 255)
                        / alpha) as u8;
                }
                rgba[dst * 4 + 3] = alpha as u8;
                depth[dst] = px_depth;
            }
        }
    }
    Some(Image {
        width: width as u32,
        height: height as u32,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, at: impl Fn(u32, u32) -> [u8; 4]) -> Image {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                rgba.extend(at(x, y));
            }
        }
        Image {
            width,
            height,
            rgba,
        }
    }

    fn art() -> Art {
        let mut art = Art::default();
        // Four cells: the first blue, the others grey.
        art.insert(
            TEMPLATE,
            image(180, 45, |x, _| {
                if x < 45 {
                    [0, 0, 90, 255]
                } else {
                    [50, 50, 50, 255]
                }
            }),
        );
        art
    }

    #[test]
    fn a_thumbnails_left_cell_is_centred_on_the_templates_first_cell() {
        let mut thumbnails = Thumbnails::default();
        // The left cell red, the right green.
        let thumbnail = image(74, 37, |x, _| {
            if x < 37 {
                [200, 0, 0, 255]
            } else {
                [0, 200, 0, 255]
            }
        });
        thumbnails.insert(
            7,
            Some(Item {
                thumbnail: Some(thumbnail),
            }),
        );
        let icon = thumbnails.icon(&art(), 7).unwrap();
        assert_eq!([icon.width, icon.height], [45, 45]);
        // (45 - 37) / 2 = 4 on each side.
        assert_eq!(icon.pixel(3, 20), Some([0, 0, 90, 255]));
        assert_eq!(icon.pixel(4, 4), Some([200, 0, 0, 255]));
        assert_eq!(icon.pixel(40, 40), Some([200, 0, 0, 255]));
        assert_eq!(icon.pixel(41, 20), Some([0, 0, 90, 255]));
        assert_eq!(icon.pixel(20, 41), Some([0, 0, 90, 255]));
    }

    #[test]
    fn a_narrow_thumbnail_is_centred_whole_rounding_toward_zero() {
        let mut thumbnails = Thumbnails::default();
        thumbnails.insert(
            7,
            Some(Item {
                thumbnail: Some(image(16, 16, |_, _| [200, 0, 0, 255])),
            }),
        );
        let icon = thumbnails.icon(&art(), 7).unwrap();
        // (45 - 16) / 2 = 14.
        assert_eq!(icon.pixel(13, 14), Some([0, 0, 90, 255]));
        assert_eq!(icon.pixel(14, 14), Some([200, 0, 0, 255]));
        assert_eq!(icon.pixel(29, 29), Some([200, 0, 0, 255]));
        assert_eq!(icon.pixel(30, 29), Some([0, 0, 90, 255]));
    }

    #[test]
    fn an_item_without_a_thumbnail_shows_the_template_and_no_item_shows_nothing() {
        let mut thumbnails = Thumbnails::default();
        thumbnails.insert(7, Some(Item { thumbnail: None }));
        thumbnails.insert(8, None);
        let icon = thumbnails.icon(&art(), 7).unwrap();
        assert!(icon.rgba.chunks(4).all(|p| p == [0, 0, 90, 255]));
        assert!(thumbnails.icon(&art(), 8).is_none());
        assert!(thumbnails.icon(&art(), 9).is_none());
    }

    /// An IFF file of `chunks`, as the game's files are laid out.
    fn iff(chunks: &[(&[u8; 4], u16, Vec<u8>)]) -> Iff {
        let mut bytes = vec![0; 64];
        bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
        for (tag, id, data) in chunks {
            bytes.extend(*tag);
            bytes.extend((76 + data.len() as u32).to_be_bytes());
            bytes.extend(id.to_be_bytes());
            bytes.extend(0u16.to_be_bytes());
            bytes.extend([0; 64]);
            bytes.extend(data);
        }
        Iff::parse(bytes).unwrap()
    }

    /// A definition naming interaction table `table`.
    fn definition(table: i16) -> Vec<u8> {
        let mut bytes = vec![0; 0x60];
        bytes[TABLE..TABLE + 2].copy_from_slice(&table.to_le_bytes());
        bytes
    }

    /// An interaction table's first words: its count, then its version.
    fn table_chunk(count: u16) -> Vec<u8> {
        [count.to_le_bytes(), 8u16.to_le_bytes()].concat()
    }

    #[test]
    fn a_definition_without_graphics_needs_an_interaction_table_with_entries() {
        let own = iff(&[
            (b"TTAB", 300, table_chunk(2)),
            (b"TTAB", 301, table_chunk(0)),
        ]);
        let semi = iff(&[(b"TTAB", 4096, table_chunk(1))]);
        // Its own file's table, with entries or without.
        assert!(interactions(&own, None, &definition(300)));
        assert!(!interactions(&own, Some(&semi), &definition(301)));
        // The semi-global file's, when its own has none of that ID.
        assert!(interactions(&own, Some(&semi), &definition(4096)));
        assert!(!interactions(&own, None, &definition(4096)));
        // No table: 0, -1 on a lead, or another negative number.
        for id in [0, -1, -2] {
            assert!(!interactions(&own, Some(&semi), &definition(id)));
        }
    }

    #[test]
    fn a_thumbnail_that_does_not_decode_is_an_error_and_keeps_none() {
        let mut definition = definition(0);
        definition[CATALOG_STRINGS..CATALOG_STRINGS + 2].copy_from_slice(&5u16.to_le_bytes());
        let file = iff(&[(b"BMP_", 5, b"XX not a bitmap".to_vec())]);
        let (kept, problem) = thumbnail(&file, &definition, true);
        assert!(kept.is_none());
        assert!(problem.is_some());
        // No BMP_ of that ID is no thumbnail and no error.
        let (kept, problem) = thumbnail(&iff(&[]), &definition, true);
        assert!(kept.is_none() && problem.is_none());
    }

    #[test]
    fn a_rendered_thumbnail_squares_the_picture_and_shrinks_it_into_the_cell() {
        // Tall: no grow, squared about its centre, shrunk by 37/60.
        assert_eq!(fit(90, 70, 20, 60), Some(([70, 70, 130, 130], 37.0 / 60.0)));
        // Small: both sides grow to the 37-px cell, so no shrink.
        assert_eq!(fit(95, 95, 10, 10), Some(([82, 82, 119, 119], 1.0)));
        // Wide: squared downward, then clamped to the surface's last pixel.
        assert_eq!(fit(1, 95, 198, 10), Some(([1, 1, 199, 199], 37.0 / 198.0)));
    }

    #[test]
    fn balloon_picture_picks_forced_zoom_or_largest_zoom_with_3px_margin() {
        let mut thumbnails = Thumbnails::default();
        let z1 = image(40, 30, |_, _| [10, 0, 0, 255]);
        let z2 = image(100, 90, |_, _| [0, 20, 0, 255]);
        let z3 = image(164, 120, |_, _| [0, 0, 30, 255]); // (168-164)/2 = 2 < 3
        thumbnails.insert_balloon(
            42,
            Some([Some(z1.clone()), Some(z2.clone()), Some(z3.clone())]),
        );

        // Auto-fit in 168x127 rejects zoom 3 (margin 2 < 3) and selects zoom 2 (margin 34, 18 >= 3).
        let auto = thumbnails.balloon_picture(42, None, 168, 127).unwrap();
        assert_eq!([auto.width, auto.height], [100, 90]);
        // Forced zoom 3 or zoom 1 overrides auto-fit (0x4385cf).
        let forced3 = thumbnails.balloon_picture(42, Some(3), 168, 127).unwrap();
        assert_eq!([forced3.width, forced3.height], [164, 120]);
        let forced1 = thumbnails.balloon_picture(42, Some(1), 168, 127).unwrap();
        assert_eq!([forced1.width, forced1.height], [40, 30]);
        // If even zoom 1 does not have 3px margin, falls back to zoom 1 (0x4386fa).
        let fallback = thumbnails.balloon_picture(42, None, 30, 20).unwrap();
        assert_eq!([fallback.width, fallback.height], [40, 30]);
    }
}
