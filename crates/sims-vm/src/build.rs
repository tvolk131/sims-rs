//! Build Mode simulation operations: `"Wall Cursor"` (`0x00000439`,
//! `0x48b68b` / `0x48a66a` / `0x48a6e5`), wall drag placement, Shift room-box,
//! and Ctrl demolition (`cNewWallTool` / `WallUndoable` `0x48af6f`..`0x496931`),
//! floor tile placement, drag rectangle, Shift room-fill, split-diagonal halves,
//! and Ctrl removal (`cNewFloorTool` / `FloorUndoable` `0x4881e6`..`0x4899d5`),
//! wallpaper face application, drag, Shift room-fill, and Ctrl stripping
//! (`cNewWallpaperTool` / `WallpaperUndoable` `0x48a08d`..`0x4939c0`),
//! terrain elevation, leveling, and grass restoration (`cNewTerrainTool`,
//! `GameData/Build.iff STR# 128`), pond water painting/removal and swimming
//! pool digging/demolition (`Arry 9` / `Arry 10`, `STR# 133`), roof pattern and
//! pitch switching (`HOUS 0` / `GameData/Roofs/*.bmp`), Architecture budget
//! accounting (`0x46d91c` category 7), and Build Mode Undo/Redo (`0x4932fa`,
//! `0x493338`).

use crate::boot::{HistoryKind, Lot, OrderError};
use crate::buy::{BuyActionKind, BuyPurchase, available_funds};
use crate::fault::{Fault, Trap};
use crate::interpreter::{Interpreter, Stream};
use crate::placement::{Place, on_lot};
use crate::primitives::budget::spend;
use sims_formats::lot::{Array, HouseView};
use sims_sim::World;
use sims_sim::load::{ExpandedWall, FLOORS, WALLS};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// The `"Wall Cursor"` object GUID looked up by name (`0x43ab8f`) in
/// `cNewWallTool::Activate` (`0x48b68b`) and `cNewWallpaperTool::Activate`
/// (`0x48a66a`).
pub const WALL_CURSOR_GUID: u32 = 0x0000_0439;

/// Budget category 7: Architecture (`0x4929fb` → `0x46d91c`, `sim_storage +0x1a0`).
pub const ARCHITECTURE_BUDGET: u8 = 7;

/// Floor ID indicating a tile split by a diagonal wall (`0x10` or `0x20`).
pub const SPLIT_FLOOR_ID: u16 = 255;

/// Side of the lot's full 64×64 arrays.
const FULL_SIZE: usize = 64;
const WALL_CELL: usize = 14;
const HEIGHTS_ARRAY: u16 = 0;
/// Lot grass array (`Arry 6`, signed `i8` per tile, `sims-sim/src/grass.rs`).
pub const GRASS_ARRAY: u16 = 6;
/// Lot target grass array (`Arry 7`, signed `i8` per tile, `sims-sim/src/grass.rs`).
pub const TARGET_GRASS_ARRAY: u16 = 7;
const FLAGS_ARRAY: [u16; 2] = [8, 108];
const FLAG_BUILDABLE: u8 = 1;
/// Tile flag `0x20` (`Arry 8`): locked terrain/river/road border that Build
/// Mode tools must not alter (`0x42737e`, `House08`/`House10` river tiles).
pub const FLAG_LOCKED: u8 = 0x20;
const POOL_ARRAY: u16 = 9;
const WATER_ARRAY: u16 = 10;
/// Sentinel array ID (`"rf"`) persisted inside [`World::arrays`] to carry
/// edited roof pitch (`f32` LE) and roof pattern stem across `World::clone`
/// and `Lot::save()`/`Lot::restore()` without altering load hashes when unread.
pub const ROOF_META_ARRAY: u16 = 0x7266;

/// Maximum orthogonal height difference between adjacent terrain vertices
/// enforced by Terrain and Pond editing tools (matching `sims-render/src/grass.rs`
/// `[-5, 5]` sun-shading slope table at `0x4c47f7`).
pub const MAX_TERRAIN_SLOPE: u8 = 5;
/// Bare dirt grass value written to `Arry 6` when terrain elevation changes
/// or grass is removed with Ctrl (`< SPARSE = -73` in `sims-render/src/grass.rs`).
pub const TERRAIN_DIRT_GRASS: i8 = -100;
/// Lush restored grass value written to `Arry 6` by the Grass tool
/// (`> DENSE = 85` in `sims-render/src/grass.rs`).
pub const GRASS_RESTORED_VALUE: i8 = 100;

/// Default Complete Collection 1.91 Terrain/Grass/Pool/Water tool prices
/// (`GameData/Build.iff STR# 128` and `STR# 133`).
pub const DEFAULT_TERRAIN_PRICE: i32 = 1;
pub const DEFAULT_GRASS_PRICE: i32 = 1;
pub const DEFAULT_POOL_PRICE: i32 = 75;
pub const DEFAULT_WATER_PRICE: i32 = 15;
/// Default base-game roof texture stem (`GameData/Roofs/r1_.bmp`).
pub const DEFAULT_ROOF_PATTERN: &str = "r1_";

/// Wall segment bitmasks (`ExpandedWall::segments`).
pub const WALL_WEST: u8 = 1;
pub const WALL_NORTH: u8 = 2;
pub const WALL_EAST: u8 = 4;
pub const WALL_SOUTH: u8 = 8;
pub const WALL_DESCENDING: u8 = 0x10;
pub const WALL_ASCENDING: u8 = 0x20;

/// Built-in wall and fence styles from `GameData/Build.iff STR# 129`
/// (`0x4cd464`..`0x4cd608`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WallStyle {
    /// Standard full wall (`style = 1`, bare pattern `0`, `$70`).
    Standard,
    /// White Picket Fence (`style = 2`, pattern `0xf8`, `$10`).
    WhitePicket,
    /// Privacy Fence (`style = 12`, pattern `0xf9`, `$35`).
    PrivacyFence,
    /// Wrought Iron Balustrade (`style = 13`, pattern `0xfa`, `$45`).
    WroughtIron,
    /// Monticello Balustrade (`style = 14`, pattern `0xfb`, `$45`).
    Monticello,
}

impl WallStyle {
    /// All five built-in wall & fence styles in native `cWinArch` sort order
    /// (`0x4cd464`..`0x4cd608`, `0x501c80`).
    pub const ALL: [Self; 5] = [
        Self::Standard,
        Self::WhitePicket,
        Self::PrivacyFence,
        Self::WroughtIron,
        Self::Monticello,
    ];

    /// Native wall style ID written into `ExpandedWall::styles`.
    pub const fn style_id(self) -> u16 {
        match self {
            Self::Standard => 1,
            Self::WhitePicket => 2,
            Self::PrivacyFence => 12,
            Self::WroughtIron => 13,
            Self::Monticello => 14,
        }
    }

    /// Native default wall pattern ID applied to newly built faces
    /// (`0x494390`..`0x4943c5`, `0x4ccde0`..`0x4cce2f`).
    pub const fn default_pattern(self) -> u16 {
        match self {
            Self::Standard => 0,
            Self::WhitePicket => 0xf8,
            Self::PrivacyFence => 0xf9,
            Self::WroughtIron => 0xfa,
            Self::Monticello => 0xfb,
        }
    }

    /// Default price from `GameData/Build.iff STR# 129` when no price table is
    /// supplied.
    pub const fn default_price(self) -> i32 {
        match self {
            Self::Standard => 70,
            Self::WhitePicket => 10,
            Self::PrivacyFence => 35,
            Self::WroughtIron => 45,
            Self::Monticello => 45,
        }
    }

    /// Whether `style_id` is a fence/balustrade that refuses wallpaper
    /// (`0x4937da`..`0x4937ec`).
    pub const fn is_fence_style(style_id: u16) -> bool {
        matches!(style_id, 2 | 12 | 13 | 14)
    }

    /// Maps a native style ID back to a [`WallStyle`], if built-in.
    pub const fn from_style_id(style_id: u16) -> Option<Self> {
        match style_id {
            1 | 22 | 23 => Some(Self::Standard),
            2 => Some(Self::WhitePicket),
            12 => Some(Self::PrivacyFence),
            13 => Some(Self::WroughtIron),
            14 => Some(Self::Monticello),
            _ => None,
        }
    }
}

/// Architecture pricing lookup for walls, floors, and wallpapers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildPrices {
    pub styles: BTreeMap<u16, i32>,
    pub floors: BTreeMap<u16, i32>,
    pub walls: BTreeMap<u16, i32>,
}

impl BuildPrices {
    /// Price of building one segment of `style_id`.
    pub fn style_price(&self, style_id: u16) -> i32 {
        if let Some(&p) = self.styles.get(&style_id) {
            return p;
        }
        WallStyle::from_style_id(style_id)
            .map(WallStyle::default_price)
            .unwrap_or(70)
    }

    /// Price of laying one full tile of floor `floor_id`.
    pub fn floor_price(&self, floor_id: u16) -> i32 {
        if floor_id == 0 || floor_id == SPLIT_FLOOR_ID {
            return 0;
        }
        self.floors.get(&floor_id).copied().unwrap_or(0)
    }

    /// Price of applying one wall face of wallpaper `pattern_id`.
    pub fn wall_price(&self, pattern_id: u16) -> i32 {
        if pattern_id == 0 || (248..=255).contains(&pattern_id) {
            return 0;
        }
        self.walls.get(&pattern_id).copied().unwrap_or(0)
    }
}

/// Native 80% Build Mode refund (`ftol(0.8 * price + 0.5)` at `0x4938c9` and
/// `0x488e5d` / `0x667d70`).
pub fn build_refund(price: i32) -> i32 {
    if price <= 0 {
        return 0;
    }
    ((price as f32) * 0.8 + 0.5).floor() as i32
}

/// Active `"Wall Cursor"` (`0x00000439`) owned by the Build Mode Wall or
/// Wallpaper tool (`0x48b68b`, `0x48a66a`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallCursor {
    /// Object ID of the `"Wall Cursor"` (`0x00000439`).
    pub id: u16,
    /// `data[0]`: `0` = wall build, `1` = wall demolish, `2` = wallpaper,
    /// `3` = wall room-box (`0x48b8b3`, `0x48a8bf`).
    pub mode: i16,
    /// `data[1]`: cursor facing (`0..7`).
    pub facing: i16,
    /// Current fine coordinates `[x, y, level]` (`[-16, -16, 1]` when hidden).
    pub position: [i32; 3],
    /// Monotonic token stamped by `Lot` so retired handles cannot alias.
    pub(crate) token: u64,
}

/// Wall drag interaction mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallDragMode {
    /// Drag a single snapped 8-way wall line (`data[0] = 0`).
    Build,
    /// Ctrl-drag to demolish a snapped wall line (`data[0] = 1`).
    Demolish,
    /// Shift-drag to build a 4-walled rectangular room box (`data[0] = 3`).
    RoomBox,
    /// Shift+Ctrl-drag to demolish a 4-walled rectangular perimeter (`data[0] = 1`).
    RoomBoxDemolish,
}

impl WallDragMode {
    /// Whether this mode demolishes walls rather than building them.
    pub const fn is_demolish(self) -> bool {
        matches!(self, Self::Demolish | Self::RoomBoxDemolish)
    }

    /// Native `"Wall Cursor"` `data[0]` graphic mode (`0x48b8b3`).
    pub const fn cursor_mode(self) -> i16 {
        match self {
            Self::Build => 0,
            Self::Demolish | Self::RoomBoxDemolish => 1,
            Self::RoomBox => 3,
        }
    }
}

/// One canonical wall edge segment owned by `tile` (`WEST`, `NORTH`,
/// `DESCENDING`, or `ASCENDING`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WallSegment {
    pub tile: [u8; 2],
    pub bit: u8,
}

/// Floor placement/removal mode (`0x488c20`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorDragMode {
    /// Single tile or dragged rectangle placement (`mode = 0`).
    Lay,
    /// Ctrl click/drag rectangle removal (`mode = 1`).
    Remove,
    /// Shift click room-fill placement (`mode = 2`).
    RoomFillLay,
    /// Shift+Ctrl click room-fill removal (`mode = 3`).
    RoomFillRemove,
}

impl FloorDragMode {
    pub const fn is_remove(self) -> bool {
        matches!(self, Self::Remove | Self::RoomFillRemove)
    }

    pub const fn is_room_fill(self) -> bool {
        matches!(self, Self::RoomFillLay | Self::RoomFillRemove)
    }
}

/// Wallpaper application/stripping mode (`0x48a633`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallpaperDragMode {
    /// Single wall face or drag (`mode = 0`).
    Apply,
    /// Ctrl single wall face or drag strip (`mode = 1`).
    Strip,
    /// Shift room-fill wallpaper (`mode = 2`).
    RoomFillApply,
    /// Shift+Ctrl room-fill strip (`mode = 3`).
    RoomFillStrip,
}

impl WallpaperDragMode {
    pub const fn is_strip(self) -> bool {
        matches!(self, Self::Strip | Self::RoomFillStrip)
    }

    pub const fn is_room_fill(self) -> bool {
        matches!(self, Self::RoomFillApply | Self::RoomFillStrip)
    }
}

/// One wall face targeted by the Wallpaper tool (`0x4936d2`, `0x493837`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WallFace {
    pub tile: [u8; 2],
    /// Segment bit on `tile`: `WEST (1)`, `NORTH (2)`, `EAST (4)`, `SOUTH (8)`,
    /// `DESCENDING (0x10)`, or `ASCENDING (0x20)`.
    pub bit: u8,
    /// For diagonal walls: `0` = first side (`patterns[4]`/`[6]`),
    /// `1` = second side (`patterns[5]`/`[7]`). Always `0` for cardinal walls.
    pub side: u8,
}

/// Computed preview / planned mutation for a wall drag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallPlan {
    pub level: u8,
    pub wall_changes: Vec<(usize, [u8; WALL_CELL], [u8; WALL_CELL])>,
    pub floor_changes: Vec<(usize, u16, u16)>,
    pub net_cost: i32,
    pub valid: bool,
    pub affordable: bool,
    pub removed_objects: Vec<u16>,
}

/// Computed preview / planned mutation for a floor click/drag/room-fill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorPlan {
    pub level: u8,
    pub floor_changes: Vec<(usize, u16, u16)>,
    pub wall_changes: Vec<(usize, [u8; WALL_CELL], [u8; WALL_CELL])>,
    pub net_cost: i32,
    pub valid: bool,
    pub affordable: bool,
}

/// Computed preview / planned mutation for a wallpaper click/drag/room-fill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallpaperPlan {
    pub level: u8,
    pub wall_changes: Vec<(usize, [u8; WALL_CELL], [u8; WALL_CELL])>,
    pub net_cost: i32,
    pub valid: bool,
    pub affordable: bool,
}

/// Committed Build Mode mutation stored on the Undo/Redo stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildHistory {
    Wall {
        level: u8,
        wall_changes: Vec<(usize, [u8; WALL_CELL], [u8; WALL_CELL])>,
        floor_changes: Vec<(usize, u16, u16)>,
        net_cost: i32,
        removed_objects: Vec<BuyPurchase>,
    },
    Floor {
        level: u8,
        floor_changes: Vec<(usize, u16, u16)>,
        wall_changes: Vec<(usize, [u8; WALL_CELL], [u8; WALL_CELL])>,
        net_cost: i32,
    },
    Wallpaper {
        level: u8,
        wall_changes: Vec<(usize, [u8; WALL_CELL], [u8; WALL_CELL])>,
        net_cost: i32,
    },
    Terrain {
        height_changes: Vec<(usize, [u8; 4], [u8; 4])>,
        grass_changes: Vec<(usize, i8, i8)>,
        net_cost: i32,
    },
    Water {
        water_changes: Vec<(usize, u8, u8)>,
        height_changes: Vec<(usize, [u8; 4], [u8; 4])>,
        grass_changes: Vec<(usize, i8, i8)>,
        created_water_array: bool,
        net_cost: i32,
    },
    Pool {
        pool_changes: Vec<(usize, u8, u8)>,
        created_pool_array: bool,
        net_cost: i32,
    },
    Roof {
        before_raw: Option<Vec<u8>>,
        after_raw: Vec<u8>,
    },
}

impl BuildHistory {
    /// Net Architecture cost of the forward operation (positive = spent,
    /// negative = refunded).
    pub fn net_cost(&self) -> i32 {
        match self {
            Self::Wall { net_cost, .. }
            | Self::Floor { net_cost, .. }
            | Self::Wallpaper { net_cost, .. }
            | Self::Terrain { net_cost, .. }
            | Self::Water { net_cost, .. }
            | Self::Pool { net_cost, .. } => *net_cost,
            Self::Roof { .. } => 0,
        }
    }

    /// Detached BuyPurchase records held by a wall demolition for Undo.
    pub fn detached_objects(&self) -> &[BuyPurchase] {
        match self {
            Self::Wall {
                removed_objects, ..
            } => removed_objects,
            Self::Floor { .. }
            | Self::Wallpaper { .. }
            | Self::Terrain { .. }
            | Self::Water { .. }
            | Self::Pool { .. }
            | Self::Roof { .. } => &[],
        }
    }
}

/// Snaps fine lot coordinates `[x, y]` to the nearest tile grid corner
/// (`0x48b717`: add 16 if `(coord & 15) >= 8`, then `>> 4`).
pub fn snap_wall_corner(fine: [i32; 2], active: usize) -> [i32; 2] {
    let snap = |v: i32| -> i32 {
        let rounded = if v.rem_euclid(16) >= 8 { v + 16 } else { v };
        (rounded >> 4).clamp(1, (active as i32 - 1).max(1))
    };
    [snap(fine[0]), snap(fine[1])]
}

/// Native `"Wall Cursor"` fine position for the Wall Tool (`0x48b7cd`):
/// `(corner_x * 16 + 1, corner_y * 16 + 1, level)`.
pub fn wall_cursor_fine(corner: [i32; 2], level: u8) -> [i32; 3] {
    [corner[0] * 16 + 1, corner[1] * 16 + 1, i32::from(level)]
}

/// Native `"Wall Cursor"` fine position for the Wallpaper Tool (`0x48a80e`):
/// `(tile_x * 16 + 8, tile_y * 16 + 8, level)` (or `+15` for descending diagonal).
pub fn wallpaper_cursor_fine(face: WallFace, level: u8) -> [i32; 3] {
    let base_x = i32::from(face.tile[0]) * 16;
    let base_y = i32::from(face.tile[1]) * 16;
    if face.bit == WALL_DESCENDING {
        [base_x + 15, base_y + 15, i32::from(level)]
    } else {
        [base_x + 8, base_y + 8, i32::from(level)]
    }
}

/// Native `"Wall Cursor"` facing `data[1]` for a wallpaper face (`0x48a813`..
/// `0x48a8a7`): `WEST -> edi 3`, `NORTH -> edi 0`, `EAST -> edi 1`,
/// `SOUTH -> edi 2`, rotated by `(-view_rotation) & 3`.
pub fn wallpaper_cursor_facing(face: WallFace, view_rotation: u8) -> Option<i16> {
    let base = match face.bit {
        WALL_WEST => 3i32,
        WALL_NORTH => 0,
        WALL_EAST => 1,
        WALL_SOUTH => 2,
        _ => return None,
    };
    let rot = (4 - i32::from(view_rotation & 3)) & 3;
    Some((((rot + base) * 2 + 8) % 8) as i16)
}

/// Computes the snapped 8-way wall segments between two grid corners
/// (`0x49474a`..`0x494c5e`).
pub fn wall_line_segments(start: [i32; 2], end: [i32; 2], active: usize) -> Vec<WallSegment> {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    if dx == 0 && dy == 0 {
        return Vec::new();
    }
    let adx = dx.abs();
    let ady = dy.abs();
    let diag_asc = (dx - dy).abs() / 2;
    let diag_desc = (dx + dy).abs() / 2;

    let (sx, sy, steps) = if adx < ady && adx <= diag_asc && adx <= diag_desc {
        (0, if dy >= 0 { 1 } else { -1 }, ady)
    } else if ady < adx && ady <= diag_asc && ady <= diag_desc {
        (if dx >= 0 { 1 } else { -1 }, 0, adx)
    } else if diag_asc < adx && diag_asc < ady && diag_asc < diag_desc {
        let s = if dx + dy >= 0 { 1 } else { -1 };
        (s, s, (dx + dy).abs() / 2)
    } else {
        let s = if dx - dy >= 0 { 1 } else { -1 };
        (s, -s, (dx - dy).abs() / 2)
    };

    let mut out = Vec::with_capacity(steps.max(0) as usize);
    let mut cx = start[0];
    let mut cy = start[1];
    for _ in 0..steps {
        if let Some(seg) = step_to_segment(cx, cy, sx, sy, active) {
            out.push(seg);
        }
        cx += sx;
        cy += sy;
    }
    out
}

/// Computes the four-sided rectangular room-box segments between opposite
/// corners `start` and `end` (Shift-drag, `0x49642c`).
pub fn wall_room_box_segments(start: [i32; 2], end: [i32; 2], active: usize) -> Vec<WallSegment> {
    let min_x = start[0].min(end[0]);
    let max_x = start[0].max(end[0]);
    let min_y = start[1].min(end[1]);
    let max_y = start[1].max(end[1]);
    if min_x == max_x && min_y == max_y {
        return Vec::new();
    }
    if min_x == max_x || min_y == max_y {
        return wall_line_segments(start, end, active);
    }
    let mut segs = Vec::new();
    // North edge (y = min_y) and South edge (y = max_y)
    for x in min_x..max_x {
        if let Some(s) = step_to_segment(x, min_y, 1, 0, active) {
            segs.push(s);
        }
        if let Some(s) = step_to_segment(x, max_y, 1, 0, active) {
            segs.push(s);
        }
    }
    // West edge (x = min_x) and East edge (x = max_x)
    for y in min_y..max_y {
        if let Some(s) = step_to_segment(min_x, y, 0, 1, active) {
            segs.push(s);
        }
        if let Some(s) = step_to_segment(max_x, y, 0, 1, active) {
            segs.push(s);
        }
    }
    segs
}

fn step_to_segment(cx: i32, cy: i32, sx: i32, sy: i32, active: usize) -> Option<WallSegment> {
    let (tx, ty, bit) = match (sx, sy) {
        (0, 1) => (cx, cy, WALL_WEST),
        (0, -1) => (cx, cy - 1, WALL_WEST),
        (1, 0) => (cx, cy, WALL_NORTH),
        (-1, 0) => (cx - 1, cy, WALL_NORTH),
        (1, 1) => (cx, cy, WALL_ASCENDING),
        (-1, -1) => (cx - 1, cy - 1, WALL_ASCENDING),
        (1, -1) => (cx, cy - 1, WALL_DESCENDING),
        (-1, 1) => (cx - 1, cy, WALL_DESCENDING),
        _ => return None,
    };
    if tx < 1 || ty < 1 || tx + 1 >= active as i32 || ty + 1 >= active as i32 {
        return None;
    }
    Some(WallSegment {
        tile: [tx as u8, ty as u8],
        bit,
    })
}

fn cell_index(x: usize, y: usize) -> usize {
    x * FULL_SIZE + y
}

fn read_floor(floors: &[u8], x: usize, y: usize) -> u16 {
    let at = cell_index(x, y) * 2;
    u16::from_le_bytes([floors[at], floors[at + 1]])
}

fn write_floor(floors: &mut [u8], x: usize, y: usize, id: u16) {
    let at = cell_index(x, y) * 2;
    floors[at..at + 2].copy_from_slice(&id.to_le_bytes());
}

fn is_flat_tile(world: &World, x: usize, y: usize) -> bool {
    let Some(heights) = world.arrays.get(&HEIGHTS_ARRAY) else {
        return true;
    };
    if heights.stride != 4 {
        return true;
    }
    let at = cell_index(x, y) * 4;
    let Some(c) = heights.data.get(at..at + 4) else {
        return false;
    };
    c[0] == c[1] && c[1] == c[2] && c[2] == c[3]
}

fn is_pool_or_water_tile(world: &World, x: usize, y: usize) -> bool {
    let idx = cell_index(x, y);
    for array_id in [POOL_ARRAY, WATER_ARRAY] {
        if let Some(arr) = world.arrays.get(&array_id)
            && arr.stride == 1
            && arr.data.get(idx).copied().is_some_and(|v| v < 16)
        {
            return true;
        }
    }
    false
}

fn is_road_tile(world: &World, x: usize, y: usize) -> bool {
    let Some(floors) = world.arrays.get(&FLOORS[0]) else {
        return false;
    };
    if floors.stride != 2 {
        return false;
    }
    // Native road/sidewalk floor IDs (9, 10, 11, 12, 13 on level 1) are off-limits
    // for residential building (`0x4887a4`).
    let f = read_floor(&floors.data, x, y);
    matches!(f, 9..=13)
}

/// Whether tile `(x, y)` on `level` (`1` or `2`) is inside the buildable area
/// (`0x4887a4`: `1 <= x <= active - 2`, `1 <= y <= active - 2`, off-road).
pub fn is_buildable_tile(world: &World, x: usize, y: usize, level: u8) -> bool {
    let active = usize::try_from(world.size).unwrap_or(0);
    if active < 4 || x < 1 || y < 1 || x + 1 >= active || y + 1 >= active {
        return false;
    }
    if level == 1 && is_road_tile(world, x, y) {
        return false;
    }
    true
}

/// Whether a level-2 tile `(x, y)` is buildable according to the lot's rebuilt
/// tile flags (array `108` bit `0x01`, computed by `World::rebuild_rooms` in
/// `sims-sim/src/rooms.rs:667`).
fn has_upper_support(world: &World, x: usize, y: usize) -> bool {
    let idx = cell_index(x, y);
    if let Some(flags) = world.arrays.get(&FLAGS_ARRAY[1])
        && flags.stride == 1
        && let Some(&bits) = flags.data.get(idx)
    {
        return (bits & FLAG_BUILDABLE) != 0;
    }
    let ground_room = world.rooms.map[0].get(idx).copied().unwrap_or(-1);
    ground_room >= 0
        && world
            .rooms
            .rooms
            .get(&ground_room)
            .is_some_and(|r| !r.outdoors)
}

/// Whether tile `(x, y)` on `level` can currently accept new wall or floor
/// construction (bounds, off-road, flat terrain, dry land on level 1, and
/// rebuilt `BUILDABLE` support on level 2).
fn tile_supports_build(world: &World, x: usize, y: usize, level: u8) -> bool {
    if !is_buildable_tile(world, x, y, level) {
        return false;
    }
    if level == 1 {
        is_flat_tile(world, x, y) && !is_pool_or_water_tile(world, x, y)
    } else {
        has_upper_support(world, x, y)
    }
}

/// Evaluates ONLY the wall-requirement bits of an object's `data[13]` rules
/// (`0x476f5a`: `0x40` diagonal required, `0x20` corner required, `0x0f`
/// directional non-fence wall required) against `walls_buf`. Forbid-only bits
/// (`0x10`, `0x100..=0x800`) never cause an object to require a wall.
fn object_missing_required_wall(obj: &sims_sim::Object, walls_buf: &[u8], active: usize) -> bool {
    let rules = obj.data.get(13).copied().unwrap_or(0) as u16;
    if (rules & 0x6f) == 0 {
        return false;
    }
    let Ok(tx) = usize::try_from(obj.position[0] >> 4) else {
        return false;
    };
    let Ok(ty) = usize::try_from(obj.position[1] >> 4) else {
        return false;
    };
    if tx >= active || ty >= active {
        return false;
    }
    let cell = ExpandedWall::get(walls_buf, active, tx, ty);
    if (rules & 0x40) != 0 && (cell.segments & (WALL_DESCENDING | WALL_ASCENDING)) == 0 {
        return true;
    }
    let unrotated: [Option<u16>; 4] = [
        ((cell.segments & WALL_WEST) != 0).then(|| cell.styles[0].unwrap_or(1)),
        ((cell.segments & WALL_NORTH) != 0).then(|| cell.styles[1].unwrap_or(1)),
        ((cell.segments & WALL_EAST) != 0).then(|| cell.styles[2].unwrap_or(1)),
        ((cell.segments & WALL_SOUTH) != 0).then(|| cell.styles[3].unwrap_or(1)),
    ];
    let facing = obj.data.get(1).copied().unwrap_or(0);
    let rotation = (((-i32::from(facing)) & 7) >> 1) as usize;
    let mut sides = [None; 4];
    for (i, style) in unrotated.into_iter().enumerate() {
        sides[(i + rotation) & 3] = style;
    }
    let has = |side: usize| sides[side].is_some();
    let corner =
        (has(0) && has(1)) || (has(0) && has(3)) || (has(1) && has(2)) || (has(3) && has(2));
    if (rules & 0x20) != 0 && !corner {
        return true;
    }
    // Native `0x47707a..0x4770db`: edge bits 2, 1, 4, 8 need non-fence walls
    // when rules bits 1, 8, 2, 4 are set respectively.
    for (side, need) in [(1, 1u16), (0, 8), (2, 2), (3, 4)] {
        if (rules & need) != 0
            && !sides[side].is_some_and(|style| !WallStyle::is_fence_style(style))
        {
            return true;
        }
    }
    false
}

/// Collects non-cursor lot objects standing on tile `(tx, ty)` at `level` that
/// genuinely depended on a wall segment removed between `before_walls` and
/// `after_walls`.
fn detached_wall_objects_on_tile(
    world: &World,
    before_walls: &[u8],
    after_walls: &[u8],
    active: usize,
    tile: [u8; 2],
    level: u8,
) -> Vec<u16> {
    let mut leads = BTreeSet::new();
    let tx = i32::from(tile[0]);
    let ty = i32::from(tile[1]);
    let lvl = i32::from(level);
    for (&id, obj) in &world.objects {
        if obj.guid == WALL_CURSOR_GUID || obj.guid == crate::buy::TILE_CURSOR_GUID {
            continue;
        }
        if !on_lot([obj.position[0], obj.position[1]]) || obj.position[2] != lvl {
            continue;
        }
        if (obj.position[0] >> 4) == tx
            && (obj.position[1] >> 4) == ty
            && !object_missing_required_wall(obj, before_walls, active)
            && object_missing_required_wall(obj, after_walls, active)
        {
            let lead = obj
                .multipart
                .as_ref()
                .and_then(|m| u16::try_from(m.parent).ok())
                .filter(|&p| p != 0)
                .unwrap_or(id);
            leads.insert(lead);
        }
    }
    leads.into_iter().collect()
}

/// Checks if a non-wall-mounted object blocks building a diagonal or crossing
/// wall on tile `(tx, ty)`.
fn tile_has_blocking_object(world: &World, tile: [u8; 2], level: u8) -> bool {
    let tx = i32::from(tile[0]);
    let ty = i32::from(tile[1]);
    let lvl = i32::from(level);
    for obj in world.objects.values() {
        if obj.guid == WALL_CURSOR_GUID || obj.guid == crate::buy::TILE_CURSOR_GUID {
            continue;
        }
        if !on_lot([obj.position[0], obj.position[1]]) || obj.position[2] != lvl {
            continue;
        }
        if (obj.position[0] >> 4) == tx && (obj.position[1] >> 4) == ty {
            // Allow if it's a tiny decorative/non-blocking marker (data[8] & 1 == 0
            // or wall-attachable object), otherwise block diagonal wall across it.
            let wall_rules = obj.data.get(13).copied().unwrap_or(0);
            if wall_rules == 0 && obj.person.is_some() {
                return true;
            }
        }
    }
    false
}

/// Plans a wall build or demolition operation without mutating `world`.
pub fn plan_wall_drag(
    world: &World,
    start_corner: [i32; 2],
    end_corner: [i32; 2],
    level: u8,
    style: WallStyle,
    mode: WallDragMode,
    prices: &BuildPrices,
) -> WallPlan {
    let active = usize::try_from(world.size).unwrap_or(0);
    let empty = WallPlan {
        level,
        wall_changes: Vec::new(),
        floor_changes: Vec::new(),
        net_cost: 0,
        valid: false,
        affordable: true,
        removed_objects: Vec::new(),
    };
    if !(1..=2).contains(&level) {
        return empty;
    }
    let level_idx = usize::from(level - 1);
    let Some(orig_walls) = world.arrays.get(&WALLS[level_idx]) else {
        return empty;
    };
    let Some(orig_floors) = world.arrays.get(&FLOORS[level_idx]) else {
        return empty;
    };
    if orig_walls.stride != WALL_CELL || orig_floors.stride != 2 {
        return empty;
    }

    let segments = match mode {
        WallDragMode::Build | WallDragMode::Demolish => {
            wall_line_segments(start_corner, end_corner, active)
        }
        WallDragMode::RoomBox | WallDragMode::RoomBoxDemolish => {
            wall_room_box_segments(start_corner, end_corner, active)
        }
    };
    if segments.is_empty() {
        return empty;
    }

    let mut walls_buf = orig_walls.data.clone();
    let mut floors_buf = orig_floors.data.clone();
    let mut net_cost = 0i32;
    let mut removed_set = BTreeSet::new();
    let mut any_change = false;
    let new_style = style.style_id();
    let new_pattern = style.default_pattern();
    let seg_price = prices.style_price(new_style);

    let mut demolished_tiles = BTreeSet::new();

    for seg in segments {
        let x = usize::from(seg.tile[0]);
        let y = usize::from(seg.tile[1]);
        if !is_buildable_tile(world, x, y, level) {
            return WallPlan {
                valid: false,
                ..empty
            };
        }
        if !mode.is_demolish() && !tile_supports_build(world, x, y, level) {
            return WallPlan {
                valid: false,
                ..empty
            };
        }
        let mut cell = ExpandedWall::get(&walls_buf, active, x, y);
        let is_diag = seg.bit == WALL_DESCENDING || seg.bit == WALL_ASCENDING;

        if mode.is_demolish() {
            if (cell.segments & seg.bit) == 0 {
                continue;
            }
            // Refund wall segment + any applied wallpapers on its faces.
            let old_style = match seg.bit {
                WALL_WEST => cell.styles[0].unwrap_or(1),
                WALL_NORTH => cell.styles[1].unwrap_or(1),
                WALL_DESCENDING => cell.styles[4].unwrap_or(1),
                WALL_ASCENDING => cell.styles[5].unwrap_or(1),
                _ => 1,
            };
            net_cost -= build_refund(prices.style_price(old_style));
            let mut preserve_neighbor_bytes: Option<(usize, [u8; 2])> = None;
            match seg.bit {
                WALL_WEST => {
                    net_cost -= build_refund(prices.wall_price(cell.patterns[0]));
                    if x > 0 {
                        let west_cell = ExpandedWall::get(&walls_buf, active, x - 1, y);
                        if (west_cell.segments & (WALL_DESCENDING | WALL_ASCENDING)) != 0 {
                            // Neighbor holds a diagonal whose side-1 pattern shares
                            // bytes `+12..+14`; preserve it across `cell.put`.
                            let off = cell_index(x - 1, y) * WALL_CELL + 12;
                            preserve_neighbor_bytes =
                                Some((off, [walls_buf[off], walls_buf[off + 1]]));
                        } else {
                            net_cost -= build_refund(prices.wall_price(west_cell.patterns[2]));
                        }
                    }
                    cell.patterns[0] = 0;
                    cell.styles[0] = Some(1);
                }
                WALL_NORTH => {
                    net_cost -= build_refund(prices.wall_price(cell.patterns[1]));
                    if y > 0 {
                        let north_cell = ExpandedWall::get(&walls_buf, active, x, y - 1);
                        if (north_cell.segments & (WALL_DESCENDING | WALL_ASCENDING)) != 0 {
                            // Neighbor holds a diagonal whose side-0 pattern shares
                            // bytes `+10..+12`; preserve it across `cell.put`.
                            let off = cell_index(x, y - 1) * WALL_CELL + 10;
                            preserve_neighbor_bytes =
                                Some((off, [walls_buf[off], walls_buf[off + 1]]));
                        } else {
                            net_cost -= build_refund(prices.wall_price(north_cell.patterns[3]));
                        }
                    }
                    cell.patterns[1] = 0;
                    cell.styles[1] = Some(1);
                }
                WALL_DESCENDING => {
                    net_cost -= build_refund(prices.wall_price(cell.patterns[4]));
                    net_cost -= build_refund(prices.wall_price(cell.patterns[5]));
                    cell.patterns[4] = 0;
                    cell.patterns[5] = 0;
                    if read_floor(&floors_buf, x, y) == SPLIT_FLOOR_ID {
                        // Restore split diagonal tile floor to the surviving half's floor.
                        let f0 = cell.styles[0].unwrap_or(0);
                        let f1 = cell.patterns[0];
                        write_floor(&mut floors_buf, x, y, if f0 != 0 { f0 } else { f1 });
                        cell.styles[0] = Some(1);
                        cell.patterns[0] = 0;
                    }
                }
                WALL_ASCENDING => {
                    net_cost -= build_refund(prices.wall_price(cell.patterns[6]));
                    net_cost -= build_refund(prices.wall_price(cell.patterns[7]));
                    cell.patterns[6] = 0;
                    cell.patterns[7] = 0;
                    if read_floor(&floors_buf, x, y) == SPLIT_FLOOR_ID {
                        let f0 = cell.styles[0].unwrap_or(0);
                        let f1 = cell.patterns[0];
                        write_floor(&mut floors_buf, x, y, if f0 != 0 { f0 } else { f1 });
                        cell.styles[0] = Some(1);
                        cell.patterns[0] = 0;
                    }
                }
                _ => {}
            }
            cell.segments &= !seg.bit;
            if cell.put(&mut walls_buf, active, x, y).is_none() {
                return WallPlan {
                    valid: false,
                    ..empty
                };
            }
            if let Some((off, saved)) = preserve_neighbor_bytes {
                walls_buf[off..off + 2].copy_from_slice(&saved);
            }
            demolished_tiles.insert([x as u8, y as u8]);
            if seg.bit == WALL_WEST && x > 0 {
                demolished_tiles.insert([(x - 1) as u8, y as u8]);
            } else if seg.bit == WALL_NORTH && y > 0 {
                demolished_tiles.insert([x as u8, (y - 1) as u8]);
            }
            any_change = true;
        } else {
            // Building a wall segment.
            if (cell.segments & seg.bit) != 0 {
                continue;
            }
            // Crossing diagonal walls on the same tile are forbidden (`World::rebuild_rooms`
            // rejects `0x30`, matching native `0x49539a`).
            if is_diag {
                if (cell.segments & (WALL_DESCENDING | WALL_ASCENDING)) != 0
                    || (cell.segments & 0x0f) != 0
                    || tile_has_blocking_object(world, seg.tile, level)
                {
                    return WallPlan {
                        valid: false,
                        ..empty
                    };
                }
                // Preserve any existing floor in both split halves if the tile was floored.
                let cur_floor = read_floor(&floors_buf, x, y);
                if cur_floor != 0 && cur_floor != SPLIT_FLOOR_ID {
                    write_floor(&mut floors_buf, x, y, SPLIT_FLOOR_ID);
                    cell.styles[0] = Some(cur_floor);
                    cell.patterns[0] = cur_floor;
                }
            } else if (cell.segments & (WALL_DESCENDING | WALL_ASCENDING)) != 0 {
                return WallPlan {
                    valid: false,
                    ..empty
                };
            }
            cell.segments |= seg.bit;
            match seg.bit {
                WALL_WEST => {
                    cell.styles[0] = Some(new_style);
                    cell.patterns[0] = new_pattern;
                    cell.flags |= 0x03;
                }
                WALL_NORTH => {
                    cell.styles[1] = Some(new_style);
                    cell.patterns[1] = new_pattern;
                    cell.flags |= 0x0c;
                }
                WALL_DESCENDING => {
                    cell.styles[4] = Some(new_style);
                    cell.patterns[4] = new_pattern;
                    cell.patterns[5] = new_pattern;
                    if read_floor(&floors_buf, x, y) != SPLIT_FLOOR_ID {
                        cell.styles[0] = Some(1);
                        cell.patterns[0] = 0;
                    }
                }
                WALL_ASCENDING => {
                    cell.styles[5] = Some(new_style);
                    cell.patterns[6] = new_pattern;
                    cell.patterns[7] = new_pattern;
                    if read_floor(&floors_buf, x, y) != SPLIT_FLOOR_ID {
                        cell.styles[0] = Some(1);
                        cell.patterns[0] = 0;
                    }
                }
                _ => {}
            }
            if cell.put(&mut walls_buf, active, x, y).is_none() {
                return WallPlan {
                    valid: false,
                    ..empty
                };
            }
            // Mirror default fence pattern onto neighbor's facing side (`0x494390`..`0x494600`),
            // skipping diagonal neighbors whose `+10`/`+12` bytes store diagonal wallpaper.
            if new_pattern != 0 {
                if seg.bit == WALL_WEST && x > 0 {
                    let mut west = ExpandedWall::get(&walls_buf, active, x - 1, y);
                    if (west.segments & (WALL_DESCENDING | WALL_ASCENDING)) == 0 {
                        west.patterns[2] = new_pattern;
                        let _ = west.put(&mut walls_buf, active, x - 1, y);
                    }
                } else if seg.bit == WALL_NORTH && y > 0 {
                    let mut north = ExpandedWall::get(&walls_buf, active, x, y - 1);
                    if (north.segments & (WALL_DESCENDING | WALL_ASCENDING)) == 0 {
                        north.patterns[3] = new_pattern;
                        let _ = north.put(&mut walls_buf, active, x, y - 1);
                    }
                }
            }
            net_cost += seg_price;
            any_change = true;
        }
    }

    for tile in demolished_tiles {
        for id in
            detached_wall_objects_on_tile(world, &orig_walls.data, &walls_buf, active, tile, level)
        {
            removed_set.insert(id);
        }
    }

    let mut wall_changes = Vec::new();
    for idx in 0..(FULL_SIZE * FULL_SIZE) {
        let off = idx * WALL_CELL;
        let before = &orig_walls.data[off..off + WALL_CELL];
        let after = &walls_buf[off..off + WALL_CELL];
        if before != after {
            let mut b = [0u8; WALL_CELL];
            let mut a = [0u8; WALL_CELL];
            b.copy_from_slice(before);
            a.copy_from_slice(after);
            wall_changes.push((idx, b, a));
        }
    }
    let mut floor_changes = Vec::new();
    for idx in 0..(FULL_SIZE * FULL_SIZE) {
        let off = idx * 2;
        let b = u16::from_le_bytes([orig_floors.data[off], orig_floors.data[off + 1]]);
        let a = u16::from_le_bytes([floors_buf[off], floors_buf[off + 1]]);
        if b != a {
            floor_changes.push((idx, b, a));
        }
    }

    let affordable = net_cost <= 0 || net_cost <= available_funds(world);
    WallPlan {
        level,
        wall_changes,
        floor_changes,
        net_cost,
        valid: any_change,
        affordable,
        removed_objects: removed_set.into_iter().collect(),
    }
}

/// Plans a floor placement, drag rectangle, Shift room-fill, or Ctrl removal
/// without mutating `world`.
#[allow(clippy::too_many_arguments)]
pub fn plan_floor_edit(
    world: &World,
    start_tile: [i32; 2],
    end_tile: [i32; 2],
    level: u8,
    floor_id: u16,
    mode: FloorDragMode,
    prices: &BuildPrices,
    split_half: Option<u8>,
) -> FloorPlan {
    let active = usize::try_from(world.size).unwrap_or(0);
    let empty = FloorPlan {
        level,
        floor_changes: Vec::new(),
        wall_changes: Vec::new(),
        net_cost: 0,
        valid: false,
        affordable: true,
    };
    if !(1..=2).contains(&level) {
        return empty;
    }
    let level_idx = usize::from(level - 1);
    let Some(orig_floors) = world.arrays.get(&FLOORS[level_idx]) else {
        return empty;
    };
    let Some(orig_walls) = world.arrays.get(&WALLS[level_idx]) else {
        return empty;
    };
    if orig_floors.stride != 2 || orig_walls.stride != WALL_CELL {
        return empty;
    }

    let target_id = if mode.is_remove() { 0 } else { floor_id };
    let new_price = prices.floor_price(target_id);
    let mut floors_buf = orig_floors.data.clone();
    let mut walls_buf = orig_walls.data.clone();

    let mut targets: Vec<(usize, usize, Option<u8>)> = Vec::new();
    if mode.is_room_fill() {
        let sx = usize::try_from(end_tile[0]).ok();
        let sy = usize::try_from(end_tile[1]).ok();
        let (Some(sx), Some(sy)) = (sx, sy) else {
            return empty;
        };
        if !is_buildable_tile(world, sx, sy, level) {
            return empty;
        }
        let clicked_idx = cell_index(sx, sy);
        let clicked_room = world
            .rooms
            .halves
            .get(&[level, sx as u8, sy as u8])
            .and_then(|halves| {
                let half = split_half.unwrap_or(0) as usize;
                halves.get(half).map(|h| h.room)
            })
            .unwrap_or_else(|| {
                world.rooms.map[level_idx]
                    .get(clicked_idx)
                    .copied()
                    .unwrap_or(-1)
            });
        if clicked_room < 0 {
            return empty;
        }
        // Do not Shift-fill the entire outdoors (room 0) unless removing — wait:
        // native `0x4893d8` refuses Shift room-fill on outdoor room 0!
        if world
            .rooms
            .rooms
            .get(&clicked_room)
            .is_some_and(|r| r.outdoors)
        {
            return empty;
        }
        if let Some(room) = world.rooms.rooms.get(&clicked_room) {
            for &[tx, ty, tl] in &room.tiles {
                if tl != level {
                    continue;
                }
                let x = usize::from(tx);
                let y = usize::from(ty);
                if !is_buildable_tile(world, x, y, level) {
                    continue;
                }
                if let Some(halves) = world.rooms.halves.get(&[level, tx, ty]) {
                    for (hi, h) in halves.iter().enumerate() {
                        if h.room == clicked_room {
                            targets.push((x, y, Some(hi as u8)));
                        }
                    }
                } else {
                    targets.push((x, y, None));
                }
            }
        }
    } else {
        let min_x = start_tile[0].min(end_tile[0]).max(1);
        let max_x = start_tile[0].max(end_tile[0]).min(active as i32 - 2);
        let min_y = start_tile[1].min(end_tile[1]).max(1);
        let max_y = start_tile[1].max(end_tile[1]).min(active as i32 - 2);
        if min_x > max_x || min_y > max_y {
            return empty;
        }
        let single = min_x == max_x && min_y == max_y;
        for x in min_x..=max_x {
            for y in min_y..=max_y {
                let ux = x as usize;
                let uy = y as usize;
                if !is_buildable_tile(world, ux, uy, level) {
                    continue;
                }
                let segs = walls_buf[cell_index(ux, uy) * WALL_CELL];
                if (segs & (WALL_DESCENDING | WALL_ASCENDING)) != 0 {
                    if single {
                        targets.push((ux, uy, Some(split_half.unwrap_or(0))));
                    } else {
                        targets.push((ux, uy, Some(0)));
                        targets.push((ux, uy, Some(1)));
                    }
                } else {
                    targets.push((ux, uy, None));
                }
            }
        }
    }

    let funds = available_funds(world);
    let mut net_cost = 0i32;
    let mut any_change = false;
    let mut affordable = true;

    for (x, y, half) in targets {
        if !mode.is_remove() && !tile_supports_build(world, x, y, level) {
            continue;
        }
        let idx = cell_index(x, y);
        let cur_floor = read_floor(&floors_buf, x, y);
        let segs = walls_buf[idx * WALL_CELL];
        let is_diag = (segs & (WALL_DESCENDING | WALL_ASCENDING)) != 0;

        if is_diag && let Some(h) = half {
            // Split diagonal tile (`floor == 255`, halves in `styles[0]` and `patterns[0]`).
            let off = idx * WALL_CELL;
            let mut h0 = if cur_floor == SPLIT_FLOOR_ID {
                u16::from_le_bytes([walls_buf[off + 2], walls_buf[off + 3]])
            } else {
                cur_floor
            };
            let mut h1 = if cur_floor == SPLIT_FLOOR_ID {
                u16::from_le_bytes([walls_buf[off + 6], walls_buf[off + 7]])
            } else {
                cur_floor
            };
            let old_half = if h == 0 { h0 } else { h1 };
            if old_half == target_id {
                continue;
            }
            let delta = if mode.is_remove() {
                -build_refund(prices.floor_price(old_half) / 2)
            } else {
                new_price / 2
            };
            if net_cost + delta > funds {
                affordable = false;
                if !mode.is_room_fill() {
                    break;
                }
                continue;
            }
            if h == 0 {
                h0 = target_id;
            } else {
                h1 = target_id;
            }
            if h0 == 0 && h1 == 0 {
                write_floor(&mut floors_buf, x, y, 0);
                walls_buf[off + 2..off + 4].copy_from_slice(&1u16.to_le_bytes());
                walls_buf[off + 6..off + 8].copy_from_slice(&0u16.to_le_bytes());
            } else {
                write_floor(&mut floors_buf, x, y, SPLIT_FLOOR_ID);
                walls_buf[off + 2..off + 4].copy_from_slice(&h0.to_le_bytes());
                walls_buf[off + 6..off + 8].copy_from_slice(&h1.to_le_bytes());
            }
            net_cost += delta;
            any_change = true;
        } else {
            if cur_floor == target_id {
                continue;
            }
            let delta = if mode.is_remove() {
                -build_refund(prices.floor_price(cur_floor))
            } else {
                new_price
            };
            if net_cost + delta > funds {
                affordable = false;
                if !mode.is_room_fill() {
                    break;
                }
                continue;
            }
            write_floor(&mut floors_buf, x, y, target_id);
            net_cost += delta;
            any_change = true;
        }
    }

    if mode.is_room_fill() && !affordable {
        // Native Shift room-fill refuses if the whole room cannot be afforded.
        return FloorPlan {
            affordable: false,
            ..empty
        };
    }

    let mut floor_changes = Vec::new();
    for idx in 0..(FULL_SIZE * FULL_SIZE) {
        let off = idx * 2;
        let b = u16::from_le_bytes([orig_floors.data[off], orig_floors.data[off + 1]]);
        let a = u16::from_le_bytes([floors_buf[off], floors_buf[off + 1]]);
        if b != a {
            floor_changes.push((idx, b, a));
        }
    }
    let mut wall_changes = Vec::new();
    for idx in 0..(FULL_SIZE * FULL_SIZE) {
        let off = idx * WALL_CELL;
        let before = &orig_walls.data[off..off + WALL_CELL];
        let after = &walls_buf[off..off + WALL_CELL];
        if before != after {
            let mut b = [0u8; WALL_CELL];
            let mut a = [0u8; WALL_CELL];
            b.copy_from_slice(before);
            a.copy_from_slice(after);
            wall_changes.push((idx, b, a));
        }
    }

    FloorPlan {
        level,
        floor_changes,
        wall_changes,
        net_cost,
        valid: any_change,
        affordable: affordable && (net_cost <= 0 || net_cost <= funds),
    }
}

/// Reads the style ID of a wall segment on `cell` (`0x467dec`).
fn face_style(cell: &ExpandedWall, face: WallFace) -> u16 {
    match face.bit {
        WALL_WEST => cell.styles[0].unwrap_or(1),
        WALL_NORTH => cell.styles[1].unwrap_or(1),
        WALL_EAST => cell.styles[2].unwrap_or(1),
        WALL_SOUTH => cell.styles[3].unwrap_or(1),
        WALL_DESCENDING => cell.styles[4].unwrap_or(1),
        WALL_ASCENDING => cell.styles[5].unwrap_or(1),
        _ => 1,
    }
}

/// Maps `(face.bit, face.side)` to the index into `ExpandedWall::patterns`
/// (`0x43517c`).
fn face_pattern_index(face: WallFace) -> Option<usize> {
    match (face.bit, face.side) {
        (WALL_WEST, _) => Some(0),
        (WALL_NORTH, _) => Some(1),
        (WALL_EAST, _) => Some(2),
        (WALL_SOUTH, _) => Some(3),
        (WALL_DESCENDING, 0) => Some(4),
        (WALL_DESCENDING, _) => Some(5),
        (WALL_ASCENDING, 0) => Some(6),
        (WALL_ASCENDING, _) => Some(7),
        _ => None,
    }
}

/// Collects every wall face bordering `room_id` on `level` for Shift room-fill
/// wallpaper (`0x48a933`).
pub fn room_wallpaper_faces(world: &World, room_id: i16, level: u8) -> Vec<WallFace> {
    if !(1..=2).contains(&level) {
        return Vec::new();
    }
    let active = usize::try_from(world.size).unwrap_or(0);
    let level_idx = usize::from(level - 1);
    let Some(walls) = world.arrays.get(&WALLS[level_idx]) else {
        return Vec::new();
    };
    if walls.stride != WALL_CELL {
        return Vec::new();
    }
    let Some(room) = world.rooms.rooms.get(&room_id) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for &[tx, ty, tl] in &room.tiles {
        if tl != level {
            continue;
        }
        let x = usize::from(tx);
        let y = usize::from(ty);
        if !is_buildable_tile(world, x, y, level) {
            continue;
        }
        let cell = ExpandedWall::get(&walls.data, active, x, y);
        for bit in [WALL_WEST, WALL_NORTH, WALL_EAST, WALL_SOUTH] {
            if (cell.segments & bit) != 0 {
                out.push(WallFace {
                    tile: [tx, ty],
                    bit,
                    side: 0,
                });
            }
        }
        if (cell.segments & (WALL_DESCENDING | WALL_ASCENDING)) != 0 {
            let bit = if (cell.segments & WALL_DESCENDING) != 0 {
                WALL_DESCENDING
            } else {
                WALL_ASCENDING
            };
            if let Some(halves) = world.rooms.halves.get(&[level, tx, ty]) {
                for (hi, h) in halves.iter().enumerate() {
                    if h.room == room_id {
                        out.push(WallFace {
                            tile: [tx, ty],
                            bit,
                            side: hi as u8,
                        });
                    }
                }
            } else {
                // Freestanding diagonal whose both sides belong to `room_id`
                // has no split-half record in `rooms.halves`.
                out.push(WallFace {
                    tile: [tx, ty],
                    bit,
                    side: 0,
                });
                out.push(WallFace {
                    tile: [tx, ty],
                    bit,
                    side: 1,
                });
            }
        }
    }
    out
}

/// Plans applying or stripping wallpaper on `faces` (or Shift room-fill from
/// the room of `faces[0]`) without mutating `world`.
pub fn plan_wallpaper_edit(
    world: &World,
    faces: &[WallFace],
    level: u8,
    pattern_id: u16,
    mode: WallpaperDragMode,
    prices: &BuildPrices,
) -> WallpaperPlan {
    let active = usize::try_from(world.size).unwrap_or(0);
    let empty = WallpaperPlan {
        level,
        wall_changes: Vec::new(),
        net_cost: 0,
        valid: false,
        affordable: true,
    };
    if !(1..=2).contains(&level) {
        return empty;
    }
    let level_idx = usize::from(level - 1);
    let Some(orig_walls) = world.arrays.get(&WALLS[level_idx]) else {
        return empty;
    };
    if orig_walls.stride != WALL_CELL || faces.is_empty() {
        return empty;
    }

    let resolved: Vec<WallFace> = if mode.is_room_fill() {
        let first = faces[0];
        let x = usize::from(first.tile[0]);
        let y = usize::from(first.tile[1]);
        let idx = cell_index(x, y);
        let room_id = world
            .rooms
            .halves
            .get(&[level, first.tile[0], first.tile[1]])
            .and_then(|halves| halves.get(usize::from(first.side)).map(|h| h.room))
            .unwrap_or_else(|| world.rooms.map[level_idx].get(idx).copied().unwrap_or(-1));
        if room_id < 0 || world.rooms.rooms.get(&room_id).is_some_and(|r| r.outdoors) {
            // Native room-fill on outdoor walls applies to connected exterior wall
            // faces along the same line; fall back to the clicked face(s).
            faces.to_vec()
        } else {
            room_wallpaper_faces(world, room_id, level)
        }
    } else {
        faces.to_vec()
    };

    let target_pattern = if mode.is_strip() { 0 } else { pattern_id };
    let apply_price = prices.wall_price(target_pattern);
    let funds = available_funds(world);
    let mut walls_buf = orig_walls.data.clone();
    let mut net_cost = 0i32;
    let mut any_change = false;
    let mut affordable = true;

    for face in resolved {
        let x = usize::from(face.tile[0]);
        let y = usize::from(face.tile[1]);
        if !is_buildable_tile(world, x, y, level) {
            continue;
        }
        let Some(pidx) = face_pattern_index(face) else {
            continue;
        };
        let mut cell = ExpandedWall::get(&walls_buf, active, x, y);
        if (cell.segments & face.bit) == 0 {
            continue;
        }
        // Native `0x4937da`: refuse to wallpaper fences/balustrades (styles 2, 12, 13, 14).
        if WallStyle::is_fence_style(face_style(&cell, face)) {
            continue;
        }
        let old = cell.patterns[pidx];
        if old == target_pattern {
            continue;
        }
        let delta = if mode.is_strip() {
            -build_refund(prices.wall_price(old))
        } else {
            apply_price
        };
        if net_cost + delta > funds {
            affordable = false;
            if !mode.is_room_fill() {
                break;
            }
            continue;
        }
        cell.patterns[pidx] = target_pattern;
        if cell.put(&mut walls_buf, active, x, y).is_none() {
            continue;
        }
        net_cost += delta;
        any_change = true;
    }

    if mode.is_room_fill() && !affordable {
        return WallpaperPlan {
            affordable: false,
            ..empty
        };
    }

    let mut wall_changes = Vec::new();
    for idx in 0..(FULL_SIZE * FULL_SIZE) {
        let off = idx * WALL_CELL;
        let before = &orig_walls.data[off..off + WALL_CELL];
        let after = &walls_buf[off..off + WALL_CELL];
        if before != after {
            let mut b = [0u8; WALL_CELL];
            let mut a = [0u8; WALL_CELL];
            b.copy_from_slice(before);
            a.copy_from_slice(after);
            wall_changes.push((idx, b, a));
        }
    }

    WallpaperPlan {
        level,
        wall_changes,
        net_cost,
        valid: any_change,
        affordable: affordable && (net_cost <= 0 || net_cost <= funds),
    }
}

/// Verifies that every cell in `wall_changes` and `floor_changes` currently
/// matches its expected pre-state (`before` when `forward == true`, `after`
/// when `forward == false`), rejecting stale or cross-lot plans.
fn cell_diffs_match(
    world: &World,
    level: u8,
    wall_changes: &[(usize, [u8; WALL_CELL], [u8; WALL_CELL])],
    floor_changes: &[(usize, u16, u16)],
    forward: bool,
) -> bool {
    if !(1..=2).contains(&level) {
        return false;
    }
    let level_idx = usize::from(level - 1);
    if !wall_changes.is_empty() {
        let Some(walls) = world.arrays.get(&WALLS[level_idx]) else {
            return false;
        };
        if walls.stride != WALL_CELL {
            return false;
        }
        for &(idx, before, after) in wall_changes {
            let off = idx.saturating_mul(WALL_CELL);
            let expected = if forward { &before } else { &after };
            if walls.data.get(off..off + WALL_CELL) != Some(expected.as_slice()) {
                return false;
            }
        }
    }
    if !floor_changes.is_empty() {
        let Some(floors) = world.arrays.get(&FLOORS[level_idx]) else {
            return false;
        };
        if floors.stride != 2 {
            return false;
        }
        for &(idx, before, after) in floor_changes {
            let off = idx.saturating_mul(2);
            let expected = if forward { before } else { after };
            if floors.data.get(off..off + 2) != Some(expected.to_le_bytes().as_slice()) {
                return false;
            }
        }
    }
    true
}

/// Applies raw sparse wall and floor cell diffs to `world`.
fn apply_cell_diffs(
    world: &mut World,
    level: u8,
    wall_changes: &[(usize, [u8; WALL_CELL], [u8; WALL_CELL])],
    floor_changes: &[(usize, u16, u16)],
    forward: bool,
) -> Result<(), Fault> {
    if !(1..=2).contains(&level) {
        return Err(Fault {
            object: 0,
            at: None,
            trap: Trap::Invalid(format!("invalid build level {level}")),
        });
    }
    let level_idx = usize::from(level - 1);
    if !wall_changes.is_empty() {
        let walls = world.arrays.get_mut(&WALLS[level_idx]).ok_or(Fault {
            object: 0,
            at: None,
            trap: Trap::Invalid("missing wall array".into()),
        })?;
        for &(idx, before, after) in wall_changes {
            let off = idx * WALL_CELL;
            let bytes = if forward { &after } else { &before };
            if let Some(dst) = walls.data.get_mut(off..off + WALL_CELL) {
                dst.copy_from_slice(bytes);
            }
        }
    }
    if !floor_changes.is_empty() {
        let floors = world.arrays.get_mut(&FLOORS[level_idx]).ok_or(Fault {
            object: 0,
            at: None,
            trap: Trap::Invalid("missing floor array".into()),
        })?;
        for &(idx, before, after) in floor_changes {
            let off = idx * 2;
            let val = if forward { after } else { before };
            if let Some(dst) = floors.data.get_mut(off..off + 2) {
                dst.copy_from_slice(&val.to_le_bytes());
            }
        }
    }
    Ok(())
}

impl Interpreter<'_> {
    /// Creates the Build Mode `"Wall Cursor"` (`0x00000439`, type 9, off-lot)
    /// with `data[0] = mode` (`0x48b68b` / `0x48a66a`), matching native
    /// `cXWallCursor` construction (post-constructor, reset, `install_main`,
    /// and `Init`, leaving `registry_word = 0xffff` without running `turn` or
    /// `finish`).
    pub fn build_cursor_open(&self, world: &mut World, mode: i16) -> Result<WallCursor, Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        let id = (1..=i16::MAX as u16)
            .find(|id| !world.objects.contains_key(id))
            .ok_or_else(|| fail(0, Trap::Unsupported("no unused object ID".into())))?;
        let mut machine = self.machine(world, id, 0, false);
        let words = machine
            .construct_tile_cursor(WALL_CURSOR_GUID, None)
            .map_err(|trap| fail(id, trap))?;
        machine
            .reset(words)
            .and_then(|_| machine.install_main())
            .map_err(|trap| fail(id, trap))?;
        self.init(world, id)?;
        if let Some(obj) = world.objects.get_mut(&id) {
            if let Some(w) = obj.data.get_mut(0) {
                *w = mode;
            }
            if let Some(w) = obj.data.get_mut(1) {
                *w = 0;
            }
        }
        Ok(WallCursor {
            id,
            mode,
            facing: 0,
            position: [-16, -16, 1],
            token: 0,
        })
    }

    /// Moves/updates `"Wall Cursor"` on the lot (`0x48b797` / `0x48a7e9`).
    pub fn build_cursor_move(
        &self,
        world: &mut World,
        cursor: &mut WallCursor,
        fine: [i32; 3],
        facing: Option<i16>,
        mode: Option<i16>,
    ) -> Result<(), Fault> {
        let Some(obj) = world
            .objects
            .get_mut(&cursor.id)
            .filter(|o| o.guid == WALL_CURSOR_GUID)
        else {
            return Ok(());
        };
        if let Some(m) = mode {
            cursor.mode = m;
            if let Some(w) = obj.data.get_mut(0) {
                *w = m;
            }
        }
        if let Some(f) = facing {
            let norm = f.rem_euclid(8);
            cursor.facing = norm;
            if let Some(w) = obj.data.get_mut(1) {
                *w = norm;
            }
        }
        // Place `"Wall Cursor"` at `fine` (unlinking from any previous tile first).
        let _ = self.take_off(world, cursor.id, false);
        let place = Place {
            on_lot: true,
            position: [fine[0], fine[1]],
            container: None,
            slot: -1,
            level: fine[2],
        };
        let _ = self.put(world, cursor.id, place, true);
        if let Some(obj) = world.objects.get(&cursor.id) {
            cursor.position = obj.position;
        }
        Ok(())
    }

    /// Hides `"Wall Cursor"` off-lot without deleting it (`0x48b781`, `0x48a8ae`).
    pub fn build_cursor_hide(
        &self,
        world: &mut World,
        cursor: &mut WallCursor,
    ) -> Result<(), Fault> {
        if world
            .objects
            .get(&cursor.id)
            .is_some_and(|o| o.guid == WALL_CURSOR_GUID)
        {
            let _ = self.take_off(world, cursor.id, false);
            if let Some(obj) = world.objects.get(&cursor.id) {
                cursor.position = obj.position;
            }
        }
        Ok(())
    }

    /// Deletes `"Wall Cursor"` (`0x48a6e5`).
    pub fn build_cursor_close(&self, world: &mut World, cursor: &WallCursor) -> Result<(), Fault> {
        if world
            .objects
            .get(&cursor.id)
            .is_some_and(|o| o.guid == WALL_CURSOR_GUID)
        {
            self.delete(world, cursor.id)?;
        }
        Ok(())
    }

    /// Recomputes rooms, room assignments, and room lighting after walls change
    /// (`0x48b5a4` → signal `0x91`).
    pub fn rebuild_architecture_rooms(&self, world: &mut World) -> Result<(), Fault> {
        world.rebuild_rooms().map_err(|e| Fault {
            object: 0,
            at: None,
            trap: Trap::Invalid(e.to_string()),
        })?;
        self.assign_rooms(world)?;
        self.update_rooms(world, 0)?;
        Ok(())
    }

    /// Commits a planned wall build or demolition to `world`, updating rooms,
    /// wall-mounted objects (detaching them for Undo), and Architecture funds.
    pub fn build_commit_wall(
        &self,
        world: &mut World,
        plan: &WallPlan,
    ) -> Result<Option<BuildHistory>, Fault> {
        if !plan.valid
            || !plan.affordable
            || !(1..=2).contains(&plan.level)
            || (plan.net_cost > 0 && plan.net_cost > available_funds(world))
            || !cell_diffs_match(
                world,
                plan.level,
                &plan.wall_changes,
                &plan.floor_changes,
                true,
            )
        {
            return Ok(None);
        }
        // Re-validate live buildability/support for every tile gaining a new
        // owned wall segment (`WEST`, `NORTH`, `DESCENDING`, `ASCENDING`),
        // rejecting stale plans whose support or terrain changed.
        for &(idx, before, after) in &plan.wall_changes {
            let added = (after[0] & !before[0])
                & (WALL_WEST | WALL_NORTH | WALL_DESCENDING | WALL_ASCENDING);
            if added != 0 {
                let x = idx / FULL_SIZE;
                let y = idx % FULL_SIZE;
                if !tile_supports_build(world, x, y, plan.level) {
                    return Ok(None);
                }
            }
        }
        // Detach any wall-attached objects on demolished segments, refunding
        // their resale value cleanly (refuse if any is undeletable).
        let mut removed_objects = Vec::new();
        for &obj_id in &plan.removed_objects {
            if !world.objects.contains_key(&obj_id) {
                continue;
            }
            if !self.can_delete(world, obj_id)? {
                return Ok(None);
            }
            let Some(cursor) = self.buy_pick_up(world, obj_id)? else {
                return Ok(None);
            };
            let Some(sold_opt) = self.buy_sell(world, &cursor)? else {
                return Ok(None);
            };
            if let Some(sold) = sold_opt {
                removed_objects.push(sold);
            }
        }
        apply_cell_diffs(
            world,
            plan.level,
            &plan.wall_changes,
            &plan.floor_changes,
            true,
        )?;
        self.rebuild_architecture_rooms(world)?;
        if plan.net_cost != 0 {
            spend(world, ARCHITECTURE_BUDGET, plan.net_cost).map_err(|trap| Fault {
                object: 0,
                at: None,
                trap,
            })?;
        }
        Ok(Some(BuildHistory::Wall {
            level: plan.level,
            wall_changes: plan.wall_changes.clone(),
            floor_changes: plan.floor_changes.clone(),
            net_cost: plan.net_cost,
            removed_objects,
        }))
    }

    /// Commits a planned floor edit to `world` and charges/refunds Architecture.
    pub fn build_commit_floor(
        &self,
        world: &mut World,
        plan: &FloorPlan,
    ) -> Result<Option<BuildHistory>, Fault> {
        if !plan.valid
            || !plan.affordable
            || !(1..=2).contains(&plan.level)
            || (plan.net_cost > 0 && plan.net_cost > available_funds(world))
            || !cell_diffs_match(
                world,
                plan.level,
                &plan.wall_changes,
                &plan.floor_changes,
                true,
            )
        {
            return Ok(None);
        }
        // Re-validate live buildability/support for every tile gaining or
        // changing to a non-empty floor (rejecting stale plans after support
        // below was removed, while always allowing full or half-tile removals).
        let level_idx = usize::from(plan.level - 1);
        let floor_after_map: BTreeMap<usize, (u16, u16)> = plan
            .floor_changes
            .iter()
            .map(|&(idx, before, after)| (idx, (before, after)))
            .collect();
        let wall_after_map: BTreeMap<usize, ([u8; WALL_CELL], [u8; WALL_CELL])> = plan
            .wall_changes
            .iter()
            .map(|&(idx, before, after)| (idx, (before, after)))
            .collect();
        let mut checked_tiles = BTreeSet::new();
        for &(idx, _, _) in &plan.floor_changes {
            checked_tiles.insert(idx);
        }
        for &(idx, _, _) in &plan.wall_changes {
            checked_tiles.insert(idx);
        }
        for idx in checked_tiles {
            let cur_floor = world
                .arrays
                .get(&FLOORS[level_idx])
                .and_then(|f| f.data.get(idx * 2..idx * 2 + 2))
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .unwrap_or(0);
            let (before_floor, after_floor) = floor_after_map
                .get(&idx)
                .copied()
                .unwrap_or((cur_floor, cur_floor));
            let laying = if after_floor == 0 {
                false
            } else if after_floor == SPLIT_FLOOR_ID {
                let (before_w, after_w) = wall_after_map.get(&idx).copied().unwrap_or_else(|| {
                    let mut raw = [0u8; WALL_CELL];
                    if let Some(s) = world
                        .arrays
                        .get(&WALLS[level_idx])
                        .and_then(|w| w.data.get(idx * WALL_CELL..idx * WALL_CELL + WALL_CELL))
                    {
                        raw.copy_from_slice(s);
                    }
                    (raw, raw)
                });
                let (bh0, bh1) = if before_floor == SPLIT_FLOOR_ID {
                    (
                        u16::from_le_bytes([before_w[2], before_w[3]]),
                        u16::from_le_bytes([before_w[6], before_w[7]]),
                    )
                } else {
                    (before_floor, before_floor)
                };
                let ah0 = u16::from_le_bytes([after_w[2], after_w[3]]);
                let ah1 = u16::from_le_bytes([after_w[6], after_w[7]]);
                (ah0 != bh0 && ah0 != 0) || (ah1 != bh1 && ah1 != 0)
            } else {
                after_floor != before_floor
            };
            if laying {
                let x = idx / FULL_SIZE;
                let y = idx % FULL_SIZE;
                if !tile_supports_build(world, x, y, plan.level) {
                    return Ok(None);
                }
            }
        }
        if plan.net_cost != 0 {
            spend(world, ARCHITECTURE_BUDGET, plan.net_cost).map_err(|trap| Fault {
                object: 0,
                at: None,
                trap,
            })?;
        }
        apply_cell_diffs(
            world,
            plan.level,
            &plan.wall_changes,
            &plan.floor_changes,
            true,
        )?;
        if plan.level == 2 {
            self.rebuild_architecture_rooms(world)?;
        }
        Ok(Some(BuildHistory::Floor {
            level: plan.level,
            floor_changes: plan.floor_changes.clone(),
            wall_changes: plan.wall_changes.clone(),
            net_cost: plan.net_cost,
        }))
    }

    /// Commits a planned wallpaper edit to `world` and charges/refunds Architecture.
    pub fn build_commit_wallpaper(
        &self,
        world: &mut World,
        plan: &WallpaperPlan,
    ) -> Result<Option<BuildHistory>, Fault> {
        if !plan.valid
            || !plan.affordable
            || !(1..=2).contains(&plan.level)
            || (plan.net_cost > 0 && plan.net_cost > available_funds(world))
            || !cell_diffs_match(world, plan.level, &plan.wall_changes, &[], true)
        {
            return Ok(None);
        }
        if plan.net_cost != 0 {
            spend(world, ARCHITECTURE_BUDGET, plan.net_cost).map_err(|trap| Fault {
                object: 0,
                at: None,
                trap,
            })?;
        }
        apply_cell_diffs(world, plan.level, &plan.wall_changes, &[], true)?;
        Ok(Some(BuildHistory::Wallpaper {
            level: plan.level,
            wall_changes: plan.wall_changes.clone(),
            net_cost: plan.net_cost,
        }))
    }

    /// Undoes one committed [`BuildHistory`] entry (`0x493e5f`, `0x4881e6`,
    /// `0x493547`). Returns `Ok(false)` if the household cannot afford to
    /// reverse a refund.
    pub fn build_undo_entry(
        &self,
        world: &mut World,
        entry: &mut BuildHistory,
    ) -> Result<bool, Fault> {
        let reverse_cost = -entry.net_cost();
        if reverse_cost > 0 && reverse_cost > available_funds(world) {
            return Ok(false);
        }
        match entry {
            BuildHistory::Wall {
                level,
                wall_changes,
                floor_changes,
                net_cost,
                removed_objects,
            } => {
                // Check affordability of restoring demolished wall objects first.
                let obj_cost: i32 = removed_objects.iter().map(|p| p.price).sum();
                if reverse_cost + obj_cost > 0 && reverse_cost + obj_cost > available_funds(world) {
                    return Ok(false);
                }
                if !cell_diffs_match(world, *level, wall_changes, floor_changes, false) {
                    return Ok(false);
                }
                apply_cell_diffs(world, *level, wall_changes, floor_changes, false)?;
                self.rebuild_architecture_rooms(world)?;
                if *net_cost != 0 {
                    spend(world, ARCHITECTURE_BUDGET, -*net_cost).map_err(|trap| Fault {
                        object: 0,
                        at: None,
                        trap,
                    })?;
                }
                for purchase in removed_objects.iter_mut().rev() {
                    if purchase.kind == BuyActionKind::Sell
                        && !purchase.placed
                        && !self.buy_undo_purchase(world, purchase)?
                    {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            BuildHistory::Floor {
                level,
                floor_changes,
                wall_changes,
                net_cost,
            } => {
                if !cell_diffs_match(world, *level, wall_changes, floor_changes, false) {
                    return Ok(false);
                }
                apply_cell_diffs(world, *level, wall_changes, floor_changes, false)?;
                if *level == 2 {
                    self.rebuild_architecture_rooms(world)?;
                }
                if *net_cost != 0 {
                    spend(world, ARCHITECTURE_BUDGET, -*net_cost).map_err(|trap| Fault {
                        object: 0,
                        at: None,
                        trap,
                    })?;
                }
                Ok(true)
            }
            BuildHistory::Wallpaper {
                level,
                wall_changes,
                net_cost,
            } => {
                if !cell_diffs_match(world, *level, wall_changes, &[], false) {
                    return Ok(false);
                }
                apply_cell_diffs(world, *level, wall_changes, &[], false)?;
                if *net_cost != 0 {
                    spend(world, ARCHITECTURE_BUDGET, -*net_cost).map_err(|trap| Fault {
                        object: 0,
                        at: None,
                        trap,
                    })?;
                }
                Ok(true)
            }
            BuildHistory::Terrain {
                height_changes,
                grass_changes,
                net_cost,
            } => {
                if !terrain_diffs_match_for_history(world, height_changes, grass_changes, false)
                    || !terrain_changes_allowed(world, height_changes, grass_changes)
                {
                    return Ok(false);
                }
                let mut staged = world.clone();
                apply_terrain_diffs(&mut staged, height_changes, grass_changes, false)?;
                if !height_changes.is_empty() {
                    self.rebuild_architecture_rooms(&mut staged)?;
                }
                if *net_cost != 0 {
                    spend(&mut staged, ARCHITECTURE_BUDGET, -*net_cost).map_err(|trap| Fault {
                        object: 0,
                        at: None,
                        trap,
                    })?;
                }
                *world = staged;
                Ok(true)
            }
            BuildHistory::Water {
                water_changes,
                height_changes,
                grass_changes,
                created_water_array,
                net_cost,
            } => {
                if !byte_array_diffs_match(
                    world,
                    WATER_ARRAY,
                    water_changes,
                    *created_water_array,
                    false,
                ) || !terrain_diffs_match_for_history(
                    world,
                    height_changes,
                    grass_changes,
                    false,
                ) || !water_changes_allowed(world, water_changes, height_changes, false)
                {
                    return Ok(false);
                }
                let mut staged = world.clone();
                apply_byte_array_diffs(
                    &mut staged,
                    WATER_ARRAY,
                    water_changes,
                    *created_water_array,
                    false,
                )?;
                apply_terrain_diffs(&mut staged, height_changes, grass_changes, false)?;
                if !height_changes.is_empty() {
                    self.rebuild_architecture_rooms(&mut staged)?;
                }
                if *net_cost != 0 {
                    spend(&mut staged, ARCHITECTURE_BUDGET, -*net_cost).map_err(|trap| Fault {
                        object: 0,
                        at: None,
                        trap,
                    })?;
                }
                *world = staged;
                Ok(true)
            }
            BuildHistory::Pool {
                pool_changes,
                created_pool_array,
                net_cost,
            } => {
                if !byte_array_diffs_match(
                    world,
                    POOL_ARRAY,
                    pool_changes,
                    *created_pool_array,
                    false,
                ) || !pool_changes_allowed(world, pool_changes, false)
                {
                    return Ok(false);
                }
                let mut staged = world.clone();
                apply_byte_array_diffs(
                    &mut staged,
                    POOL_ARRAY,
                    pool_changes,
                    *created_pool_array,
                    false,
                )?;
                self.rebuild_architecture_rooms(&mut staged)?;
                if *net_cost != 0 {
                    spend(&mut staged, ARCHITECTURE_BUDGET, -*net_cost).map_err(|trap| Fault {
                        object: 0,
                        at: None,
                        trap,
                    })?;
                }
                *world = staged;
                Ok(true)
            }
            BuildHistory::Roof {
                before_raw,
                after_raw,
            } => {
                if !roof_meta_matches(world, Some(after_raw.as_slice())) {
                    return Ok(false);
                }
                apply_roof_meta_raw(world, before_raw.as_deref());
                Ok(true)
            }
        }
    }

    /// Redoes one undone [`BuildHistory`] entry (`0x49409a`, `0x488230`,
    /// `0x49359c`). Returns `Ok(false)` if the household cannot afford it.
    pub fn build_redo_entry(
        &self,
        world: &mut World,
        entry: &mut BuildHistory,
    ) -> Result<bool, Fault> {
        let forward_cost = entry.net_cost();
        if forward_cost > 0 && forward_cost > available_funds(world) {
            return Ok(false);
        }
        match entry {
            BuildHistory::Wall {
                level,
                wall_changes,
                floor_changes,
                net_cost,
                removed_objects,
            } => {
                if !cell_diffs_match(world, *level, wall_changes, floor_changes, true) {
                    return Ok(false);
                }
                for purchase in removed_objects.iter_mut() {
                    if purchase.kind == BuyActionKind::Sell
                        && purchase.placed
                        && !self.buy_redo_purchase(world, purchase)?
                    {
                        return Ok(false);
                    }
                }
                apply_cell_diffs(world, *level, wall_changes, floor_changes, true)?;
                self.rebuild_architecture_rooms(world)?;
                if *net_cost != 0 {
                    spend(world, ARCHITECTURE_BUDGET, *net_cost).map_err(|trap| Fault {
                        object: 0,
                        at: None,
                        trap,
                    })?;
                }
                Ok(true)
            }
            BuildHistory::Floor {
                level,
                floor_changes,
                wall_changes,
                net_cost,
            } => {
                if !cell_diffs_match(world, *level, wall_changes, floor_changes, true) {
                    return Ok(false);
                }
                apply_cell_diffs(world, *level, wall_changes, floor_changes, true)?;
                if *level == 2 {
                    self.rebuild_architecture_rooms(world)?;
                }
                if *net_cost != 0 {
                    spend(world, ARCHITECTURE_BUDGET, *net_cost).map_err(|trap| Fault {
                        object: 0,
                        at: None,
                        trap,
                    })?;
                }
                Ok(true)
            }
            BuildHistory::Wallpaper {
                level,
                wall_changes,
                net_cost,
            } => {
                if !cell_diffs_match(world, *level, wall_changes, &[], true) {
                    return Ok(false);
                }
                apply_cell_diffs(world, *level, wall_changes, &[], true)?;
                if *net_cost != 0 {
                    spend(world, ARCHITECTURE_BUDGET, *net_cost).map_err(|trap| Fault {
                        object: 0,
                        at: None,
                        trap,
                    })?;
                }
                Ok(true)
            }
            BuildHistory::Terrain {
                height_changes,
                grass_changes,
                net_cost,
            } => {
                if !terrain_diffs_match_for_history(world, height_changes, grass_changes, true)
                    || !terrain_changes_allowed(world, height_changes, grass_changes)
                {
                    return Ok(false);
                }
                let mut staged = world.clone();
                apply_terrain_diffs(&mut staged, height_changes, grass_changes, true)?;
                if !height_changes.is_empty() {
                    self.rebuild_architecture_rooms(&mut staged)?;
                }
                if *net_cost != 0 {
                    spend(&mut staged, ARCHITECTURE_BUDGET, *net_cost).map_err(|trap| Fault {
                        object: 0,
                        at: None,
                        trap,
                    })?;
                }
                *world = staged;
                Ok(true)
            }
            BuildHistory::Water {
                water_changes,
                height_changes,
                grass_changes,
                created_water_array,
                net_cost,
            } => {
                if !byte_array_diffs_match(
                    world,
                    WATER_ARRAY,
                    water_changes,
                    *created_water_array,
                    true,
                ) || !terrain_diffs_match_for_history(world, height_changes, grass_changes, true)
                    || !water_changes_allowed(world, water_changes, height_changes, true)
                {
                    return Ok(false);
                }
                let mut staged = world.clone();
                apply_byte_array_diffs(
                    &mut staged,
                    WATER_ARRAY,
                    water_changes,
                    *created_water_array,
                    true,
                )?;
                apply_terrain_diffs(&mut staged, height_changes, grass_changes, true)?;
                if !height_changes.is_empty() {
                    self.rebuild_architecture_rooms(&mut staged)?;
                }
                if *net_cost != 0 {
                    spend(&mut staged, ARCHITECTURE_BUDGET, *net_cost).map_err(|trap| Fault {
                        object: 0,
                        at: None,
                        trap,
                    })?;
                }
                *world = staged;
                Ok(true)
            }
            BuildHistory::Pool {
                pool_changes,
                created_pool_array,
                net_cost,
            } => {
                if !byte_array_diffs_match(
                    world,
                    POOL_ARRAY,
                    pool_changes,
                    *created_pool_array,
                    true,
                ) || !pool_changes_allowed(world, pool_changes, true)
                {
                    return Ok(false);
                }
                let mut staged = world.clone();
                apply_byte_array_diffs(
                    &mut staged,
                    POOL_ARRAY,
                    pool_changes,
                    *created_pool_array,
                    true,
                )?;
                self.rebuild_architecture_rooms(&mut staged)?;
                if *net_cost != 0 {
                    spend(&mut staged, ARCHITECTURE_BUDGET, *net_cost).map_err(|trap| Fault {
                        object: 0,
                        at: None,
                        trap,
                    })?;
                }
                *world = staged;
                Ok(true)
            }
            BuildHistory::Roof {
                before_raw,
                after_raw,
            } => {
                if !roof_meta_matches(world, before_raw.as_deref()) {
                    return Ok(false);
                }
                apply_roof_meta_raw(world, Some(after_raw.as_slice()));
                Ok(true)
            }
        }
    }
}

/// Applies a [`WallPlan`], [`FloorPlan`], or [`WallpaperPlan`] to a cloned
/// [`World`] (or raw arrays) for live Build Mode rendering previews.
pub fn preview_wall_on_world(world: &mut World, plan: &WallPlan) {
    let _ = apply_cell_diffs(
        world,
        plan.level,
        &plan.wall_changes,
        &plan.floor_changes,
        true,
    );
}

pub fn preview_floor_on_world(world: &mut World, plan: &FloorPlan) {
    let _ = apply_cell_diffs(
        world,
        plan.level,
        &plan.wall_changes,
        &plan.floor_changes,
        true,
    );
}

pub fn preview_wallpaper_on_world(world: &mut World, plan: &WallpaperPlan) {
    let _ = apply_cell_diffs(world, plan.level, &plan.wall_changes, &[], true);
}

// ============================================================================
// Build Mode Part 2: Terrain, Water/Pond, Swimming Pool & Roof Style/Pitch
// ============================================================================

/// Pricing table for Build Mode Part 2 tools (`GameData/Build.iff STR# 128`
/// and `STR# 133`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolPrices {
    pub terrain_raise: i32,
    pub terrain_lower: i32,
    pub terrain_level: i32,
    pub grass: i32,
    pub pool: i32,
    pub water: i32,
}

impl Default for ToolPrices {
    fn default() -> Self {
        Self {
            terrain_raise: DEFAULT_TERRAIN_PRICE,
            terrain_lower: DEFAULT_TERRAIN_PRICE,
            terrain_level: DEFAULT_TERRAIN_PRICE,
            grass: DEFAULT_GRASS_PRICE,
            pool: DEFAULT_POOL_PRICE,
            water: DEFAULT_WATER_PRICE,
        }
    }
}

/// Terrain editing mode (`BuildTool::Terrain`, `GameData/Build.iff STR# 128`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerrainMode {
    /// Raise terrain vertex/vertices (`+$1` per modified vertex).
    Raise,
    /// Lower terrain vertex/vertices (`+$1` per modified vertex).
    Lower,
    /// Flatten rectangle to the anchor (`start`) vertex height (`+$1` per modified vertex).
    Level,
    /// Flatten rectangle to the rounded average height of its editable vertices (`+$1` per modified vertex).
    LevelAverage,
    /// Paint healthy green grass back over dirt/sloped tiles (`+$1` per tile).
    RestoreGrass,
    /// Ctrl-paint to remove grass back to bare soil (`$0`).
    RemoveGrass,
}

/// Computed preview / planned mutation for a Terrain or Grass tool edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainPlan {
    pub mode: TerrainMode,
    pub height_changes: Vec<(usize, [u8; 4], [u8; 4])>,
    pub grass_changes: Vec<(usize, i8, i8)>,
    pub net_cost: i32,
    pub valid: bool,
    pub affordable: bool,
}

/// Pond/Water editing mode (`BuildTool::Water` — Water Tool, `STR# 133[2]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WaterMode {
    /// Paint pond water tiles (`+$15` per tile) with basin terrain depression.
    Paint,
    /// Ctrl-remove pond water tiles (`-$12`, 80% refund per tile).
    Remove,
}

/// Computed preview / planned mutation for a Pond/Water edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaterPlan {
    pub mode: WaterMode,
    pub water_changes: Vec<(usize, u8, u8)>,
    pub height_changes: Vec<(usize, [u8; 4], [u8; 4])>,
    pub grass_changes: Vec<(usize, i8, i8)>,
    pub created_water_array: bool,
    pub net_cost: i32,
    pub valid: bool,
    pub affordable: bool,
}

/// Swimming Pool editing mode (`BuildTool::Water` — Pool Tool, `STR# 133[1]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PoolMode {
    /// Drag-dig swimming pool tiles (`+$75` per tile).
    Dig,
    /// Ctrl-demolish swimming pool tiles (`-$60`, 80% refund per tile).
    Demolish,
}

/// Computed preview / planned mutation for a Swimming Pool edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolPlan {
    pub mode: PoolMode,
    pub pool_changes: Vec<(usize, u8, u8)>,
    pub created_pool_array: bool,
    pub net_cost: i32,
    pub valid: bool,
    pub affordable: bool,
}

/// Complete Collection 1.91 roof pitch presets (`HOUS 0` float pitch &
/// `Res_CPanel.h` `3900..=3904`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoofPitch {
    /// Flat roof (`0.0f`, `kBldSbTlRoofFlat = 3904`).
    Flat,
    /// Shallow roof (`0.4f`, `kBldSbTlRoofShallow = 3900`).
    Shallow,
    /// Medium roof (`0.6f`, `kBldSbTlRoofMedium = 3901`).
    #[default]
    Medium,
    /// Steep roof (`1.0f`, `kBldSbTlRoofSteep = 3902`).
    Steep,
}

impl RoofPitch {
    /// All four native roof pitch presets in shallowest-to-steepest order.
    pub const ALL: [Self; 4] = [Self::Flat, Self::Shallow, Self::Medium, Self::Steep];

    /// Native `HOUS 0` IEEE-754 `f32` roof slope value.
    pub const fn as_f32(self) -> f32 {
        match self {
            Self::Flat => 0.0,
            Self::Shallow => 0.4,
            Self::Medium => 0.6,
            Self::Steep => 1.0,
        }
    }

    /// Maps an arbitrary `HOUS 0` `f32` pitch value to the closest preset.
    pub fn from_f32(value: f32) -> Self {
        if !value.is_finite() {
            return Self::Medium;
        }
        let mut best = Self::Medium;
        let mut best_dist = f32::INFINITY;
        for preset in Self::ALL {
            let d = (value - preset.as_f32()).abs();
            if d < best_dist {
                best_dist = d;
                best = preset;
            }
        }
        best
    }
}

/// Active lot roof texture stem and pitch preset (`HOUS 0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoofState {
    /// Texture stem inside `GameData/Roofs/` without `.bmp` (e.g. `"r1_"`).
    pub pattern: String,
    /// Roof slope preset (`Flat`, `Shallow`, `Medium`, `Steep`).
    pub pitch: RoofPitch,
}

impl Default for RoofState {
    fn default() -> Self {
        Self {
            pattern: DEFAULT_ROOF_PATTERN.to_owned(),
            pitch: RoofPitch::Medium,
        }
    }
}

impl RoofState {
    /// Decodes the native roof pitch (`f32` bits in `HouseView::unknown_word`)
    /// and texture stem (`HouseView::roof`) from a parsed `HOUS 0` chunk.
    pub fn from_house_view(view: &HouseView) -> Self {
        let pitch_f32 = f32::from_bits(view.unknown_word);
        let stem = normalize_roof_stem(&view.roof);
        Self {
            pattern: if stem.is_empty() {
                DEFAULT_ROOF_PATTERN.to_owned()
            } else {
                stem
            },
            pitch: RoofPitch::from_f32(pitch_f32),
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.pattern.len());
        out.extend_from_slice(&self.pitch.as_f32().to_le_bytes());
        out.extend_from_slice(self.pattern.as_bytes());
        out
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 4 {
            return None;
        }
        let pitch_f32 = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let pattern = std::str::from_utf8(&bytes[4..]).ok()?.to_owned();
        if pattern.trim().is_empty() {
            return None;
        }
        Some(Self {
            pattern,
            pitch: RoofPitch::from_f32(pitch_f32),
        })
    }
}

/// Computed planned mutation for changing roof pattern and/or pitch (`$0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoofPlan {
    pub before_raw: Option<Vec<u8>>,
    pub before: RoofState,
    pub after: RoofState,
    pub valid: bool,
}

fn normalize_roof_stem(raw: &str) -> String {
    let trimmed = raw.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".bmp") {
        trimmed[..trimmed.len() - 4].to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn is_locked_tile(world: &World, x: usize, y: usize) -> bool {
    let idx = cell_index(x, y);
    world
        .arrays
        .get(&FLAGS_ARRAY[0])
        .filter(|a| a.stride == 1)
        .and_then(|a| a.data.get(idx).copied())
        .is_some_and(|bits| (bits & FLAG_LOCKED) != 0)
}

/// Whether tile `(x, y)` is currently a swimming pool tile (`Arry 9 < 16`).
pub fn is_pool_tile(world: &World, x: usize, y: usize) -> bool {
    if x >= FULL_SIZE || y >= FULL_SIZE {
        return false;
    }
    let idx = cell_index(x, y);
    world
        .arrays
        .get(&POOL_ARRAY)
        .filter(|a| a.stride == 1)
        .and_then(|a| a.data.get(idx).copied())
        .is_some_and(|v| v < 16)
}

/// Whether tile `(x, y)` is currently a pond/river water tile (`Arry 10 < 16`).
pub fn is_water_tile(world: &World, x: usize, y: usize) -> bool {
    if x >= FULL_SIZE || y >= FULL_SIZE {
        return false;
    }
    let idx = cell_index(x, y);
    world
        .arrays
        .get(&WATER_ARRAY)
        .filter(|a| a.stride == 1)
        .and_then(|a| a.data.get(idx).copied())
        .is_some_and(|v| v < 16)
}

fn tile_has_any_wall(world: &World, x: usize, y: usize, level_idx: usize) -> bool {
    let active = usize::try_from(world.size).unwrap_or(0);
    let Some(walls) = world.arrays.get(&WALLS[level_idx]) else {
        return false;
    };
    if walls.stride != WALL_CELL || x >= active || y >= active {
        return false;
    }
    ExpandedWall::get(&walls.data, active, x, y).segments != 0
}

fn tile_has_any_floor(world: &World, x: usize, y: usize, level_idx: usize) -> bool {
    let Some(floors) = world.arrays.get(&FLOORS[level_idx]) else {
        return false;
    };
    if floors.stride != 2 {
        return false;
    }
    read_floor(&floors.data, x, y) != 0
}

fn tile_has_non_cursor_object(world: &World, x: usize, y: usize, level: Option<u8>) -> bool {
    let tx = x as i32;
    let ty = y as i32;
    for obj in world.objects.values() {
        if obj.guid == WALL_CURSOR_GUID || obj.guid == crate::buy::TILE_CURSOR_GUID {
            continue;
        }
        if !on_lot([obj.position[0], obj.position[1]]) {
            continue;
        }
        if let Some(lvl) = level
            && obj.position[2] != i32::from(lvl)
        {
            continue;
        }
        if (obj.position[0] >> 4) == tx && (obj.position[1] >> 4) == ty {
            return true;
        }
    }
    false
}

/// Checks whether a single ground tile `(x, y)` allows terrain elevation
/// changes (bounds, off-road, unlocked, no walls/floors/pool/water/objects).
fn tile_allows_terrain_elevation(world: &World, x: usize, y: usize, allow_water: bool) -> bool {
    if !is_buildable_tile(world, x, y, 1) || is_locked_tile(world, x, y) {
        return false;
    }
    if is_pool_tile(world, x, y) || (!allow_water && is_water_tile(world, x, y)) {
        return false;
    }
    if tile_has_any_wall(world, x, y, 0)
        || tile_has_any_wall(world, x, y, 1)
        || tile_has_any_floor(world, x, y, 0)
        || tile_has_any_floor(world, x, y, 1)
    {
        return false;
    }
    !tile_has_non_cursor_object(world, x, y, None)
}

/// Reads the elevation (`u8` terrain steps) of grid vertex `(vx, vy)` from
/// `Arry 0` (`heights`), where tile `(tx, ty)` stores corners
/// `[SW(0,1), NW(0,0), NE(1,0), SE(1,1)]`.
pub fn terrain_vertex_height(world: &World, vx: usize, vy: usize) -> Option<u8> {
    let heights = world.arrays.get(&HEIGHTS_ARRAY)?;
    if heights.stride != 4 {
        return None;
    }
    read_vertex_height(&heights.data, vx, vy)
}

fn read_vertex_height(heights: &[u8], vx: usize, vy: usize) -> Option<u8> {
    if vx < FULL_SIZE && vy < FULL_SIZE {
        return heights.get(cell_index(vx, vy) * 4 + 1).copied();
    }
    if vx == FULL_SIZE && vy < FULL_SIZE {
        return heights.get(cell_index(vx - 1, vy) * 4 + 2).copied();
    }
    if vx < FULL_SIZE && vy == FULL_SIZE {
        return heights.get(cell_index(vx, vy - 1) * 4).copied();
    }
    if vx == FULL_SIZE && vy == FULL_SIZE {
        return heights.get(cell_index(vx - 1, vy - 1) * 4 + 3).copied();
    }
    None
}

/// Writes `value` into every tile corner sharing vertex `(vx, vy)` in `heights`.
fn write_vertex_height(heights: &mut [u8], vx: usize, vy: usize, value: u8) {
    if vx > 0 && vy > 0 && vx <= FULL_SIZE && vy <= FULL_SIZE {
        heights[cell_index(vx - 1, vy - 1) * 4 + 3] = value;
    }
    if vx < FULL_SIZE && vy > 0 && vy <= FULL_SIZE {
        heights[cell_index(vx, vy - 1) * 4] = value;
    }
    if vx > 0 && vy < FULL_SIZE && vx <= FULL_SIZE {
        heights[cell_index(vx - 1, vy) * 4 + 2] = value;
    }
    if vx < FULL_SIZE && vy < FULL_SIZE {
        heights[cell_index(vx, vy) * 4 + 1] = value;
    }
}

/// Whether grid vertex `(vx, vy)` can be raised, lowered, or leveled (all four
/// incident tiles are editable buildable land without roads, locks, walls,
/// floors, pools, water, or objects).
pub fn is_terrain_vertex_editable(world: &World, vx: usize, vy: usize) -> bool {
    is_terrain_vertex_editable_inner(world, vx, vy, false)
}

fn is_terrain_vertex_editable_inner(
    world: &World,
    vx: usize,
    vy: usize,
    allow_water: bool,
) -> bool {
    let active = usize::try_from(world.size).unwrap_or(0);
    if active < 4 || vx < 2 || vy < 2 || vx + 1 >= active || vy + 1 >= active {
        return false;
    }
    for (tx, ty) in [(vx - 1, vy - 1), (vx, vy - 1), (vx - 1, vy), (vx, vy)] {
        if !tile_allows_terrain_elevation(world, tx, ty, allow_water) {
            return false;
        }
    }
    true
}

/// Whether tile `(x, y)` can be painted with Grass or stripped to soil.
pub fn is_grass_tile_editable(world: &World, x: usize, y: usize) -> bool {
    is_buildable_tile(world, x, y, 1)
        && !is_locked_tile(world, x, y)
        && !is_pool_or_water_tile(world, x, y)
        && !tile_has_any_floor(world, x, y, 0)
}

fn vertex_slope_ok(heights: &[u8], active: usize, vx: usize, vy: usize, new_h: u8) -> bool {
    let Some(cur_h) = read_vertex_height(heights, vx, vy) else {
        return false;
    };
    let neighbors = [
        (vx.checked_sub(1), Some(vy)),
        ((vx < active).then_some(vx + 1), Some(vy)),
        (Some(vx), vy.checked_sub(1)),
        (Some(vx), (vy < active).then_some(vy + 1)),
    ];
    for (nx, ny) in neighbors {
        let (Some(nx), Some(ny)) = (nx, ny) else {
            continue;
        };
        let Some(nh) = read_vertex_height(heights, nx, ny) else {
            continue;
        };
        let new_diff = new_h.abs_diff(nh);
        let old_diff = cur_h.abs_diff(nh);
        if new_diff > MAX_TERRAIN_SLOPE && new_diff > old_diff {
            return false;
        }
    }
    true
}

/// Plans a Terrain raise/lower/level or Grass restore/remove edit without
/// mutating `world`.
pub fn plan_terrain_edit(
    world: &World,
    start: [i32; 2],
    end: [i32; 2],
    mode: TerrainMode,
    prices: &ToolPrices,
) -> TerrainPlan {
    let active = usize::try_from(world.size).unwrap_or(0);
    let empty = TerrainPlan {
        mode,
        height_changes: Vec::new(),
        grass_changes: Vec::new(),
        net_cost: 0,
        valid: false,
        affordable: true,
    };
    if active < 4 {
        return empty;
    }

    match mode {
        TerrainMode::RestoreGrass | TerrainMode::RemoveGrass => {
            let Some(orig_grass) = world.arrays.get(&GRASS_ARRAY).filter(|a| a.stride == 1) else {
                return empty;
            };
            let target_grass = world
                .arrays
                .get(&TARGET_GRASS_ARRAY)
                .filter(|a| a.stride == 1);
            let min_x = start[0].min(end[0]).max(1);
            let max_x = start[0].max(end[0]).min(active as i32 - 2);
            let min_y = start[1].min(end[1]).max(1);
            let max_y = start[1].max(end[1]).min(active as i32 - 2);
            if min_x > max_x || min_y > max_y {
                return empty;
            }
            let funds = available_funds(world);
            let mut grass_changes = Vec::new();
            let mut net_cost = 0i32;
            let mut affordable = true;
            for x in min_x..=max_x {
                for y in min_y..=max_y {
                    let ux = x as usize;
                    let uy = y as usize;
                    if !is_grass_tile_editable(world, ux, uy) {
                        continue;
                    }
                    let idx = cell_index(ux, uy);
                    let cur = orig_grass.data[idx] as i8;
                    let next = match mode {
                        TerrainMode::RestoreGrass => {
                            let target = target_grass
                                .and_then(|t| t.data.get(idx).copied())
                                .map(|b| b as i8)
                                .unwrap_or(GRASS_RESTORED_VALUE);
                            let desired = GRASS_RESTORED_VALUE.max(target);
                            if cur >= desired {
                                continue;
                            }
                            desired
                        }
                        TerrainMode::RemoveGrass => {
                            if cur <= TERRAIN_DIRT_GRASS {
                                continue;
                            }
                            TERRAIN_DIRT_GRASS
                        }
                        _ => unreachable!(),
                    };
                    let step_cost = if mode == TerrainMode::RestoreGrass {
                        prices.grass.max(0)
                    } else {
                        0
                    };
                    if net_cost.saturating_add(step_cost) > funds {
                        affordable = false;
                        break;
                    }
                    net_cost = net_cost.saturating_add(step_cost);
                    grass_changes.push((idx, cur, next));
                }
                if !affordable {
                    break;
                }
            }
            let valid = !grass_changes.is_empty();
            TerrainPlan {
                mode,
                height_changes: Vec::new(),
                grass_changes,
                net_cost,
                valid,
                affordable: affordable && (net_cost <= 0 || net_cost <= funds),
            }
        }
        TerrainMode::Raise
        | TerrainMode::Lower
        | TerrainMode::Level
        | TerrainMode::LevelAverage => {
            let Some(orig_heights) = world.arrays.get(&HEIGHTS_ARRAY).filter(|a| a.stride == 4)
            else {
                return empty;
            };
            let min_x = start[0].min(end[0]).max(2);
            let max_x = start[0].max(end[0]).min(active as i32 - 2);
            let min_y = start[1].min(end[1]).max(2);
            let max_y = start[1].max(end[1]).min(active as i32 - 2);
            if min_x > max_x || min_y > max_y {
                return empty;
            }

            let mut heights_buf = orig_heights.data.clone();
            let mut editable = Vec::new();
            for vx in min_x..=max_x {
                for vy in min_y..=max_y {
                    let uvx = vx as usize;
                    let uvy = vy as usize;
                    if is_terrain_vertex_editable(world, uvx, uvy) {
                        editable.push((uvx, uvy));
                    }
                }
            }
            if editable.is_empty() {
                return empty;
            }

            let mut changed_vertices = 0i32;
            match mode {
                TerrainMode::Raise | TerrainMode::Lower => {
                    for &(vx, vy) in &editable {
                        let Some(cur) = read_vertex_height(&heights_buf, vx, vy) else {
                            continue;
                        };
                        let next = if mode == TerrainMode::Raise {
                            cur.saturating_add(1)
                        } else {
                            cur.saturating_sub(1)
                        };
                        if next == cur || !vertex_slope_ok(&heights_buf, active, vx, vy, next) {
                            continue;
                        }
                        write_vertex_height(&mut heights_buf, vx, vy, next);
                        changed_vertices += 1;
                    }
                }
                TerrainMode::Level | TerrainMode::LevelAverage => {
                    let target_h = if mode == TerrainMode::Level {
                        let ax = start[0].clamp(0, active as i32) as usize;
                        let ay = start[1].clamp(0, active as i32) as usize;
                        let Some(h) = read_vertex_height(&orig_heights.data, ax, ay) else {
                            return empty;
                        };
                        h
                    } else {
                        let sum: u64 = editable
                            .iter()
                            .filter_map(|&(vx, vy)| {
                                read_vertex_height(&orig_heights.data, vx, vy).map(u64::from)
                            })
                            .sum();
                        let count = editable.len() as u64;
                        ((sum + count / 2) / count) as u8
                    };
                    let editable_set: BTreeSet<(usize, usize)> = editable.iter().copied().collect();
                    let mut lo: BTreeMap<(usize, usize), i32> = BTreeMap::new();
                    let mut hi: BTreeMap<(usize, usize), i32> = BTreeMap::new();
                    for &v in &editable {
                        lo.insert(v, 0);
                        hi.insert(v, i32::from(u8::MAX));
                    }
                    // Seed bounds from fixed (non-editable) neighbors outside the drag rectangle.
                    let slope = i32::from(MAX_TERRAIN_SLOPE);
                    for &(vx, vy) in &editable {
                        let cur = i32::from(
                            read_vertex_height(&orig_heights.data, vx, vy).unwrap_or(target_h),
                        );
                        for (nx, ny) in [
                            (vx.checked_sub(1), Some(vy)),
                            ((vx < active).then_some(vx + 1), Some(vy)),
                            (Some(vx), vy.checked_sub(1)),
                            (Some(vx), (vy < active).then_some(vy + 1)),
                        ] {
                            let (Some(nx), Some(ny)) = (nx, ny) else {
                                continue;
                            };
                            if editable_set.contains(&(nx, ny)) {
                                continue;
                            }
                            if let Some(nh) = read_vertex_height(&orig_heights.data, nx, ny) {
                                let nh = i32::from(nh);
                                let allowed = slope.max((cur - nh).abs());
                                *lo.get_mut(&(vx, vy)).unwrap() = lo[&(vx, vy)].max(nh - allowed);
                                *hi.get_mut(&(vx, vy)).unwrap() = hi[&(vx, vy)].min(nh + allowed);
                            }
                        }
                    }
                    // Relax feasible intervals across internal edges of the editable set,
                    // using the same pre-existing slope allowance as fixed boundary edges
                    // so `cur` always remains feasible even across pre-existing cliffs.
                    let mut changed = true;
                    let mut guard = editable.len().saturating_add(1);
                    while changed && guard > 0 {
                        changed = false;
                        guard -= 1;
                        for &(vx, vy) in &editable {
                            let cur_v = i32::from(
                                read_vertex_height(&orig_heights.data, vx, vy).unwrap_or(target_h),
                            );
                            let mut cur_lo = lo[&(vx, vy)];
                            let mut cur_hi = hi[&(vx, vy)];
                            for (nx, ny) in [
                                (vx.checked_sub(1), Some(vy)),
                                ((vx < active).then_some(vx + 1), Some(vy)),
                                (Some(vx), vy.checked_sub(1)),
                                (Some(vx), (vy < active).then_some(vy + 1)),
                            ] {
                                let (Some(nx), Some(ny)) = (nx, ny) else {
                                    continue;
                                };
                                if !editable_set.contains(&(nx, ny)) {
                                    continue;
                                }
                                let cur_n = i32::from(
                                    read_vertex_height(&orig_heights.data, nx, ny)
                                        .unwrap_or(target_h),
                                );
                                let edge_limit = slope.max((cur_v - cur_n).abs());
                                let nlo = lo[&(nx, ny)] - edge_limit;
                                let nhi = hi[&(nx, ny)] + edge_limit;
                                if nlo > cur_lo {
                                    cur_lo = nlo;
                                    lo.insert((vx, vy), nlo);
                                    changed = true;
                                }
                                if nhi < cur_hi {
                                    cur_hi = nhi;
                                    hi.insert((vx, vy), nhi);
                                    changed = true;
                                }
                            }
                        }
                    }
                    for &(vx, vy) in &editable {
                        let Some(cur) = read_vertex_height(&orig_heights.data, vx, vy) else {
                            continue;
                        };
                        let v_lo = lo[&(vx, vy)];
                        let v_hi = hi[&(vx, vy)];
                        let next = if v_lo <= v_hi {
                            i32::from(target_h).clamp(v_lo, v_hi).clamp(0, 255) as u8
                        } else {
                            cur
                        };
                        if next != cur {
                            write_vertex_height(&mut heights_buf, vx, vy, next);
                            changed_vertices += 1;
                        }
                    }
                    // Final safety validation across every edge incident to an edited vertex:
                    // no edge may exceed MAX_TERRAIN_SLOPE unless it did not steepen beyond its
                    // original pre-existing difference.
                    let mut slope_valid = true;
                    'validate_edges: for &(vx, vy) in &editable {
                        let Some(orig_v) = read_vertex_height(&orig_heights.data, vx, vy) else {
                            continue;
                        };
                        let Some(new_v) = read_vertex_height(&heights_buf, vx, vy) else {
                            continue;
                        };
                        for (nx, ny) in [
                            (vx.checked_sub(1), Some(vy)),
                            ((vx < active).then_some(vx + 1), Some(vy)),
                            (Some(vx), vy.checked_sub(1)),
                            (Some(vx), (vy < active).then_some(vy + 1)),
                        ] {
                            let (Some(nx), Some(ny)) = (nx, ny) else {
                                continue;
                            };
                            let (Some(orig_n), Some(new_n)) = (
                                read_vertex_height(&orig_heights.data, nx, ny),
                                read_vertex_height(&heights_buf, nx, ny),
                            ) else {
                                continue;
                            };
                            let old_diff = orig_v.abs_diff(orig_n);
                            let new_diff = new_v.abs_diff(new_n);
                            if new_diff > MAX_TERRAIN_SLOPE && new_diff > old_diff {
                                slope_valid = false;
                                break 'validate_edges;
                            }
                        }
                    }
                    if !slope_valid {
                        return empty;
                    }
                }
                TerrainMode::RestoreGrass | TerrainMode::RemoveGrass => unreachable!(),
            }

            let mut height_changes = Vec::new();
            let mut disturbed_tiles = Vec::new();
            for idx in 0..(FULL_SIZE * FULL_SIZE) {
                let off = idx * 4;
                let before = [
                    orig_heights.data[off],
                    orig_heights.data[off + 1],
                    orig_heights.data[off + 2],
                    orig_heights.data[off + 3],
                ];
                let after = [
                    heights_buf[off],
                    heights_buf[off + 1],
                    heights_buf[off + 2],
                    heights_buf[off + 3],
                ];
                if before != after {
                    height_changes.push((idx, before, after));
                    disturbed_tiles.push(idx);
                }
            }
            let mut grass_changes = Vec::new();
            if let Some(orig_grass) = world.arrays.get(&GRASS_ARRAY).filter(|a| a.stride == 1) {
                for idx in disturbed_tiles {
                    let cur = orig_grass.data[idx] as i8;
                    if cur > TERRAIN_DIRT_GRASS {
                        grass_changes.push((idx, cur, TERRAIN_DIRT_GRASS));
                    }
                }
            }

            let unit_price = match mode {
                TerrainMode::Raise => prices.terrain_raise.max(0),
                TerrainMode::Lower => prices.terrain_lower.max(0),
                TerrainMode::Level | TerrainMode::LevelAverage => prices.terrain_level.max(0),
                TerrainMode::RestoreGrass | TerrainMode::RemoveGrass => 0,
            };
            let net_cost = changed_vertices.saturating_mul(unit_price);
            let affordable = net_cost <= 0 || net_cost <= available_funds(world);
            TerrainPlan {
                mode,
                valid: !height_changes.is_empty(),
                height_changes,
                grass_changes,
                net_cost,
                affordable,
            }
        }
    }
}

/// Recomputes the native 4-bit cardinal connectivity bitmask (`0x00..=0x0f`:
/// `N=1, W=2, S=4, E=8`) only for `edited_tiles` and their in-bounds cardinal
/// neighbors, preserving untouched distant cells (including `0x10` sentinels).
fn recompute_water_pool_masks_around(
    buf: &mut [u8],
    active: usize,
    edited_tiles: &[(usize, usize)],
) {
    let limit = active.min(FULL_SIZE);
    let mut dirty = BTreeSet::new();
    for &(x, y) in edited_tiles {
        if x < limit && y < limit {
            dirty.insert((x, y));
            if y > 0 {
                dirty.insert((x, y - 1));
            }
            if x > 0 {
                dirty.insert((x - 1, y));
            }
            if y + 1 < limit {
                dirty.insert((x, y + 1));
            }
            if x + 1 < limit {
                dirty.insert((x + 1, y));
            }
        }
    }
    let snapshot: Vec<bool> = buf.iter().map(|&v| v < 16).collect();
    for (x, y) in dirty {
        let idx = cell_index(x, y);
        if !snapshot[idx] {
            continue;
        }
        let mut mask = 0u8;
        if y > 0 && snapshot[cell_index(x, y - 1)] {
            mask |= 0x01; // North
        }
        if x > 0 && snapshot[cell_index(x - 1, y)] {
            mask |= 0x02; // West
        }
        if y + 1 < limit && snapshot[cell_index(x, y + 1)] {
            mask |= 0x04; // South
        }
        if x + 1 < limit && snapshot[cell_index(x + 1, y)] {
            mask |= 0x08; // East
        }
        buf[idx] = mask;
    }
}

fn vertex_borders_water(water_buf: &[u8], active: usize, vx: usize, vy: usize) -> bool {
    let limit = active.min(FULL_SIZE);
    let candidates = [
        (vx.checked_sub(1), vy.checked_sub(1)),
        ((vx < limit).then_some(vx), vy.checked_sub(1)),
        (vx.checked_sub(1), (vy < limit).then_some(vy)),
        ((vx < limit).then_some(vx), (vy < limit).then_some(vy)),
    ];
    candidates.into_iter().any(|(tx, ty)| {
        let (Some(tx), Some(ty)) = (tx, ty) else {
            return false;
        };
        tx < limit && ty < limit && water_buf[cell_index(tx, ty)] < 16
    })
}

fn fill_removed_pond_basin(
    world: &World,
    orig_heights: &[u8],
    heights_buf: &mut [u8],
    water_buf: &[u8],
    active: usize,
    candidate_vertices: &BTreeSet<(usize, usize)>,
) {
    let fillable: BTreeSet<(usize, usize)> = candidate_vertices
        .iter()
        .copied()
        .filter(|&(vx, vy)| {
            is_terrain_vertex_editable_inner(world, vx, vy, true)
                && !vertex_borders_water(water_buf, active, vx, vy)
        })
        .collect();
    if fillable.is_empty() {
        return;
    }
    let mut visited = BTreeSet::new();
    let slope = i32::from(MAX_TERRAIN_SLOPE);
    for &seed in &fillable {
        if visited.contains(&seed) {
            continue;
        }
        let mut comp = Vec::new();
        let mut queue = vec![seed];
        visited.insert(seed);
        while let Some((vx, vy)) = queue.pop() {
            comp.push((vx, vy));
            for (nx, ny) in [
                (vx.checked_sub(1), Some(vy)),
                ((vx < active).then_some(vx + 1), Some(vy)),
                (Some(vx), vy.checked_sub(1)),
                (Some(vx), (vy < active).then_some(vy + 1)),
            ] {
                let (Some(nx), Some(ny)) = (nx, ny) else {
                    continue;
                };
                if fillable.contains(&(nx, ny)) && visited.insert((nx, ny)) {
                    queue.push((nx, ny));
                }
            }
        }
        let comp_set: BTreeSet<(usize, usize)> = comp.iter().copied().collect();
        // Collect surrounding dry bank perimeter vertices (not in this fillable
        // component and not bordering surviving water).
        let mut bank_vertices = BTreeSet::new();
        for &(vx, vy) in &comp {
            for (nx, ny) in [
                (vx.checked_sub(1), Some(vy)),
                ((vx < active).then_some(vx + 1), Some(vy)),
                (Some(vx), vy.checked_sub(1)),
                (Some(vx), (vy < active).then_some(vy + 1)),
            ] {
                let (Some(nx), Some(ny)) = (nx, ny) else {
                    continue;
                };
                if !comp_set.contains(&(nx, ny))
                    && !vertex_borders_water(water_buf, active, nx, ny)
                    && read_vertex_height(orig_heights, nx, ny).is_some()
                {
                    bank_vertices.insert((nx, ny));
                }
            }
        }
        if bank_vertices.is_empty() {
            continue;
        }
        let bank_sum: u32 = bank_vertices
            .iter()
            .filter_map(|&(nx, ny)| read_vertex_height(orig_heights, nx, ny).map(u32::from))
            .sum();
        let bank_count = bank_vertices.len() as u32;
        let Some(bank_target) = (bank_sum + bank_count / 2).checked_div(bank_count) else {
            continue;
        };
        let bank_target = bank_target as u8;

        // Compute slope-feasible bounds [lo, hi] against all pinned neighbors
        // (including protected vertices and vertices bordering surviving water).
        let mut lo: BTreeMap<(usize, usize), i32> = BTreeMap::new();
        let mut hi: BTreeMap<(usize, usize), i32> = BTreeMap::new();
        for &v in &comp {
            lo.insert(v, 0);
            hi.insert(v, i32::from(u8::MAX));
        }
        for &(vx, vy) in &comp {
            let cur = i32::from(read_vertex_height(orig_heights, vx, vy).unwrap_or(bank_target));
            for (nx, ny) in [
                (vx.checked_sub(1), Some(vy)),
                ((vx < active).then_some(vx + 1), Some(vy)),
                (Some(vx), vy.checked_sub(1)),
                (Some(vx), (vy < active).then_some(vy + 1)),
            ] {
                let (Some(nx), Some(ny)) = (nx, ny) else {
                    continue;
                };
                if comp_set.contains(&(nx, ny)) {
                    continue;
                }
                if let Some(nh) = read_vertex_height(orig_heights, nx, ny) {
                    let nh = i32::from(nh);
                    let allowed = slope.max((cur - nh).abs());
                    *lo.get_mut(&(vx, vy)).unwrap() = lo[&(vx, vy)].max(nh - allowed);
                    *hi.get_mut(&(vx, vy)).unwrap() = hi[&(vx, vy)].min(nh + allowed);
                }
            }
        }
        let mut changed = true;
        let mut guard = comp.len().saturating_add(1);
        while changed && guard > 0 {
            changed = false;
            guard -= 1;
            for &(vx, vy) in &comp {
                let cur_v =
                    i32::from(read_vertex_height(orig_heights, vx, vy).unwrap_or(bank_target));
                let mut cur_lo = lo[&(vx, vy)];
                let mut cur_hi = hi[&(vx, vy)];
                for (nx, ny) in [
                    (vx.checked_sub(1), Some(vy)),
                    ((vx < active).then_some(vx + 1), Some(vy)),
                    (Some(vx), vy.checked_sub(1)),
                    (Some(vx), (vy < active).then_some(vy + 1)),
                ] {
                    let (Some(nx), Some(ny)) = (nx, ny) else {
                        continue;
                    };
                    if !comp_set.contains(&(nx, ny)) {
                        continue;
                    }
                    let cur_n =
                        i32::from(read_vertex_height(orig_heights, nx, ny).unwrap_or(bank_target));
                    let edge_limit = slope.max((cur_v - cur_n).abs());
                    let nlo = lo[&(nx, ny)] - edge_limit;
                    let nhi = hi[&(nx, ny)] + edge_limit;
                    if nlo > cur_lo {
                        cur_lo = nlo;
                        lo.insert((vx, vy), nlo);
                        changed = true;
                    }
                    if nhi < cur_hi {
                        cur_hi = nhi;
                        hi.insert((vx, vy), nhi);
                        changed = true;
                    }
                }
            }
        }
        for &(vx, vy) in &comp {
            let Some(cur) = read_vertex_height(orig_heights, vx, vy) else {
                continue;
            };
            if bank_target <= cur {
                continue;
            }
            let desired = cur.saturating_add(1).min(bank_target);
            let v_lo = lo[&(vx, vy)];
            let v_hi = hi[&(vx, vy)];
            let next = if v_lo <= v_hi {
                i32::from(desired).clamp(v_lo, v_hi).clamp(0, 255) as u8
            } else {
                cur
            };
            if next != cur {
                write_vertex_height(heights_buf, vx, vy, next);
            }
        }
    }
}

fn tile_supports_water_paint(world: &World, x: usize, y: usize) -> bool {
    is_buildable_tile(world, x, y, 1)
        && !is_locked_tile(world, x, y)
        && !is_pool_tile(world, x, y)
        && !tile_has_any_wall(world, x, y, 0)
        && !tile_has_any_wall(world, x, y, 1)
        && !tile_has_any_floor(world, x, y, 0)
        && !tile_has_non_cursor_object(world, x, y, Some(1))
}

fn tile_supports_water_remove(world: &World, x: usize, y: usize) -> bool {
    is_buildable_tile(world, x, y, 1)
        && !is_locked_tile(world, x, y)
        && !tile_has_non_cursor_object(world, x, y, Some(1))
}

/// Plans painting or Ctrl-removing pond water tiles (`Arry 10`) with native
/// Complete Collection 1.91 connectivity masks and basin terrain depression.
pub fn plan_water_edit(
    world: &World,
    start_tile: [i32; 2],
    end_tile: [i32; 2],
    mode: WaterMode,
    prices: &ToolPrices,
) -> WaterPlan {
    let active = usize::try_from(world.size).unwrap_or(0);
    let empty = WaterPlan {
        mode,
        water_changes: Vec::new(),
        height_changes: Vec::new(),
        grass_changes: Vec::new(),
        created_water_array: false,
        net_cost: 0,
        valid: false,
        affordable: true,
    };
    if active < 4 {
        return empty;
    }
    let max_valid = active as i32 - 2;
    let min_x = start_tile[0].min(end_tile[0]);
    let max_x = start_tile[0].max(end_tile[0]);
    let min_y = start_tile[1].min(end_tile[1]);
    let max_y = start_tile[1].max(end_tile[1]);
    if min_x < 1
        || min_y < 1
        || max_x > max_valid
        || max_y > max_valid
        || min_x > max_x
        || min_y > max_y
    {
        return empty;
    }

    let created_water_array = !world.arrays.contains_key(&WATER_ARRAY);
    let orig_water: Vec<u8> = match world.arrays.get(&WATER_ARRAY) {
        Some(a) if a.stride == 1 && a.data.len() == FULL_SIZE * FULL_SIZE => a.data.clone(),
        Some(_) => return empty,
        None => vec![0xff; FULL_SIZE * FULL_SIZE],
    };
    let mut water_buf = orig_water.clone();
    let mut changed_tiles = Vec::new();

    for x in min_x..=max_x {
        for y in min_y..=max_y {
            let ux = x as usize;
            let uy = y as usize;
            let idx = cell_index(ux, uy);
            let is_wet = orig_water[idx] < 16;
            match mode {
                WaterMode::Paint => {
                    if is_wet {
                        continue;
                    }
                    if !tile_supports_water_paint(world, ux, uy) {
                        return empty;
                    }
                    water_buf[idx] = 0;
                    changed_tiles.push((ux, uy));
                }
                WaterMode::Remove => {
                    if !is_wet {
                        continue;
                    }
                    if !tile_supports_water_remove(world, ux, uy) {
                        return empty;
                    }
                    water_buf[idx] = 0xff;
                    changed_tiles.push((ux, uy));
                }
            }
        }
    }

    if changed_tiles.is_empty() {
        return empty;
    }

    recompute_water_pool_masks_around(&mut water_buf, active, &changed_tiles);

    let mut water_changes = Vec::new();
    for idx in 0..(FULL_SIZE * FULL_SIZE) {
        if orig_water[idx] != water_buf[idx] {
            water_changes.push((idx, orig_water[idx], water_buf[idx]));
        }
    }

    // Depress mutable basin vertices on Paint, or fill removed basin from the
    // surrounding dry bank on Remove (while pinning protected & wet-bordering
    // vertices).
    let mut height_changes = Vec::new();
    let mut grass_changes = Vec::new();
    if let Some(orig_heights) = world.arrays.get(&HEIGHTS_ARRAY).filter(|a| a.stride == 4) {
        let mut heights_buf = orig_heights.data.clone();
        let mut candidate_vertices = BTreeSet::new();
        for &(tx, ty) in &changed_tiles {
            for (vx, vy) in [(tx, ty), (tx + 1, ty), (tx, ty + 1), (tx + 1, ty + 1)] {
                candidate_vertices.insert((vx, vy));
            }
        }
        match mode {
            WaterMode::Paint => {
                for (vx, vy) in candidate_vertices {
                    if !is_terrain_vertex_editable_inner(world, vx, vy, true) {
                        continue;
                    }
                    let Some(cur) = read_vertex_height(&heights_buf, vx, vy) else {
                        continue;
                    };
                    let next = cur.saturating_sub(1);
                    if next != cur && vertex_slope_ok(&heights_buf, active, vx, vy, next) {
                        write_vertex_height(&mut heights_buf, vx, vy, next);
                    }
                }
            }
            WaterMode::Remove => {
                fill_removed_pond_basin(
                    world,
                    &orig_heights.data,
                    &mut heights_buf,
                    &water_buf,
                    active,
                    &candidate_vertices,
                );
            }
        }
        let mut disturbed = Vec::new();
        for idx in 0..(FULL_SIZE * FULL_SIZE) {
            let off = idx * 4;
            let b = [
                orig_heights.data[off],
                orig_heights.data[off + 1],
                orig_heights.data[off + 2],
                orig_heights.data[off + 3],
            ];
            let a = [
                heights_buf[off],
                heights_buf[off + 1],
                heights_buf[off + 2],
                heights_buf[off + 3],
            ];
            if b != a {
                height_changes.push((idx, b, a));
                disturbed.push(idx);
            }
        }
        if let Some(orig_grass) = world.arrays.get(&GRASS_ARRAY).filter(|a| a.stride == 1) {
            for idx in disturbed {
                let cur = orig_grass.data[idx] as i8;
                if cur > TERRAIN_DIRT_GRASS {
                    grass_changes.push((idx, cur, TERRAIN_DIRT_GRASS));
                }
            }
        }
    }

    let count = changed_tiles.len() as i32;
    let net_cost = match mode {
        WaterMode::Paint => count.saturating_mul(prices.water.max(0)),
        WaterMode::Remove => -count.saturating_mul(build_refund(prices.water)),
    };
    let affordable = net_cost <= 0 || net_cost <= available_funds(world);
    WaterPlan {
        mode,
        water_changes,
        height_changes,
        grass_changes,
        created_water_array,
        net_cost,
        valid: true,
        affordable,
    }
}

fn tile_flat_elevation(world: &World, x: usize, y: usize) -> Option<u8> {
    let heights = world.arrays.get(&HEIGHTS_ARRAY)?;
    if heights.stride != 4 {
        return None;
    }
    let off = cell_index(x, y) * 4;
    let c = heights.data.get(off..off + 4)?;
    (c[0] == c[1] && c[1] == c[2] && c[2] == c[3]).then_some(c[0])
}

fn tile_supports_pool_dig(world: &World, x: usize, y: usize) -> bool {
    is_buildable_tile(world, x, y, 1)
        && !is_locked_tile(world, x, y)
        && is_flat_tile(world, x, y)
        && !is_water_tile(world, x, y)
        && !tile_has_any_wall(world, x, y, 0)
        && !tile_has_any_wall(world, x, y, 1)
        && !tile_has_any_floor(world, x, y, 0)
        && !tile_has_any_floor(world, x, y, 1)
        && !tile_has_non_cursor_object(world, x, y, Some(1))
}

fn tile_supports_pool_demolish(world: &World, x: usize, y: usize) -> bool {
    is_buildable_tile(world, x, y, 1)
        && !is_locked_tile(world, x, y)
        && !tile_has_non_cursor_object(world, x, y, Some(1))
}

/// Plans swimming pool drag digging or Ctrl demolition (`Arry 9`, flat-ground
/// & no-wall/no-floor/no-object guards, exact `$75` pricing and 80% `$60` refund).
pub fn plan_pool_edit(
    world: &World,
    start_tile: [i32; 2],
    end_tile: [i32; 2],
    mode: PoolMode,
    prices: &ToolPrices,
) -> PoolPlan {
    let active = usize::try_from(world.size).unwrap_or(0);
    let empty = PoolPlan {
        mode,
        pool_changes: Vec::new(),
        created_pool_array: false,
        net_cost: 0,
        valid: false,
        affordable: true,
    };
    if active < 4 {
        return empty;
    }
    let max_valid = active as i32 - 2;
    let min_x = start_tile[0].min(end_tile[0]);
    let max_x = start_tile[0].max(end_tile[0]);
    let min_y = start_tile[1].min(end_tile[1]);
    let max_y = start_tile[1].max(end_tile[1]);
    if min_x < 1
        || min_y < 1
        || max_x > max_valid
        || max_y > max_valid
        || min_x > max_x
        || min_y > max_y
    {
        return empty;
    }

    let created_pool_array = !world.arrays.contains_key(&POOL_ARRAY);
    let orig_pool: Vec<u8> = match world.arrays.get(&POOL_ARRAY) {
        Some(a) if a.stride == 1 && a.data.len() == FULL_SIZE * FULL_SIZE => a.data.clone(),
        Some(_) => return empty,
        None => vec![0xff; FULL_SIZE * FULL_SIZE],
    };
    let mut pool_buf = orig_pool.clone();
    let mut changed_tiles = Vec::new();
    let mut pool_elev: Option<u8> = None;

    for x in min_x..=max_x {
        for y in min_y..=max_y {
            let ux = x as usize;
            let uy = y as usize;
            let idx = cell_index(ux, uy);
            let is_pool = orig_pool[idx] < 16;
            match mode {
                PoolMode::Dig => {
                    let Some(elev) = tile_flat_elevation(world, ux, uy) else {
                        return empty;
                    };
                    if let Some(expected) = pool_elev {
                        if elev != expected {
                            return empty;
                        }
                    } else {
                        pool_elev = Some(elev);
                    }
                    if is_pool {
                        continue;
                    }
                    if !tile_supports_pool_dig(world, ux, uy) {
                        return empty;
                    }
                    // Adjacent existing pool tiles must share the same flat elevation.
                    for (nx, ny) in [
                        (ux.checked_sub(1), Some(uy)),
                        ((ux + 1 < active).then_some(ux + 1), Some(uy)),
                        (Some(ux), uy.checked_sub(1)),
                        (Some(ux), (uy + 1 < active).then_some(uy + 1)),
                    ] {
                        let (Some(nx), Some(ny)) = (nx, ny) else {
                            continue;
                        };
                        if orig_pool[cell_index(nx, ny)] < 16
                            && tile_flat_elevation(world, nx, ny) != Some(elev)
                        {
                            return empty;
                        }
                    }
                    pool_buf[idx] = 0;
                    changed_tiles.push((ux, uy));
                }
                PoolMode::Demolish => {
                    if !is_pool {
                        continue;
                    }
                    if !tile_supports_pool_demolish(world, ux, uy) {
                        return empty;
                    }
                    pool_buf[idx] = 0xff;
                    changed_tiles.push((ux, uy));
                }
            }
        }
    }

    if changed_tiles.is_empty() {
        return empty;
    }

    recompute_water_pool_masks_around(&mut pool_buf, active, &changed_tiles);

    let mut pool_changes = Vec::new();
    for idx in 0..(FULL_SIZE * FULL_SIZE) {
        if orig_pool[idx] != pool_buf[idx] {
            pool_changes.push((idx, orig_pool[idx], pool_buf[idx]));
        }
    }

    let changed_count = changed_tiles.len() as i32;

    let net_cost = match mode {
        PoolMode::Dig => changed_count.saturating_mul(prices.pool.max(0)),
        PoolMode::Demolish => -changed_count.saturating_mul(build_refund(prices.pool)),
    };
    let affordable = net_cost <= 0 || net_cost <= available_funds(world);
    PoolPlan {
        mode,
        pool_changes,
        created_pool_array,
        net_cost,
        valid: true,
        affordable,
    }
}

/// Reads the lot's active [`RoofState`] from `world`, falling back to `base`
/// (parsed from the house file's `HOUS 0` via [`RoofState::from_house_view`])
/// when no Build Mode roof edit has overridden it yet.
pub fn roof_state_with_base(world: &World, base: &RoofState) -> RoofState {
    world
        .arrays
        .get(&ROOF_META_ARRAY)
        .and_then(|arr| RoofState::decode(&arr.data))
        .unwrap_or_else(|| base.clone())
}

/// Reads the lot's active [`RoofState`] from `world`, falling back to
/// [`RoofState::default`] (`r1_`, `Medium`) when never edited.
pub fn roof_state(world: &World) -> RoofState {
    roof_state_with_base(world, &RoofState::default())
}

/// Plans changing the lot's roof texture pattern and/or pitch (`$0`), using
/// `base` (parsed from the house file's `HOUS 0`) as the untouched fallback so
/// partial edits preserve the untouched field and selecting `"r1_"` on a
/// non-default house is not treated as a no-op.
pub fn plan_roof_edit_with_base(
    world: &World,
    base: &RoofState,
    pattern: Option<&str>,
    pitch: Option<RoofPitch>,
) -> RoofPlan {
    let before_raw = world
        .arrays
        .get(&ROOF_META_ARRAY)
        .map(|arr| arr.data.clone());
    let before = roof_state_with_base(world, base);
    let next_pattern = match pattern {
        Some(raw) => {
            let norm = normalize_roof_stem(raw);
            if norm.is_empty() {
                return RoofPlan {
                    before_raw,
                    after: before.clone(),
                    before,
                    valid: false,
                };
            }
            norm
        }
        None => before.pattern.clone(),
    };
    let next_pitch = pitch.unwrap_or(before.pitch);
    let after = RoofState {
        pattern: next_pattern,
        pitch: next_pitch,
    };
    let valid = after != before;
    RoofPlan {
        before_raw,
        before,
        after,
        valid,
    }
}

/// Plans changing the lot's roof texture pattern and/or pitch (`$0`) against
/// [`RoofState::default`]. Prefer [`plan_roof_edit_with_base`] when a house's
/// `HOUS 0` roof baseline is known.
pub fn plan_roof_edit(world: &World, pattern: Option<&str>, pitch: Option<RoofPitch>) -> RoofPlan {
    plan_roof_edit_with_base(world, &RoofState::default(), pattern, pitch)
}

/// Plans switching the lot's roof pattern stem against a house `HOUS 0` baseline.
pub fn plan_roof_pattern_with_base(world: &World, base: &RoofState, pattern: &str) -> RoofPlan {
    plan_roof_edit_with_base(world, base, Some(pattern), None)
}

/// Plans switching the lot's roof pitch preset against a house `HOUS 0` baseline.
pub fn plan_roof_pitch_with_base(world: &World, base: &RoofState, pitch: RoofPitch) -> RoofPlan {
    plan_roof_edit_with_base(world, base, None, Some(pitch))
}

/// Plans switching the lot's roof pattern stem (e.g. `"r2_"` or `"r2_.bmp"`).
pub fn plan_roof_pattern(world: &World, pattern: &str) -> RoofPlan {
    plan_roof_edit(world, Some(pattern), None)
}

/// Plans switching the lot's roof pitch preset.
pub fn plan_roof_pitch(world: &World, pitch: RoofPitch) -> RoofPlan {
    plan_roof_edit(world, None, Some(pitch))
}

fn grass_regrowth_compatible(world: &World, idx: usize, expected: i8, cur: i8) -> bool {
    if cur == expected {
        return true;
    }
    // Natural `World::grow_grass` (0x47b3c8) monotonically raises Arry 6 toward
    // Arry 7 (`TARGET_GRASS_ARRAY`) when `g < target`. Accept any intermediate
    // regrowth value in `(expected, target]` so lawn regrowth never strands
    // Undo/Redo while out-of-band manual edits outside that interval still fail.
    if let Some(target_arr) = world
        .arrays
        .get(&TARGET_GRASS_ARRAY)
        .filter(|a| a.stride == 1)
        && let Some(&target_byte) = target_arr.data.get(idx)
    {
        let target = target_byte as i8;
        if expected < target && cur > expected && cur <= target {
            return true;
        }
    }
    false
}

fn terrain_diffs_match_inner(
    world: &World,
    height_changes: &[(usize, [u8; 4], [u8; 4])],
    grass_changes: &[(usize, i8, i8)],
    forward: bool,
    allow_grass_regrowth: bool,
) -> bool {
    if !height_changes.is_empty() {
        let Some(heights) = world.arrays.get(&HEIGHTS_ARRAY).filter(|a| a.stride == 4) else {
            return false;
        };
        for &(idx, before, after) in height_changes {
            let off = idx.saturating_mul(4);
            let expected = if forward { &before } else { &after };
            if heights.data.get(off..off + 4) != Some(expected.as_slice()) {
                return false;
            }
        }
    }
    if !grass_changes.is_empty() {
        let Some(grass) = world.arrays.get(&GRASS_ARRAY).filter(|a| a.stride == 1) else {
            return false;
        };
        for &(idx, before, after) in grass_changes {
            let expected = if forward { before } else { after };
            let Some(cur_byte) = grass.data.get(idx).copied() else {
                return false;
            };
            let cur = cur_byte as i8;
            if allow_grass_regrowth {
                if !grass_regrowth_compatible(world, idx, expected, cur) {
                    return false;
                }
            } else if cur != expected {
                return false;
            }
        }
    }
    true
}

fn terrain_diffs_match(
    world: &World,
    height_changes: &[(usize, [u8; 4], [u8; 4])],
    grass_changes: &[(usize, i8, i8)],
    forward: bool,
) -> bool {
    terrain_diffs_match_inner(world, height_changes, grass_changes, forward, false)
}

fn terrain_diffs_match_for_history(
    world: &World,
    height_changes: &[(usize, [u8; 4], [u8; 4])],
    grass_changes: &[(usize, i8, i8)],
    forward: bool,
) -> bool {
    terrain_diffs_match_inner(world, height_changes, grass_changes, forward, true)
}

fn touched_height_vertices(
    height_changes: &[(usize, [u8; 4], [u8; 4])],
) -> BTreeSet<(usize, usize)> {
    let mut touched = BTreeSet::new();
    for &(idx, before, after) in height_changes {
        let tx = idx / FULL_SIZE;
        let ty = idx % FULL_SIZE;
        // Corners: 0=(tx, ty+1), 1=(tx, ty), 2=(tx+1, ty), 3=(tx+1, ty+1)
        let corners = [(tx, ty + 1), (tx, ty), (tx + 1, ty), (tx + 1, ty + 1)];
        for (ci, v) in corners.into_iter().enumerate() {
            if before[ci] != after[ci] {
                touched.insert(v);
            }
        }
    }
    touched
}

fn terrain_changes_allowed(
    world: &World,
    height_changes: &[(usize, [u8; 4], [u8; 4])],
    grass_changes: &[(usize, i8, i8)],
) -> bool {
    if !height_changes.is_empty() {
        for (vx, vy) in touched_height_vertices(height_changes) {
            if !is_terrain_vertex_editable(world, vx, vy) {
                return false;
            }
        }
    } else {
        for &(idx, _, _) in grass_changes {
            let tx = idx / FULL_SIZE;
            let ty = idx % FULL_SIZE;
            if !is_grass_tile_editable(world, tx, ty) {
                return false;
            }
        }
    }
    true
}

fn water_changes_allowed(
    world: &World,
    water_changes: &[(usize, u8, u8)],
    height_changes: &[(usize, [u8; 4], [u8; 4])],
    forward: bool,
) -> bool {
    for &(idx, before, after) in water_changes {
        let (src, dst) = if forward {
            (before, after)
        } else {
            (after, before)
        };
        let tx = idx / FULL_SIZE;
        let ty = idx % FULL_SIZE;
        if src >= 16 && dst < 16 && !tile_supports_water_paint(world, tx, ty) {
            return false;
        }
        if src < 16 && dst >= 16 && !tile_supports_water_remove(world, tx, ty) {
            return false;
        }
    }
    if !height_changes.is_empty() {
        for (vx, vy) in touched_height_vertices(height_changes) {
            if !is_terrain_vertex_editable_inner(world, vx, vy, true) {
                return false;
            }
        }
    }
    true
}

fn pool_changes_allowed(world: &World, pool_changes: &[(usize, u8, u8)], forward: bool) -> bool {
    let active = usize::try_from(world.size).unwrap_or(0);
    let mut becoming_dry = BTreeSet::new();
    for &(idx, before, after) in pool_changes {
        let (src, dst) = if forward {
            (before, after)
        } else {
            (after, before)
        };
        if src < 16 && dst >= 16 {
            becoming_dry.insert((idx / FULL_SIZE, idx % FULL_SIZE));
        }
    }
    let mut dig_elev: Option<u8> = None;
    for &(idx, before, after) in pool_changes {
        let (src, dst) = if forward {
            (before, after)
        } else {
            (after, before)
        };
        let tx = idx / FULL_SIZE;
        let ty = idx % FULL_SIZE;
        if src >= 16 && dst < 16 {
            if !tile_supports_pool_dig(world, tx, ty) {
                return false;
            }
            let Some(elev) = tile_flat_elevation(world, tx, ty) else {
                return false;
            };
            if let Some(expected) = dig_elev {
                if elev != expected {
                    return false;
                }
            } else {
                dig_elev = Some(elev);
            }
            for (nx, ny) in [
                (tx.checked_sub(1), Some(ty)),
                ((tx + 1 < active).then_some(tx + 1), Some(ty)),
                (Some(tx), ty.checked_sub(1)),
                (Some(tx), (ty + 1 < active).then_some(ty + 1)),
            ] {
                let (Some(nx), Some(ny)) = (nx, ny) else {
                    continue;
                };
                if !becoming_dry.contains(&(nx, ny))
                    && is_pool_tile(world, nx, ny)
                    && tile_flat_elevation(world, nx, ny) != Some(elev)
                {
                    return false;
                }
            }
        }
        if src < 16 && dst >= 16 && !tile_supports_pool_demolish(world, tx, ty) {
            return false;
        }
    }
    true
}

fn apply_terrain_diffs(
    world: &mut World,
    height_changes: &[(usize, [u8; 4], [u8; 4])],
    grass_changes: &[(usize, i8, i8)],
    forward: bool,
) -> Result<(), Fault> {
    if !height_changes.is_empty() {
        let heights = world.arrays.get_mut(&HEIGHTS_ARRAY).ok_or(Fault {
            object: 0,
            at: None,
            trap: Trap::Invalid("missing terrain heights array".into()),
        })?;
        for &(idx, before, after) in height_changes {
            let off = idx * 4;
            let bytes = if forward { &after } else { &before };
            if let Some(dst) = heights.data.get_mut(off..off + 4) {
                dst.copy_from_slice(bytes);
            }
        }
    }
    if !grass_changes.is_empty() {
        let grass = world.arrays.get_mut(&GRASS_ARRAY).ok_or(Fault {
            object: 0,
            at: None,
            trap: Trap::Invalid("missing grass array".into()),
        })?;
        for &(idx, before, after) in grass_changes {
            let val = if forward { after as u8 } else { before as u8 };
            if let Some(dst) = grass.data.get_mut(idx) {
                *dst = val;
            }
        }
    }
    Ok(())
}

fn byte_array_diffs_match(
    world: &World,
    array_id: u16,
    changes: &[(usize, u8, u8)],
    created_array: bool,
    forward: bool,
) -> bool {
    if changes.is_empty() {
        return true;
    }
    let active = usize::try_from(world.size).unwrap_or(0).min(FULL_SIZE);
    let mut change_map: BTreeMap<usize, u8> = BTreeMap::new();
    for &(idx, before, after) in changes {
        let dst = if forward { after } else { before };
        change_map.insert(idx, dst);
    }

    let existing_data: Option<&[u8]> = if forward && created_array {
        if world.arrays.contains_key(&array_id) {
            return false;
        }
        if !changes.iter().all(|&(_, before, _)| before == 0xff) {
            return false;
        }
        None
    } else {
        let Some(arr) = world
            .arrays
            .get(&array_id)
            .filter(|a| a.stride == 1 && a.data.len() == FULL_SIZE * FULL_SIZE)
        else {
            return false;
        };
        for &(idx, before, after) in changes {
            let expected = if forward { before } else { after };
            if arr.data.get(idx).copied() != Some(expected) {
                return false;
            }
        }
        // When undoing an entry that created the array, verify untouched cells
        // still hold the initial dry `0xff` value so Undo never deletes
        // independent edits or `0x10` sentinels elsewhere in the array.
        if !forward && created_array {
            for (idx, &val) in arr.data.iter().enumerate() {
                if !change_map.contains_key(&idx) && val != 0xff {
                    return false;
                }
            }
        }
        Some(&arr.data)
    };

    // Validate cardinal-neighbor connectivity so stale adjacent plans cannot
    // commit or replay disconnected 4-bit masks.
    let live_byte = |idx: usize| -> u8 { existing_data.map_or(0xff, |d| d[idx]) };
    let next_byte = |idx: usize| -> u8 {
        change_map
            .get(&idx)
            .copied()
            .unwrap_or_else(|| live_byte(idx))
    };
    for &(idx, before, after) in changes {
        let (src, dst) = if forward {
            (before, after)
        } else {
            (after, before)
        };
        let tx = idx / FULL_SIZE;
        let ty = idx % FULL_SIZE;
        if tx >= active || ty >= active {
            continue;
        }
        let toggled = (src < 16) != (dst < 16);
        if toggled {
            for (nx, ny) in [
                (tx.checked_sub(1), Some(ty)),
                ((tx + 1 < active).then_some(tx + 1), Some(ty)),
                (Some(tx), ty.checked_sub(1)),
                (Some(tx), (ty + 1 < active).then_some(ty + 1)),
            ] {
                let (Some(nx), Some(ny)) = (nx, ny) else {
                    continue;
                };
                let nidx = cell_index(nx, ny);
                if !change_map.contains_key(&nidx) && live_byte(nidx) < 16 {
                    return false;
                }
            }
        }
        if dst < 16 {
            let mut expected_mask = 0u8;
            if ty > 0 && next_byte(cell_index(tx, ty - 1)) < 16 {
                expected_mask |= 0x01;
            }
            if tx > 0 && next_byte(cell_index(tx - 1, ty)) < 16 {
                expected_mask |= 0x02;
            }
            if ty + 1 < active && next_byte(cell_index(tx, ty + 1)) < 16 {
                expected_mask |= 0x04;
            }
            if tx + 1 < active && next_byte(cell_index(tx + 1, ty)) < 16 {
                expected_mask |= 0x08;
            }
            if dst != expected_mask {
                return false;
            }
        }
    }
    true
}

fn apply_byte_array_diffs(
    world: &mut World,
    array_id: u16,
    changes: &[(usize, u8, u8)],
    created_array: bool,
    forward: bool,
) -> Result<(), Fault> {
    if changes.is_empty() {
        return Ok(());
    }
    if forward && created_array && !world.arrays.contains_key(&array_id) {
        world.arrays.insert(
            array_id,
            Array {
                width: FULL_SIZE,
                height: FULL_SIZE,
                stride: 1,
                encoding: 0,
                data: vec![0xff; FULL_SIZE * FULL_SIZE],
            },
        );
    }
    if !forward && created_array {
        world.arrays.remove(&array_id);
        return Ok(());
    }
    let arr = world.arrays.get_mut(&array_id).ok_or(Fault {
        object: 0,
        at: None,
        trap: Trap::Invalid(format!("missing lot array {array_id}")),
    })?;
    for &(idx, before, after) in changes {
        let val = if forward { after } else { before };
        if let Some(dst) = arr.data.get_mut(idx) {
            *dst = val;
        }
    }
    Ok(())
}

fn roof_meta_matches(world: &World, expected: Option<&[u8]>) -> bool {
    let current = world
        .arrays
        .get(&ROOF_META_ARRAY)
        .map(|a| a.data.as_slice());
    current == expected
}

fn apply_roof_meta_raw(world: &mut World, bytes: Option<&[u8]>) {
    match bytes {
        Some(raw) => {
            world.arrays.insert(
                ROOF_META_ARRAY,
                Array {
                    width: 1,
                    height: 1,
                    stride: raw.len(),
                    encoding: 0,
                    data: raw.to_vec(),
                },
            );
        }
        None => {
            world.arrays.remove(&ROOF_META_ARRAY);
        }
    }
}

/// Applies a [`TerrainPlan`] to a cloned [`World`] for live preview rendering.
pub fn preview_terrain_on_world(world: &mut World, plan: &TerrainPlan) {
    let _ = apply_terrain_diffs(world, &plan.height_changes, &plan.grass_changes, true);
}

/// Applies a [`WaterPlan`] to a cloned [`World`] for live preview rendering.
pub fn preview_water_on_world(world: &mut World, plan: &WaterPlan) {
    let _ = apply_byte_array_diffs(
        world,
        WATER_ARRAY,
        &plan.water_changes,
        plan.created_water_array,
        true,
    );
    let _ = apply_terrain_diffs(world, &plan.height_changes, &plan.grass_changes, true);
}

/// Applies a [`PoolPlan`] to a cloned [`World`] for live preview rendering.
pub fn preview_pool_on_world(world: &mut World, plan: &PoolPlan) {
    let _ = apply_byte_array_diffs(
        world,
        POOL_ARRAY,
        &plan.pool_changes,
        plan.created_pool_array,
        true,
    );
}

/// Applies a [`RoofPlan`] to a cloned [`World`] for live preview rendering.
pub fn preview_roof_on_world(world: &mut World, plan: &RoofPlan) {
    if plan.valid {
        let raw = plan.after.encode();
        apply_roof_meta_raw(world, Some(&raw));
    }
}

impl Interpreter<'_> {
    /// Commits a planned Terrain or Grass edit to `world`, rebuilding tile
    /// flags when terrain heights changed and charging Architecture category `7`.
    /// Staged transactionally so any fault leaves `world` untouched.
    pub fn build_commit_terrain(
        &self,
        world: &mut World,
        plan: &TerrainPlan,
    ) -> Result<Option<BuildHistory>, Fault> {
        if !plan.valid
            || !plan.affordable
            || (plan.net_cost > 0 && plan.net_cost > available_funds(world))
            || !terrain_diffs_match(world, &plan.height_changes, &plan.grass_changes, true)
            || !terrain_changes_allowed(world, &plan.height_changes, &plan.grass_changes)
        {
            return Ok(None);
        }
        let mut staged = world.clone();
        apply_terrain_diffs(&mut staged, &plan.height_changes, &plan.grass_changes, true)?;
        if !plan.height_changes.is_empty() {
            self.rebuild_architecture_rooms(&mut staged)?;
        }
        if plan.net_cost != 0 {
            spend(&mut staged, ARCHITECTURE_BUDGET, plan.net_cost).map_err(|trap| Fault {
                object: 0,
                at: None,
                trap,
            })?;
        }
        *world = staged;
        Ok(Some(BuildHistory::Terrain {
            height_changes: plan.height_changes.clone(),
            grass_changes: plan.grass_changes.clone(),
            net_cost: plan.net_cost,
        }))
    }

    /// Commits a planned Pond/Water edit to `world`, updating `Arry 10`,
    /// terrain depression/grass diffs, tile flags, and Architecture category `7`.
    /// Staged transactionally so any fault leaves `world` untouched.
    pub fn build_commit_water(
        &self,
        world: &mut World,
        plan: &WaterPlan,
    ) -> Result<Option<BuildHistory>, Fault> {
        if !plan.valid
            || !plan.affordable
            || (plan.net_cost > 0 && plan.net_cost > available_funds(world))
            || !byte_array_diffs_match(
                world,
                WATER_ARRAY,
                &plan.water_changes,
                plan.created_water_array,
                true,
            )
            || !terrain_diffs_match(world, &plan.height_changes, &plan.grass_changes, true)
            || !water_changes_allowed(world, &plan.water_changes, &plan.height_changes, true)
        {
            return Ok(None);
        }
        let mut staged = world.clone();
        apply_byte_array_diffs(
            &mut staged,
            WATER_ARRAY,
            &plan.water_changes,
            plan.created_water_array,
            true,
        )?;
        apply_terrain_diffs(&mut staged, &plan.height_changes, &plan.grass_changes, true)?;
        if !plan.height_changes.is_empty() {
            self.rebuild_architecture_rooms(&mut staged)?;
        }
        if plan.net_cost != 0 {
            spend(&mut staged, ARCHITECTURE_BUDGET, plan.net_cost).map_err(|trap| Fault {
                object: 0,
                at: None,
                trap,
            })?;
        }
        *world = staged;
        Ok(Some(BuildHistory::Water {
            water_changes: plan.water_changes.clone(),
            height_changes: plan.height_changes.clone(),
            grass_changes: plan.grass_changes.clone(),
            created_water_array: plan.created_water_array,
            net_cost: plan.net_cost,
        }))
    }

    /// Commits a planned Swimming Pool dig/demolish edit to `world`, updating
    /// `Arry 9`, rebuilding rooms & tile flags, and charging/refunding
    /// Architecture category `7`. Staged transactionally so any fault leaves
    /// `world` untouched.
    pub fn build_commit_pool(
        &self,
        world: &mut World,
        plan: &PoolPlan,
    ) -> Result<Option<BuildHistory>, Fault> {
        if !plan.valid
            || !plan.affordable
            || (plan.net_cost > 0 && plan.net_cost > available_funds(world))
            || !byte_array_diffs_match(
                world,
                POOL_ARRAY,
                &plan.pool_changes,
                plan.created_pool_array,
                true,
            )
            || !pool_changes_allowed(world, &plan.pool_changes, true)
        {
            return Ok(None);
        }
        let mut staged = world.clone();
        apply_byte_array_diffs(
            &mut staged,
            POOL_ARRAY,
            &plan.pool_changes,
            plan.created_pool_array,
            true,
        )?;
        self.rebuild_architecture_rooms(&mut staged)?;
        if plan.net_cost != 0 {
            spend(&mut staged, ARCHITECTURE_BUDGET, plan.net_cost).map_err(|trap| Fault {
                object: 0,
                at: None,
                trap,
            })?;
        }
        *world = staged;
        Ok(Some(BuildHistory::Pool {
            pool_changes: plan.pool_changes.clone(),
            created_pool_array: plan.created_pool_array,
            net_cost: plan.net_cost,
        }))
    }

    /// Commits a planned Roof pattern/pitch change (`$0`) to `world`.
    pub fn build_commit_roof(
        &self,
        world: &mut World,
        plan: &RoofPlan,
    ) -> Result<Option<BuildHistory>, Fault> {
        if !plan.valid || !roof_meta_matches(world, plan.before_raw.as_deref()) {
            return Ok(None);
        }
        let after_raw = plan.after.encode();
        apply_roof_meta_raw(world, Some(&after_raw));
        Ok(Some(BuildHistory::Roof {
            before_raw: plan.before_raw.clone(),
            after_raw,
        }))
    }
}

fn lot_ready(lot: &Lot) -> Result<(), OrderError> {
    if lot.halted {
        return Err(OrderError::Halted);
    }
    if lot.asking.is_some() || lot.answered {
        return Err(OrderError::Asking);
    }
    if lot.load.finished.is_none() {
        return Err(OrderError::Incomplete);
    }
    Ok(())
}

fn commit_build_history_on_lot<F>(lot: &mut Lot, commit: F) -> Result<bool, OrderError>
where
    F: FnOnce(&Interpreter<'_>, &mut World) -> Result<Option<BuildHistory>, Fault>,
{
    lot_ready(lot)?;
    if lot.active_buy_cursor.is_some() {
        return Ok(false);
    }
    let sounds = RefCell::new(Vec::new());
    let interpreter = Interpreter {
        sounds: Some(&sounds),
        view_rotation: lot.view_rotation,
        view_centre: lot.view_centre,
        ..Interpreter::new(&lot.program, &lot.catalog, Stream::Global)
    };
    let mut world = lot.world.clone();
    let Some(entry) = commit(&interpreter, &mut world).map_err(OrderError::Refused)? else {
        return Ok(false);
    };
    let mut keep = BTreeSet::new();
    crate::buy::collect_buy_ids(&lot.buy_undo, &mut keep);
    for e in &lot.build_undo {
        crate::buy::collect_buy_ids(e.detached_objects(), &mut keep);
    }
    let mut build_redo_detached = Vec::new();
    for e in &lot.build_redo {
        build_redo_detached.extend_from_slice(e.detached_objects());
    }
    interpreter
        .buy_purge_detached(&mut world, &lot.buy_redo, &keep)
        .map_err(OrderError::Refused)?;
    interpreter
        .buy_purge_detached(&mut world, &build_redo_detached, &keep)
        .map_err(OrderError::Refused)?;
    lot.world = world;
    lot.build_undo.push(entry);
    lot.build_redo.clear();
    lot.buy_redo.clear();
    lot.undo_order.push(HistoryKind::Build);
    lot.redo_order.clear();
    Ok(true)
}

impl Lot {
    /// Commits a planned Terrain raise/lower/level or Grass edit to
    /// `self.world`, updating Architecture funds and the unified Undo/Redo stack.
    pub fn build_commit_terrain(&mut self, plan: &TerrainPlan) -> Result<bool, OrderError> {
        commit_build_history_on_lot(self, |interp, world| {
            interp.build_commit_terrain(world, plan)
        })
    }

    /// Commits a planned Pond/Water edit to `self.world`, updating Architecture
    /// funds and the unified Undo/Redo stack.
    pub fn build_commit_water(&mut self, plan: &WaterPlan) -> Result<bool, OrderError> {
        commit_build_history_on_lot(self, |interp, world| interp.build_commit_water(world, plan))
    }

    /// Commits a planned Swimming Pool dig or demolish edit to `self.world`,
    /// rebuilding rooms/flags, updating Architecture funds, and pushing to the
    /// unified Undo/Redo stack.
    pub fn build_commit_pool(&mut self, plan: &PoolPlan) -> Result<bool, OrderError> {
        commit_build_history_on_lot(self, |interp, world| interp.build_commit_pool(world, plan))
    }

    /// Returns the lot's current [`RoofState`] (pattern stem and pitch preset),
    /// falling back to `base` (from `HOUS 0`) when never edited.
    pub fn roof_state_with_base(&self, base: &RoofState) -> RoofState {
        roof_state_with_base(&self.world, base)
    }

    /// Returns the lot's current [`RoofState`] (pattern stem and pitch preset).
    pub fn roof_state(&self) -> RoofState {
        roof_state(&self.world)
    }

    /// Commits a planned Roof pattern/pitch change (`$0`) to `self.world`,
    /// recording it on the unified Undo/Redo stack and persisting across
    /// `Lot::save()`/`Lot::restore()`.
    pub fn build_commit_roof(&mut self, plan: &RoofPlan) -> Result<bool, OrderError> {
        commit_build_history_on_lot(self, |interp, world| interp.build_commit_roof(world, plan))
    }
}
