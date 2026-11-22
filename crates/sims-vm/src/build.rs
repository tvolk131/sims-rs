//! Build Mode simulation operations: `"Wall Cursor"` (`0x00000439`,
//! `0x48b68b` / `0x48a66a` / `0x48a6e5`), wall drag placement, Shift room-box,
//! and Ctrl demolition (`cNewWallTool` / `WallUndoable` `0x48af6f`..`0x496931`),
//! floor tile placement, drag rectangle, Shift room-fill, split-diagonal halves,
//! and Ctrl removal (`cNewFloorTool` / `FloorUndoable` `0x4881e6`..`0x4899d5`),
//! wallpaper face application, drag, Shift room-fill, and Ctrl stripping
//! (`cNewWallpaperTool` / `WallpaperUndoable` `0x48a08d`..`0x4939c0`),
//! Architecture budget accounting (`0x46d91c` category 7), and Build Mode
//! Undo/Redo (`0x4932fa`, `0x493338`).

use crate::buy::{BuyActionKind, BuyPurchase, available_funds};
use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::placement::{Place, on_lot};
use crate::primitives::budget::spend;
use sims_sim::World;
use sims_sim::load::{ExpandedWall, FLOORS, WALLS};
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
const POOL_ARRAY: u16 = 4;

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
}

impl BuildHistory {
    /// Net Architecture cost of the forward operation (positive = spent,
    /// negative = refunded).
    pub fn net_cost(&self) -> i32 {
        match self {
            Self::Wall { net_cost, .. }
            | Self::Floor { net_cost, .. }
            | Self::Wallpaper { net_cost, .. } => *net_cost,
        }
    }

    /// Detached BuyPurchase records held by a wall demolition for Undo.
    pub fn detached_objects(&self) -> &[BuyPurchase] {
        match self {
            Self::Wall {
                removed_objects, ..
            } => removed_objects,
            Self::Floor { .. } | Self::Wallpaper { .. } => &[],
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

fn is_pool_tile(world: &World, x: usize, y: usize) -> bool {
    let Some(pool) = world.arrays.get(&POOL_ARRAY) else {
        return false;
    };
    if pool.stride != 1 {
        return false;
    }
    pool.data.get(cell_index(x, y)).copied().unwrap_or(0) != 0
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

/// Whether a level-2 tile `(x, y)` has structural support underneath on level 1
/// (inside a ground-floor enclosed room, or directly above a level-1 wall).
fn has_upper_support(world: &World, x: usize, y: usize) -> bool {
    let idx = cell_index(x, y);
    let ground_room = world.rooms.map[0].get(idx).copied().unwrap_or(-1);
    if ground_room >= 0
        && world
            .rooms
            .rooms
            .get(&ground_room)
            .is_some_and(|r| !r.outdoors)
    {
        return true;
    }
    if let Some(walls) = world.arrays.get(&WALLS[0])
        && walls.stride == WALL_CELL
        && walls.data.get(idx * WALL_CELL).copied().unwrap_or(0) != 0
    {
        return true;
    }
    false
}

/// Collects non-cursor lot objects standing on tile `(tx, ty)` at `level` that
/// depend on a wall segment (`data[13] != 0`, wall attachment rules) or have
/// `wall_style` cut-out shapes (doors/windows).
fn wall_attached_objects(world: &World, tile: [u8; 2], level: u8) -> Vec<u16> {
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
        if (obj.position[0] >> 4) == tx && (obj.position[1] >> 4) == ty {
            let wall_rules = obj.data.get(13).copied().unwrap_or(0);
            if wall_rules != 0 {
                let lead = obj
                    .multipart
                    .as_ref()
                    .and_then(|m| u16::try_from(m.parent).ok())
                    .filter(|&p| p != 0)
                    .unwrap_or(id);
                leads.insert(lead);
            }
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

    for seg in segments {
        let x = usize::from(seg.tile[0]);
        let y = usize::from(seg.tile[1]);
        if !is_buildable_tile(world, x, y, level) {
            return WallPlan {
                valid: false,
                ..empty
            };
        }
        if !mode.is_demolish() {
            if level == 1 && (!is_flat_tile(world, x, y) || is_pool_tile(world, x, y)) {
                return WallPlan {
                    valid: false,
                    ..empty
                };
            }
            if level == 2 && !has_upper_support(world, x, y) {
                return WallPlan {
                    valid: false,
                    ..empty
                };
            }
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
            match seg.bit {
                WALL_WEST => {
                    net_cost -= build_refund(prices.wall_price(cell.patterns[0]));
                    if x > 0 {
                        let west_cell = ExpandedWall::get(&walls_buf, active, x - 1, y);
                        net_cost -= build_refund(prices.wall_price(west_cell.patterns[2]));
                    }
                    cell.patterns[0] = 0;
                    cell.styles[0] = Some(1);
                }
                WALL_NORTH => {
                    net_cost -= build_refund(prices.wall_price(cell.patterns[1]));
                    if y > 0 {
                        let north_cell = ExpandedWall::get(&walls_buf, active, x, y - 1);
                        net_cost -= build_refund(prices.wall_price(north_cell.patterns[3]));
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
            // Collect wall-mounted objects on this tile (and adjacent side for cardinal walls).
            for id in wall_attached_objects(world, [x as u8, y as u8], level) {
                removed_set.insert(id);
            }
            if seg.bit == WALL_WEST && x > 0 {
                for id in wall_attached_objects(world, [(x - 1) as u8, y as u8], level) {
                    removed_set.insert(id);
                }
            } else if seg.bit == WALL_NORTH && y > 0 {
                for id in wall_attached_objects(world, [x as u8, (y - 1) as u8], level) {
                    removed_set.insert(id);
                }
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
            // Mirror default fence pattern onto neighbor's facing side (`0x494390`..`0x494600`).
            if new_pattern != 0 {
                if seg.bit == WALL_WEST && x > 0 {
                    let mut west = ExpandedWall::get(&walls_buf, active, x - 1, y);
                    west.patterns[2] = new_pattern;
                    let _ = west.put(&mut walls_buf, active, x - 1, y);
                } else if seg.bit == WALL_NORTH && y > 0 {
                    let mut north = ExpandedWall::get(&walls_buf, active, x, y - 1);
                    north.patterns[3] = new_pattern;
                    let _ = north.put(&mut walls_buf, active, x, y - 1);
                }
            }
            net_cost += seg_price;
            any_change = true;
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
        if !mode.is_remove() {
            if level == 1 && (!is_flat_tile(world, x, y) || is_pool_tile(world, x, y)) {
                continue;
            }
            if level == 2 && !has_upper_support(world, x, y) {
                continue;
            }
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
        let Some(obj) = world.objects.get_mut(&cursor.id) else {
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
        if world.objects.contains_key(&cursor.id) {
            let _ = self.take_off(world, cursor.id, false);
            if let Some(obj) = world.objects.get(&cursor.id) {
                cursor.position = obj.position;
            }
        }
        Ok(())
    }

    /// Deletes `"Wall Cursor"` (`0x48a6e5`).
    pub fn build_cursor_close(&self, world: &mut World, cursor: &WallCursor) -> Result<(), Fault> {
        if world.objects.contains_key(&cursor.id) {
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
            let _ = spend(world, ARCHITECTURE_BUDGET, plan.net_cost);
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
        apply_cell_diffs(
            world,
            plan.level,
            &plan.wall_changes,
            &plan.floor_changes,
            true,
        )?;
        if plan.net_cost != 0 {
            let _ = spend(world, ARCHITECTURE_BUDGET, plan.net_cost);
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
        apply_cell_diffs(world, plan.level, &plan.wall_changes, &[], true)?;
        if plan.net_cost != 0 {
            let _ = spend(world, ARCHITECTURE_BUDGET, plan.net_cost);
        }
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
                    let _ = spend(world, ARCHITECTURE_BUDGET, -*net_cost);
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
                if *net_cost != 0 {
                    let _ = spend(world, ARCHITECTURE_BUDGET, -*net_cost);
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
                    let _ = spend(world, ARCHITECTURE_BUDGET, -*net_cost);
                }
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
                    let _ = spend(world, ARCHITECTURE_BUDGET, *net_cost);
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
                if *net_cost != 0 {
                    let _ = spend(world, ARCHITECTURE_BUDGET, *net_cost);
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
                    let _ = spend(world, ARCHITECTURE_BUDGET, *net_cost);
                }
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
