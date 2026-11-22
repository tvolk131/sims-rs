//! Build Mode Part 2 visual rendering primitives:
//!
//! 1. **Terrain tool overlay** (`Raise`, `Lower`, `Level`, `Grass`, `Soil` —
//!    `Build.iff STR# 128`, `cSimpleAltTool`, `cLevelDirtTool`, `cGrassTool`):
//!    projects sloped tile highlights, vertex-brush elevation markers, and
//!    valid/blocked tinting onto the lot's heightfield (`Lot::heights`).
//! 2. **Water & Pool digging preview** (`cPoolTool` `0x48e94c`–`0x48fe8b`,
//!    `cWaterTool` `0x497178`–`0x497e48`, `Build.iff STR# 133`): validates
//!    bounds, flatness, and obstructions, then composites translucent pool
//!    water, pool-border coping/edge tiles, natural pond water, and red
//!    blocked/invalid-slope indicators depth-correctly over [`Canvas`].
//! 3. **Roof style & pitch catalog + isometric preview renderer**
//!    (`0x4153c4` `GameData/Roofs/`, `cWinRoofPanel` `0x529d31`–`0x52a1f2`,
//!    `cRoofLayer` `0x56ea40`–`0x56f2a0`, `cRoofMeshFactory`
//!    `0x56f567`–`0x56f8a0`): loads Complete Collection's `32×32` BMP roof
//!    pattern catalog and rasterizes textured, sun-shaded, depth-buffered
//!    isometric roof previews matching `Sims.exe`'s pitch presets and world
//!    projection.
//!
//! Every helper here is pure presentation: it reads [`Lot`] (and optionally
//! [`World`]) without mutating simulation state and composites over an existing
//! [`Canvas`] without altering static lot rendering.

use crate::camera::Camera;
use crate::canvas::{Canvas, clamp_depth, pack_rgb, quantize565, unpack_rgb};
use crate::depth::Depth;
use crate::light;
use crate::lot::{self, Lot};
use crate::{Error, Result};
use sims_content::Install;
use sims_formats::bitmap;
use sims_sim::World;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Native world units per lot tile (`cRoofMeshFactory` `[0x6714e8] = 3.0`).
pub const TILE_WORLD_UNITS: f32 = 3.0;

/// Half-tile step in world units (`[0x666578] = 0.5` × `3.0`).
pub const HALF_TILE_WORLD_UNITS: f32 = 1.5;

/// Near-zoom vertical screen pixels per world unit:
/// `16` near-zoom pixels per terrain step, where 1 terrain step is
/// `3 / (2 * sqrt(6))` world units (`0x434df6`, `depth.rs:297`), yielding
/// `32 * sqrt(6) / 3 ≈ 26.12789` near-zoom pixels per world unit.
const NEAR_PX_PER_WORLD_UNIT: f32 = 26.127_89;

/// Maximum roof pitch slope accepted by `cRoofLayer::SetPitch` (`0x56f0e9`:
/// `[0x66657c] = 0.0`, `[0x66740c] = 2.0`).
pub const MAX_ROOF_SLOPE: f32 = 2.0;

/// Default roof pattern stem when a house specifies none or its pattern is
/// missing (`0x6ceabc` = `"r1_"`).
pub const DEFAULT_ROOF_PATTERN: &str = "r1_";

/// Pool and Water array IDs in `sims_sim::World::arrays` (matching
/// `sims-vm::build`).
const POOL_ARRAY_ID: u16 = 9;
const WATER_ARRAY_ID: u16 = 10;

/// Native maximum terrain corner height step (`cSimpleAltTool` -> `0x490f54`:
/// caps candidate vertex elevation at `0x40` = 64).
pub const MAX_TERRAIN_HEIGHT: u8 = 64;

/// Maximum slope delta across adjacent corners of a tile before terrain
/// lighting/clamping saturates (`grass.rs`: `-5..=5` slope table).
const MAX_TERRAIN_SLOPE_DELTA: i32 = 5;

/// Depth offset placed on translucent ground overlays so they sit strictly
/// nearer than floors (`+65`) and grass (`+80`) while still yielding to
/// standing walls and objects (`<= 0`).
const GROUND_OVERLAY_DEPTH_BIAS: i32 = 32;

/// Road and sidewalk floor IDs (`Build.iff` / `sims-ui::build`).
fn is_road_floor(floor_id: u16) -> bool {
    (9..=13).contains(&floor_id)
}

/// Statistics returned by overlay draw passes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OverlayStats {
    pub tiles_drawn: usize,
    pub blocked_tiles: usize,
    pub pixels_written: usize,
}

// ============================================================================
// 1. Terrain Tool Visual Overlay
// ============================================================================

/// Complete Collection terrain sculpting and landscaping tools
/// (`Build.iff STR# 128`: Raise, Lower, Level, Grass + Ctrl-Grass Soil).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerrainTool {
    Raise,
    Lower,
    Level,
    Grass,
    Soil,
}

impl TerrainTool {
    /// Native per-action/per-tile cost from `Build.iff STR# 128` (`$1`).
    pub const fn unit_cost(self) -> i32 {
        1
    }
}

/// Target geometry for a [`TerrainOverlay`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerrainTarget {
    /// Vertex corner `[vx, vy]` in `0..=lot.size` (used by `Raise` / `Lower`).
    Vertex([i32; 2]),
    /// Tile rectangle from `start` to `end` inclusive (used by `Level`).
    Rect { start: [i32; 2], end: [i32; 2] },
    /// Tile brush centered at `center` with Chebyshev `radius` (`0` = single
    /// tile; used by `Grass` / `Soil`).
    Brush { center: [i32; 2], radius: u8 },
}

/// Input specification for a Build Mode terrain preview overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainOverlay {
    pub tool: TerrainTool,
    pub target: TerrainTarget,
}

/// Per-tile validation result for a terrain preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerrainTileStatus {
    Valid,
    OutOfBounds,
    BlockedByRoad,
    BlockedByWall,
    BlockedByFloor,
    BlockedByObject,
    BlockedByWater,
    HeightLimit,
    SlopeLimit,
}

impl TerrainTileStatus {
    pub const fn is_valid(self) -> bool {
        matches!(self, Self::Valid)
    }
}

/// One tile covered by a terrain tool preview and its validation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainPreviewTile {
    pub tile: [i32; 2],
    pub status: TerrainTileStatus,
}

/// Validated terrain overlay ready to render onto a [`Canvas`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainPreviewPlan {
    pub tool: TerrainTool,
    pub target: TerrainTarget,
    /// Reference height in terrain steps (anchor height for `Level`, target
    /// vertex height for `Raise`/`Lower`, center tile height for `Grass`/`Soil`).
    pub anchor_height: u8,
    pub tiles: Vec<TerrainPreviewTile>,
    pub valid: bool,
    pub cost: i32,
}

/// Validates a [`TerrainOverlay`] against `lot` (and optional `world` for
/// objects and pool/water arrays `9`/`10`).
pub fn validate_terrain_overlay(
    lot: &Lot,
    world: Option<&World>,
    overlay: &TerrainOverlay,
) -> TerrainPreviewPlan {
    let size = lot.size as i32;
    let mut tiles = Vec::new();
    let anchor_height = match overlay.target {
        TerrainTarget::Vertex(v) => corner_height(lot, v),
        TerrainTarget::Rect { start, .. } => tile_reference_height(lot, start),
        TerrainTarget::Brush { center, .. } => tile_reference_height(lot, center),
    };

    let candidate_tiles: Vec<[i32; 2]> = match overlay.target {
        TerrainTarget::Vertex([vx, vy]) => {
            if vx < 0 || vy < 0 || vx > size || vy > size {
                vec![[vx, vy]]
            } else {
                vec![[vx - 1, vy - 1], [vx, vy - 1], [vx - 1, vy], [vx, vy]]
            }
        }
        TerrainTarget::Rect { start, end } => {
            let (x0, x1) = ordered_pair(start[0], end[0]);
            let (y0, y1) = ordered_pair(start[1], end[1]);
            // Cap preview grid to valid lot-sized spans so pathological input
            // coordinates cannot allocate gigabytes.
            let x0c = x0.clamp(-1, size);
            let x1c = x1.clamp(-1, size);
            let y0c = y0.clamp(-1, size);
            let y1c = y1.clamp(-1, size);
            let mut out = Vec::new();
            for x in x0c..=x1c {
                for y in y0c..=y1c {
                    out.push([x, y]);
                }
            }
            out
        }
        TerrainTarget::Brush { center, radius } => {
            let r = i32::from(radius.min(8));
            let mut out = Vec::new();
            for dx in -r..=r {
                for dy in -r..=r {
                    out.push([center[0].saturating_add(dx), center[1].saturating_add(dy)]);
                }
            }
            out
        }
    };

    let vertex_limit_status = match (overlay.tool, overlay.target) {
        (TerrainTool::Raise, TerrainTarget::Vertex(v)) => {
            let h = corner_height(lot, v);
            let candidate = h.saturating_add(1).min(MAX_TERRAIN_HEIGHT);
            if candidate == h {
                Some(TerrainTileStatus::HeightLimit)
            } else if vertex_exceeds_slope(lot, v, i32::from(candidate)) {
                Some(TerrainTileStatus::SlopeLimit)
            } else {
                None
            }
        }
        (TerrainTool::Lower, TerrainTarget::Vertex(v)) => {
            let h = corner_height(lot, v);
            if h == 0 {
                Some(TerrainTileStatus::HeightLimit)
            } else if vertex_exceeds_slope(lot, v, i32::from(h) - 1) {
                Some(TerrainTileStatus::SlopeLimit)
            } else {
                None
            }
        }
        _ => None,
    };

    let is_sculpt = matches!(
        overlay.tool,
        TerrainTool::Raise | TerrainTool::Lower | TerrainTool::Level
    );

    for tile in candidate_tiles {
        let status = if let Some(limit) = vertex_limit_status {
            if !is_inner_tile(lot, tile) {
                TerrainTileStatus::OutOfBounds
            } else {
                limit
            }
        } else {
            validate_terrain_tile(lot, world, tile, is_sculpt)
        };
        tiles.push(TerrainPreviewTile { tile, status });
    }

    let valid = match overlay.tool {
        TerrainTool::Raise | TerrainTool::Lower => {
            // Vertex must sit strictly inside editable lot corners (`1..size-1`
            // interior tiles -> vertices `2..=size-2`, or any vertex touching at
            // least one valid tile with no blocked touching tiles).
            let vertex_in_bounds = match overlay.target {
                TerrainTarget::Vertex([vx, vy]) => vx >= 1 && vy >= 1 && vx < size && vy < size,
                _ => true,
            };
            vertex_in_bounds && !tiles.is_empty() && tiles.iter().all(|t| t.status.is_valid())
        }
        TerrainTool::Level => !tiles.is_empty() && tiles.iter().all(|t| t.status.is_valid()),
        TerrainTool::Grass | TerrainTool::Soil => tiles.iter().any(|t| t.status.is_valid()),
    };

    let valid_count = tiles.iter().filter(|t| t.status.is_valid()).count() as i32;
    let cost = match overlay.tool {
        TerrainTool::Raise | TerrainTool::Lower => {
            if valid {
                overlay.tool.unit_cost()
            } else {
                0
            }
        }
        TerrainTool::Level | TerrainTool::Grass | TerrainTool::Soil => {
            valid_count * overlay.tool.unit_cost()
        }
    };

    TerrainPreviewPlan {
        tool: overlay.tool,
        target: overlay.target,
        anchor_height,
        tiles,
        valid,
        cost,
    }
}

fn validate_terrain_tile(
    lot: &Lot,
    world: Option<&World>,
    tile: [i32; 2],
    is_sculpt: bool,
) -> TerrainTileStatus {
    if !is_inner_tile(lot, tile) {
        return TerrainTileStatus::OutOfBounds;
    }
    let (tx, ty) = (tile[0] as usize, tile[1] as usize);
    let cell = Lot::cell(tx, ty);
    let floor = lot.levels[0].floors[cell];
    if is_road_floor(floor) {
        return TerrainTileStatus::BlockedByRoad;
    }
    if world.is_some_and(|w| is_pool_or_water_tile(w, tx, ty)) {
        return TerrainTileStatus::BlockedByWater;
    }
    if floor != 0 {
        return TerrainTileStatus::BlockedByFloor;
    }
    if is_sculpt {
        if tile_has_any_wall(lot, tile) {
            return TerrainTileStatus::BlockedByWall;
        }
        if world.is_some_and(|w| tile_has_blocking_object(w, tile)) {
            return TerrainTileStatus::BlockedByObject;
        }
    }
    TerrainTileStatus::Valid
}

/// Draws a validated [`TerrainPreviewPlan`] over `canvas`, projected onto
/// `lot.heights` and depth-tested against `canvas.depth`.
pub fn draw_terrain_overlay(
    lot: &Lot,
    camera: &Camera,
    plan: &TerrainPreviewPlan,
    canvas: &mut Canvas,
) -> OverlayStats {
    let depth = Depth::new(camera);
    let mut stats = OverlayStats::default();

    let (valid_fill, valid_edge, fill_alpha) = match plan.tool {
        TerrainTool::Raise => ([72, 224, 128], [180, 255, 200], 96u8),
        TerrainTool::Lower => ([72, 184, 232], [180, 236, 255], 96u8),
        TerrainTool::Level => ([240, 212, 84], [255, 242, 168], 96u8),
        TerrainTool::Grass => ([56, 196, 68], [164, 248, 148], 108u8),
        TerrainTool::Soil => ([168, 116, 64], [224, 176, 120], 108u8),
    };
    let blocked_fill = [224, 52, 52];
    let blocked_edge = [255, 136, 136];

    for entry in &plan.tiles {
        let tile = entry.tile;
        if !lot.contains(tile[0], tile[1]) {
            continue;
        }
        let tile_ok = entry.status.is_valid()
            && (plan.valid || matches!(plan.tool, TerrainTool::Grass | TerrainTool::Soil));
        let (fill, edge) = if tile_ok {
            (valid_fill, valid_edge)
        } else {
            stats.blocked_tiles += 1;
            (blocked_fill, blocked_edge)
        };
        let corners = sloped_tile_vertices(lot, camera, &depth, tile, GROUND_OVERLAY_DEPTH_BIAS);
        stats.pixels_written += raster_quad_blended(canvas, corners, fill, fill_alpha, false);
        stats.pixels_written += draw_quad_outline(canvas, corners, edge, 210);
        stats.tiles_drawn += 1;
    }

    // For Raise / Lower, draw the vertex-brush crosshair + elevation arrow at
    // the projected terrain vertex.
    if let TerrainTarget::Vertex(vertex) = plan.target
        && vertex[0] >= 0
        && vertex[1] >= 0
        && vertex[0] <= lot.size as i32
        && vertex[1] <= lot.size as i32
    {
        let h = corner_height(lot, vertex);
        let p = project_corner_vertex(camera, &depth, vertex, f32::from(h) * 16.0, 1, 0);
        let color = if plan.valid { valid_edge } else { blocked_edge };
        let up = matches!(plan.tool, TerrainTool::Raise);
        stats.pixels_written += draw_vertex_brush_marker(canvas, p, up, camera.zoom.level(), color);
    }

    // For Level, draw the target flat-plane reference border at anchor_height.
    if plan.tool == TerrainTool::Level
        && let TerrainTarget::Rect { start, end } = plan.target
    {
        let (x0, x1) = ordered_pair(start[0], end[0]);
        let (y0, y1) = ordered_pair(start[1], end[1]);
        let size = lot.size as i32;
        if x0 >= 0 && y0 >= 0 && x1 < size && y1 < size {
            let elev = f32::from(plan.anchor_height) * 16.0;
            let outline_color = if plan.valid {
                [255, 255, 196]
            } else {
                blocked_edge
            };
            let rect_corners = [
                project_corner_vertex(camera, &depth, [x0, y0], elev, 1, 8),
                project_corner_vertex(camera, &depth, [x1 + 1, y0], elev, 1, 8),
                project_corner_vertex(camera, &depth, [x1 + 1, y1 + 1], elev, 1, 8),
                project_corner_vertex(camera, &depth, [x0, y1 + 1], elev, 1, 8),
            ];
            stats.pixels_written += draw_quad_outline(canvas, rect_corners, outline_color, 235);
        }
    }

    stats
}

/// Convenience helper that validates and draws a [`TerrainOverlay`] in one call.
pub fn draw_terrain_preview(
    lot: &Lot,
    world: Option<&World>,
    camera: &Camera,
    overlay: &TerrainOverlay,
    canvas: &mut Canvas,
) -> (TerrainPreviewPlan, OverlayStats) {
    let plan = validate_terrain_overlay(lot, world, overlay);
    let stats = draw_terrain_overlay(lot, camera, &plan, canvas);
    (plan, stats)
}

// ============================================================================
// 2. Water & Pool Digging Preview
// ============================================================================

/// Water excavation tools (`Build.iff STR# 133`: Pool `$75`, Water/Pond `$15`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WaterTool {
    Pool,
    Pond,
}

impl WaterTool {
    /// Per-tile excavation cost from `Build.iff STR# 133`.
    pub const fn tile_cost(self) -> i32 {
        match self {
            Self::Pool => 75,
            Self::Pond => 15,
        }
    }
}

/// Dig vs Ctrl-drag remove/fill action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WaterAction {
    Dig,
    Remove,
}

/// Input specification for a Water or Swimming Pool drag preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterOverlay {
    pub tool: WaterTool,
    pub action: WaterAction,
    pub start: [i32; 2],
    pub end: [i32; 2],
}

/// Per-tile validation result for a Water / Pool preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WaterTileStatus {
    Valid,
    InvalidSlope,
    OutOfBounds,
    BlockedByRoad,
    BlockedByWall,
    BlockedByFloor,
    BlockedByObject,
    BlockedByWater,
}

impl WaterTileStatus {
    pub const fn is_valid(self) -> bool {
        matches!(self, Self::Valid)
    }
}

/// Coping border side bits indicating which neighbor direction holds pool water.
pub const COPING_WEST: u8 = 1;
pub const COPING_NORTH: u8 = 2;
pub const COPING_EAST: u8 = 4;
pub const COPING_SOUTH: u8 = 8;
pub const COPING_CORNER: u8 = 0x10;

/// One interior tile of a Water or Pool drag preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterPreviewTile {
    pub tile: [i32; 2],
    pub status: WaterTileStatus,
}

/// One exterior border/coping tile around a Swimming Pool preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopingTile {
    pub tile: [i32; 2],
    /// Bitmask of [`COPING_WEST`], [`COPING_NORTH`], [`COPING_EAST`],
    /// [`COPING_SOUTH`], and [`COPING_CORNER`].
    pub edge_mask: u8,
    pub status: WaterTileStatus,
}

/// Validated Water / Pool preview plan ready to composite over [`Canvas`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaterPreviewPlan {
    pub tool: WaterTool,
    pub action: WaterAction,
    /// Normalized `[min, max]` inclusive tile rectangle.
    pub rect: [[i32; 2]; 2],
    /// Flat terrain height at the anchor tile (used for pool flatness checks).
    pub target_height: u8,
    pub tiles: Vec<WaterPreviewTile>,
    pub coping_tiles: Vec<CopingTile>,
    pub valid: bool,
    pub cost: i32,
}

/// Validates a [`WaterOverlay`] against `lot` (and optional `world`), matching
/// `cPoolTool::Validate` (`0x48fa7b`) and `cWaterTool` (`0x497178`).
pub fn validate_water_overlay(
    lot: &Lot,
    world: Option<&World>,
    overlay: &WaterOverlay,
) -> WaterPreviewPlan {
    let size = lot.size as i32;
    let (x0, x1) = ordered_pair(overlay.start[0], overlay.end[0]);
    let (y0, y1) = ordered_pair(overlay.start[1], overlay.end[1]);
    let rect = [[x0, y0], [x1, y1]];
    let target_height = tile_reference_height(lot, [x0, y0]);

    let x0c = x0.clamp(-1, size);
    let x1c = x1.clamp(-1, size);
    let y0c = y0.clamp(-1, size);
    let y1c = y1.clamp(-1, size);

    let mut tiles = Vec::new();
    for x in x0c..=x1c {
        for y in y0c..=y1c {
            let status = validate_water_tile(
                lot,
                world,
                [x, y],
                overlay.tool,
                overlay.action,
                target_height,
                false,
            );
            tiles.push(WaterPreviewTile {
                tile: [x, y],
                status,
            });
        }
    }

    let rect_within_clamp = x0 == x0c && x1 == x1c && y0 == y0c && y1 == y1c;
    let mut coping_tiles = Vec::new();
    if overlay.tool == WaterTool::Pool && overlay.action == WaterAction::Dig && rect_within_clamp {
        for x in (x0c - 1)..=(x1c + 1) {
            for y in (y0c - 1)..=(y1c + 1) {
                if x >= x0c && x <= x1c && y >= y0c && y <= y1c {
                    continue;
                }
                let mut mask = 0u8;
                if x == x0c - 1 {
                    mask |= COPING_EAST;
                }
                if x == x1c + 1 {
                    mask |= COPING_WEST;
                }
                if y == y0c - 1 {
                    mask |= COPING_SOUTH;
                }
                if y == y1c + 1 {
                    mask |= COPING_NORTH;
                }
                if mask.count_ones() >= 2 {
                    mask |= COPING_CORNER;
                }
                // Visual coping border surrounds the excavation rectangle; native
                // `cPoolTool::Validate` (`0x48fa7b`) validates the excavation
                // rectangle itself, so outer-lot-edge coping (`0` or `size-1`)
                // renders cleanly without invalidating a pool starting at `x=1`.
                let status = if lot.contains(x, y) {
                    let (tx, ty) = (x as usize, y as usize);
                    let floor = lot.levels[0].floors[Lot::cell(tx, ty)];
                    if is_road_floor(floor) {
                        WaterTileStatus::BlockedByRoad
                    } else if world.is_some_and(|w| has_water_mask(w, WATER_ARRAY_ID, tx, ty)) {
                        WaterTileStatus::BlockedByWater
                    } else {
                        WaterTileStatus::Valid
                    }
                } else {
                    WaterTileStatus::OutOfBounds
                };
                coping_tiles.push(CopingTile {
                    tile: [x, y],
                    edge_mask: mask,
                    status,
                });
            }
        }
    }

    let interior_valid = !tiles.is_empty() && tiles.iter().all(|t| t.status.is_valid());
    let valid = rect_within_clamp && interior_valid;

    // `0x48fa06`..`0x48fa0c`: charge only for newly excavated tiles, not tiles
    // that already hold the same pool/water surface.
    let new_tiles = tiles
        .iter()
        .filter(|t| {
            t.status.is_valid()
                && lot.contains(t.tile[0], t.tile[1])
                && !world.is_some_and(|w| {
                    let arr_id = match overlay.tool {
                        WaterTool::Pool => POOL_ARRAY_ID,
                        WaterTool::Pond => WATER_ARRAY_ID,
                    };
                    has_water_mask(w, arr_id, t.tile[0] as usize, t.tile[1] as usize)
                })
        })
        .count() as i32;
    let cost = match overlay.action {
        WaterAction::Dig if valid => new_tiles * overlay.tool.tile_cost(),
        _ => 0,
    };

    WaterPreviewPlan {
        tool: overlay.tool,
        action: overlay.action,
        rect,
        target_height,
        tiles,
        coping_tiles,
        valid,
        cost,
    }
}

fn validate_water_tile(
    lot: &Lot,
    world: Option<&World>,
    tile: [i32; 2],
    tool: WaterTool,
    action: WaterAction,
    target_height: u8,
    is_coping: bool,
) -> WaterTileStatus {
    // cPoolTool::Validate (`0x48fa95`..`0x48faee`): pool tiles must sit inside
    // `1..size-1` so there is at least a 1-tile perimeter around the lot.
    if !is_inner_tile(lot, tile) {
        return WaterTileStatus::OutOfBounds;
    }
    let (tx, ty) = (tile[0] as usize, tile[1] as usize);
    let cell = Lot::cell(tx, ty);
    let floor = lot.levels[0].floors[cell];

    if action == WaterAction::Remove {
        return if is_road_floor(floor) {
            WaterTileStatus::BlockedByRoad
        } else {
            WaterTileStatus::Valid
        };
    }

    if is_road_floor(floor) {
        return WaterTileStatus::BlockedByRoad;
    }

    // Native flatness check for both Pool (`0x48faf9`..`0x48fb11`) and Water/Pond
    // (`0x49838e`..`0x4983a6`, `0x4984aa`..`0x4984cf`): all four corners of every
    // selected tile must match the anchor's flat elevation `target_height`.
    let h = lot.heights[cell];
    if h[0] != target_height
        || h[1] != target_height
        || h[2] != target_height
        || h[3] != target_height
    {
        return WaterTileStatus::InvalidSlope;
    }

    if tile_has_any_wall(lot, tile) {
        return WaterTileStatus::BlockedByWall;
    }

    if !is_coping {
        if floor != 0 {
            return WaterTileStatus::BlockedByFloor;
        }
        if world.is_some_and(|w| match tool {
            WaterTool::Pool => has_water_mask(w, WATER_ARRAY_ID, tx, ty),
            WaterTool::Pond => has_water_mask(w, POOL_ARRAY_ID, tx, ty),
        }) {
            return WaterTileStatus::BlockedByWater;
        }
        if world.is_some_and(|w| tile_has_blocking_object(w, tile)) {
            return WaterTileStatus::BlockedByObject;
        }
    } else if world.is_some_and(|w| has_water_mask(w, WATER_ARRAY_ID, tx, ty)) {
        return WaterTileStatus::BlockedByWater;
    }

    WaterTileStatus::Valid
}

/// Composites a validated [`WaterPreviewPlan`] depth-correctly over `canvas`.
pub fn draw_water_overlay(
    lot: &Lot,
    camera: &Camera,
    plan: &WaterPreviewPlan,
    canvas: &mut Canvas,
) -> OverlayStats {
    let depth = Depth::new(camera);
    let mut stats = OverlayStats::default();

    // 1. Draw pool-border coping tiles first so the inner water rim can frame
    //    cleanly against them.
    for coping in &plan.coping_tiles {
        if !lot.contains(coping.tile[0], coping.tile[1]) {
            continue;
        }
        let corners =
            sloped_tile_vertices(lot, camera, &depth, coping.tile, GROUND_OVERLAY_DEPTH_BIAS);
        let ok = coping.status.is_valid();
        let (fill, edge, alpha) = if ok {
            // Warm Complete Collection pool-deck limestone coping.
            ([218, 212, 198], [244, 240, 232], 172u8)
        } else {
            stats.blocked_tiles += 1;
            ([220, 56, 56], [255, 148, 148], 150u8)
        };
        // Draw the coping strip biased toward the side bordering the pool.
        let strip = coping_strip_quad(corners, coping.edge_mask);
        stats.pixels_written += raster_quad_blended(canvas, strip, fill, alpha, false);
        stats.pixels_written += draw_quad_outline(canvas, strip, edge, 210);
        stats.tiles_drawn += 1;
    }

    // 2. Draw interior pool or pond water tiles.
    for entry in &plan.tiles {
        if !lot.contains(entry.tile[0], entry.tile[1]) {
            continue;
        }
        let corners =
            sloped_tile_vertices(lot, camera, &depth, entry.tile, GROUND_OVERLAY_DEPTH_BIAS);
        let ok = entry.status.is_valid() && plan.valid;
        let (fill, edge, alpha) = match (plan.tool, plan.action, ok) {
            (_, WaterAction::Remove, true) => ([204, 176, 112], [248, 224, 160], 132u8),
            (WaterTool::Pool, WaterAction::Dig, true) => {
                // Bright Complete Collection swimming-pool turquoise.
                ([44, 176, 232], [144, 232, 255], 168u8)
            }
            (WaterTool::Pond, WaterAction::Dig, true) => {
                // Deeper natural pond teal-blue.
                ([36, 112, 152], [92, 168, 196], 168u8)
            }
            (_, _, false) => {
                stats.blocked_tiles += 1;
                if entry.status == WaterTileStatus::InvalidSlope {
                    // Distinct amber-red highlight for invalid-slope tiles.
                    ([232, 88, 40], [255, 180, 120], 160u8)
                } else {
                    ([224, 48, 48], [255, 136, 136], 160u8)
                }
            }
        };
        stats.pixels_written += raster_quad_blended(canvas, corners, fill, alpha, false);
        stats.pixels_written += draw_quad_outline(canvas, corners, edge, 196);
        stats.tiles_drawn += 1;
    }

    stats
}

/// Convenience helper that validates and draws a [`WaterOverlay`] in one call.
pub fn draw_water_preview(
    lot: &Lot,
    world: Option<&World>,
    camera: &Camera,
    overlay: &WaterOverlay,
    canvas: &mut Canvas,
) -> (WaterPreviewPlan, OverlayStats) {
    let plan = validate_water_overlay(lot, world, overlay);
    let stats = draw_water_overlay(lot, camera, &plan, canvas);
    (plan, stats)
}

/// Narrows a coping border tile quad toward the edge(s) touching the pool so
/// both straight coping edges and the four corner stones meet seamlessly around
/// the pool perimeter.
fn coping_strip_quad(corners: [ScreenVertex; 4], edge_mask: u8) -> [ScreenVertex; 4] {
    // corners order: [0]=(0,0), [1]=(1,0), [2]=(1,1), [3]=(0,1)
    let bilerp = |u: f32, v: f32| {
        let top_x = corners[0].x + (corners[1].x - corners[0].x) * u;
        let top_y = corners[0].y + (corners[1].y - corners[0].y) * u;
        let top_z = corners[0].z + (corners[1].z - corners[0].z) * u;
        let bot_x = corners[3].x + (corners[2].x - corners[3].x) * u;
        let bot_y = corners[3].y + (corners[2].y - corners[3].y) * u;
        let bot_z = corners[3].z + (corners[2].z - corners[3].z) * u;
        ScreenVertex {
            x: top_x + (bot_x - top_x) * v,
            y: top_y + (bot_y - top_y) * v,
            z: top_z + (bot_z - top_z) * v,
        }
    };
    const RIM: f32 = 0.45;
    let (u0, u1) = if edge_mask & COPING_WEST != 0 {
        (0.0, RIM)
    } else if edge_mask & COPING_EAST != 0 {
        (1.0 - RIM, 1.0)
    } else {
        (0.0, 1.0)
    };
    let (v0, v1) = if edge_mask & COPING_NORTH != 0 {
        (0.0, RIM)
    } else if edge_mask & COPING_SOUTH != 0 {
        (1.0 - RIM, 1.0)
    } else {
        (0.0, 1.0)
    };
    [
        bilerp(u0, v0),
        bilerp(u1, v0),
        bilerp(u1, v1),
        bilerp(u0, v1),
    ]
}

// ============================================================================
// 3. Roof Style & Pitch Catalog + Isometric Roof Preview Renderer
// ============================================================================

/// Complete Collection roof pitch presets (`cWinRoofPanel` `0x529d31`–`0x529d9d`,
/// `0x52a11b`–`0x52a1f2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RoofPitch {
    /// Flat roof (`0.0`, `0x529d4b` `fldz`).
    Flat,
    /// Shallow pitch (`0.4`, `0x529d93` `[0x666a44]`).
    Shallow,
    /// Medium pitch (`0.6`, `0x529d8b` `[0x666a40]`, default in `0x56dfc6`).
    #[default]
    Medium,
    /// Steep pitch (`1.0`, `0x529d87` `fld1`).
    Steep,
}

impl RoofPitch {
    pub const ALL: [RoofPitch; 4] = [Self::Flat, Self::Shallow, Self::Medium, Self::Steep];

    /// Native slope multiplier passed to `cRoofLayer::SetPitch` (`0x56f0e9`).
    pub const fn slope(self) -> f32 {
        match self {
            Self::Flat => 0.0,
            Self::Shallow => 0.4,
            Self::Medium => 0.6,
            Self::Steep => 1.0,
        }
    }

    /// Clamps any custom slope to `Sims.exe`'s native `[0.0, 2.0]` range
    /// (`0x56f0fc`–`0x56f120`).
    pub fn clamp_slope(slope: f32) -> f32 {
        if slope.is_finite() {
            slope.clamp(0.0, MAX_ROOF_SLOPE)
        } else {
            Self::Medium.slope()
        }
    }

    /// Maps a floating-point slope back to the highlighted `cWinRoofPanel`
    /// button (`0x52a121`–`0x52a1e7`).
    pub fn from_slope(slope: f32) -> Self {
        const EPS: f32 = 1e-6;
        if !slope.is_finite() {
            return Self::Medium;
        }
        if (slope - 0.4).abs() <= EPS {
            Self::Shallow
        } else if (slope - 0.6).abs() <= EPS {
            Self::Medium
        } else if slope < 0.1 {
            Self::Flat
        } else {
            Self::Steep
        }
    }
}

/// Isometric roof geometry styles supported by the Build Mode roof preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RoofStyle {
    /// Native `Sims.exe` 4-way hipped roof rising symmetrically from all eaves.
    #[default]
    Hip,
    /// Peaked gable roof running along the footprint's longer axis.
    Gable,
    /// Steep outer mansard skirt with a shallow upper crown.
    Mansard,
    /// Flat parapet roof with raised perimeter trim.
    Flat,
}

impl RoofStyle {
    pub const ALL: [RoofStyle; 4] = [Self::Hip, Self::Gable, Self::Mansard, Self::Flat];
}

/// One decoded roof texture pattern from `GameData/Roofs/*.bmp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoofPattern {
    /// Stem name (e.g. `"r1_"`, `"SuperstarGoldRoof"`), matching `HOUS` roof
    /// strings and `0x6ceabc`.
    pub name: String,
    /// Original BMP filename (e.g. `"r1_.bmp"`).
    pub file_name: String,
    pub width: u32,
    pub height: u32,
    /// Top-down straight RGBA bytes (`width * height * 4`).
    pub rgba: Vec<u8>,
}

impl RoofPattern {
    /// Constructs a solid-color fallback pattern (useful in unit tests or when
    /// a custom install lacks a requested roof BMP).
    pub fn solid(name: impl Into<String>, rgb: [u8; 3]) -> Self {
        let name = name.into();
        let file_name = format!("{name}.bmp");
        let mut rgba = Vec::with_capacity(32 * 32 * 4);
        for _ in 0..(32 * 32) {
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        Self {
            name,
            file_name,
            width: 32,
            height: 32,
            rgba,
        }
    }

    /// Wraps and samples `(u, v)` texture coordinates from this roof pattern.
    #[inline]
    pub fn sample_rgb(&self, u: f32, v: f32) -> [u8; 3] {
        if self.width == 0
            || self.height == 0
            || self.rgba.len() < 4
            || !u.is_finite()
            || !v.is_finite()
        {
            return [128, 128, 128];
        }
        let u_wrap = u - u.floor();
        let v_wrap = v - v.floor();
        let tx = ((u_wrap * self.width as f32) as usize).min(self.width as usize - 1);
        let ty = ((v_wrap * self.height as f32) as usize).min(self.height as usize - 1);
        let Some(idx) = ty
            .checked_mul(self.width as usize)
            .and_then(|row| row.checked_add(tx))
            .and_then(|pixel| pixel.checked_mul(4))
        else {
            return [128, 128, 128];
        };
        if let Some(slice) = self.rgba.get(idx..idx + 3) {
            [slice[0], slice[1], slice[2]]
        } else {
            [128, 128, 128]
        }
    }
}

/// Complete Collection `GameData/Roofs/*.bmp` catalog (`0x4153c4`).
#[derive(Debug, Clone, Default)]
pub struct RoofCatalog {
    patterns: Vec<RoofPattern>,
    by_key: BTreeMap<String, usize>,
}

impl RoofCatalog {
    /// Loads every `*.bmp` roof texture from `install`'s `GameData/Roofs/`
    /// directory (`0x4153c4`).
    pub fn load(install: &Install) -> Result<Self> {
        let dir = install.file("GameData/Roofs")?;
        Self::from_dir(&dir)
    }

    /// Loads every `*.bmp` roof texture from `dir` in deterministic order
    /// (native base patterns `r1_`..`r4_` first, followed by remaining patterns
    /// sorted ASCII case-insensitively).
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let paths = sorted_bmp_paths(dir)?;
        let mut patterns = Vec::with_capacity(paths.len());
        for path in paths {
            let bytes =
                std::fs::read(&path).map_err(|e| Error::new(format!("{}: {e}", path.display())))?;
            let image =
                bitmap::bmp(&bytes).map_err(|e| Error::new(format!("{}: {e}", path.display())))?;
            let file_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| Error::new(format!("{}: non-UTF-8 filename", path.display())))?
                .to_owned();
            let name = strip_bmp_ext(&file_name).to_owned();
            patterns.push(RoofPattern {
                name,
                file_name,
                width: image.width,
                height: image.height,
                rgba: image.rgba,
            });
        }
        patterns.sort_by(|a, b| {
            let rank = |name: &str| match name.to_ascii_lowercase().as_str() {
                "r1_" => 0u8,
                "r2_" => 1,
                "r3_" => 2,
                "r4_" => 3,
                "gravel1" => 4,
                "tar1" => 5,
                _ => 6,
            };
            rank(&a.name)
                .cmp(&rank(&b.name))
                .then_with(|| {
                    a.name
                        .to_ascii_lowercase()
                        .cmp(&b.name.to_ascii_lowercase())
                })
                .then_with(|| a.name.cmp(&b.name))
        });
        let mut by_key = BTreeMap::new();
        for (i, p) in patterns.iter().enumerate() {
            by_key.insert(p.name.to_ascii_lowercase(), i);
        }
        Ok(Self { patterns, by_key })
    }

    /// Looks up a roof pattern by stem or filename (case-insensitive, optional
    /// `.bmp` suffix).
    pub fn get(&self, name: &str) -> Option<&RoofPattern> {
        let key = strip_bmp_ext(name.trim()).to_ascii_lowercase();
        let &idx = self.by_key.get(&key)?;
        self.patterns.get(idx)
    }

    /// Returns `"r1_"` (`0x6ceabc`) if present, otherwise the first loaded
    /// pattern.
    pub fn default_pattern(&self) -> Option<&RoofPattern> {
        self.get(DEFAULT_ROOF_PATTERN)
            .or_else(|| self.patterns.first())
    }

    pub fn patterns(&self) -> &[RoofPattern] {
        &self.patterns
    }

    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}

#[allow(clippy::disallowed_methods)]
fn sorted_bmp_paths(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| Error::new(format!("{}: {e}", dir.display())))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Error::new(format!("{}: {e}", dir.display())))?;
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("bmp"))
        {
            paths.push(path);
        }
    }
    paths.sort_by(|a, b| {
        let ka = a
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let kb = b
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        ka.cmp(&kb).then_with(|| a.cmp(b))
    });
    Ok(paths)
}

fn strip_bmp_ext(name: &str) -> &str {
    let bytes = name.as_bytes();
    if bytes.len() >= 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".bmp") {
        // Trailing 4 bytes are ASCII (`.bmp`), so `len - 4` is always a UTF-8 char boundary.
        &name[..name.len() - 4]
    } else {
        name
    }
}

/// Configuration for rendering an isometric roof preview on a lot.
#[derive(Debug, Clone, PartialEq)]
pub struct RoofPreviewConfig {
    /// Roof pattern stem (e.g. `"r1_"`). Used by [`draw_roof_preview_from_catalog`].
    pub pattern_name: String,
    pub style: RoofStyle,
    pub pitch: RoofPitch,
    /// Optional custom pitch slope overriding [`RoofPreviewConfig::pitch`],
    /// clamped to `0.0..=2.0` (`0x56f0e9`).
    pub custom_slope: Option<f32>,
    /// Optional explicit storey level (`1` or `2`). When `None`, roofs are
    /// generated automatically above every enclosed storey (`1` and `2`).
    pub level: Option<i32>,
    /// Optional explicit tile footprint. When `None`, enclosed indoor rooms
    /// are detected automatically from `lot`.
    pub footprint: Option<Vec<[i32; 2]>>,
}

impl Default for RoofPreviewConfig {
    fn default() -> Self {
        Self {
            pattern_name: DEFAULT_ROOF_PATTERN.to_owned(),
            style: RoofStyle::Hip,
            pitch: RoofPitch::Medium,
            custom_slope: None,
            level: None,
            footprint: None,
        }
    }
}

impl RoofPreviewConfig {
    /// Effective slope multiplier (`0.0..=2.0`), respecting `RoofStyle::Flat`.
    pub fn effective_slope(&self) -> f32 {
        if self.style == RoofStyle::Flat {
            0.0
        } else {
            match self.custom_slope {
                Some(s) => RoofPitch::clamp_slope(s),
                None => self.pitch.slope(),
            }
        }
    }
}

/// Statistics returned by [`draw_roof_preview`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoofStats {
    pub footprint_tiles: usize,
    pub triangles_drawn: usize,
    pub pixels_written: usize,
}

/// Detects enclosed indoor tiles eligible for roofing on level 1 and level 2,
/// including diagonally split tiles (`Lot::halves`) that hold an indoor half.
///
/// Level-1 tiles covered by a level-2 enclosed tile (or level-2 floor) are
/// excluded from the level-1 roof footprint so multi-storey houses do not draw
/// a redundant roof through the second storey.
pub fn detect_roof_footprints(lot: &Lot) -> [Vec<[i32; 2]>; 2] {
    let mut level2_set = BTreeSet::new();
    let mut level1_set = BTreeSet::new();

    for level_idx in [1usize, 0usize] {
        let level = &lot.levels[level_idx];
        for x in 1..lot.size.saturating_sub(1) {
            for y in 1..lot.size.saturating_sub(1) {
                let cell = Lot::cell(x, y);
                let is_indoor = if let Some(rooms) = &level.rooms {
                    let room = rooms[cell];
                    if room >= 0 {
                        !lot.outdoor_rooms.contains(&room)
                    } else if room == sims_sim::rooms::NO_ROOM {
                        lot.halves
                            .get(&[level_idx as u8 + 1, x as u8, y as u8])
                            .is_some_and(|halves| {
                                halves
                                    .iter()
                                    .any(|&(_, r)| r >= 0 && !lot.outdoor_rooms.contains(&r))
                            })
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !is_indoor {
                    continue;
                }
                let tile = [x as i32, y as i32];
                if level_idx == 1 {
                    level2_set.insert(tile);
                } else if !level2_set.contains(&tile) && lot.levels[1].floors[cell] == 0 {
                    level1_set.insert(tile);
                }
            }
        }
    }

    [
        level1_set.into_iter().collect(),
        level2_set.into_iter().collect(),
    ]
}

/// Renders an isometric roof preview onto `canvas` using `pattern` and `config`,
/// matching `Sims.exe`'s roof pitch scale (`TILE_WORLD_UNITS = 3.0`, half-tile
/// sub-grid `0.5`) and house camera depth space ([`Depth::point`]).
pub fn draw_roof_preview(
    lot: &Lot,
    camera: &Camera,
    pattern: &RoofPattern,
    config: &RoofPreviewConfig,
    canvas: &mut Canvas,
) -> RoofStats {
    let depth = Depth::new(camera);
    let slope = config.effective_slope();
    let sun = roof_sun_direction(lot.time_of_day);
    let brightest = lot.brightest();
    let mut stats = RoofStats::default();

    let layers: Vec<(i32, Vec<[i32; 2]>)> = if let Some(explicit) = &config.footprint {
        let level = config.level.unwrap_or(1).clamp(1, 2);
        vec![(level, explicit.clone())]
    } else {
        let [l1, l2] = detect_roof_footprints(lot);
        match config.level {
            Some(1) => vec![(1, l1)],
            Some(2) => vec![(2, l2)],
            _ => vec![(1, l1), (2, l2)],
        }
    };

    for (base_level, tiles) in layers {
        if tiles.is_empty() {
            continue;
        }
        let tile_set: BTreeSet<[i32; 2]> = tiles
            .into_iter()
            .filter(|&[x, y]| lot.contains(x, y))
            .collect();
        if tile_set.is_empty() {
            continue;
        }
        stats.footprint_tiles += tile_set.len();
        draw_roof_layer(
            lot,
            camera,
            &depth,
            pattern,
            config.style,
            slope,
            base_level,
            &tile_set,
            sun,
            brightest,
            canvas,
            &mut stats,
        );
    }

    stats
}

/// Convenience wrapper looking up `config.pattern_name` in `catalog` (falling
/// back to [`RoofCatalog::default_pattern`]).
pub fn draw_roof_preview_from_catalog(
    lot: &Lot,
    camera: &Camera,
    catalog: &RoofCatalog,
    config: &RoofPreviewConfig,
    canvas: &mut Canvas,
) -> Result<RoofStats> {
    let pattern = catalog
        .get(&config.pattern_name)
        .or_else(|| catalog.default_pattern())
        .ok_or_else(|| Error::new("roof catalog is empty"))?;
    Ok(draw_roof_preview(lot, camera, pattern, config, canvas))
}

#[allow(clippy::too_many_arguments)]
fn draw_roof_layer(
    lot: &Lot,
    camera: &Camera,
    depth: &Depth,
    pattern: &RoofPattern,
    style: RoofStyle,
    slope: f32,
    base_level: i32,
    tile_set: &BTreeSet<[i32; 2]>,
    sun: [f32; 3],
    brightest: f32,
    canvas: &mut Canvas,
    stats: &mut RoofStats,
) {
    // Build a half-tile (2x resolution) grid matching `cRoofMeshFactory`
    // (`0.5` tile steps at `0x56f7fa`), so even 1-tile wide rooms have a crisp
    // center ridge vertex.
    let mut min_tx = i32::MAX;
    let mut min_ty = i32::MAX;
    let mut max_tx = i32::MIN;
    let mut max_ty = i32::MIN;
    for &[tx, ty] in tile_set {
        min_tx = min_tx.min(tx);
        min_ty = min_ty.min(ty);
        max_tx = max_tx.max(tx);
        max_ty = max_ty.max(ty);
    }

    // Half-tile cell coordinates span `[0..w_sub] x [0..h_sub]` where each lot
    // tile `(tx, ty)` covers `2 x 2` half-cells.
    let w_sub = ((max_tx - min_tx + 1) * 2) as usize;
    let h_sub = ((max_ty - min_ty + 1) * 2) as usize;
    let vw = w_sub + 1;
    let vh = h_sub + 1;

    // Determine exact triangle coverage for each half-tile sub-cell.
    // Non-split tiles cover both triangles (`0b11`) with automatic ridge split.
    // Split-diagonal tiles (`NO_ROOM` + `Lot::halves`) force the sub-cell split
    // along the tile's diagonal wall (`ASCENDING` `(0,0)-(1,1)` or `DESCENDING`
    // `(1,0)-(0,1)`) and test each triangle's exact centroid against
    // `Lot::room_at`, giving exact diagonal triangle clipping with zero
    // stair-stepping along 45° walls.
    #[derive(Clone, Copy)]
    enum SubCell {
        Empty,
        Full,
        /// Forced `(0,0)-(1,1)` split: bit 0 = `(v00,v10,v11)`, bit 1 = `(v00,v11,v01)`.
        Split0011(u8),
        /// Forced `(1,0)-(0,1)` split: bit 0 = `(v00,v10,v01)`, bit 1 = `(v10,v11,v01)`.
        Split1001(u8),
    }
    impl SubCell {
        fn is_full(self) -> bool {
            matches!(self, Self::Full) || matches!(self, Self::Split0011(3) | Self::Split1001(3))
        }
        fn is_empty(self) -> bool {
            matches!(self, Self::Empty) || matches!(self, Self::Split0011(0) | Self::Split1001(0))
        }
        /// Whether any retained triangle in this half-tile sub-cell owns the
        /// cardinal boundary edge facing `(dx, dy)` (`(-1,0)`, `(1,0)`,
        /// `(0,-1)`, or `(0,1)`).
        fn owns_edge(self, dx: isize, dy: isize) -> bool {
            match self {
                Self::Empty => false,
                Self::Full => true,
                // bit 0 = (v00,v10,v11) owns -y (0,-1) and +x (1,0);
                // bit 1 = (v00,v11,v01) owns -x (-1,0) and +y (0,1).
                Self::Split0011(mask) => match (dx, dy) {
                    (0, -1) | (1, 0) => (mask & 1) != 0,
                    (-1, 0) | (0, 1) => (mask & 2) != 0,
                    _ => false,
                },
                // bit 0 = (v00,v10,v01) owns -y (0,-1) and -x (-1,0);
                // bit 1 = (v10,v11,v01) owns +x (1,0) and +y (0,1).
                Self::Split1001(mask) => match (dx, dy) {
                    (0, -1) | (-1, 0) => (mask & 1) != 0,
                    (1, 0) | (0, 1) => (mask & 2) != 0,
                    _ => false,
                },
            }
        }
    }
    let level_idx = (base_level - 1).clamp(0, 1) as usize;
    let mut sub_cells = vec![SubCell::Empty; w_sub * h_sub];
    for &[tx, ty] in tile_set {
        let sx0 = ((tx - min_tx) * 2) as usize;
        let sy0 = ((ty - min_ty) * 2) as usize;
        let cell = Lot::cell(tx as usize, ty as usize);
        let split_halves = lot.levels[level_idx]
            .rooms
            .as_ref()
            .is_some_and(|r| r[cell] == sims_sim::rooms::NO_ROOM)
            .then(|| lot.halves.get(&[base_level as u8, tx as u8, ty as u8]))
            .flatten();
        let is_indoor_fine = |fx: i32, fy: i32| {
            lot.room_at(level_idx, [tx * 16 + fx, ty * 16 + fy])
                .is_some_and(|r| !lot.outdoor_rooms.contains(&r))
        };
        for dx in 0..2 {
            for dy in 0..2 {
                let sub = if let Some(halves) = split_halves {
                    // Triangles 1 (`fy > fx`) and 3 (`fx > fy`) split along
                    // ascending `(0,0)-(1,1)`; triangles 2 (`fx + fy > 16`)
                    // and 4 (`fx + fy < 16`) split along descending `(1,0)-(0,1)`.
                    let ascending = halves.iter().any(|&(t, _)| t == 1 || t == 3);
                    let ox = dx as i32 * 8;
                    let oy = dy as i32 * 8;
                    if ascending {
                        // Tri 0: (0,0)-(8,0)-(8,8), centroid ≈ (5, 3) -> fx > fy
                        // Tri 1: (0,0)-(8,8)-(0,8), centroid ≈ (3, 5) -> fy > fx
                        let mut mask = 0u8;
                        if is_indoor_fine(ox + 5, oy + 3) {
                            mask |= 1;
                        }
                        if is_indoor_fine(ox + 3, oy + 5) {
                            mask |= 2;
                        }
                        if mask == 0 {
                            SubCell::Empty
                        } else if mask == 3 {
                            SubCell::Full
                        } else {
                            SubCell::Split0011(mask)
                        }
                    } else {
                        // Tri 0: (0,0)-(8,0)-(0,8), centroid ≈ (3, 3) -> fx + fy < 8
                        // Tri 1: (8,0)-(8,8)-(0,8), centroid ≈ (5, 5) -> fx + fy > 8
                        let mut mask = 0u8;
                        if is_indoor_fine(ox + 3, oy + 3) {
                            mask |= 1;
                        }
                        if is_indoor_fine(ox + 5, oy + 5) {
                            mask |= 2;
                        }
                        if mask == 0 {
                            SubCell::Empty
                        } else if mask == 3 {
                            SubCell::Full
                        } else {
                            SubCell::Split1001(mask)
                        }
                    }
                } else {
                    SubCell::Full
                };
                sub_cells[(sx0 + dx) * h_sub + (sy0 + dy)] = sub;
            }
        }
    }

    // Roof rests on top of `base_level`'s walls (`roof_level = base_level + 1`).
    let roof_level = base_level + 1;

    // Compute per-vertex roof height in world units above the eaves.
    let gable_along_x = (max_tx - min_tx) >= (max_ty - min_ty);
    let mut vertex_world_h = vec![0.0f32; vw * vh];
    let v_idx = |vx: usize, vy: usize| vx * vh + vy;

    // Compute Chebyshev distance (in half-tile steps) from the boundary of the
    // roofed region for every vertex. A vertex touching any non-full (empty or
    // diagonally clipped) sub-cell sits on an eave (`dist = 0`).
    let mut dist = vec![0i32; vw * vh];
    let cell_full = |cx: isize, cy: isize| -> bool {
        cx >= 0
            && cy >= 0
            && (cx as usize) < w_sub
            && (cy as usize) < h_sub
            && sub_cells[cx as usize * h_sub + cy as usize].is_full()
    };
    let cell_owns_edge = |cx: isize, cy: isize, dx: isize, dy: isize| -> bool {
        cx >= 0
            && cy >= 0
            && (cx as usize) < w_sub
            && (cy as usize) < h_sub
            && sub_cells[cx as usize * h_sub + cy as usize].owns_edge(dx, dy)
    };
    let mut queue = std::collections::VecDeque::new();
    for vx in 0..vw {
        for vy in 0..vh {
            let ix = vx as isize;
            let iy = vy as isize;
            let all_four = cell_full(ix - 1, iy - 1)
                && cell_full(ix, iy - 1)
                && cell_full(ix - 1, iy)
                && cell_full(ix, iy);
            if !all_four {
                dist[v_idx(vx, vy)] = 0;
                queue.push_back((vx, vy));
            } else {
                dist[v_idx(vx, vy)] = i32::MAX;
            }
        }
    }
    while let Some((vx, vy)) = queue.pop_front() {
        let d0 = dist[v_idx(vx, vy)];
        for dx in -1isize..=1 {
            for dy in -1isize..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let nx = vx as isize + dx;
                let ny = vy as isize + dy;
                if nx >= 0 && ny >= 0 && (nx as usize) < vw && (ny as usize) < vh {
                    let ni = v_idx(nx as usize, ny as usize);
                    if dist[ni] > d0 + 1 {
                        dist[ni] = d0 + 1;
                        queue.push_back((nx as usize, ny as usize));
                    }
                }
            }
        }
    }

    // For gable roofs, compute distance strictly perpendicular to the ridge axis.
    let mut gable_dist = vec![0i32; vw * vh];
    if style == RoofStyle::Gable {
        let mut gq = std::collections::VecDeque::new();
        for vx in 0..vw {
            for vy in 0..vh {
                let ix = vx as isize;
                let iy = vy as isize;
                let on_eave = if gable_along_x {
                    // Ridge runs along X -> eaves are along Y boundaries.
                    // A vertex is off the eave only when at least one side along X
                    // has both Y-adjacent sub-cells meeting cleanly across the Y edge.
                    let left_closed =
                        cell_owns_edge(ix - 1, iy - 1, 0, 1) && cell_owns_edge(ix - 1, iy, 0, -1);
                    let right_closed =
                        cell_owns_edge(ix, iy - 1, 0, 1) && cell_owns_edge(ix, iy, 0, -1);
                    !(left_closed || right_closed)
                } else {
                    // Ridge runs along Y -> eaves are along X boundaries.
                    let top_closed =
                        cell_owns_edge(ix - 1, iy - 1, 1, 0) && cell_owns_edge(ix, iy - 1, -1, 0);
                    let bot_closed =
                        cell_owns_edge(ix - 1, iy, 1, 0) && cell_owns_edge(ix, iy, -1, 0);
                    !(top_closed || bot_closed)
                };
                if on_eave {
                    gable_dist[v_idx(vx, vy)] = 0;
                    gq.push_back((vx, vy));
                } else {
                    gable_dist[v_idx(vx, vy)] = i32::MAX;
                }
            }
        }
        let dirs: [(isize, isize); 2] = if gable_along_x {
            [(0, -1), (0, 1)]
        } else {
            [(-1, 0), (1, 0)]
        };
        while let Some((vx, vy)) = gq.pop_front() {
            let d0 = gable_dist[v_idx(vx, vy)];
            for (dx, dy) in dirs {
                let nx = vx as isize + dx;
                let ny = vy as isize + dy;
                if nx >= 0 && ny >= 0 && (nx as usize) < vw && (ny as usize) < vh {
                    let ni = v_idx(nx as usize, ny as usize);
                    if gable_dist[ni] > d0 + 1 {
                        gable_dist[ni] = d0 + 1;
                        gq.push_back((nx as usize, ny as usize));
                    }
                }
            }
        }
    }

    for vx in 0..vw {
        for vy in 0..vh {
            let i = v_idx(vx, vy);
            let d = dist[i].max(0) as f32;
            vertex_world_h[i] = match style {
                RoofStyle::Hip => d * HALF_TILE_WORLD_UNITS * slope,
                RoofStyle::Gable => {
                    let gd = gable_dist[i].clamp(0, 64) as f32;
                    gd * HALF_TILE_WORLD_UNITS * slope
                }
                RoofStyle::Mansard => {
                    // Steep outer skirt for the first full tile (2 half-steps),
                    // then a shallow upper crown.
                    let skirt =
                        d.min(2.0) * HALF_TILE_WORLD_UNITS * (slope * 1.4).min(MAX_ROOF_SLOPE);
                    let crown = (d - 2.0).max(0.0) * HALF_TILE_WORLD_UNITS * (slope * 0.15);
                    skirt + crown
                }
                RoofStyle::Flat => {
                    // Slight raised parapet rim around perimeter vertices.
                    if d == 0.0 { 0.25 } else { 0.0 }
                }
            };
        }
    }

    // Project a half-tile vertex `(vx, vy)` anchored to its supporting terrain
    // height (`0x56ed12`–`0x56ed41`), then add the pitch increment
    // (`0x56ee4b`–`0x56ee73`), so rooms on separate terraces get accurate eaves.
    let make_vertex = |vx: usize, vy: usize, world_h: f32| -> RoofVertex {
        let fine_ix = min_tx * 16 + vx as i32 * 8;
        let fine_iy = min_ty * 16 + vy as i32 * 8;
        let max_fine = (lot.size as i32 * 16).saturating_sub(1).max(0);
        let terrain_steps =
            lot.altitude_at([fine_ix.clamp(0, max_fine), fine_iy.clamp(0, max_fine)]);
        let view = rotate_fine_f32(camera.rotation, [fine_ix as f32, fine_iy as f32]);
        let elev_near = terrain_steps * 16.0 + world_h * NEAR_PX_PER_WORLD_UNIT;
        let [sx, sy] = camera.project_fine_f32(view, roof_level, elev_near);
        let z = depth.point(view, elev_near, roof_level);
        RoofVertex {
            sx,
            sy,
            z,
            // World position in view space for directional sun lighting.
            wx: view[0] * (TILE_WORLD_UNITS / 16.0),
            wy: view[1] * (TILE_WORLD_UNITS / 16.0),
            wz: terrain_steps * (16.0 / NEAR_PX_PER_WORLD_UNIT) + world_h,
            u: (vx as f32) * 0.5,
            v: (vy as f32) * 0.5,
        }
    };

    // Emit and rasterize up to 2 triangles per covered half-tile cell.
    for sx in 0..w_sub {
        for sy in 0..h_sub {
            let cell_kind = sub_cells[sx * h_sub + sy];
            if cell_kind.is_empty() {
                continue;
            }
            let h00 = vertex_world_h[v_idx(sx, sy)];
            let h10 = vertex_world_h[v_idx(sx + 1, sy)];
            let h11 = vertex_world_h[v_idx(sx + 1, sy + 1)];
            let h01 = vertex_world_h[v_idx(sx, sy + 1)];

            let v00 = make_vertex(sx, sy, h00);
            let v10 = make_vertex(sx + 1, sy, h10);
            let v11 = make_vertex(sx + 1, sy + 1, h11);
            let v01 = make_vertex(sx, sy + 1, h01);

            let (tris, tri_mask) = match cell_kind {
                SubCell::Split0011(mask) => ([[v00, v10, v11], [v00, v11, v01]], mask),
                SubCell::Split1001(mask) => ([[v00, v10, v01], [v10, v11, v01]], mask),
                SubCell::Full => {
                    // Split along the diagonal that preserves crisp hip ridges.
                    let split_00_11 = (h00 - h11).abs() <= (h10 - h01).abs();
                    if split_00_11 {
                        ([[v00, v10, v11], [v00, v11, v01]], 0b11)
                    } else {
                        ([[v00, v10, v01], [v10, v11, v01]], 0b11)
                    }
                }
                SubCell::Empty => continue,
            };

            for (ti, tri) in tris.into_iter().enumerate() {
                if (tri_mask & (1 << ti)) == 0 {
                    continue;
                }
                let shade = triangle_roof_shade(tri, sun, brightest);
                stats.pixels_written += raster_roof_triangle(canvas, tri, pattern, shade);
                stats.triangles_drawn += 1;
            }

            // Close vertical gable-end walls where a Gable roof terminates
            // above the eaves at an exterior boundary.
            if style == RoofStyle::Gable {
                let neighbors = [
                    ((-1isize, 0isize), (sx, sy, h00), (sx, sy + 1, h01)),
                    ((1, 0), (sx + 1, sy, h10), (sx + 1, sy + 1, h11)),
                    ((0, -1), (sx, sy, h00), (sx + 1, sy, h10)),
                    ((0, 1), (sx, sy + 1, h01), (sx + 1, sy + 1, h11)),
                ];
                for ((dx, dy), (ax, ay, ha), (bx, by, hb)) in neighbors {
                    if cell_kind.owns_edge(dx, dy)
                        && !cell_owns_edge(sx as isize + dx, sy as isize + dy, -dx, -dy)
                        && (ha > 0.01 || hb > 0.01)
                    {
                        let top_a = make_vertex(ax, ay, ha);
                        let top_b = make_vertex(bx, by, hb);
                        let bot_a = make_vertex(ax, ay, 0.0);
                        let bot_b = make_vertex(bx, by, 0.0);
                        let gable_shade = (0.72 * brightest).clamp(0.25, 1.0);
                        if ha > 0.01 {
                            stats.pixels_written += raster_roof_triangle(
                                canvas,
                                [bot_a, top_a, top_b],
                                pattern,
                                gable_shade,
                            );
                            stats.triangles_drawn += 1;
                        }
                        if hb > 0.01 {
                            stats.pixels_written += raster_roof_triangle(
                                canvas,
                                [bot_a, top_b, bot_b],
                                pattern,
                                gable_shade,
                            );
                            stats.triangles_drawn += 1;
                        }
                    }
                }
            }
        }
    }
}

/// Sun direction in view space matching `grass::light_table_at` (`+x` azimuth,
/// time-of-day zenith `0.1π / 0.2π / 0.3π`).
fn roof_sun_direction(time_of_day: i16) -> [f32; 3] {
    let tenths = match time_of_day {
        0 => 0.1,
        1 | 3 => 0.2,
        2 => 0.3,
        _ => 0.1,
    };
    let lat = f64::from(std::f32::consts::PI) * tenths;
    let s = libm::sin(lat) as f32;
    let c = libm::cos(lat) as f32;
    [s, 0.0, c]
}

fn triangle_roof_shade(tri: [RoofVertex; 3], sun: [f32; 3], brightest: f32) -> f32 {
    let ax = tri[1].wx - tri[0].wx;
    let ay = tri[1].wy - tri[0].wy;
    let az = tri[1].wz - tri[0].wz;
    let bx = tri[2].wx - tri[0].wx;
    let by = tri[2].wy - tri[0].wy;
    let bz = tri[2].wz - tri[0].wz;
    let mut nx = ay * bz - az * by;
    let mut ny = az * bx - ax * bz;
    let mut nz = ax * by - ay * bx;
    // Keep roof normals pointing skyward (`+z`).
    if nz < 0.0 {
        nx = -nx;
        ny = -ny;
        nz = -nz;
    }
    let len = (nx * nx + ny * ny + nz * nz).sqrt();
    if len <= 1e-6 {
        return (0.85 * brightest).clamp(0.2, 1.0);
    }
    let dot = (nx * sun[0] + ny * sun[1] + nz * sun[2]) / len;
    // Ambient floor + directional sun term matching Sims 1 daylight contrast.
    let diffuse = 0.42 + 0.58 * dot.clamp(0.0, 1.0);
    (diffuse * brightest).clamp(0.2, 1.0)
}

// ============================================================================
// Shared Geometry, Validation & Rasterization Helpers
// ============================================================================

#[derive(Debug, Clone, Copy)]
struct ScreenVertex {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Debug, Clone, Copy)]
struct RoofVertex {
    sx: f32,
    sy: f32,
    z: f32,
    wx: f32,
    wy: f32,
    wz: f32,
    u: f32,
    v: f32,
}

fn ordered_pair(a: i32, b: i32) -> (i32, i32) {
    if a <= b { (a, b) } else { (b, a) }
}

fn is_inner_tile(lot: &Lot, [x, y]: [i32; 2]) -> bool {
    x >= 1 && y >= 1 && x < lot.size as i32 - 1 && y < lot.size as i32 - 1
}

/// Reads the terrain height (in steps) at lot corner `[cx, cy]` in `0..=size`,
/// matching `Painter::corner_elevation` (`scene.rs:797`).
fn corner_height(lot: &Lot, [cx, cy]: [i32; 2]) -> u8 {
    let last = lot.size as i32 - 1;
    let tx = cx.clamp(0, last);
    let ty = cy.clamp(0, last);
    let h = lot.heights[Lot::cell(tx as usize, ty as usize)];
    let idx = match (cx - tx, cy - ty) {
        (0, 1) => 0,
        (0, 0) => 1,
        (1, 0) => 2,
        _ => 3,
    };
    h[idx]
}

fn tile_reference_height(lot: &Lot, [x, y]: [i32; 2]) -> u8 {
    let last = lot.size as i32 - 1;
    let tx = x.clamp(0, last) as usize;
    let ty = y.clamp(0, last) as usize;
    lot.heights[Lot::cell(tx, ty)][1]
}

fn vertex_exceeds_slope(lot: &Lot, [vx, vy]: [i32; 2], candidate_h: i32) -> bool {
    let size = lot.size as i32;
    if vx < 0 || vy < 0 || vx > size || vy > size {
        return false;
    }
    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
        let nx = vx + dx;
        let ny = vy + dy;
        if nx >= 0 && ny >= 0 && nx <= size && ny <= size {
            let nh = i32::from(corner_height(lot, [nx, ny]));
            if (candidate_h - nh).abs() > MAX_TERRAIN_SLOPE_DELTA {
                return true;
            }
        }
    }
    false
}

fn tile_has_any_wall(lot: &Lot, [x, y]: [i32; 2]) -> bool {
    if !lot.contains(x, y) {
        return false;
    }
    let cell = Lot::cell(x as usize, y as usize);
    if lot.levels[0].walls[cell].segments != 0 {
        return true;
    }
    // Check neighbor shared east/south edges (`WEST` on `x + 1`, `NORTH` on `y + 1`).
    if lot.contains(x + 1, y)
        && lot.levels[0].walls[Lot::cell((x + 1) as usize, y as usize)].segments & lot::WEST != 0
    {
        return true;
    }
    if lot.contains(x, y + 1)
        && lot.levels[0].walls[Lot::cell(x as usize, (y + 1) as usize)].segments & lot::NORTH != 0
    {
        return true;
    }
    false
}

/// Returns true when `world`'s pool (`Arry 9`) or water (`Arry 10`) array marks
/// tile `(tx, ty)` as excavated water/pool: native `cPoolLayer`/`cWaterLayer`
/// stores `0..16` (4-bit neighbor mask) for wet tiles and `0xFF` (`>= 16`) for
/// dry land (matching `sims-vm::build::is_pool_or_water_tile`).
fn has_water_mask(world: &World, array_id: u16, tx: usize, ty: usize) -> bool {
    let Some(arr) = world.arrays.get(&array_id) else {
        return false;
    };
    arr.stride == 1
        && arr
            .cell(tx, ty)
            .and_then(|bytes| bytes.first().copied())
            .is_some_and(|v| v < 16)
}

fn is_pool_or_water_tile(world: &World, tx: usize, ty: usize) -> bool {
    has_water_mask(world, POOL_ARRAY_ID, tx, ty) || has_water_mask(world, WATER_ARRAY_ID, tx, ty)
}

fn tile_has_blocking_object(world: &World, [tx, ty]: [i32; 2]) -> bool {
    world.objects.values().any(|o| {
        o.position[2] == 1
            && o.position[0] >= 0
            && o.position[1] >= 0
            && o.position[0] >> 4 == tx
            && o.position[1] >> 4 == ty
    })
}

fn rotate_corner(rotation: u8, [x, y]: [i32; 2]) -> [i32; 2] {
    let m = lot::FULL as i32;
    match rotation & 3 {
        1 => [m - y, x],
        2 => [m - x, m - y],
        3 => [y, m - x],
        _ => [x, y],
    }
}

fn rotate_fine_f32(rotation: u8, [x, y]: [f32; 2]) -> [f32; 2] {
    let m = (lot::FULL * 16) as f32;
    match rotation & 3 {
        1 => [m - y, x],
        2 => [m - x, m - y],
        3 => [y, m - x],
        _ => [x, y],
    }
}

fn project_corner_vertex(
    camera: &Camera,
    depth: &Depth,
    corner: [i32; 2],
    elev_near: f32,
    level: i32,
    depth_bias: i32,
) -> ScreenVertex {
    let view = rotate_corner(camera.rotation, corner);
    let fine = [view[0] as f32 * 16.0, view[1] as f32 * 16.0];
    let [x, y] = camera.project_fine_f32(fine, level, elev_near);
    let z = depth.point(fine, elev_near, level) + depth_bias as f32;
    ScreenVertex { x, y, z }
}

/// Returns the 4 projected vertices of lot tile `[tx, ty]` in order
/// `[(tx, ty), (tx + 1, ty), (tx + 1, ty + 1), (tx, ty + 1)]`.
///
/// Clamps each vertex's depth to sit just in front of the tile's constant
/// grass depth (`depth.grass(view_top, top_h) - 1`, `scene.rs:2246`) so steep
/// 5-step slopes never self-occlude their own overlay while preserving
/// foreground occlusion by standing walls and objects.
fn sloped_tile_vertices(
    lot: &Lot,
    camera: &Camera,
    depth: &Depth,
    [tx, ty]: [i32; 2],
    depth_bias: i32,
) -> [ScreenVertex; 4] {
    let h = lot.heights[Lot::cell(tx as usize, ty as usize)];
    // Native height corner order (`scene.rs:798`):
    // h[0]=(0,1), h[1]=(0,0), h[2]=(1,0), h[3]=(1,1)
    let corner_pairs = [
        ([tx, ty], h[1]),
        ([tx + 1, ty], h[2]),
        ([tx + 1, ty + 1], h[3]),
        ([tx, ty + 1], h[0]),
    ];
    let (top_corner, top_h) = corner_pairs
        .into_iter()
        .min_by_key(|&(c, _)| {
            let v = rotate_corner(camera.rotation, c);
            v[0] + v[1]
        })
        .unwrap_or(([tx, ty], h[1]));
    let max_surface_z =
        (depth.grass(rotate_corner(camera.rotation, top_corner), i32::from(top_h)) - 1) as f32;
    corner_pairs.map(|(corner, ch)| {
        let mut v =
            project_corner_vertex(camera, depth, corner, f32::from(ch) * 16.0, 1, depth_bias);
        v.z = v.z.min(max_surface_z);
        v
    })
}

fn edge_fn(ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32) -> f32 {
    (px - ax) * (by - ay) - (py - ay) * (bx - ax)
}

/// Rasterizes a quad split along its native `(0,0)-(1,1)` diagonal (matching
/// `Lot::altitude_at`), alpha-blended and depth-tested against `canvas.depth`.
fn raster_quad_blended(
    canvas: &mut Canvas,
    q: [ScreenVertex; 4],
    rgb: [u8; 3],
    alpha: u8,
    write_depth: bool,
) -> usize {
    raster_tri_blended(canvas, [q[0], q[1], q[2]], rgb, alpha, write_depth)
        + raster_tri_blended(canvas, [q[0], q[2], q[3]], rgb, alpha, write_depth)
}

fn is_top_left_edge(ax: f32, ay: f32, bx: f32, by: f32) -> bool {
    (ay == by && bx < ax) || by < ay
}

fn raster_tri_blended(
    canvas: &mut Canvas,
    tri: [ScreenVertex; 3],
    rgb: [u8; 3],
    alpha: u8,
    write_depth: bool,
) -> usize {
    if canvas.width == 0 || canvas.height == 0 || alpha == 0 || !canvas.has_pixel_storage() {
        return 0;
    }
    let [a, mut b, mut c] = tri;
    if !(a.x.is_finite()
        && a.y.is_finite()
        && b.x.is_finite()
        && b.y.is_finite()
        && c.x.is_finite()
        && c.y.is_finite())
    {
        return 0;
    }
    let mut area = edge_fn(a.x, a.y, b.x, b.y, c.x, c.y);
    if area.abs() < 1e-4 {
        return 0;
    }
    if area < 0.0 {
        std::mem::swap(&mut b, &mut c);
        area = -area;
    }
    let inv_area = 1.0 / area;
    let tl0 = is_top_left_edge(b.x, b.y, c.x, c.y);
    let tl1 = is_top_left_edge(c.x, c.y, a.x, a.y);
    let tl2 = is_top_left_edge(a.x, a.y, b.x, b.y);

    let min_x = (a.x.min(b.x).min(c.x).floor() as i64).clamp(0, canvas.width as i64) as usize;
    let max_x = (a.x.max(b.x).max(c.x).ceil() as i64).clamp(0, canvas.width as i64) as usize;
    let min_y = (a.y.min(b.y).min(c.y).floor() as i64).clamp(0, canvas.height as i64) as usize;
    let max_y = (a.y.max(b.y).max(c.y).ceil() as i64).clamp(0, canvas.height as i64) as usize;
    if min_x >= max_x || min_y >= max_y {
        return 0;
    }

    let width = canvas.width;
    let mut written = 0usize;
    for py in min_y..max_y {
        let cy = py as f32 + 0.5;
        let row = py * width;
        for px in min_x..max_x {
            let cx = px as f32 + 0.5;
            let w0 = edge_fn(b.x, b.y, c.x, c.y, cx, cy);
            let w1 = edge_fn(c.x, c.y, a.x, a.y, cx, cy);
            let w2 = edge_fn(a.x, a.y, b.x, b.y, cx, cy);
            let inside = (w0 > 0.0 || (w0 == 0.0 && tl0))
                && (w1 > 0.0 || (w1 == 0.0 && tl1))
                && (w2 > 0.0 || (w2 == 0.0 && tl2));
            if !inside {
                continue;
            }
            let z = (w0 * a.z + w1 * b.z + w2 * c.z) * inv_area;
            let zi = z.round() as i32;
            let idx = row + px;
            if zi > i32::from(canvas.depth[idx]) {
                continue;
            }
            if write_depth {
                canvas.depth[idx] = clamp_depth(zi);
            }
            canvas.color[idx] = pack_rgb(blend_overlay(unpack_rgb(canvas.color[idx]), rgb, alpha));
            written += 1;
        }
    }
    written
}

fn raster_roof_triangle(
    canvas: &mut Canvas,
    tri: [RoofVertex; 3],
    pattern: &RoofPattern,
    shade: f32,
) -> usize {
    if canvas.width == 0 || canvas.height == 0 || !canvas.has_pixel_storage() {
        return 0;
    }
    let [a, mut b, mut c] = tri;
    if !(a.sx.is_finite()
        && a.sy.is_finite()
        && b.sx.is_finite()
        && b.sy.is_finite()
        && c.sx.is_finite()
        && c.sy.is_finite())
    {
        return 0;
    }
    let mut area = edge_fn(a.sx, a.sy, b.sx, b.sy, c.sx, c.sy);
    if area.abs() < 1e-4 {
        return 0;
    }
    if area < 0.0 {
        std::mem::swap(&mut b, &mut c);
        area = -area;
    }
    let inv_area = 1.0 / area;
    let tl0 = is_top_left_edge(b.sx, b.sy, c.sx, c.sy);
    let tl1 = is_top_left_edge(c.sx, c.sy, a.sx, a.sy);
    let tl2 = is_top_left_edge(a.sx, a.sy, b.sx, b.sy);

    let min_x = (a.sx.min(b.sx).min(c.sx).floor() as i64).clamp(0, canvas.width as i64) as usize;
    let max_x = (a.sx.max(b.sx).max(c.sx).ceil() as i64).clamp(0, canvas.width as i64) as usize;
    let min_y = (a.sy.min(b.sy).min(c.sy).floor() as i64).clamp(0, canvas.height as i64) as usize;
    let max_y = (a.sy.max(b.sy).max(c.sy).ceil() as i64).clamp(0, canvas.height as i64) as usize;
    if min_x >= max_x || min_y >= max_y {
        return 0;
    }

    let width = canvas.width;
    let shade = shade.clamp(0.0, 1.0);
    let mut written = 0usize;
    for py in min_y..max_y {
        let cy = py as f32 + 0.5;
        let row = py * width;
        for px in min_x..max_x {
            let cx = px as f32 + 0.5;
            let w0 = edge_fn(b.sx, b.sy, c.sx, c.sy, cx, cy);
            let w1 = edge_fn(c.sx, c.sy, a.sx, a.sy, cx, cy);
            let w2 = edge_fn(a.sx, a.sy, b.sx, b.sy, cx, cy);
            let inside = (w0 > 0.0 || (w0 == 0.0 && tl0))
                && (w1 > 0.0 || (w1 == 0.0 && tl1))
                && (w2 > 0.0 || (w2 == 0.0 && tl2));
            if !inside {
                continue;
            }
            let z = (w0 * a.z + w1 * b.z + w2 * c.z) * inv_area;
            let zi = z.round() as i32;
            let idx = row + px;
            if zi > i32::from(canvas.depth[idx]) {
                continue;
            }
            let u = (w0 * a.u + w1 * b.u + w2 * c.u) * inv_area;
            let v = (w0 * a.v + w1 * b.v + w2 * c.v) * inv_area;
            let tex = pattern.sample_rgb(u, v);
            let lit = [
                (f32::from(tex[0]) * shade + 0.5).clamp(0.0, 255.0) as u8,
                (f32::from(tex[1]) * shade + 0.5).clamp(0.0, 255.0) as u8,
                (f32::from(tex[2]) * shade + 0.5).clamp(0.0, 255.0) as u8,
            ];
            canvas.color[idx] = pack_rgb(quantize565(lit));
            canvas.depth[idx] = clamp_depth(zi);
            written += 1;
        }
    }
    written
}

fn draw_quad_outline(canvas: &mut Canvas, q: [ScreenVertex; 4], rgb: [u8; 3], alpha: u8) -> usize {
    draw_line_depth(canvas, q[0], q[1], rgb, alpha)
        + draw_line_depth(canvas, q[1], q[2], rgb, alpha)
        + draw_line_depth(canvas, q[2], q[3], rgb, alpha)
        + draw_line_depth(canvas, q[3], q[0], rgb, alpha)
}

fn draw_line_depth(
    canvas: &mut Canvas,
    a: ScreenVertex,
    b: ScreenVertex,
    rgb: [u8; 3],
    alpha: u8,
) -> usize {
    if !(a.x.is_finite() && a.y.is_finite() && b.x.is_finite() && b.y.is_finite()) {
        return 0;
    }
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let steps = dx.abs().max(dy.abs()).ceil().min(4096.0) as i32;
    if steps <= 0 {
        return blend_depth_pixel(
            canvas,
            a.x.round() as i32,
            a.y.round() as i32,
            a.z.round() as i32,
            rgb,
            alpha,
        );
    }
    let inv = 1.0 / steps as f32;
    let mut written = 0usize;
    for i in 0..=steps {
        let t = i as f32 * inv;
        let px = (a.x + dx * t).round() as i32;
        let py = (a.y + dy * t).round() as i32;
        let pz = (a.z + (b.z - a.z) * t).round() as i32;
        written += blend_depth_pixel(canvas, px, py, pz, rgb, alpha);
    }
    written
}

fn draw_vertex_brush_marker(
    canvas: &mut Canvas,
    center: ScreenVertex,
    up: bool,
    zoom_level: u32,
    rgb: [u8; 3],
) -> usize {
    let scale = (1 << zoom_level.saturating_sub(1)) as f32;
    let shaft = 8.0 * scale;
    let head = 3.5 * scale;
    let tip_y = if up {
        center.y - shaft
    } else {
        center.y + shaft
    };
    let tip = ScreenVertex {
        x: center.x,
        y: tip_y,
        z: center.z - 8.0,
    };
    let base = ScreenVertex {
        x: center.x,
        y: center.y,
        z: center.z - 8.0,
    };
    let wing_y = if up { tip_y + head } else { tip_y - head };
    let left = ScreenVertex {
        x: center.x - head,
        y: wing_y,
        z: center.z - 8.0,
    };
    let right = ScreenVertex {
        x: center.x + head,
        y: wing_y,
        z: center.z - 8.0,
    };
    // Small diamond around the vertex + vertical elevation arrow.
    let r = 2.5 * scale;
    let d0 = ScreenVertex {
        x: center.x,
        y: center.y - r * 0.5,
        z: center.z - 8.0,
    };
    let d1 = ScreenVertex {
        x: center.x + r,
        y: center.y,
        z: center.z - 8.0,
    };
    let d2 = ScreenVertex {
        x: center.x,
        y: center.y + r * 0.5,
        z: center.z - 8.0,
    };
    let d3 = ScreenVertex {
        x: center.x - r,
        y: center.y,
        z: center.z - 8.0,
    };
    draw_quad_outline(canvas, [d0, d1, d2, d3], rgb, 240)
        + draw_line_depth(canvas, base, tip, rgb, 255)
        + draw_line_depth(canvas, tip, left, rgb, 255)
        + draw_line_depth(canvas, tip, right, rgb, 255)
}

fn blend_depth_pixel(
    canvas: &mut Canvas,
    x: i32,
    y: i32,
    depth: i32,
    rgb: [u8; 3],
    alpha: u8,
) -> usize {
    if !canvas.has_pixel_storage() {
        return 0;
    }
    let (Ok(px), Ok(py)) = (usize::try_from(x), usize::try_from(y)) else {
        return 0;
    };
    if px >= canvas.width || py >= canvas.height {
        return 0;
    }
    let idx = py * canvas.width + px;
    if depth > i32::from(canvas.depth[idx]) {
        return 0;
    }
    canvas.color[idx] = pack_rgb(blend_overlay(unpack_rgb(canvas.color[idx]), rgb, alpha));
    1
}

#[inline]
fn blend_overlay(dst: [u8; 3], src: [u8; 3], alpha: u8) -> [u8; 3] {
    let a = u32::from(alpha);
    let inv = 255 - a;
    quantize565([
        ((u32::from(src[0]) * a + u32::from(dst[0]) * inv + 127) / 255) as u8,
        ((u32::from(src[1]) * a + u32::from(dst[1]) * inv + 127) / 255) as u8,
        ((u32::from(src[2]) * a + u32::from(dst[2]) * inv + 127) / 255) as u8,
    ])
}

/// Ensures `light::brightest` remains linked/accessible for callers inspecting
/// roof daylight modulation.
pub fn roof_daylight_cap(time_of_day: i16) -> f32 {
    light::brightest(time_of_day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Walls, Zoom};
    use crate::canvas::EMPTY_DEPTH;
    use crate::cutaway::Focus;
    use sims_formats::lot::Array;

    fn test_world(size: i16) -> World {
        let mut world = World::empty();
        world.size = size;
        let full = lot::FULL * lot::FULL;
        for (id, stride) in [(0, 4), (1, 1), (2, 8), (101, 1), (102, 8)] {
            world.arrays.insert(
                id,
                Array {
                    width: lot::FULL,
                    height: lot::FULL,
                    stride,
                    encoding: 0,
                    data: vec![0; full * stride],
                },
            );
        }
        world
    }

    fn camera_for(lot: &Lot, zoom: Zoom, rotation: u8) -> Camera {
        let mut c = Camera {
            zoom,
            rotation,
            level: 2,
            origin: [0, 0],
            walls: Walls::Up,
            focus: Focus::default(),
        };
        c.center_on(lot.built_center(), [160, 140]);
        c
    }

    #[test]
    fn roof_pitch_presets_match_sims_exe_cwinroofpanel_values() {
        assert_eq!(RoofPitch::Flat.slope(), 0.0);
        assert_eq!(RoofPitch::Shallow.slope(), 0.4);
        assert_eq!(RoofPitch::Medium.slope(), 0.6);
        assert_eq!(RoofPitch::Steep.slope(), 1.0);
        assert_eq!(RoofPitch::from_slope(0.0), RoofPitch::Flat);
        assert_eq!(RoofPitch::from_slope(0.05), RoofPitch::Flat);
        assert_eq!(RoofPitch::from_slope(0.4), RoofPitch::Shallow);
        assert_eq!(RoofPitch::from_slope(0.6), RoofPitch::Medium);
        assert_eq!(RoofPitch::from_slope(1.0), RoofPitch::Steep);
        assert_eq!(RoofPitch::clamp_slope(-1.0), 0.0);
        assert_eq!(RoofPitch::clamp_slope(5.0), MAX_ROOF_SLOPE);
        assert_eq!(RoofPitch::clamp_slope(f32::NAN), 0.6);
    }

    #[test]
    fn terrain_overlay_validates_and_draws_across_sloped_tiles_and_rotations() {
        let mut world = test_world(16);
        // Raise corner (6, 6) to height 2 across its 4 touching tiles.
        let mut lot = Lot::from_world(&world).unwrap();
        lot.heights[Lot::cell(5, 5)][3] = 2;
        lot.heights[Lot::cell(6, 5)][0] = 2;
        lot.heights[Lot::cell(5, 6)][2] = 2;
        lot.heights[Lot::cell(6, 6)][1] = 2;
        // Place a road floor on (8, 8) to verify blocked status.
        lot.levels[0].floors[Lot::cell(8, 8)] = 9;
        world.arrays.get_mut(&1).unwrap().data[Lot::cell(8, 8)] = 9;

        let valid_raise = validate_terrain_overlay(
            &lot,
            Some(&world),
            &TerrainOverlay {
                tool: TerrainTool::Raise,
                target: TerrainTarget::Vertex([6, 6]),
            },
        );
        assert!(valid_raise.valid);
        assert_eq!(valid_raise.cost, 1);
        assert_eq!(valid_raise.anchor_height, 2);
        assert_eq!(valid_raise.tiles.len(), 4);

        let blocked_level = validate_terrain_overlay(
            &lot,
            Some(&world),
            &TerrainOverlay {
                tool: TerrainTool::Level,
                target: TerrainTarget::Rect {
                    start: [7, 7],
                    end: [8, 8],
                },
            },
        );
        assert!(!blocked_level.valid);
        assert!(
            blocked_level
                .tiles
                .iter()
                .any(|t| t.tile == [8, 8] && t.status == TerrainTileStatus::BlockedByRoad)
        );

        for rot in 0..4u8 {
            let cam = camera_for(&lot, Zoom::Medium, rot);
            let mut canvas = Canvas::new(320, 280, [40, 80, 40]);
            let stats = draw_terrain_overlay(&lot, &cam, &valid_raise, &mut canvas);
            assert_eq!(stats.tiles_drawn, 4);
            assert_eq!(stats.blocked_tiles, 0);
            assert!(stats.pixels_written > 100, "rot {rot} should paint pixels");
        }
    }

    #[test]
    fn pool_preview_detects_invalid_slopes_and_draws_coping_and_water() {
        let world = test_world(16);
        let mut lot = Lot::from_world(&world).unwrap();
        let cam = camera_for(&lot, Zoom::Medium, 0);

        let flat_plan = validate_water_overlay(
            &lot,
            Some(&world),
            &WaterOverlay {
                tool: WaterTool::Pool,
                action: WaterAction::Dig,
                start: [5, 5],
                end: [7, 6],
            },
        );
        assert!(flat_plan.valid);
        assert_eq!(flat_plan.tiles.len(), 6);
        assert_eq!(flat_plan.cost, 6 * 75);
        // 3x2 pool has a 5x4 bounding box -> 20 - 6 = 14 coping border tiles.
        assert_eq!(flat_plan.coping_tiles.len(), 14);

        let mut canvas = Canvas::new(320, 280, [40, 90, 40]);
        let stats = draw_water_overlay(&lot, &cam, &flat_plan, &mut canvas);
        assert_eq!(stats.tiles_drawn, 20);
        assert_eq!(stats.blocked_tiles, 0);
        assert!(stats.pixels_written > 400);

        // Introduce a slope on one tile -> pool must fail with InvalidSlope.
        lot.heights[Lot::cell(6, 5)][2] = 3;
        let sloped_plan = validate_water_overlay(
            &lot,
            Some(&world),
            &WaterOverlay {
                tool: WaterTool::Pool,
                action: WaterAction::Dig,
                start: [5, 5],
                end: [7, 6],
            },
        );
        assert!(!sloped_plan.valid);
        assert_eq!(sloped_plan.cost, 0);
        assert!(
            sloped_plan
                .tiles
                .iter()
                .any(|t| t.tile == [6, 5] && t.status == WaterTileStatus::InvalidSlope)
        );

        // Native Arry 9 / Arry 10 dry tiles store 0xFF (`>= 16`); wet tiles store `0..16`.
        let mut water_world = test_world(16);
        let mut pond_arr = vec![0xFFu8; lot::FULL * lot::FULL];
        pond_arr[Lot::cell(5, 5)] = 0; // isolated pond tile
        water_world.arrays.insert(
            WATER_ARRAY_ID,
            Array {
                width: lot::FULL,
                height: lot::FULL,
                stride: 1,
                encoding: 0,
                data: pond_arr,
            },
        );
        let flat_lot = Lot::from_world(&water_world).unwrap();
        let on_dry = validate_water_overlay(
            &flat_lot,
            Some(&water_world),
            &WaterOverlay {
                tool: WaterTool::Pool,
                action: WaterAction::Dig,
                start: [8, 8],
                end: [9, 9],
            },
        );
        assert!(on_dry.valid, "0xFF entries in Arry 10 are dry land");
        let on_pond = validate_water_overlay(
            &flat_lot,
            Some(&water_world),
            &WaterOverlay {
                tool: WaterTool::Pool,
                action: WaterAction::Dig,
                start: [5, 5],
                end: [6, 6],
            },
        );
        assert!(!on_pond.valid);
        assert!(
            on_pond
                .tiles
                .iter()
                .any(|t| t.tile == [5, 5] && t.status == WaterTileStatus::BlockedByWater)
        );
    }

    #[test]
    fn water_overlay_respects_foreground_depth_occlusion() {
        let world = test_world(16);
        let lot = Lot::from_world(&world).unwrap();
        let cam = camera_for(&lot, Zoom::Near, 0);
        let plan = validate_water_overlay(
            &lot,
            Some(&world),
            &WaterOverlay {
                tool: WaterTool::Pond,
                action: WaterAction::Dig,
                start: [7, 7],
                end: [8, 8],
            },
        );
        assert!(plan.valid);
        assert_eq!(plan.cost, 4 * 15);

        // Pre-fill entire depth buffer with a near foreground wall (depth 0).
        let mut occluded = Canvas::new(320, 280, [10, 10, 10]);
        occluded.depth.fill(0);
        let stats = draw_water_overlay(&lot, &cam, &plan, &mut occluded);
        assert_eq!(
            stats.pixels_written, 0,
            "foreground wall at depth 0 must occlude overlay"
        );
        assert!(occluded.color.iter().all(|&c| c == pack_rgb([10, 10, 10])));
    }

    #[test]
    fn roof_preview_renders_all_styles_and_pitches_with_depth_writes() {
        let world = test_world(16);
        let mut lot = Lot::from_world(&world).unwrap();
        let mut rooms = vec![0i16; lot::FULL * lot::FULL];
        for x in 5..=9 {
            for y in 5..=8 {
                rooms[Lot::cell(x, y)] = 1;
            }
        }
        lot.outdoor_rooms.insert(0);
        lot.levels[0].rooms = Some(rooms);

        let [l1, l2] = detect_roof_footprints(&lot);
        assert_eq!(l1.len(), 20);
        assert!(l2.is_empty());

        let pattern = RoofPattern::solid("test_slate", [140, 110, 96]);
        let mut cam = camera_for(&lot, Zoom::Far, 0);
        cam.center_on([7 * 16 + 8, 7 * 16], [160, 210]);

        let mut steep_canvas = Canvas::new(320, 280, [115, 113, 115]);
        let mut flat_canvas = Canvas::new(320, 280, [115, 113, 115]);

        for style in RoofStyle::ALL {
            for pitch in RoofPitch::ALL {
                let mut canvas = Canvas::new(320, 280, [115, 113, 115]);
                let cfg = RoofPreviewConfig {
                    style,
                    pitch,
                    ..RoofPreviewConfig::default()
                };
                let stats = draw_roof_preview(&lot, &cam, &pattern, &cfg, &mut canvas);
                assert_eq!(stats.footprint_tiles, 20);
                assert!(stats.triangles_drawn > 0);
                assert!(stats.pixels_written > 200, "{style:?} {pitch:?}");
                assert!(
                    canvas.depth.iter().any(|&d| d < EMPTY_DEPTH),
                    "roof should write 16-bit depth"
                );
                if style == RoofStyle::Hip && pitch == RoofPitch::Steep {
                    steep_canvas = canvas;
                } else if style == RoofStyle::Hip && pitch == RoofPitch::Flat {
                    flat_canvas = canvas;
                }
            }
        }
        // Steep roof must rise visibly higher on screen than a flat roof.
        let top_row = |c: &Canvas| {
            (0..c.height)
                .find(|&y| (0..c.width).any(|x| c.depth[y * c.width + x] < EMPTY_DEPTH))
                .unwrap()
        };
        assert!(
            top_row(&steep_canvas) < top_row(&flat_canvas),
            "steep roof peak ({}) should project higher on screen than flat roof ({})",
            top_row(&steep_canvas),
            top_row(&flat_canvas)
        );
    }

    #[test]
    fn strip_bmp_ext_and_sample_rgb_handle_utf8_and_non_finite_safely() {
        assert_eq!(strip_bmp_ext("r1_.BMP"), "r1_");
        assert_eq!(strip_bmp_ext("café.bmp"), "café");
        assert_eq!(strip_bmp_ext("é.bp"), "é.bp");
        assert_eq!(strip_bmp_ext("🔥🔥"), "🔥🔥");

        let p = RoofPattern::solid("test", [100, 120, 140]);
        assert_eq!(p.sample_rgb(f32::NAN, 0.5), [128, 128, 128]);
        assert_eq!(p.sample_rgb(0.5, f32::INFINITY), [128, 128, 128]);
        assert_eq!(p.sample_rgb(-0.25, 1.75), [100, 120, 140]);

        // Oversized manual pattern must never overflow byte indexing.
        let huge = RoofPattern {
            name: "huge".into(),
            file_name: "huge.bmp".into(),
            width: u32::MAX,
            height: u32::MAX,
            rgba: vec![255; 16],
        };
        assert_eq!(huge.sample_rgb(0.99, 0.99), [128, 128, 128]);
    }

    #[test]
    fn sol_r1_parity_pond_flatness_pool_edge_billing_height_cap_sloped_grass_and_roofs() {
        // 1. Pond rejects sloped tiles `[0, 0, 0, 1]` matching `0x49838e`..`0x4984cf`.
        let world = test_world(16);
        let mut lot = Lot::from_world(&world).unwrap();
        lot.heights[Lot::cell(5, 5)] = [0, 0, 0, 1];
        let pond_sloped = validate_water_overlay(
            &lot,
            Some(&world),
            &WaterOverlay {
                tool: WaterTool::Pond,
                action: WaterAction::Dig,
                start: [5, 5],
                end: [5, 5],
            },
        );
        assert!(!pond_sloped.valid);
        assert_eq!(pond_sloped.tiles[0].status, WaterTileStatus::InvalidSlope);

        // 2. Pool starting at interior edge `x = 1` is valid even though visual
        //    coping reaches `x = 0`, and billing skips already-wet pool tiles.
        let mut pool_world = test_world(16);
        let mut pool_arr = vec![0xFFu8; lot::FULL * lot::FULL];
        pool_arr[Lot::cell(1, 2)] = 0; // already excavated pool tile
        pool_world.arrays.insert(
            POOL_ARRAY_ID,
            Array {
                width: lot::FULL,
                height: lot::FULL,
                stride: 1,
                encoding: 0,
                data: pool_arr,
            },
        );
        let flat_lot = Lot::from_world(&pool_world).unwrap();
        let edge_pool = validate_water_overlay(
            &flat_lot,
            Some(&pool_world),
            &WaterOverlay {
                tool: WaterTool::Pool,
                action: WaterAction::Dig,
                start: [1, 2],
                end: [2, 2],
            },
        );
        assert!(
            edge_pool.valid,
            "pool at x=1 must not fail on coping at x=0"
        );
        assert_eq!(edge_pool.cost, 75, "only 1 newly excavated tile is billed");

        // 3. Raise Terrain rejects vertex already at native ceiling 64 (`0x40`).
        let mut high_lot = flat_lot.clone();
        high_lot.heights.fill([MAX_TERRAIN_HEIGHT; 4]);
        let capped = validate_terrain_overlay(
            &high_lot,
            None,
            &TerrainOverlay {
                tool: TerrainTool::Raise,
                target: TerrainTarget::Vertex([6, 6]),
            },
        );
        assert!(!capped.valid);
        assert_eq!(capped.cost, 0);
        assert!(
            capped
                .tiles
                .iter()
                .all(|t| t.status == TerrainTileStatus::HeightLimit)
        );

        // 4. Extreme coordinates never overflow.
        let extreme_v = validate_terrain_overlay(
            &flat_lot,
            None,
            &TerrainOverlay {
                tool: TerrainTool::Raise,
                target: TerrainTarget::Vertex([i32::MIN, i32::MAX]),
            },
        );
        assert!(!extreme_v.valid);
        let extreme_pool = validate_water_overlay(
            &flat_lot,
            None,
            &WaterOverlay {
                tool: WaterTool::Pool,
                action: WaterAction::Dig,
                start: [i32::MIN, i32::MIN],
                end: [i32::MAX, i32::MAX],
            },
        );
        assert!(!extreme_pool.valid);

        // 5. Sloped 5-step grass tile must not self-occlude its own preview overlay.
        let mut sloped_lot = flat_lot.clone();
        // Corner (0,0) = 5 (view-top at rot 0), side corners = 0 -> 5-step drop.
        sloped_lot.heights[Lot::cell(6, 6)] = [0, 5, 0, 0];
        let mut cam = camera_for(&sloped_lot, Zoom::Near, 0);
        cam.center_on([6 * 16 + 8, 6 * 16 + 8], [160, 140]);
        let d = Depth::new(&cam);
        let grass_z = clamp_depth(d.grass(rotate_corner(cam.rotation, [6, 6]), 5));
        let mut grass_canvas = Canvas::new(320, 280, [40, 90, 40]);
        grass_canvas.depth.fill(grass_z);
        let mut empty_canvas = Canvas::new(320, 280, [40, 90, 40]);
        let plan = validate_terrain_overlay(
            &sloped_lot,
            None,
            &TerrainOverlay {
                tool: TerrainTool::Grass,
                target: TerrainTarget::Brush {
                    center: [6, 6],
                    radius: 0,
                },
            },
        );
        let s_grass = draw_terrain_overlay(&sloped_lot, &cam, &plan, &mut grass_canvas);
        let s_empty = draw_terrain_overlay(&sloped_lot, &cam, &plan, &mut empty_canvas);
        assert_eq!(
            s_grass.pixels_written, s_empty.pixels_written,
            "sloped grass depth must not clip any pixels of its own overlay"
        );

        // 6. Roofs on separate terraces sit at their own terrain heights, and
        //    split-diagonal indoor halves are included in detect_roof_footprints.
        let mut terrace_lot = flat_lot.clone();
        for x in 2..=4 {
            for y in 9..=11 {
                terrace_lot.heights[Lot::cell(x, y)] = [0; 4];
            }
        }
        for x in 9..=11 {
            for y in 2..=4 {
                terrace_lot.heights[Lot::cell(x, y)] = [8; 4];
            }
        }
        let mut rooms = vec![0i16; lot::FULL * lot::FULL];
        rooms[Lot::cell(3, 10)] = 1;
        rooms[Lot::cell(10, 3)] = 2;
        rooms[Lot::cell(6, 6)] = sims_sim::rooms::NO_ROOM;
        terrace_lot.outdoor_rooms.insert(0);
        terrace_lot.levels[0].rooms = Some(rooms);
        terrace_lot.halves.insert([1, 6, 6], [(4, 1), (2, 0)]);
        let [l1, _] = detect_roof_footprints(&terrace_lot);
        assert!(
            l1.contains(&[6, 6]),
            "split tile with indoor half must be included in automatic roof footprint"
        );
        let pattern = RoofPattern::solid("terrace", [150, 120, 100]);
        let mut c_low = Canvas::new(320, 320, [115, 113, 115]);
        let mut c_high = Canvas::new(320, 320, [115, 113, 115]);
        let mut cam_low = camera_for(&terrace_lot, Zoom::Far, 0);
        cam_low.center_on([3 * 16 + 8, 10 * 16 + 8], [160, 220]);
        let mut cam_high = camera_for(&terrace_lot, Zoom::Far, 0);
        cam_high.center_on([10 * 16 + 8, 3 * 16 + 8], [160, 220]);
        let cfg_flat = RoofPreviewConfig {
            style: RoofStyle::Flat,
            pitch: RoofPitch::Flat,
            ..RoofPreviewConfig::default()
        };
        draw_roof_preview(&terrace_lot, &cam_low, &pattern, &cfg_flat, &mut c_low);
        draw_roof_preview(&terrace_lot, &cam_high, &pattern, &cfg_flat, &mut c_high);
        let top_at_center = |c: &Canvas| {
            (0..c.height)
                .find(|&y| c.depth[y * c.width + 160] < EMPTY_DEPTH)
                .unwrap()
        };
        // 8 terrain steps at Far zoom = 8 * 4 = 32 px higher on screen.
        let diff = top_at_center(&c_low) as i32 - top_at_center(&c_high) as i32;
        assert!(
            (diff - 32).abs() <= 2,
            "8-step higher terrace roof should sit ~32 px higher at Far zoom (got {diff})"
        );

        // 7. Split-diagonal roof tiles clip to the exact indoor triangle (for
        //    both ascending (0,0)-(1,1) and descending (1,0)-(0,1) diagonals),
        //    including probes inside the diagonal-crossed half-cells and a
        //    nonzero-pitch Gable roof where Sol's `fx > fy` ascending case must
        //    not emit a gable-end wall across the outdoor half.
        let cfg_zero_elev = RoofPreviewConfig {
            style: RoofStyle::Hip,
            pitch: RoofPitch::Flat,
            ..RoofPreviewConfig::default()
        };
        for (halves, indoor_offs, outdoor_offs) in [
            // Ascending diagonal: triangle 1 (fy > fx) indoor, triangle 3 (fx > fy) outdoor.
            // Probes include (2, 6) vs (6, 2) inside the diagonal-crossed (0,0) half-cell
            // and (10, 14) vs (14, 10) inside the (1,1) half-cell.
            (
                [(1u8, 1i16), (3u8, 0i16)],
                [[3, 13], [2, 6], [10, 14]],
                [[13, 3], [6, 2], [14, 10]],
            ),
            // Ascending diagonal flipped: triangle 3 (fx > fy) indoor, triangle 1 (fy > fx) outdoor.
            (
                [(3u8, 1i16), (1u8, 0i16)],
                [[13, 3], [6, 2], [14, 10]],
                [[3, 13], [2, 6], [10, 14]],
            ),
            // Descending diagonal: triangle 4 (fx + fy < 16) indoor, triangle 2 (fx + fy > 16) outdoor.
            // Probes include (10, 2) vs (14, 6) inside the (1,0) half-cell and (2, 10) vs (6, 14) inside (0,1).
            (
                [(4u8, 1i16), (2u8, 0i16)],
                [[3, 3], [10, 2], [2, 10]],
                [[13, 13], [14, 6], [6, 14]],
            ),
        ] {
            let mut diag_lot = flat_lot.clone();
            let mut diag_rooms = vec![0i16; lot::FULL * lot::FULL];
            diag_rooms[Lot::cell(6, 6)] = sims_sim::rooms::NO_ROOM;
            diag_lot.outdoor_rooms.insert(0);
            diag_lot.levels[0].rooms = Some(diag_rooms);
            diag_lot.halves.insert([1, 6, 6], halves);
            let mut diag_cam = camera_for(&diag_lot, Zoom::Near, 0);
            // Roof sits on level 2 (232 px above level 1 at Near zoom), so anchor
            // level 1 near the bottom of the canvas so the roof falls near y = 58.
            diag_cam.center_on([6 * 16 + 8, 6 * 16 + 8], [160, 290]);
            let mut diag_canvas = Canvas::new(320, 320, [115, 113, 115]);
            let diag_stats = draw_roof_preview(
                &diag_lot,
                &diag_cam,
                &pattern,
                &cfg_zero_elev,
                &mut diag_canvas,
            );
            assert!(
                diag_stats.pixels_written > 0,
                "indoor triangle {halves:?} should draw roof pixels"
            );
            for indoor_off in indoor_offs {
                let indoor_pt =
                    diag_cam.project_fine([6 * 16 + indoor_off[0], 6 * 16 + indoor_off[1]], 2, 0);
                assert!(
                    diag_canvas.depth
                        [indoor_pt[1] as usize * diag_canvas.width + indoor_pt[0] as usize]
                        < EMPTY_DEPTH,
                    "indoor diagonal probe {indoor_off:?} for {halves:?} must be covered"
                );
            }
            for outdoor_off in outdoor_offs {
                let outdoor_pt =
                    diag_cam.project_fine([6 * 16 + outdoor_off[0], 6 * 16 + outdoor_off[1]], 2, 0);
                assert_eq!(
                    diag_canvas.depth
                        [outdoor_pt[1] as usize * diag_canvas.width + outdoor_pt[0] as usize],
                    EMPTY_DEPTH,
                    "outdoor diagonal probe {outdoor_off:?} for {halves:?} must have zero overhang"
                );
            }
        }

        // Nonzero-pitch Gable roof on a single ascending split tile (`fx > fy`
        // indoors) must not emit a gable-end wall across the outdoor `(fy > fx)` half.
        let mut gable_diag_lot = flat_lot.clone();
        let mut gable_rooms = vec![0i16; lot::FULL * lot::FULL];
        gable_rooms[Lot::cell(6, 6)] = sims_sim::rooms::NO_ROOM;
        gable_diag_lot.outdoor_rooms.insert(0);
        gable_diag_lot.levels[0].rooms = Some(gable_rooms);
        gable_diag_lot.halves.insert([1, 6, 6], [(3, 1), (1, 0)]);
        let mut gable_cam = camera_for(&gable_diag_lot, Zoom::Near, 0);
        gable_cam.center_on([6 * 16 + 8, 6 * 16 + 8], [160, 290]);
        let cfg_gable = RoofPreviewConfig {
            style: RoofStyle::Gable,
            pitch: RoofPitch::Medium,
            ..RoofPreviewConfig::default()
        };
        let mut gable_canvas = Canvas::new(320, 320, [115, 113, 115]);
        let gable_stats = draw_roof_preview(
            &gable_diag_lot,
            &gable_cam,
            &pattern,
            &cfg_gable,
            &mut gable_canvas,
        );
        assert!(gable_stats.pixels_written > 0);
        for outdoor_off in [[3, 13], [2, 6], [4, 10]] {
            let outdoor_pt =
                gable_cam.project_fine([6 * 16 + outdoor_off[0], 6 * 16 + outdoor_off[1]], 2, 0);
            assert_eq!(
                gable_canvas.depth
                    [outdoor_pt[1] as usize * gable_canvas.width + outdoor_pt[0] as usize],
                EMPTY_DEPTH,
                "nonzero-pitch split Gable roof must not leak wall/roof pixels onto outdoor probe {outdoor_off:?}"
            );
        }
    }
}
