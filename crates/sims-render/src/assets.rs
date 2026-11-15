//! The game's art, decoded on first use and kept.
//!
//! - Floors are in `GameData/floors.iff`: floor `id` is SPR2 `id`, `id + 256`
//!   and `id + 512` for the far, medium and near zooms.
//! - Walls are in `GameData/walls.iff`. A wall's *shape* (style) is the SPR#
//!   `style * 2 + cut` plus 0, 512 or 1024 by zoom; it has no palette and
//!   only its alpha is used, as a mask. A wall's *pattern* (wallpaper) is
//!   the SPR# `id` plus 1536, 1792 or 2048 by zoom. Each has four frames:
//!   the two edge directions, then the two diagonals.
//! - The wood trim along the top of walls is SPR# 4096 to 4098 by zoom for
//!   standing walls and 5120 to 5122 for cut ones (6144 to 6146 hold their
//!   depth), each with its own palette: 46 frames, one per way the walls
//!   meeting at a corner can run (the original's 0x53c227).
//! - Objects are drawn from their definition's draw groups (DGRP) and SPR2
//!   sprites, found through the content catalog by GUID.
//!
//! The same layout is read by the old prototype in
//! `legacy/crates/sims-render/src/walls.rs` and `floors.rs`.

use crate::camera::Zoom;
use crate::canvas::Image;
use crate::pools::Pools;
use crate::shadows::{self, Mask};
use crate::{Error, Result};
use sims_content::{Catalog, Index, Install};
use sims_formats::execution::Slots;
use sims_formats::iff::Iff;
use sims_formats::sprite::{DrawGroup, LegacySpriteSet, Palette, SpriteSet};
use std::collections::BTreeMap;

/// One sprite of an object's draw group, ready to place.
#[derive(Debug, Clone)]
pub struct Part {
    pub image: Image,
    /// The SPR2 ID, which the object's dynamic sprite flags refer to.
    pub sprite: u16,
    /// The sprite's left edge and bottom, relative to the object's anchor.
    pub offset: [i32; 2],
    pub flip: bool,
    /// The sprite's place in the world relative to the object: x and y
    /// in sixteenths of a tile in the object's own frame (before its
    /// facing turns it), and z up in height units (2 << zoom pixels). z
    /// raises the sprite on screen; all three move its depth (see the
    /// decisions log).
    pub object: [f32; 3],
}

/// What the renderer needs from an object's definition.
#[derive(Debug, Clone)]
pub struct Definition {
    resource: usize,
    /// The first draw group (OBJD word 3, counting the version as words 0
    /// and 1) and how many there are (word 4).
    pub graphic: u16,
    pub graphics: u16,
    /// The object's type (OBJD word 9): 8 is a window, 2 a person.
    pub kind: i16,
    /// The SPR2 ID of the first dynamic sprite (OBJD byte 0x80).
    pub dynamic_base: i16,
    /// The wall style the object puts on the wall it stands in (OBJD byte
    /// 0x5c): 22 and 23 mean the object brings its own wall shape.
    pub wall_style: i16,
    /// The first SPR# of that shape (OBJD byte 0xa2): full walls at
    /// `+ 0, 1, 2` by zoom, cut walls at `+ 3, 4, 5`.
    pub wall_shape: u16,
    /// Where the object holds what it contains: its SLOT entries of kind
    /// 0, in order (an object's data word 3 picks one). x and y are in
    /// sixteenths of a tile in the object's own frame, z is the height
    /// the entry's category sets, in height units (see [`slot_height`]).
    pub slots: Vec<[f32; 3]>,
    /// The shadow it casts on the floor of its tile: its shadow code
    /// before any turning, or None for no shadow (see [`crate::shadows`]).
    pub shadow: Option<u8>,
    /// How faint that shadow is: added to every mask byte (OBJD word 79,
    /// byte 0x9e).
    pub shadow_strength: i16,
}

impl Definition {
    /// Whether the object cuts its own shape into the wall it is in.
    pub fn shapes_wall(&self) -> bool {
        matches!(self.wall_style, 22 | 23) && self.wall_shape != 0
    }
}

pub struct Assets {
    index: Index,
    catalog: Catalog,
    floors: Iff,
    walls: Iff,
    files: BTreeMap<usize, Iff>,
    definitions: BTreeMap<u32, Option<Definition>>,
    floor_images: BTreeMap<(u16, usize), Option<Image>>,
    patterns: BTreeMap<(u16, usize), Option<[Image; 4]>>,
    shapes: BTreeMap<(u16, bool, usize), Option<[Image; 4]>>,
    groups: BTreeMap<(usize, u16, u32, usize), Vec<Part>>,
    object_shapes: BTreeMap<(usize, u16, bool, usize), Option<[Image; 4]>>,
    trims: BTreeMap<(bool, usize), Vec<Image>>,
    pools: Option<Pools>,
    shadow_masks: BTreeMap<(usize, u8), Option<Mask>>,
}

impl Assets {
    pub fn new(install: &Install, index: Index, catalog: Catalog) -> Result<Self> {
        let read = |path: &str| -> Result<Iff> {
            let path = install.file(path)?;
            let bytes =
                std::fs::read(&path).map_err(|e| Error::new(format!("{}: {e}", path.display())))?;
            Ok(Iff::parse(bytes)?)
        };
        Ok(Self {
            floors: read("GameData/floors.iff")?,
            walls: read("GameData/walls.iff")?,
            index,
            catalog,
            files: BTreeMap::new(),
            definitions: BTreeMap::new(),
            floor_images: BTreeMap::new(),
            patterns: BTreeMap::new(),
            shapes: BTreeMap::new(),
            groups: BTreeMap::new(),
            object_shapes: BTreeMap::new(),
            trims: BTreeMap::new(),
            pools: read_pools(install),
            shadow_masks: BTreeMap::new(),
        })
    }

    /// The lamps' pools of light, if the game's `lightMap.dat` is there
    /// and the size the original's layout gives. The original works the
    /// maps out itself when the file is missing (0x4bb5bc, 0x4a38b6);
    /// that is not ported, so without the file no pools are drawn.
    pub fn pools(&self) -> Option<&Pools> {
        self.pools.as_ref()
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// The floor's image at a zoom (0 far to 2 near), or None if the game
    /// has no such floor.
    pub fn floor(&mut self, id: u16, zoom: usize) -> Option<&Image> {
        let floors = &self.floors;
        self.floor_images
            .entry((id, zoom))
            .or_insert_with(|| {
                let sprite = id.checked_add([0, 256, 512][zoom])?;
                let chunk = floors.get(b"SPR2", sprite)?;
                let data = floors.data(chunk);
                let frame = SpriteSet::parse(data).ok()?.decode(data, 0, floors).ok()?;
                Some(frame.into())
            })
            .as_ref()
    }

    /// The floor shadow mask of a shadow code at a zoom (0 far to 2 near),
    /// or None if the code has none or the game lacks its sprites (see
    /// [`crate::shadows`]).
    pub fn shadow_mask(&mut self, zoom: usize, code: u8) -> Option<&Mask> {
        let floors = &self.floors;
        self.shadow_masks
            .entry((zoom, code))
            .or_insert_with(|| {
                let step = u16::try_from(zoom).ok()?;
                if let Some(drawn) = shadows::DRAWN.iter().position(|&c| c == code) {
                    let id = 0x300 + 0x10 * drawn as u16 + 0x20 * step;
                    return drawn_mask(floors, id);
                }
                let tile = tile_mask(floors, 0x100 * step)?;
                shadows::generate(&tile, code)
            })
            .as_ref()
    }

    /// The four frames of a wall pattern at a zoom, with its own palette.
    pub fn pattern(&mut self, id: u16, zoom: usize) -> Option<&[Image; 4]> {
        let walls = &self.walls;
        self.patterns
            .entry((id, zoom))
            .or_insert_with(|| {
                let sprite = id.checked_add([1536, 1792, 2048][zoom])?;
                let chunk = walls.get(b"SPR#", sprite)?;
                let data = walls.data(chunk);
                let set = LegacySpriteSet::parse(data).ok()?;
                four(|i| set.decode(data, i, walls).ok().map(Image::from))
            })
            .as_ref()
    }

    /// The four frames of a wall shape at a zoom. Only their alpha means
    /// anything.
    pub fn shape(&mut self, style: u16, cut: bool, zoom: usize) -> Option<&[Image; 4]> {
        let walls = &self.walls;
        self.shapes
            .entry((style, cut, zoom))
            .or_insert_with(|| {
                let sprite = style
                    .checked_mul(2)?
                    .checked_add(u16::from(cut) + [0, 512, 1024][zoom])?;
                let chunk = walls.get(b"SPR#", sprite)?;
                let data = walls.data(chunk);
                let set = LegacySpriteSet::parse(data).ok()?;
                let gray = Palette(vec![[255, 255, 255]; 256]);
                four(|i| {
                    set.decode_with_palette(data, i, &gray)
                        .ok()
                        .map(Image::from)
                })
            })
            .as_ref()
    }

    /// The wall-top trim frames at a zoom, for standing or cut walls. Empty
    /// if the game lacks them, or if they hold far more pixels than the
    /// game's do.
    pub fn trim(&mut self, cut: bool, zoom: Zoom) -> &[Image] {
        /// The frames the corner table can name.
        const FRAMES: usize = 46;
        /// The game's trims are a few thousand pixels at the near zoom.
        const BUDGET: usize = 1 << 20;
        let walls = &self.walls;
        self.trims.entry((cut, zoom.index())).or_insert_with(|| {
            let id = if cut { 5120 } else { 4096 } + zoom.index() as u16;
            let Some(chunk) = walls.get(b"SPR#", id) else {
                return Vec::new();
            };
            let data = walls.data(chunk);
            let Ok(set) = LegacySpriteSet::parse(data) else {
                return Vec::new();
            };
            let mut images = Vec::new();
            let mut pixels = 0usize;
            for i in 0..set.frames.len().min(FRAMES) {
                let Ok(frame) = set.decode(data, i, walls) else {
                    return Vec::new();
                };
                pixels = pixels.saturating_add(frame.rgba.len() / 4);
                if pixels > BUDGET {
                    return Vec::new();
                }
                images.push(Image::from(frame));
            }
            images
        })
    }

    /// The parts of an object's definition the renderer uses.
    pub fn definition(&mut self, guid: u32) -> Option<Definition> {
        if let Some(found) = self.definitions.get(&guid) {
            return found.clone();
        }
        let found = self.load_definition(guid);
        self.definitions.insert(guid, found.clone());
        found
    }

    fn load_definition(&mut self, guid: u32) -> Option<Definition> {
        let object = self.catalog.object(guid)?.clone();
        let iff = self.file(object.resource)?;
        let chunk = iff.get(b"OBJD", object.chunk)?;
        let data = iff.data(chunk);
        let word = |n: usize| {
            data.get(n * 2..n * 2 + 2)
                .map_or(0, |b| i16::from_le_bytes([b[0], b[1]]))
        };
        let slots = iff
            .get(b"SLOT", word(20) as u16)
            .and_then(|c| Slots::parse(iff.data(c)).ok())
            .map(|s| {
                s.entries
                    .iter()
                    .filter(|e| e.kind == 0)
                    .map(|e| [e.offset[0], e.offset[1], slot_height(e.height, e.offset[2])])
                    .collect()
            })
            .unwrap_or_default();
        let shadow = shadow_code(iff, &word);
        Some(Definition {
            resource: object.resource,
            graphic: word(3) as u16,
            graphics: word(4) as u16,
            kind: word(9),
            dynamic_base: word(0x80 / 2),
            wall_style: word(0x5c / 2),
            wall_shape: word(0xa2 / 2) as u16,
            slots,
            shadow,
            shadow_strength: word(SHADOW_STRENGTH),
        })
    }

    fn file(&mut self, resource: usize) -> Option<&Iff> {
        if !self.files.contains_key(&resource) {
            let bytes = self.index.resources.get(resource)?.read().ok()?;
            self.files.insert(resource, Iff::parse(bytes).ok()?);
        }
        self.files.get(&resource)
    }

    /// The sprites of draw group `group` of an object's file, for a view
    /// direction (one bit of 1, 4, 16 and 64) and zoom (0 far to 2 near).
    /// Empty if the group or the view is missing.
    pub fn group(
        &mut self,
        definition: &Definition,
        group: u16,
        direction: u32,
        zoom: usize,
    ) -> &[Part] {
        let key = (definition.resource, group, direction, zoom);
        if !self.groups.contains_key(&key) {
            let parts = self
                .file(definition.resource)
                .map(|iff| load_group(iff, group, direction, zoom as u32 + 1))
                .unwrap_or_default();
            self.groups.insert(key, parts);
        }
        &self.groups[&key]
    }
}

impl Assets {
    /// The four frames of the wall shape an object cuts into its wall
    /// (the old prototype's `load_object_wall_shapes` in
    /// `legacy/crates/sims-content/src/surfaces.rs`).
    pub fn object_shape(
        &mut self,
        definition: &Definition,
        cut: bool,
        zoom: usize,
    ) -> Option<&[Image; 4]> {
        let key = (definition.resource, definition.wall_shape, cut, zoom);
        if !self.object_shapes.contains_key(&key) {
            let id = definition
                .wall_shape
                .checked_add(u16::from(cut) * 3 + zoom as u16);
            let shape = id.and_then(|id| {
                let iff = self.file(definition.resource)?;
                let chunk = iff.get(b"SPR#", id)?;
                let data = iff.data(chunk);
                let set = LegacySpriteSet::parse(data).ok()?;
                let gray = Palette(vec![[255, 255, 255]; 256]);
                four(|i| {
                    set.decode_with_palette(data, i, &gray)
                        .ok()
                        .map(Image::from)
                })
            });
            self.object_shapes.insert(key, shape);
        }
        self.object_shapes[&key].as_ref()
    }
}

/// The height a SLOT entry holds its object at, in height units: set by
/// the entry's height category (low table 2.5, table and end table 4,
/// counter 4.05), or the entry's own z for the categories that keep it.
/// This is the original's conversion as the old prototype read it and
/// checked against the original's values
/// (`legacy/crates/sims-vm/src/routing.rs`, `containment_height`).
pub fn slot_height(category: i32, z: f32) -> f32 {
    match category {
        1 => 0.0,
        2 => 2.5,
        3 | 8 => 4.0,
        4 => f32::from_bits(0x4081_999a),
        _ => z,
    }
}

/// Reads `GameData/lightMap.dat` (the original opens "lightmap.dat" in
/// the game's data folder, 0x4b5700) and its pool maps, or None if it is
/// missing, unreadable or not the expected size.
fn read_pools(install: &Install) -> Option<Pools> {
    let path = install.file("GameData/lightMap.dat").ok()?;
    Pools::parse(&std::fs::read(path).ok()?)
}

/// Definition words of the floor shadow: the tile's place in its group
/// (high byte x, low byte y), the word that keeps a tile out of its
/// neighbours' shadow, the shadow word and the strength.
const GROUP: usize = 10;
const GROUP_PLACE: usize = 11;
const SHADOW_SKIP: usize = 56;
const SHADOW: usize = 57;
const SHADOW_STRENGTH: usize = 0x9e / 2;

/// The shadow code of a definition whose words `word` gives, among the
/// definitions of its file (0x4573e7, see [`crate::shadows`]).
fn shadow_code(iff: &Iff, word: &impl Fn(usize) -> i16) -> Option<u8> {
    if word(SHADOW) == -1 {
        return None;
    }
    let group = word(GROUP);
    if group == 0 {
        return Some(word(SHADOW) as u8);
    }
    let places: Vec<u16> = iff
        .all(b"OBJD")
        .filter_map(|chunk| {
            let data = iff.data(chunk);
            let w = |n: usize| {
                data.get(n * 2..n * 2 + 2)
                    .map_or(0, |b| i16::from_le_bytes([b[0], b[1]]))
            };
            (w(GROUP) == group && w(SHADOW_SKIP) == 0 && w(SHADOW) != -1)
                .then(|| w(GROUP_PLACE) as u16)
        })
        .collect();
    let place = word(GROUP_PLACE);
    // The original refuses a negative place, then packs the rest as a
    // word with only `y`'s low byte (0x43ac5c, 0x43185a), so a `y` of 256
    // wraps to 0.
    let shaded =
        |x: i32, y: i32| x >= 0 && y >= 0 && places.contains(&(((x << 8) | (y & 0xff)) as u16));
    Some(shadows::group_code(
        i32::from(place >> 8),
        i32::from(place & 0xff),
        shaded,
    ))
}

/// One SPR2 frame of `floors.iff` as palette indices: its width, height,
/// transparent index and, for each pixel, the index or None where the
/// frame draws nothing.
fn indices(floors: &Iff, id: u16) -> Option<(usize, usize, u8, Vec<Option<u8>>)> {
    let data = floors.data(floors.get(b"SPR2", id)?);
    let set = SpriteSet::parse(data).ok()?;
    let frame = data.get(set.frames.first()?.clone())?;
    let transparent = *frame.get(10)?;
    let identity = Palette((0..=255u8).map(|i| [i; 3]).collect());
    let decoded = sims_formats::sprite::Frame::decode(frame, &identity).ok()?;
    let pixels = decoded
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| (p[3] != 0).then_some(p[0]))
        .collect();
    Some((
        decoded.width as usize,
        decoded.height as usize,
        transparent,
        pixels,
    ))
}

/// A floor tile's shape for working out shadow masks: its indices, 255
/// where it draws nothing or its transparent index (0x4a3cd3 and its
/// decoder).
fn tile_mask(floors: &Iff, id: u16) -> Option<Mask> {
    let (width, height, transparent, pixels) = indices(floors, id)?;
    let values = pixels
        .into_iter()
        .map(|p| p.filter(|&i| i != transparent).unwrap_or(255))
        .collect();
    Some(Mask {
        width,
        height,
        values,
    })
}

/// A hand-drawn shadow mask: its indices, with the transparent index
/// where it draws nothing, and the runs of the transparent index at each
/// row's two ends made 255 (0x4c90c1, 0x4c92af).
fn drawn_mask(floors: &Iff, id: u16) -> Option<Mask> {
    let (width, height, transparent, pixels) = indices(floors, id)?;
    let mut values: Vec<u8> = pixels
        .into_iter()
        .map(|p| p.unwrap_or(transparent))
        .collect();
    for row in values.chunks_mut(width.max(1)) {
        for v in row.iter_mut().take_while(|v| **v == transparent) {
            *v = 255;
        }
        for v in row.iter_mut().rev().take_while(|v| **v == transparent) {
            *v = 255;
        }
    }
    Some(Mask {
        width,
        height,
        values,
    })
}

fn four(mut frame: impl FnMut(usize) -> Option<Image>) -> Option<[Image; 4]> {
    Some([frame(0)?, frame(1)?, frame(2)?, frame(3)?])
}

fn load_group(iff: &Iff, group: u16, direction: u32, zoom: u32) -> Vec<Part> {
    let Some(chunk) = iff.get(b"DGRP", group) else {
        return Vec::new();
    };
    let Ok(dgrp) = DrawGroup::parse(iff.data(chunk)) else {
        return Vec::new();
    };
    let Some(view) = dgrp
        .views
        .iter()
        .find(|v| v.direction == direction && v.zoom == zoom)
    else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    for s in &view.sprites {
        let Ok(id) = u16::try_from(s.id) else {
            continue;
        };
        let Some(chunk) = iff.get(b"SPR2", id) else {
            continue;
        };
        let data = iff.data(chunk);
        let Ok(frame) =
            SpriteSet::parse(data).and_then(|set| set.decode(data, s.frame as usize, iff))
        else {
            continue;
        };
        if frame.width == 0 || frame.height == 0 {
            continue;
        }
        let image = Image::from(frame);
        parts.push(Part {
            offset: [s.x, s.y],
            flip: s.flags & 1 != 0,
            object: s.object,
            sprite: id,
            image,
        });
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An IFF holding an OBJD per word list, each set at the given words.
    fn definitions(objects: &[&[(usize, i16)]]) -> Iff {
        let mut bytes = vec![0; 64];
        bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
        for (id, words) in objects.iter().enumerate() {
            let mut data = vec![0u8; 2 * (SHADOW + 1)];
            for &(n, w) in *words {
                data[n * 2..n * 2 + 2].copy_from_slice(&w.to_le_bytes());
            }
            bytes.extend(b"OBJD");
            bytes.extend((76 + data.len() as u32).to_be_bytes());
            bytes.extend((id as u16 + 128).to_be_bytes());
            bytes.extend(0u16.to_be_bytes());
            bytes.extend([0; 64]);
            bytes.extend(data);
        }
        Iff::parse(bytes).expect("a well-formed file")
    }

    #[test]
    fn a_group_neighbour_wraps_in_the_places_low_byte() {
        // Two tiles of one group: one at place 0x00ff, one at 0x0000. The
        // first's neighbour one step along y is (0, 256), which the
        // original packs as 0x0000, so it is shaded there (bit 2).
        let first: &[(usize, i16)] = &[(GROUP, 1), (GROUP_PLACE, 0x00ff), (SHADOW, 0)];
        let second: &[(usize, i16)] = &[(GROUP, 1), (GROUP_PLACE, 0), (SHADOW, 0)];
        let iff = definitions(&[first, second]);
        let word = |words: &'static [(usize, i16)]| {
            move |n: usize| words.iter().find(|&&(m, _)| m == n).map_or(0, |&(_, w)| w)
        };
        assert_eq!(shadow_code(&iff, &word(first)), Some(4));
        // The second's neighbour one step back along y is (0, -1), which
        // the original refuses rather than wrapping to 0x00ff.
        assert_eq!(shadow_code(&iff, &word(second)), Some(0));
    }
}
