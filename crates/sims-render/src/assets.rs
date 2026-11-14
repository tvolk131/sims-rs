//! The game's art, decoded on first use and kept.
//!
//! - Floors are in `GameData/floors.iff`: floor `id` is SPR2 `id`, `id + 256`
//!   and `id + 512` for the far, medium and near zooms.
//! - Walls are in `GameData/walls.iff`. A wall's *shape* (style) is the SPR#
//!   `style * 2 + cut` plus 0, 512 or 1024 by zoom; it has no palette and
//!   only its alpha is used, as a mask. A wall's *pattern* (wallpaper) is
//!   the SPR# `id` plus 1536, 1792 or 2048 by zoom. Each has four frames:
//!   the two edge directions, then the two diagonals.
//! - Objects are drawn from their definition's draw groups (DGRP) and SPR2
//!   sprites, found through the content catalog by GUID.
//!
//! The same layout is read by the old prototype in
//! `legacy/crates/sims-render/src/walls.rs` and `floors.rs`.

use crate::canvas::Image;
use crate::{Error, Result};
use sims_content::{Catalog, Index, Install};
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
    /// The sprite's place in the world relative to the object, in tiles:
    /// x and y in the object's own frame (before its facing turns it),
    /// and z up. It moves the sprite's depth, not its screen position
    /// (see the decisions log).
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
    /// The SPR2 ID of the first dynamic sprite (OBJD byte 0x80).
    pub dynamic_base: i16,
    /// The wall style the object puts on the wall it stands in (OBJD byte
    /// 0x5c): 22 and 23 mean the object brings its own wall shape.
    pub wall_style: i16,
    /// The first SPR# of that shape (OBJD byte 0xa2): full walls at
    /// `+ 0, 1, 2` by zoom, cut walls at `+ 3, 4, 5`.
    pub wall_shape: u16,
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
        })
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
        Some(Definition {
            resource: object.resource,
            graphic: word(3) as u16,
            graphics: word(4) as u16,
            dynamic_base: word(0x80 / 2),
            wall_style: word(0x5c / 2),
            wall_shape: word(0xa2 / 2) as u16,
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
