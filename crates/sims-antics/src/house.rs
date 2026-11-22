//! Clean-room procedural floor/wall surface (`.flr` / `.wll`) compiler and
//! starter `House01.iff` + `Neighborhood.iff` binary builders.
//!
//! All emitted IFF 2.5 files validate cleanly through `sims_formats` (`Iff`,
//! `SpriteSet`, `LegacySpriteSet`, `Array`, `House`, `HouseView`, `Simulator`,
//! `surface_map`, `ObjectTypes`, `Objects`, `Family`, `Neighbors`, `Neighborhood`).

use crate::art::{FurnitureShape, generate_furniture_sprites};
use crate::compile::compile_antics;
use crate::encode::{
    encode_bhav, encode_dgrp, encode_iff_with_rsmp, encode_legacy_spr, encode_objd, encode_objf,
    encode_palt, encode_spr2, encode_string_table,
};
use crate::error::{Error, Result};
use crate::ir::{
    IFF_CONVERTED_FLAG, LegacySprChunk, LegacySprFrame, ObjdSpec, ObjfChunk, PaltChunk, Spr2Chunk,
    Spr2Frame, Spr2Pixel, StringEntry, StringFormat, StringTableChunk,
};
use crate::manifest::{SpriteSpec, TomlValue, parse_toml};
use sims_formats::execution::ObjectFunction;
use sims_formats::iff::ChunkData;
use std::collections::BTreeMap;

/// Bare ground-level wall cell matching `sims_sim::load::BARE_GROUND_WALL`.
const BARE_GROUND_WALL: [u8; 14] = [0, 0x0f, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];

// ============================================================================
// Part 1: Procedural Floor (`.flr`) & Wall (`.wll`) Surface Compiler
// ============================================================================

/// Procedural surface pattern kind for clean-room floor tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorPattern {
    Plank,
    Tile,
    Carpet,
    Checker,
}

/// Procedural surface pattern kind for clean-room wall coverings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallPattern {
    Plaster,
    Brick,
    Tile,
    Panel,
}

/// Specification for one clean-room floor tile (`.flr`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorSurfaceSpec {
    pub file_name: String,
    pub saved_id: u16,
    pub name: String,
    pub price: u16,
    pub description: String,
    pub pattern: FloorPattern,
    pub primary_rgb: [u8; 3],
    pub accent_rgb: [u8; 3],
}

/// Specification for one clean-room wall covering (`.wll`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallSurfaceSpec {
    pub file_name: String,
    pub saved_id: u16,
    pub name: String,
    pub price: u16,
    pub description: String,
    pub pattern: WallPattern,
    pub primary_rgb: [u8; 3],
    pub accent_rgb: [u8; 3],
}

/// Parsed `surfaces.toml` manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfacesManifest {
    pub floors: Vec<FloorSurfaceSpec>,
    pub walls: Vec<WallSurfaceSpec>,
}

/// Compiled procedural `.flr` and `.wll` IFF files.
#[derive(Debug, Clone)]
pub struct CompiledSurfaces {
    pub floors: BTreeMap<String, Vec<u8>>,
    pub walls: BTreeMap<String, Vec<u8>>,
    pub floor_specs: Vec<FloorSurfaceSpec>,
    pub wall_specs: Vec<WallSurfaceSpec>,
}

impl SurfacesManifest {
    /// Parses `open-pack/surfaces.toml`.
    pub fn parse(source: &str) -> Result<Self> {
        let root = parse_toml(source)?;
        let mut floors = Vec::new();
        let mut walls = Vec::new();

        for (key, value) in &root {
            match (key.as_str(), value) {
                ("floor", TomlValue::Array(arr)) => {
                    for item in arr {
                        let TomlValue::Table(t) = item else {
                            return Err(Error::new("[[floor]] entries must be tables"));
                        };
                        let file_name = req_str(t, "file")?;
                        validate_surface_filename(&file_name, ".flr")?;
                        let saved_id = req_u16(t, "id")?;
                        if saved_id < 30 || saved_id == 255 || saved_id == u16::MAX {
                            return Err(Error::new(format!(
                                "[[floor]] id {saved_id} must be a valid plug-in floor key (>= 30, != 255, != 65535)"
                            )));
                        }
                        let name = req_str(t, "name")?;
                        let price = opt_u16(t, "price")?.unwrap_or(10);
                        let description = opt_str(t, "description").unwrap_or_default();
                        let pattern = match opt_str(t, "pattern").as_deref() {
                            Some("plank") | None => FloorPattern::Plank,
                            Some("tile") => FloorPattern::Tile,
                            Some("carpet") => FloorPattern::Carpet,
                            Some("checker") => FloorPattern::Checker,
                            Some(other) => {
                                return Err(Error::new(format!(
                                    "unknown [[floor]].pattern '{other}'"
                                )));
                            }
                        };
                        let primary_rgb = req_rgb(t, "primary_rgb")?;
                        let accent_rgb = req_rgb(t, "accent_rgb")?;
                        floors.push(FloorSurfaceSpec {
                            file_name,
                            saved_id,
                            name,
                            price,
                            description,
                            pattern,
                            primary_rgb,
                            accent_rgb,
                        });
                    }
                }
                ("wall", TomlValue::Array(arr)) => {
                    for item in arr {
                        let TomlValue::Table(t) = item else {
                            return Err(Error::new("[[wall]] entries must be tables"));
                        };
                        let file_name = req_str(t, "file")?;
                        validate_surface_filename(&file_name, ".wll")?;
                        let saved_id = req_u16(t, "id")?;
                        if saved_id < 31 || (248..=255).contains(&saved_id) || saved_id == u16::MAX
                        {
                            return Err(Error::new(format!(
                                "[[wall]] id {saved_id} must be a valid plug-in wall key (>= 31, not 248..=255 or 65535)"
                            )));
                        }
                        let name = req_str(t, "name")?;
                        let price = opt_u16(t, "price")?.unwrap_or(10);
                        let description = opt_str(t, "description").unwrap_or_default();
                        let pattern = match opt_str(t, "pattern").as_deref() {
                            Some("plaster") | None => WallPattern::Plaster,
                            Some("brick") => WallPattern::Brick,
                            Some("tile") => WallPattern::Tile,
                            Some("panel") => WallPattern::Panel,
                            Some(other) => {
                                return Err(Error::new(format!(
                                    "unknown [[wall]].pattern '{other}'"
                                )));
                            }
                        };
                        let primary_rgb = req_rgb(t, "primary_rgb")?;
                        let accent_rgb = req_rgb(t, "accent_rgb")?;
                        walls.push(WallSurfaceSpec {
                            file_name,
                            saved_id,
                            name,
                            price,
                            description,
                            pattern,
                            primary_rgb,
                            accent_rgb,
                        });
                    }
                }
                (other, _) => {
                    return Err(Error::new(format!(
                        "unknown section '{other}' in surfaces.toml"
                    )));
                }
            }
        }

        Ok(Self { floors, walls })
    }
}

/// Compiles all `.flr` and `.wll` surface files from a [`SurfacesManifest`].
pub fn compile_surfaces(manifest: &SurfacesManifest) -> Result<CompiledSurfaces> {
    let mut floors = BTreeMap::new();
    let mut seen_floor_lower = std::collections::BTreeSet::new();
    let mut seen_floor_ids = std::collections::BTreeSet::new();
    let mut walls = BTreeMap::new();
    let mut seen_wall_lower = std::collections::BTreeSet::new();
    let mut seen_wall_ids = std::collections::BTreeSet::new();
    for spec in &manifest.floors {
        if !seen_floor_lower.insert(spec.file_name.to_ascii_lowercase()) {
            return Err(Error::new(format!(
                "duplicate/case-colliding floor file '{}'",
                spec.file_name
            )));
        }
        if !seen_floor_ids.insert(spec.saved_id) {
            return Err(Error::new(format!(
                "duplicate floor saved id {}",
                spec.saved_id
            )));
        }
        let bytes = compile_floor_iff(spec)?;
        floors.insert(spec.file_name.clone(), bytes);
    }
    for spec in &manifest.walls {
        if !seen_wall_lower.insert(spec.file_name.to_ascii_lowercase()) {
            return Err(Error::new(format!(
                "duplicate/case-colliding wall file '{}'",
                spec.file_name
            )));
        }
        if !seen_wall_ids.insert(spec.saved_id) {
            return Err(Error::new(format!(
                "duplicate wall saved id {}",
                spec.saved_id
            )));
        }
        let bytes = compile_wall_iff(spec)?;
        walls.insert(spec.file_name.clone(), bytes);
    }
    Ok(CompiledSurfaces {
        floors,
        walls,
        floor_specs: manifest.floors.clone(),
        wall_specs: manifest.walls.clone(),
    })
}

/// Compiles a single clean-room `.flr` IFF file with `STR# 0`, `PALT 1`, and
/// 3 zoom-level `SPR2` chunks (`1` far, `257` mid, `513` near).
pub fn compile_floor_iff(spec: &FloorSurfaceSpec) -> Result<Vec<u8>> {
    let mut palette = SurfacePalette::new();
    let base = palette.intern(spec.primary_rgb);
    let light = palette.intern(shade_rgb(spec.primary_rgb, 112, 100));
    let dark = palette.intern(shade_rgb(spec.primary_rgb, 84, 100));
    let accent = palette.intern(spec.accent_rgb);

    let mut chunks = Vec::new();
    let str0 = StringTableChunk::new(
        *b"STR#",
        0,
        "Floor Info",
        StringFormat::Localized,
        vec![
            StringEntry::english(&spec.name),
            StringEntry::english(spec.price.to_string()),
            StringEntry::english(&spec.description),
        ],
    );
    chunks.push(ChunkData {
        tag: *b"STR#",
        id: 0,
        flags: IFF_CONVERTED_FLAG,
        label: str0.label.clone(),
        data: encode_string_table(&str0)?,
    });

    let palt = PaltChunk {
        id: 1,
        label: format!("{} Palette", spec.name),
        colors: palette.colors,
    };
    chunks.push(ChunkData {
        tag: *b"PALT",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: palt.label.clone(),
        data: encode_palt(&palt)?,
    });

    // Zoom 0 (far: 32x16), Zoom 1 (mid: 64x32), Zoom 2 (near: 128x64) -> SPR2 IDs 1, 257, 513.
    for (zoom_idx, (w, h)) in [(32u16, 16u16), (64, 32), (128, 64)]
        .into_iter()
        .enumerate()
    {
        let sprite_id = 1u16 + (zoom_idx as u16) * 256;
        let width = usize::from(w);
        let height = usize::from(h);
        let hw = width / 2;
        let hh = height / 2;
        let mut pixels = vec![
            Spr2Pixel {
                color: 0,
                depth: 255,
                alpha: 0,
            };
            width * height
        ];
        for y in 0..height {
            let dy = if y < hh { hh - 1 - y } else { y - hh };
            for x in 0..width {
                let dx = if x < hw { hw - 1 - x } else { x - hw };
                // Isometric diamond mask: dx / hw + dy / hh < 1.
                if dx * hh + dy * hw < hw * hh {
                    let color = match spec.pattern {
                        FloorPattern::Plank => {
                            let band = (x + y * 2) / (4 << zoom_idx).max(2);
                            if (x + y * 2) % (4 << zoom_idx).max(2) == 0 {
                                accent
                            } else if band.is_multiple_of(2) {
                                base
                            } else {
                                light
                            }
                        }
                        FloorPattern::Tile => {
                            let step = (4 << zoom_idx).max(4);
                            if x.is_multiple_of(step) || y.is_multiple_of(step / 2) {
                                accent
                            } else {
                                base
                            }
                        }
                        FloorPattern::Carpet => {
                            if (x + y).is_multiple_of(2) {
                                base
                            } else if (x * 3 + y * 5).is_multiple_of(7) {
                                accent
                            } else {
                                dark
                            }
                        }
                        FloorPattern::Checker => {
                            let cell = (4 << zoom_idx).max(4);
                            if ((x / cell) + (y / (cell / 2))).is_multiple_of(2) {
                                base
                            } else {
                                accent
                            }
                        }
                    };
                    pixels[y * width + x] = Spr2Pixel {
                        color,
                        depth: 200,
                        alpha: 31,
                    };
                }
            }
        }
        let spr2 = Spr2Chunk {
            id: sprite_id,
            label: format!("{} Z{}", spec.name, zoom_idx),
            version: 1001,
            default_palette: 1,
            frames: vec![Spr2Frame {
                width: w,
                height: h,
                flags: 0x07,
                palette_id: 1,
                transparent_index: 0,
                x: 0,
                y: 0,
                pixels,
            }],
        };
        chunks.push(ChunkData {
            tag: *b"SPR2",
            id: sprite_id,
            flags: IFF_CONVERTED_FLAG,
            label: spr2.label.clone(),
            data: encode_spr2(&spr2)?,
        });
    }

    encode_iff_with_rsmp(&chunks)
}

/// Compiles a single clean-room `.wll` IFF file with `STR# 0`, `PALT 1`, and
/// 3 zoom-level legacy `SPR#` chunks (`1` far, `257` mid, `513` near), each
/// containing 4 projection frames matching `sims-render`.
pub fn compile_wall_iff(spec: &WallSurfaceSpec) -> Result<Vec<u8>> {
    let mut palette = SurfacePalette::new();
    let base = palette.intern(spec.primary_rgb);
    let light = palette.intern(shade_rgb(spec.primary_rgb, 110, 100));
    let dark = palette.intern(shade_rgb(spec.primary_rgb, 80, 100));
    let accent = palette.intern(spec.accent_rgb);

    let mut chunks = Vec::new();
    let str0 = StringTableChunk::new(
        *b"STR#",
        0,
        "Wall Info",
        StringFormat::Localized,
        vec![
            StringEntry::english(&spec.name),
            StringEntry::english(spec.price.to_string()),
            StringEntry::english(&spec.description),
        ],
    );
    chunks.push(ChunkData {
        tag: *b"STR#",
        id: 0,
        flags: IFF_CONVERTED_FLAG,
        label: str0.label.clone(),
        data: encode_string_table(&str0)?,
    });

    let palt = PaltChunk {
        id: 1,
        label: format!("{} Palette", spec.name),
        colors: palette.colors,
    };
    chunks.push(ChunkData {
        tag: *b"PALT",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: palt.label.clone(),
        data: encode_palt(&palt)?,
    });

    // Zoom 0 (far: 16x48), Zoom 1 (mid: 32x96), Zoom 2 (near: 64x192) -> SPR# IDs 1, 257, 513.
    for (zoom_idx, (w, h)) in [(16u16, 48u16), (32, 96), (64, 192)]
        .into_iter()
        .enumerate()
    {
        let sprite_id = 1u16 + (zoom_idx as u16) * 256;
        let width = usize::from(w);
        let height = usize::from(h);
        let mut frames = Vec::with_capacity(4);
        for side in 0..4usize {
            let mut pixels = vec![None; width * height];
            let side_shade = if side % 2 == 0 { base } else { dark };
            for y in 0..height {
                for x in 0..width {
                    let c = match spec.pattern {
                        WallPattern::Plaster => {
                            if y >= height - (3 << zoom_idx).min(height) {
                                accent
                            } else if (x + y).is_multiple_of(5) {
                                light
                            } else {
                                side_shade
                            }
                        }
                        WallPattern::Brick => {
                            let row_h = (3 << zoom_idx).max(3);
                            let col_w = (6 << zoom_idx).max(6);
                            let r = y / row_h;
                            let offset = if r.is_multiple_of(2) { 0 } else { col_w / 2 };
                            if y.is_multiple_of(row_h) || (x + offset).is_multiple_of(col_w) {
                                accent
                            } else {
                                side_shade
                            }
                        }
                        WallPattern::Tile => {
                            let step = (4 << zoom_idx).max(4);
                            if x.is_multiple_of(step) || y.is_multiple_of(step) {
                                accent
                            } else if y < height / 2 {
                                light
                            } else {
                                side_shade
                            }
                        }
                        WallPattern::Panel => {
                            let step = (4 << zoom_idx).max(4);
                            if y == height / 2 || y >= height - (2 << zoom_idx) {
                                accent
                            } else if x.is_multiple_of(step) {
                                dark
                            } else {
                                side_shade
                            }
                        }
                    };
                    pixels[y * width + x] = Some(c);
                }
            }
            frames.push(LegacySprFrame {
                width: w,
                height: h,
                pixels,
            });
        }
        let spr = LegacySprChunk {
            id: sprite_id,
            label: format!("{} Z{}", spec.name, zoom_idx),
            version: 503,
            default_palette: 1,
            frames,
        };
        chunks.push(ChunkData {
            tag: *b"SPR#",
            id: sprite_id,
            flags: IFF_CONVERTED_FLAG,
            label: spr.label.clone(),
            data: encode_legacy_spr(&spr)?,
        });
    }

    encode_iff_with_rsmp(&chunks)
}

struct SurfacePalette {
    colors: Vec<[u8; 3]>,
    index: BTreeMap<[u8; 3], u8>,
}

impl SurfacePalette {
    fn new() -> Self {
        let mut index = BTreeMap::new();
        index.insert([0, 0, 0], 0);
        Self {
            colors: vec![[0, 0, 0]],
            index,
        }
    }

    fn intern(&mut self, rgb: [u8; 3]) -> u8 {
        if let Some(&idx) = self.index.get(&rgb) {
            return idx;
        }
        let idx = self.colors.len().min(255) as u8;
        if self.colors.len() < 256 {
            self.colors.push(rgb);
            self.index.insert(rgb, idx);
        }
        idx
    }
}

fn shade_rgb(rgb: [u8; 3], num: u16, den: u16) -> [u8; 3] {
    [
        ((u16::from(rgb[0]) * num / den).min(255)) as u8,
        ((u16::from(rgb[1]) * num / den).min(255)) as u8,
        ((u16::from(rgb[2]) * num / den).min(255)) as u8,
    ]
}

// ============================================================================
// Part 2: Starter House (`House01.iff`) & 2-Sim Household (`Neighborhood.iff`)
// ============================================================================

/// One rectangular room stamped into the starter house's floor and wall grids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StarterRoomSpec {
    pub name: String,
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
    pub floor_id: u16,
    pub wall_pattern: u16,
    pub wall_style: u16,
    pub enclose_walls: bool,
}

/// One placed furniture item inside the starter house.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StarterPlacementSpec {
    pub object_slug: String,
    pub x: i32,
    pub y: i32,
    pub level: i32,
    pub direction: i16,
}

/// One starter Sim in the clean-room household.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StarterSimSpec {
    pub name: String,
    pub guid: u32,
    pub neighbor_id: i16,
    pub x: i32,
    pub y: i32,
    pub cooking: i16,
    pub mechanical: i16,
    pub charisma: i16,
    pub neat: i16,
    pub outgoing: i16,
    pub active: i16,
    pub playful: i16,
    pub nice: i16,
}

/// Wall edge for an interior or exterior doorway opening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorwaySide {
    West,
    North,
}

/// One traversable wall opening punched through a room boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StarterDoorwaySpec {
    pub x: usize,
    pub y: usize,
    pub side: DoorwaySide,
}

/// Parsed `starter-house.toml` manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StarterHouseManifest {
    pub house_name: String,
    pub family_name: String,
    pub family_bio: String,
    pub lot_number: i32,
    pub funds: i32,
    pub camera_direction: i16,
    pub roof: String,
    pub rooms: Vec<StarterRoomSpec>,
    pub doorways: Vec<StarterDoorwaySpec>,
    pub placements: Vec<StarterPlacementSpec>,
    pub sims: Vec<StarterSimSpec>,
}

impl StarterHouseManifest {
    /// Parses `open-pack/starter-house.toml`.
    pub fn parse(source: &str) -> Result<Self> {
        let root = parse_toml(source)?;
        for (key, value) in &root {
            match (key.as_str(), value) {
                ("house", TomlValue::Table(_)) => {}
                ("room" | "doorway" | "placement" | "sim", TomlValue::Array(_)) => {}
                (other, _) => {
                    return Err(Error::new(format!(
                        "unknown or invalid section '{other}' in starter-house.toml"
                    )));
                }
            }
        }
        let house = match root.get("house") {
            Some(TomlValue::Table(t)) => t,
            _ => return Err(Error::new("starter-house.toml requires a [house] section")),
        };

        let house_name = req_str(house, "name")?;
        let family_name =
            opt_str(house, "family_name").unwrap_or_else(|| "Open Household".to_owned());
        let family_bio = opt_str(house, "family_bio").unwrap_or_default();
        let lot_number = opt_i32(house, "lot_number")?.unwrap_or(1);
        if !(1..=99).contains(&lot_number) {
            return Err(Error::new(format!(
                "[house].lot_number {lot_number} must be in 1..=99"
            )));
        }
        let funds = opt_i32(house, "funds")?.unwrap_or(20_000);
        let camera_direction = opt_i16(house, "camera_direction")?.unwrap_or(0);
        let roof = opt_str(house, "roof").unwrap_or_else(|| "open_shingle".to_owned());

        let mut rooms = Vec::new();
        if let Some(TomlValue::Array(arr)) = root.get("room") {
            for item in arr {
                let TomlValue::Table(t) = item else {
                    return Err(Error::new("[[room]] entries must be tables"));
                };
                let name = opt_str(t, "name").unwrap_or_else(|| "Room".to_owned());
                let x0 = usize::from(req_u16(t, "x0")?);
                let y0 = usize::from(req_u16(t, "y0")?);
                let x1 = usize::from(req_u16(t, "x1")?);
                let y1 = usize::from(req_u16(t, "y1")?);
                if x0 >= x1 || y0 >= y1 || x1 >= 64 || y1 >= 64 {
                    return Err(Error::new(format!(
                        "[[room]] '{name}' bounds ({x0}..{x1}, {y0}..{y1}) outside 0..64"
                    )));
                }
                let floor_id = opt_u16(t, "floor_id")?.unwrap_or(30);
                let wall_pattern = opt_u16(t, "wall_pattern")?.unwrap_or(31);
                let wall_style = opt_u16(t, "wall_style")?.unwrap_or(1);
                let enclose_walls = opt_bool(t, "enclose_walls")?.unwrap_or(true);
                rooms.push(StarterRoomSpec {
                    name,
                    x0,
                    y0,
                    x1,
                    y1,
                    floor_id,
                    wall_pattern,
                    wall_style,
                    enclose_walls,
                });
            }
        }

        let mut doorways = Vec::new();
        if let Some(TomlValue::Array(arr)) = root.get("doorway") {
            for item in arr {
                let TomlValue::Table(t) = item else {
                    return Err(Error::new("[[doorway]] entries must be tables"));
                };
                let x = usize::from(req_u16(t, "x")?);
                let y = usize::from(req_u16(t, "y")?);
                if x >= 64 || y >= 64 {
                    return Err(Error::new(format!("[[doorway]] ({x}, {y}) outside 0..64")));
                }
                let side = match req_str(t, "side")?.as_str() {
                    "west" => DoorwaySide::West,
                    "north" => DoorwaySide::North,
                    other => {
                        return Err(Error::new(format!(
                            "unknown [[doorway]].side '{other}' (expected 'west' or 'north')"
                        )));
                    }
                };
                doorways.push(StarterDoorwaySpec { x, y, side });
            }
        }

        let mut placements = Vec::new();
        if let Some(TomlValue::Array(arr)) = root.get("placement") {
            for item in arr {
                let TomlValue::Table(t) = item else {
                    return Err(Error::new("[[placement]] entries must be tables"));
                };
                let object_slug = req_str(t, "object")?;
                let x = opt_i32(t, "x")?.ok_or_else(|| Error::new("[[placement]] requires 'x'"))?;
                let y = opt_i32(t, "y")?.ok_or_else(|| Error::new("[[placement]] requires 'y'"))?;
                if !(0..64).contains(&x) || !(0..64).contains(&y) {
                    return Err(Error::new(format!(
                        "[[placement]] for '{object_slug}' ({x}, {y}) outside 0..64"
                    )));
                }
                let level = opt_i32(t, "level")?.unwrap_or(1);
                if !(1..=2).contains(&level) {
                    return Err(Error::new(format!(
                        "[[placement]] for '{object_slug}' level {level} must be 1 or 2"
                    )));
                }
                let direction = opt_i16(t, "direction")?.unwrap_or(0);
                if !matches!(direction, 0 | 2 | 4 | 6) {
                    return Err(Error::new(format!(
                        "[[placement]] for '{object_slug}' direction {direction} must be an even cardinal facing (0, 2, 4, or 6)"
                    )));
                }
                placements.push(StarterPlacementSpec {
                    object_slug,
                    x,
                    y,
                    level,
                    direction,
                });
            }
        }

        let mut sims = Vec::new();
        let mut seen_sim_guids = std::collections::BTreeSet::new();
        let mut seen_neighbor_ids = std::collections::BTreeSet::new();
        if let Some(TomlValue::Array(arr)) = root.get("sim") {
            for (idx, item) in arr.iter().enumerate() {
                let TomlValue::Table(t) = item else {
                    return Err(Error::new("[[sim]] entries must be tables"));
                };
                let name = req_str(t, "name")?;
                let guid = req_u32(t, "guid")?;
                if !seen_sim_guids.insert(guid) {
                    return Err(Error::new(format!("duplicate [[sim]] guid 0x{guid:08X}")));
                }
                let neighbor_id = opt_i16(t, "neighbor_id")?.unwrap_or((idx + 1) as i16);
                if neighbor_id <= 0 {
                    return Err(Error::new(format!(
                        "[[sim]] '{name}' neighbor_id {neighbor_id} must be positive"
                    )));
                }
                if !seen_neighbor_ids.insert(neighbor_id) {
                    return Err(Error::new(format!(
                        "duplicate [[sim]] neighbor_id {neighbor_id}"
                    )));
                }
                let x = opt_i32(t, "x")?.unwrap_or(28);
                let y = opt_i32(t, "y")?.unwrap_or(28);
                if !(0..64).contains(&x) || !(0..64).contains(&y) {
                    return Err(Error::new(format!(
                        "[[sim]] '{name}' coordinates ({x}, {y}) outside 0..64"
                    )));
                }
                sims.push(StarterSimSpec {
                    name,
                    guid,
                    neighbor_id,
                    x,
                    y,
                    cooking: opt_i16(t, "cooking")?.unwrap_or(200),
                    mechanical: opt_i16(t, "mechanical")?.unwrap_or(200),
                    charisma: opt_i16(t, "charisma")?.unwrap_or(200),
                    neat: opt_i16(t, "neat")?.unwrap_or(500),
                    outgoing: opt_i16(t, "outgoing")?.unwrap_or(500),
                    active: opt_i16(t, "active")?.unwrap_or(500),
                    playful: opt_i16(t, "playful")?.unwrap_or(500),
                    nice: opt_i16(t, "nice")?.unwrap_or(600),
                });
            }
        }
        sims.sort_by_key(|s| s.neighbor_id);
        for (idx, sim) in sims.iter().enumerate() {
            let expected = (idx + 1) as i16;
            if sim.neighbor_id != expected {
                return Err(Error::new(format!(
                    "[[sim]] neighbor_ids must be contiguous 1..={} (expected {expected}, got {})",
                    sims.len(),
                    sim.neighbor_id
                )));
            }
        }

        Ok(Self {
            house_name,
            family_name,
            family_bio,
            lot_number,
            funds,
            camera_direction,
            roof,
            rooms,
            doorways,
            placements,
            sims,
        })
    }
}

/// Metadata for a single tile (or the sole tile of a single-tile object) inside [`PlacedObjectInfo`].
#[derive(Debug, Clone)]
pub struct PlacedTileInfo {
    pub name: String,
    pub guid: u32,
    pub attributes: usize,
    pub group: [i32; 3],
    pub is_lead: bool,
}

/// Metadata for a compiled object passed into [`build_starter_house_iff`].
#[derive(Debug, Clone)]
pub struct PlacedObjectInfo {
    pub slug: String,
    pub name: String,
    pub price: u16,
    pub is_multipart: bool,
    pub tiles: Vec<PlacedTileInfo>,
}

/// Builds a complete clean-room `House01.iff` containing `HOUS` (#0, #1), `SIMI`,
/// all 14 `ARRY` chunks (64×64 wide surfaces), `FLRm`, `WALm`, `OBJT`, and `OBJM`
/// (with all placed furniture objects AND both starter Sim instances).
pub fn build_starter_house_iff(
    manifest: &StarterHouseManifest,
    surfaces: &CompiledSurfaces,
    catalog: &BTreeMap<String, PlacedObjectInfo>,
) -> Result<Vec<u8>> {
    const SIZE: usize = 64;
    const HOUSE_VERSION: u32 = 0x49;

    // 1. Build 64x64 ground floor & wall arrays (column-major: (x * 64 + y) * stride).
    let mut ground_floors = vec![0u8; SIZE * SIZE * 2];
    let mut ground_walls = Vec::with_capacity(SIZE * SIZE * 14);
    for _ in 0..SIZE * SIZE {
        ground_walls.extend_from_slice(&BARE_GROUND_WALL);
    }

    let set_floor = |grid: &mut [u8], x: usize, y: usize, floor_id: u16| {
        let at = (x * SIZE + y) * 2;
        grid[at..at + 2].copy_from_slice(&floor_id.to_le_bytes());
    };
    let set_wall_west = |grid: &mut [u8], x: usize, y: usize, style: u16, pat: u16| {
        let c = (x * SIZE + y) * 14;
        grid[c] |= 1;
        grid[c + 2..c + 4].copy_from_slice(&style.to_le_bytes());
        grid[c + 6..c + 8].copy_from_slice(&pat.to_le_bytes());
        if x > 0 {
            let w = ((x - 1) * SIZE + y) * 14;
            grid[w] |= 4;
            grid[w + 12..w + 14].copy_from_slice(&pat.to_le_bytes());
        }
    };
    let set_wall_north = |grid: &mut [u8], x: usize, y: usize, style: u16, pat: u16| {
        let c = (x * SIZE + y) * 14;
        grid[c] |= 2;
        grid[c + 4..c + 6].copy_from_slice(&style.to_le_bytes());
        grid[c + 8..c + 10].copy_from_slice(&pat.to_le_bytes());
        if y > 0 {
            let n = (x * SIZE + (y - 1)) * 14;
            grid[n] |= 8;
            grid[n + 10..n + 12].copy_from_slice(&pat.to_le_bytes());
        }
    };

    let mut arch_value: i32 = 0;
    for room in &manifest.rooms {
        for x in room.x0..room.x1 {
            for y in room.y0..room.y1 {
                set_floor(&mut ground_floors, x, y, room.floor_id);
                arch_value += 10;
            }
        }
        if room.enclose_walls {
            for y in room.y0..room.y1 {
                set_wall_west(
                    &mut ground_walls,
                    room.x0,
                    y,
                    room.wall_style,
                    room.wall_pattern,
                );
                set_wall_west(
                    &mut ground_walls,
                    room.x1,
                    y,
                    room.wall_style,
                    room.wall_pattern,
                );
                arch_value += 70;
            }
            for x in room.x0..room.x1 {
                set_wall_north(
                    &mut ground_walls,
                    x,
                    room.y0,
                    room.wall_style,
                    room.wall_pattern,
                );
                set_wall_north(
                    &mut ground_walls,
                    x,
                    room.y1,
                    room.wall_style,
                    room.wall_pattern,
                );
                arch_value += 70;
            }
        }
    }

    let clear_wall_west = |grid: &mut [u8], x: usize, y: usize| {
        let c = (x * SIZE + y) * 14;
        grid[c] &= !1;
        if x > 0 {
            let w = ((x - 1) * SIZE + y) * 14;
            grid[w] &= !4;
        }
    };
    let clear_wall_north = |grid: &mut [u8], x: usize, y: usize| {
        let c = (x * SIZE + y) * 14;
        grid[c] &= !2;
        if y > 0 {
            let n = (x * SIZE + (y - 1)) * 14;
            grid[n] &= !8;
        }
    };

    for dw in &manifest.doorways {
        match dw.side {
            DoorwaySide::West => clear_wall_west(&mut ground_walls, dw.x, dw.y),
            DoorwaySide::North => clear_wall_north(&mut ground_walls, dw.x, dw.y),
        }
    }

    // 2. Build objt + ObjM + ground/upper object grids (Arry #3, #103).
    let mut ground_grid = vec![0u8; SIZE * SIZE * 2];
    let mut upper_grid = vec![0u8; SIZE * SIZE * 2];
    let mut type_entries: Vec<(u32, u16, u16, String)> = Vec::new(); // (guid, type_id, kind, name)
    let mut guid_to_type_id: BTreeMap<u32, u16> = BTreeMap::new();
    let mut next_type_id = 1u16;

    struct SavedInstancePlan {
        id: u16,
        type_id: u16,
        x: i32,
        y: i32,
        level: i32,
        direction: i16,
        attributes: usize,
        multipart: Option<([i32; 3], [i32; 3], i16)>, // (offset, group, parent)
        sim: Option<StarterSimSpec>,
    }

    let mut plans: Vec<SavedInstancePlan> = Vec::new();
    let mut next_obj_id = 1u16;
    let mut objects_value: i32 = 0;

    for pl in &manifest.placements {
        let info = catalog.get(&pl.object_slug).ok_or_else(|| {
            Error::new(format!(
                "starter-house.toml references unknown object '{}'",
                pl.object_slug
            ))
        })?;
        if info.tiles.is_empty() {
            return Err(Error::new(format!(
                "object '{}' has no placeable tile definitions",
                pl.object_slug
            )));
        }
        objects_value += i32::from(info.price);
        let lead_group = info
            .tiles
            .iter()
            .find(|t| t.is_lead)
            .unwrap_or(&info.tiles[0])
            .group;
        let lead_id = next_obj_id;
        for tile in &info.tiles {
            let dx = tile.group[0].wrapping_sub(lead_group[0]);
            let dy = tile.group[1].wrapping_sub(lead_group[1]);
            let dz = tile.group[2].wrapping_sub(lead_group[2]);
            let (rx, ry) = match pl.direction & 7 {
                0 => (dx, dy),
                2 => (dy.wrapping_neg(), dx),
                4 => (dx.wrapping_neg(), dy.wrapping_neg()),
                6 => (dy, dx.wrapping_neg()),
                _ => (0, 0),
            };
            let tile_x = pl.x + rx;
            let tile_y = pl.y + ry;
            let tile_level = pl.level + dz;
            if !(0..64).contains(&tile_x)
                || !(0..64).contains(&tile_y)
                || !(1..=2).contains(&tile_level)
            {
                return Err(Error::new(format!(
                    "multi-tile placement '{}' tile '{}' at ({tile_x}, {tile_y}, L{tile_level}) outside lot bounds",
                    pl.object_slug, tile.name
                )));
            }
            let type_id = *guid_to_type_id.entry(tile.guid).or_insert_with(|| {
                let tid = next_type_id;
                next_type_id += 1;
                type_entries.push((tile.guid, tid, 4, tile.name.clone()));
                tid
            });
            let id = next_obj_id;
            next_obj_id += 1;
            let multipart = if info.is_multipart {
                let parent = if tile.is_lead { 0 } else { lead_id as i16 };
                Some(([rx, ry, dz], tile.group, parent))
            } else {
                None
            };
            plans.push(SavedInstancePlan {
                id,
                type_id,
                x: tile_x,
                y: tile_y,
                level: tile_level,
                direction: pl.direction,
                attributes: tile.attributes,
                multipart,
                sim: None,
            });
        }
    }

    for sim in &manifest.sims {
        let type_id = *guid_to_type_id.entry(sim.guid).or_insert_with(|| {
            let tid = next_type_id;
            next_type_id += 1;
            type_entries.push((sim.guid, tid, 2, sim.name.clone()));
            tid
        });
        let id = next_obj_id;
        next_obj_id += 1;
        plans.push(SavedInstancePlan {
            id,
            type_id,
            x: sim.x,
            y: sim.y,
            level: 1,
            direction: 0,
            attributes: 0,
            multipart: None,
            sim: Some(sim.clone()),
        });
    }

    // Link objects occupying the same (level, x, y) tile via slot[0] chaining and write
    // head IDs to Arry #3 (ground) and Arry #103 (upper).
    let mut next_on_tile: BTreeMap<u16, i16> = BTreeMap::new();
    let mut tile_head: BTreeMap<(i32, i32, i32), u16> = BTreeMap::new();
    for plan in plans.iter().rev() {
        let prev = tile_head
            .insert((plan.level, plan.x, plan.y), plan.id)
            .unwrap_or(0);
        next_on_tile.insert(plan.id, prev as i16);
    }
    for (&(level, x, y), &head_id) in &tile_head {
        let at = (x as usize * SIZE + y as usize) * 2;
        let grid = if level == 2 {
            &mut upper_grid
        } else {
            &mut ground_grid
        };
        grid[at..at + 2].copy_from_slice(&(head_id as i16).to_le_bytes());
    }

    // Encode objt #1.
    let mut objt = Vec::new();
    objt.extend_from_slice(&0u32.to_le_bytes());
    objt.extend_from_slice(&3u32.to_le_bytes());
    objt.extend_from_slice(b"tjbo");
    for (guid, type_id, kind, name) in &type_entries {
        objt.extend_from_slice(&guid.to_le_bytes());
        // Sentinel non-zero signatures (clean-room BHAVs default to revision 0 -> signature 0)
        // so initial lot load triggers `visit_objects` reset/Init and boots saved Sims onto Main.
        objt.extend_from_slice(&1u32.to_le_bytes()); // init_signature
        objt.extend_from_slice(&1u32.to_le_bytes()); // main_signature
        objt.extend_from_slice(&type_id.to_le_bytes());
        objt.extend_from_slice(&kind.to_le_bytes());
        write_padded_cstr(&mut objt, name)?;
        objt.extend_from_slice(&0u32.to_le_bytes()); // v3 extra
    }

    // Encode ObjM #1.
    let mut objm = Vec::new();
    objm.extend_from_slice(&0u32.to_le_bytes());
    objm.extend_from_slice(&HOUSE_VERSION.to_le_bytes());
    objm.extend_from_slice(b"MjbO");
    objm.push(1); // compressed field stream
    let mut reg = FieldWriter::new(objm.len());
    for plan in &plans {
        reg.u16(plan.id);
        reg.u16(plan.type_id);
    }
    reg.u16(0);
    objm.extend_from_slice(&reg.finish());

    for plan in &plans {
        let origin = objm.len() + 4;
        let mut fw = FieldWriter::new(origin);
        // footprint [min_y, min_x, max_y, max_x], position [x, y, level] in subtile units (16/tile).
        // Standing Sims have a 6-unit footprint (`sims-vm/src/placement.rs:786`); furniture uses 16.
        let px = plan.x * 16 + 8;
        let py = plan.y * 16 + 8;
        let half = if plan.sim.is_some() { 3 } else { 8 };
        fw.i32(py - half);
        fw.i32(px - half);
        fw.i32(py + half);
        fw.i32(px + half);
        fw.i32(px);
        fw.i32(py);
        fw.i32(plan.level);
        fw.i16(0); // unknown_short
        fw.i16(plan.attributes as i16);
        for _ in 0..plan.attributes {
            fw.i16(0);
        }
        for _ in 0..8 {
            fw.i16(0); // temps
        }
        for w_idx in 0..72usize {
            let val = match w_idx {
                1 => plan.direction,
                11 => plan.id as i16,
                14 => -1,
                29 => 1, // room 1
                67 => 5,
                _ => 0,
            };
            fw.i16(val);
        }
        fw.i16(0); // obsolete_word
        fw.i32(0); // stack frame count
        fw.i32(512); // stack_capacity_bytes
        fw.i32(-1); // relationship_flag
        fw.i32(0); // relationship count
        fw.i16(1); // slots count (slot 0 = tile chain)
        fw.i16(0);
        fw.i16(next_on_tile.get(&plan.id).copied().unwrap_or(0));
        fw.i16(0); // dynamic_sprites count
        if let Some((offset, group, parent)) = plan.multipart {
            for v in offset {
                fw.i32(v);
            }
            for v in group {
                fw.i32(v);
            }
            fw.i16(parent);
        }
        if let Some(sim) = &plan.sim {
            fw.i32(0); // animation_events
            fw.i32(0); // engaged
            for _ in 0..10 {
                fw.string("")?;
            }
            fw.i32(0); // accessories count
            for _ in 0..3 {
                fw.string("")?;
            }
            fw.i32(0); // routing_state
            for _ in 0..9 {
                fw.f32(0.0);
            }
            // current motives (16) and previous motives (16)
            let mut motives = [40.0f32; 16];
            motives[5] = 65.0; // energy
            motives[6] = 60.0; // comfort
            motives[7] = 55.0; // hunger
            motives[8] = 70.0; // hygiene
            motives[9] = 65.0; // bladder
            motives[13] = 50.0; // room
            motives[14] = 50.0; // social
            motives[15] = 50.0; // fun
            for m in motives {
                fw.f32(m);
            }
            for m in motives {
                fw.f32(m);
            }
            // 256 person_data words (version 0x49 >= 0x47)
            let pdata = build_person_data(sim, manifest.lot_number as i16);
            for w in pdata {
                fw.i16(w);
            }
            fw.i32(0); // routing_frames
            // active & last idle interactions
            write_idle_interaction(&mut fw);
            write_idle_interaction(&mut fw);
            fw.i32(0); // queue count
            fw.i32(0); // object_uses count
            fw.i32(0); // motive_deltas count
        }
        fw.i32(0); // registry_word
        let record = fw.finish();
        let end = (objm.len() + 4 + record.len() - 12) as u32;
        objm.extend_from_slice(&end.to_le_bytes());
        objm.extend_from_slice(&record);
    }

    // 3. Build HOUS #0, HOUS #1, SIMI #1, FLRm #1, WALm #1, and all 14 ARRY chunks.
    let mut chunks = Vec::new();

    // HOUS #0 (HouseView)
    let mut hous0 = Vec::new();
    hous0.extend_from_slice(&0u32.to_le_bytes());
    hous0.extend_from_slice(&HOUSE_VERSION.to_le_bytes());
    hous0.extend_from_slice(b"SUOH");
    for _ in 0..4 {
        hous0.extend_from_slice(&0i32.to_le_bytes());
    }
    hous0.extend_from_slice(&manifest.camera_direction.to_le_bytes());
    hous0.extend_from_slice(&0i16.to_le_bytes());
    hous0.extend_from_slice(&0i16.to_le_bytes());
    hous0.extend_from_slice(&0u32.to_le_bytes());
    write_win1252_cstr(&mut hous0, &manifest.roof)?;
    chunks.push(ChunkData {
        tag: *b"HOUS",
        id: 0,
        flags: IFF_CONVERTED_FLAG,
        label: "House View".to_owned(),
        data: hous0,
    });

    // HOUS #1 (House model)
    let mut hous1 = Vec::new();
    hous1.extend_from_slice(&0u32.to_le_bytes());
    hous1.extend_from_slice(&HOUSE_VERSION.to_le_bytes());
    hous1.extend_from_slice(b"SUOH");
    write_padded_cstr(&mut hous1, &manifest.house_name)?;
    for _ in 0..12 {
        hous1.extend_from_slice(&0i32.to_le_bytes());
    }
    chunks.push(ChunkData {
        tag: *b"HOUS",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: manifest.house_name.clone(),
        data: hous1,
    });

    // SIMI #1 (Simulator)
    let mut simi = Vec::new();
    simi.extend_from_slice(&0u32.to_le_bytes());
    simi.extend_from_slice(&HOUSE_VERSION.to_le_bytes());
    simi.extend_from_slice(b"IMIS");
    let mut globals = [0i16; 64];
    globals[9] = 1; // family ID 1
    globals[10] = manifest.lot_number as i16;
    globals[13] = (manifest.funds % 10_000) as i16;
    globals[14] = (manifest.funds / 10_000) as i16;
    globals[15] = 1;
    globals[29] = 30;
    globals[30] = 1; // free will on
    for g in globals {
        simi.extend_from_slice(&g.to_le_bytes());
    }
    simi.extend_from_slice(&0u32.to_le_bytes()); // ticks
    simi.extend_from_slice(&0i16.to_le_bytes()); // scheduler_cursor
    simi.extend_from_slice(&manifest.funds.to_le_bytes());
    simi.extend_from_slice(&0x1234_5678u32.to_le_bytes()); // lot_rng
    simi.extend_from_slice(&0x8765_4321u32.to_le_bytes()); // global_rng
    simi.extend_from_slice(&0i32.to_le_bytes()); // pending_funds
    simi.extend_from_slice(&(arch_value + objects_value).to_le_bytes()); // lot_value
    simi.extend_from_slice(&objects_value.to_le_bytes());
    simi.extend_from_slice(&arch_value.to_le_bytes());
    // Budget day 0 valid + 8 zero categories, days 1..6 empty.
    simi.extend_from_slice(&1i32.to_le_bytes());
    for _ in 0..8 {
        simi.extend_from_slice(&0i32.to_le_bytes());
    }
    for _ in 0..5 {
        simi.extend_from_slice(&0i32.to_le_bytes());
    }
    chunks.push(ChunkData {
        tag: *b"SIMI",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: "Simulator".to_owned(),
        data: simi,
    });

    // FLRm #1 & WALm #1
    let mut flrm = Vec::new();
    flrm.extend_from_slice(&0u32.to_le_bytes());
    flrm.extend_from_slice(&0u32.to_le_bytes());
    flrm.extend_from_slice(b"mRLF");
    flrm.extend_from_slice(&(surfaces.floor_specs.len() as u32).to_le_bytes());
    for f in &surfaces.floor_specs {
        write_padded_cstr(&mut flrm, &f.file_name.to_ascii_lowercase())?;
        flrm.extend_from_slice(&1u32.to_le_bytes());
        flrm.extend_from_slice(&f.saved_id.to_le_bytes());
        flrm.extend_from_slice(&1u16.to_le_bytes()); // local pattern 1
        let fallback = u32::from(f.primary_rgb[0])
            | (u32::from(f.primary_rgb[1]) << 8)
            | (u32::from(f.primary_rgb[2]) << 16);
        flrm.extend_from_slice(&fallback.to_le_bytes());
    }
    chunks.push(ChunkData {
        tag: *b"FLRm",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: "Floor Map".to_owned(),
        data: flrm,
    });

    let mut walm = Vec::new();
    walm.extend_from_slice(&0u32.to_le_bytes());
    walm.extend_from_slice(&0u32.to_le_bytes());
    walm.extend_from_slice(b"mLAW");
    walm.extend_from_slice(&(surfaces.wall_specs.len() as u32).to_le_bytes());
    for w in &surfaces.wall_specs {
        write_padded_cstr(&mut walm, &w.file_name.to_ascii_lowercase())?;
        walm.extend_from_slice(&1u32.to_le_bytes());
        walm.extend_from_slice(&w.saved_id.to_le_bytes());
        walm.extend_from_slice(&1u16.to_le_bytes()); // local pattern 1
        let fallback = u32::from(w.primary_rgb[0])
            | (u32::from(w.primary_rgb[1]) << 8)
            | (u32::from(w.primary_rgb[2]) << 16);
        walm.extend_from_slice(&fallback.to_le_bytes());
    }
    chunks.push(ChunkData {
        tag: *b"WALm",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: "Wall Map".to_owned(),
        data: walm,
    });

    // Arry chunks (all 14 arrays read by `sims_sim::load`, with native case-sensitive tags,
    // 4-byte height array #0, and dry 0xff pool/water arrays #9/#10).
    let dry_surface = vec![0xffu8; SIZE * SIZE];
    let arry_specs: &[(u16, usize, Option<&[u8]>)] = &[
        (0, 4, None),
        (4, 1, None),
        (11, 2, Some(&ground_floors)),
        (12, 14, Some(&ground_walls)),
        (3, 2, Some(&ground_grid)),
        (8, 1, None),
        (111, 2, None),
        (112, 14, None),
        (103, 2, Some(&upper_grid)),
        (108, 1, None),
        (6, 1, None),
        (7, 1, None),
        (9, 1, Some(&dry_surface)),
        (10, 1, Some(&dry_surface)),
    ];
    for &(id, stride, maybe_data) in arry_specs {
        let mut buf = Vec::with_capacity(20 + SIZE * SIZE * stride);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&(SIZE as u32).to_le_bytes());
        buf.extend_from_slice(&(SIZE as u32).to_le_bytes());
        buf.extend_from_slice(&(stride as u32).to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // encoding 0 (raw)
        match maybe_data {
            Some(bytes) => buf.extend_from_slice(bytes),
            None => buf.resize(20 + SIZE * SIZE * stride, 0),
        }
        chunks.push(ChunkData {
            tag: *b"Arry",
            id,
            flags: IFF_CONVERTED_FLAG,
            label: format!("Array {id}"),
            data: buf,
        });
    }

    chunks.push(ChunkData {
        tag: *b"objt",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: "Object Types".to_owned(),
        data: objt,
    });
    chunks.push(ChunkData {
        tag: *b"ObjM",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: "Saved Objects".to_owned(),
        data: objm,
    });

    encode_iff_with_rsmp(&chunks)
}

/// Builds a self-contained clean-room character `.iff` (`OBJD` kind=2, `OBJf`,
/// `BHAV` Init + `IdleForInput` Main, `CTSS`, `PALT`/`SPR2`/`DGRP`) for a starter Sim
/// so `House01.iff` resolves every saved GUID from the emitted bundle.
pub fn build_starter_sim_iff(sim: &StarterSimSpec) -> Result<Vec<u8>> {
    let script = r#"
tree 4096 "Init" {
    return true;
}

tree 4097 "Main" (args: 1) {
    while 0 == 0 {
        param[0] := 1;
        idle_for_input(param[0], 1);
    }
}
"#;
    let compiled = compile_antics(script)?;
    let mut chunks = Vec::new();

    let mut objd_spec = ObjdSpec::new(128, &sim.name, sim.guid);
    objd_spec.object_type = 2; // Person
    objd_spec.price = 0;
    objd_spec.base_graphic_id = 100;
    objd_spec.num_graphics = 1;
    objd_spec.tree_table_id = 0;
    objd_spec.slots_id = 0;
    objd_spec.catalog_strings_id = 128;
    objd_spec.init_id = 4096;
    objd_spec.main_id = 4097;
    objd_spec.num_attributes = 0;
    chunks.push(ChunkData {
        tag: *b"OBJD",
        id: 128,
        flags: IFF_CONVERTED_FLAG,
        label: sim.name.clone(),
        data: encode_objd(&objd_spec.to_chunk())?,
    });

    let objf = ObjfChunk::new(
        128,
        format!("{} Functions", sim.name),
        vec![
            ObjectFunction {
                condition: 0,
                action: 4096,
            },
            ObjectFunction {
                condition: 0,
                action: 4097,
            },
        ],
    );
    chunks.push(ChunkData {
        tag: *b"OBJf",
        id: 128,
        flags: IFF_CONVERTED_FLAG,
        label: objf.label.clone(),
        data: encode_objf(&objf)?,
    });

    for (&id, bhav) in &compiled.behaviors {
        chunks.push(ChunkData {
            tag: *b"BHAV",
            id,
            flags: IFF_CONVERTED_FLAG,
            label: bhav.label.clone(),
            data: encode_bhav(bhav)?,
        });
    }

    let ctss = StringTableChunk::new(
        *b"CTSS",
        128,
        sim.name.clone(),
        StringFormat::Localized,
        vec![
            StringEntry::english(&sim.name),
            StringEntry::english("Clean-room starter household Sim."),
        ],
    );
    chunks.push(ChunkData {
        tag: *b"CTSS",
        id: 128,
        flags: IFF_CONVERTED_FLAG,
        label: ctss.label.clone(),
        data: encode_string_table(&ctss)?,
    });

    let spec = SpriteSpec {
        shape: FurnitureShape::FloorLamp,
        primary_rgb: [104, 148, 198],
        accent_rgb: [228, 192, 160],
        trim_rgb: [66, 78, 96],
    };
    let art = generate_furniture_sprites(&spec, 100, 100, 100, &sim.name);
    chunks.push(ChunkData {
        tag: *b"PALT",
        id: art.palt.id,
        flags: IFF_CONVERTED_FLAG,
        label: art.palt.label.clone(),
        data: encode_palt(&art.palt)?,
    });
    chunks.push(ChunkData {
        tag: *b"SPR2",
        id: art.spr2.id,
        flags: IFF_CONVERTED_FLAG,
        label: art.spr2.label.clone(),
        data: encode_spr2(&art.spr2)?,
    });
    chunks.push(ChunkData {
        tag: *b"DGRP",
        id: art.dgrp.id,
        flags: IFF_CONVERTED_FLAG,
        label: art.dgrp.label.clone(),
        data: encode_dgrp(&art.dgrp)?,
    });

    encode_iff_with_rsmp(&chunks)
}

/// Builds a clean-room `Neighborhood.iff` containing `NGBH`, `NBRS` (both starter
/// Sims), `FAMI` #1 (v9), and `FAMs` #1.
pub fn build_starter_neighborhood_iff(manifest: &StarterHouseManifest) -> Result<Vec<u8>> {
    let mut chunks = Vec::new();

    // NGBH #1 (version 73)
    let mut ngbh = Vec::new();
    ngbh.extend_from_slice(&0u32.to_le_bytes());
    ngbh.extend_from_slice(&73u32.to_le_bytes());
    ngbh.extend_from_slice(b"HBGN");
    for _ in 0..16 {
        ngbh.extend_from_slice(&0i16.to_le_bytes());
    }
    ngbh.extend_from_slice(&0u32.to_le_bytes()); // inventories count
    // 11 option bytes in file order: (0,1), (2,3), (10), (4,5,6,7), (8,9)
    ngbh.extend_from_slice(&[0, 1, 0, 1, 1, 0, 1, 0, 1, 0, 1]);
    chunks.push(ChunkData {
        tag: *b"NGBH",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: "Neighborhood".to_owned(),
        data: ngbh,
    });

    // NBRS #1 (version 10)
    let mut nbrs = Vec::new();
    nbrs.extend_from_slice(&0u32.to_le_bytes());
    nbrs.extend_from_slice(&10u32.to_le_bytes());
    nbrs.extend_from_slice(b"SRBN");
    nbrs.extend_from_slice(&(manifest.sims.len() as u32).to_le_bytes());
    for sim in &manifest.sims {
        nbrs.extend_from_slice(&1i32.to_le_bytes()); // present
        nbrs.extend_from_slice(&10u32.to_le_bytes()); // neighbor version 10
        nbrs.extend_from_slice(&1i32.to_le_bytes()); // version_extra
        write_padded_cstr(&mut nbrs, &sim.name)?;
        nbrs.extend_from_slice(&0i32.to_le_bytes()); // unknown_zero
        nbrs.extend_from_slice(&1i32.to_le_bytes()); // person_mode > 0
        for w in build_person_data(sim, manifest.lot_number as i16) {
            nbrs.extend_from_slice(&w.to_le_bytes());
        }
        nbrs.extend_from_slice(&sim.neighbor_id.to_le_bytes());
        nbrs.extend_from_slice(&sim.guid.to_le_bytes());
        nbrs.extend_from_slice(&(-1i32).to_le_bytes()); // unknown_negative
        nbrs.extend_from_slice(&0u32.to_le_bytes()); // rel_count
    }
    chunks.push(ChunkData {
        tag: *b"NBRS",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: "Neighbors".to_owned(),
        data: nbrs,
    });

    // FAMI #1 (version 9)
    let mut fami = Vec::new();
    fami.extend_from_slice(&0u32.to_le_bytes());
    fami.extend_from_slice(&9u32.to_le_bytes());
    fami.extend_from_slice(b"IMAF");
    fami.extend_from_slice(&manifest.lot_number.to_le_bytes());
    fami.extend_from_slice(&1i32.to_le_bytes()); // family number
    fami.extend_from_slice(&manifest.funds.to_le_bytes());
    fami.extend_from_slice(&0i32.to_le_bytes()); // architecture_value
    fami.extend_from_slice(&0i32.to_le_bytes()); // friends
    fami.extend_from_slice(&0u32.to_le_bytes()); // flags
    fami.extend_from_slice(&(manifest.sims.len() as u32).to_le_bytes());
    for sim in &manifest.sims {
        fami.extend_from_slice(&sim.guid.to_le_bytes());
    }
    fami.extend_from_slice(&[0u8; 16]); // v9 secondary house + 6 magic words
    chunks.push(ChunkData {
        tag: *b"FAMI",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: manifest.family_name.clone(),
        data: fami,
    });

    // FAMs #1 (localized family name & bio)
    let fams = StringTableChunk::new(
        *b"FAMs",
        1,
        manifest.family_name.clone(),
        StringFormat::Localized,
        vec![
            StringEntry::english(&manifest.family_name),
            StringEntry::english(&manifest.family_bio),
        ],
    );
    chunks.push(ChunkData {
        tag: *b"FAMs",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: fams.label.clone(),
        data: encode_string_table(&fams)?,
    });

    encode_iff_with_rsmp(&chunks)
}

fn build_person_data(sim: &StarterSimSpec, lot_number: i16) -> [i16; 256] {
    let mut data = [0i16; 256];
    data[0] = sim.nice;
    data[1] = sim.active;
    data[3] = sim.playful;
    data[4] = sim.outgoing;
    data[5] = sim.neat;
    data[10] = sim.cooking;
    data[11] = sim.charisma;
    data[12] = sim.mechanical;
    data[32] = 0; // resident (not visitor)
    data[58] = 25; // adult age
    data[61] = 1; // family ID 1
    data[67] = lot_number; // home lot
    data
}

fn write_idle_interaction(fw: &mut FieldWriter) {
    fw.i32(0); // join
    fw.i32(0); // uid
    fw.i16(0); // caller
    fw.i16(0); // target
    fw.i16(0); // icon
    fw.i32(-1); // table_index
    for _ in 0..4 {
        fw.i16(0); // args
    }
    fw.i32(0); // priority
    fw.u16(0); // action
    fw.f32(0.0); // attenuation
    fw.u32(0); // flags
}

/// MSB-first signed field-stream writer, exact inverse of `sims_formats::field::Fields`.
struct FieldWriter {
    origin: usize,
    bytes: Vec<u8>,
    current: u8,
    bits_used: u8,
}

impl FieldWriter {
    fn new(origin: usize) -> Self {
        Self {
            origin,
            bytes: Vec::new(),
            current: 0,
            bits_used: 0,
        }
    }

    fn push_bits(&mut self, value: u32, count: usize) {
        for shift in (0..count).rev() {
            let bit = ((value >> shift) & 1) as u8;
            self.current = (self.current << 1) | bit;
            self.bits_used += 1;
            if self.bits_used == 8 {
                self.bytes.push(self.current);
                self.current = 0;
                self.bits_used = 0;
            }
        }
    }

    fn flush_partial_byte(&mut self) {
        if self.bits_used > 0 {
            let rem = 8 - self.bits_used;
            self.bytes.push(self.current << rem);
            self.current = 0;
            self.bits_used = 0;
        }
    }

    fn signed(&mut self, value: i32, widths: [usize; 4]) {
        if value == 0 {
            self.push_bits(0, 1);
            return;
        }
        let selector = widths
            .iter()
            .position(|&w| w == 32 || (-(1 << (w - 1))..1 << (w - 1)).contains(&value))
            .unwrap_or(3);
        let width = widths[selector];
        let mask = if width == 32 {
            u32::MAX
        } else {
            (1u32 << width) - 1
        };
        self.push_bits(1, 1);
        self.push_bits(selector as u32, 2);
        self.push_bits((value as u32) & mask, width);
    }

    fn i16(&mut self, v: i16) {
        self.signed(i32::from(v), [5, 8, 13, 16]);
    }

    fn u16(&mut self, v: u16) {
        self.i16(v as i16);
    }

    fn i32(&mut self, v: i32) {
        self.signed(v, [6, 11, 21, 32]);
    }

    fn u32(&mut self, v: u32) {
        self.i32(v as i32);
    }

    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }

    fn string(&mut self, s: &str) -> Result<()> {
        if s.as_bytes().contains(&0) {
            return Err(Error::new("field string contains embedded NUL"));
        }
        self.flush_partial_byte();
        self.bytes.extend_from_slice(s.as_bytes());
        self.bytes.push(0);
        if (self.origin + self.bytes.len()) & 1 != 0 {
            self.bytes.push(0);
        }
        Ok(())
    }

    fn finish(mut self) -> Vec<u8> {
        self.flush_partial_byte();
        self.bytes
    }
}

fn encode_win1252_checked(s: &str) -> Result<Vec<u8>> {
    if s.as_bytes().contains(&0) {
        return Err(Error::new(format!("C-string '{s}' contains embedded NUL")));
    }
    let (bytes, _, had_errors) = encoding_rs::WINDOWS_1252.encode(s);
    if had_errors {
        return Err(Error::new(format!(
            "string '{s}' contains characters not representable in Windows-1252"
        )));
    }
    Ok(bytes.into_owned())
}

fn write_win1252_cstr(out: &mut Vec<u8>, s: &str) -> Result<()> {
    let encoded = encode_win1252_checked(s)?;
    out.extend_from_slice(&encoded);
    out.push(0);
    Ok(())
}

fn write_padded_cstr(out: &mut Vec<u8>, s: &str) -> Result<()> {
    let encoded = encode_win1252_checked(s)?;
    out.extend_from_slice(&encoded);
    out.push(0);
    if out.len() & 1 != 0 {
        out.push(0);
    }
    Ok(())
}

fn validate_surface_filename(name: &str, ext: &str) -> Result<()> {
    let valid = !name.is_empty()
        && !name.starts_with('.')
        && !name.contains("..")
        && name.ends_with(ext)
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    if !valid {
        return Err(Error::new(format!(
            "invalid surface filename '{name}' (must be plain ASCII ending in '{ext}')"
        )));
    }
    Ok(())
}

fn req_str(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<String> {
    opt_str(t, key).ok_or_else(|| Error::new(format!("missing required string key '{key}'")))
}

fn opt_str(t: &BTreeMap<String, TomlValue>, key: &str) -> Option<String> {
    match t.get(key) {
        Some(TomlValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn req_u16(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<u16> {
    opt_u16(t, key)?.ok_or_else(|| Error::new(format!("missing required u16 key '{key}'")))
}

fn opt_u16(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<u16>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Int(v)) => u16::try_from(*v)
            .map(Some)
            .map_err(|_| Error::new(format!("key '{key}' value {v} out of u16 range"))),
        Some(_) => Err(Error::new(format!("key '{key}' must be an integer"))),
    }
}

fn opt_i16(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<i16>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Int(v)) => i16::try_from(*v)
            .map(Some)
            .map_err(|_| Error::new(format!("key '{key}' value {v} out of i16 range"))),
        Some(_) => Err(Error::new(format!("key '{key}' must be an integer"))),
    }
}

fn req_u32(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<u32> {
    match t.get(key) {
        Some(TomlValue::Int(v)) => u32::try_from(*v)
            .map_err(|_| Error::new(format!("key '{key}' value {v} out of u32 range"))),
        Some(_) => Err(Error::new(format!("key '{key}' must be an integer"))),
        None => Err(Error::new(format!("missing required u32 key '{key}'"))),
    }
}

fn opt_i32(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<i32>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Int(v)) => i32::try_from(*v)
            .map(Some)
            .map_err(|_| Error::new(format!("key '{key}' value {v} out of i32 range"))),
        Some(_) => Err(Error::new(format!("key '{key}' must be an integer"))),
    }
}

fn opt_bool(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<Option<bool>> {
    match t.get(key) {
        None => Ok(None),
        Some(TomlValue::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(Error::new(format!("key '{key}' must be a boolean"))),
    }
}

fn req_rgb(t: &BTreeMap<String, TomlValue>, key: &str) -> Result<[u8; 3]> {
    match t.get(key) {
        Some(TomlValue::Array(items)) if items.len() == 3 => {
            let mut rgb = [0u8; 3];
            for (i, item) in items.iter().enumerate() {
                let TomlValue::Int(v) = item else {
                    return Err(Error::new(format!("'{key}' elements must be integers")));
                };
                rgb[i] = u8::try_from(*v)
                    .map_err(|_| Error::new(format!("'{key}' channel {v} out of 0..=255")))?;
            }
            Ok(rgb)
        }
        _ => Err(Error::new(format!("missing or invalid [r, g, b] '{key}'"))),
    }
}
