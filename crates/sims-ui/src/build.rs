//! Build Mode catalog indexing, layout, drawing, and hit-testing
//! (`cWinArch` at `0x5004f5`..`0x501a69`, `cArchCatalog` at `0x4cd149`..
//! `0x4cd9a5`, and `GameData/Build.iff` `STR# 129/130/131/134/135`).

use crate::art::Art;
use crate::command::{BuildItemId, BuildTool, Command};
use crate::frame::{Frame, Rect};
use crate::hud::STRIP;
use crate::thumbnail::{SIZE as CELL, TEMPLATE, Thumbnails};
use sims_content::ui_graphics::US_ENGLISH;
use sims_content::{Catalog, Index, Install, SurfacePlugin, atoi};
use sims_formats::bitmap::{self, Image};
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// UIGraphics resource IDs for the Build Mode panel (`Res_CPanel.h`,
/// `0x5004f5`..`0x500720`).
pub mod id {
    pub const UNDO: i32 = 64;
    pub const REDO: i32 = 65;
    pub const TOOL_TERRAIN: i32 = 66;
    pub const TOOL_TREE: i32 = 67;
    pub const TOOL_WATER: i32 = 68;
    pub const TOOL_FLOOR: i32 = 69;
    pub const TOOL_WALL: i32 = 70;
    pub const TOOL_DOOR: i32 = 71;
    pub const TOOL_WINDOW: i32 = 72;
    pub const TOOL_WALLPAPER: i32 = 73;
    pub const TOOL_STAIRS: i32 = 74;
    pub const TOOL_FIREPLACE: i32 = 75;
    pub const TOOL_ROOF: i32 = 76;
    pub const TOOL_HAND: i32 = 77;
    pub const PREV_ARROW: i32 = 100;
    pub const NEXT_ARROW: i32 = 101;
    pub const SUBTOOL_TERRAIN_DOWN: i32 = 3516;
    pub const SUBTOOL_TERRAIN_LEVEL: i32 = 3517;
    pub const SUBTOOL_TERRAIN_UP: i32 = 3518;
    pub const SUBTOOL_GRASS: i32 = 3519;
    pub const SUBTOOL_WATER: i32 = 3520;
    pub const SUBTOOL_POOL: i32 = 3800;
    pub const SUBTOOL_ROOF_SHALLOW: i32 = 3900;
    pub const SUBTOOL_ROOF_MEDIUM: i32 = 3901;
    pub const SUBTOOL_ROOF_STEEP: i32 = 3902;
    pub const ROOF_PATTERN_TEMPLATE: i32 = 3903;
    pub const SUBTOOL_ROOF_FLAT: i32 = 3904;
    pub const BUILD_PATCH: i32 = 4805;
    pub const TOOTHPICK_LEFT: i32 = 4912;
    pub const TOOTHPICK_RIGHT: i32 = 4913;
}

/// Returns every UIGraphics resource ID drawn by the Build Mode panel, for
/// optional loading in `Hud::open`.
pub fn resource_ids() -> Vec<i32> {
    vec![
        id::UNDO,
        id::REDO,
        id::TOOL_TERRAIN,
        id::TOOL_TREE,
        id::TOOL_WATER,
        id::TOOL_FLOOR,
        id::TOOL_WALL,
        id::TOOL_DOOR,
        id::TOOL_WINDOW,
        id::TOOL_WALLPAPER,
        id::TOOL_STAIRS,
        id::TOOL_FIREPLACE,
        id::TOOL_ROOF,
        id::TOOL_HAND,
        id::PREV_ARROW,
        id::NEXT_ARROW,
        id::SUBTOOL_TERRAIN_DOWN,
        id::SUBTOOL_TERRAIN_LEVEL,
        id::SUBTOOL_TERRAIN_UP,
        id::SUBTOOL_GRASS,
        id::SUBTOOL_WATER,
        id::SUBTOOL_POOL,
        id::SUBTOOL_ROOF_SHALLOW,
        id::SUBTOOL_ROOF_MEDIUM,
        id::SUBTOOL_ROOF_STEEP,
        id::ROOF_PATTERN_TEMPLATE,
        id::SUBTOOL_ROOF_FLAT,
        id::BUILD_PATCH,
        id::TOOTHPICK_LEFT,
        id::TOOTHPICK_RIGHT,
        TEMPLATE,
    ]
}

/// Native `cWinArch` tool button table (`0x6f2bf0`..`0x6f2c58` and
/// `0x6cc28c`..`0x6cc2bc`): `(tool, resource_id, [x, y_offset_from_strip_top], fallback_tip)`.
const TOOL_BUTTONS: [(BuildTool, i32, [i32; 2], &str); 12] = [
    (
        BuildTool::Terrain,
        id::TOOL_TERRAIN,
        [278, 4],
        "Terrain Tools",
    ),
    (BuildTool::Water, id::TOOL_WATER, [325, 6], "Water Tools"),
    (
        BuildTool::Wall,
        id::TOOL_WALL,
        [364, 4],
        "Wall & Fence Tool",
    ),
    (
        BuildTool::Wallpaper,
        id::TOOL_WALLPAPER,
        [408, 3],
        "Wallpaper Tool",
    ),
    (BuildTool::Stairs, id::TOOL_STAIRS, [450, 4], "Stair Tool"),
    (
        BuildTool::Fireplace,
        id::TOOL_FIREPLACE,
        [502, 8],
        "Fireplace Tool",
    ),
    (BuildTool::Plants, id::TOOL_TREE, [279, 57], "Plant Tool"),
    (BuildTool::Floor, id::TOOL_FLOOR, [317, 64], "Floor Tool"),
    (BuildTool::Doors, id::TOOL_DOOR, [367, 55], "Door Tool"),
    (
        BuildTool::Windows,
        id::TOOL_WINDOW,
        [409, 55],
        "Window Tool",
    ),
    (BuildTool::Roof, id::TOOL_ROOF, [449, 55], "Roof Tool"),
    (BuildTool::Hand, id::TOOL_HAND, [502, 55], "Hand Tool"),
];

const UNDO_POS: [i32; 2] = [231, 21];
const REDO_POS: [i32; 2] = [231, 53];
const PREV_ARROW_X: i32 = 539;
const ARROW_Y: i32 = 32;
const GRID_LEFT: i32 = 556;
const GRID_TOP: i32 = 5;
const GRID_ROWS: usize = 2;
const GRID_RIGHT_MARGIN: i32 = 19;
const NEXT_ARROW_MARGIN: i32 = 18;
const FALLBACK_BTN: [i32; 2] = [36, 36];

/// `Build.iff` `STR#` chunk IDs (`0x4cd453`, `0x4cd696`, `0x4cd71b`).
const STR_TERRAIN: u16 = 128;
const STR_WALLS: u16 = 129;
const STR_FLOORS: u16 = 130;
const STR_WALLPAPERS: u16 = 131;
const STR_POOL: u16 = 133;
const STR_PLUGIN_FLOOR: u16 = 134;
const STR_PLUGIN_WALL: u16 = 135;

/// Narrow-viewport threshold below which Build Mode compacts the tool strip so
/// catalog slots never overlap tool buttons (`Roof` switches below 652 px when
/// its pitch column needs extra width).
const NARROW_WIDTH: i32 = 620;
const TOOL_STRIP_LEFT: i32 = 268;
const TOOL_COL_GAP: i32 = 2;
/// Minimum horizontal room required to render all 12 Build tools in a compact
/// 4×3 grid directly clickable through `BuildLayout::press`.
const COMPACT_TOOL_COLS: i32 = 4;
const COMPACT_TOOL_ROWS: i32 = 3;
const COMPACT_TOOL_MIN_W: i32 = 14;

/// Native 6-column × 2-row pairing of [`TOOL_BUTTONS`] indices (top, bottom)
/// used when paginating tool buttons on sub-620 px narrow viewports.
const TOOL_COLUMNS: [(usize, usize); 6] = [(0, 6), (1, 7), (2, 8), (3, 9), (4, 10), (5, 11)];

/// OBJD byte offsets shared with `buy.rs`.
const OBJD_TYPE: usize = 0x12;
const OBJD_MASTER: usize = 0x14;
const OBJD_SUB_INDEX: usize = 0x16;
const OBJD_PRICE: usize = 0x24;
const OBJD_GRAPHICS: usize = 0x38;
const OBJD_TTAB: usize = 0x52;
const OBJD_CTSS: usize = 0x62;
const OBJD_BUILD_TYPE: usize = 0x8a;
const PERSON_TYPE: i16 = 2;

/// Native wall/fence catalog entries in `cWinArch` order (`0x4cd464`..
/// `0x4cd608`, `0x501c80`): `(style_id, str_entry_1based, default_pattern, default_price, fallback_name)`.
const BUILTIN_WALL_STYLES: [(u16, usize, u16, i32, &str); 5] = [
    (1, 1, 0, 70, "Wall Tool"),
    (2, 2, 0xf8, 10, "White Picket Fence"),
    (12, 4, 0xf9, 35, "Privacy Fence"),
    (13, 3, 0xfa, 45, "Wrought Iron Balustrade"),
    (14, 5, 0xfb, 45, "Monticello Balustrade"),
];

/// Terrain subtool selection (`BuildTool::Terrain`, `GameData/Build.iff STR# 128`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TerrainSubtool {
    #[default]
    Raise,
    Lower,
    Level,
    Grass,
}

impl TerrainSubtool {
    pub const ALL: [Self; 4] = [Self::Raise, Self::Lower, Self::Level, Self::Grass];

    pub const fn resource_id(self) -> i32 {
        match self {
            Self::Raise => id::SUBTOOL_TERRAIN_UP,
            Self::Lower => id::SUBTOOL_TERRAIN_DOWN,
            Self::Level => id::SUBTOOL_TERRAIN_LEVEL,
            Self::Grass => id::SUBTOOL_GRASS,
        }
    }

    const fn str_entry(self) -> usize {
        match self {
            Self::Raise => 1,
            Self::Lower => 2,
            Self::Level => 3,
            Self::Grass => 4,
        }
    }

    pub const fn fallback_name(self) -> &'static str {
        match self {
            Self::Raise => "Raise Terrain",
            Self::Lower => "Lower Terrain",
            Self::Level => "Level Terrain",
            Self::Grass => "Grass",
        }
    }
}

/// Water/Pool subtool selection (`BuildTool::Water`, `GameData/Build.iff STR# 133`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WaterSubtool {
    #[default]
    Pool,
    Water,
}

impl WaterSubtool {
    pub const ALL: [Self; 2] = [Self::Pool, Self::Water];

    pub const fn resource_id(self) -> i32 {
        match self {
            Self::Pool => id::SUBTOOL_POOL,
            Self::Water => id::SUBTOOL_WATER,
        }
    }

    const fn str_entry(self) -> usize {
        match self {
            Self::Pool => 1,
            Self::Water => 2,
        }
    }

    pub const fn fallback_price(self) -> i32 {
        match self {
            Self::Pool => 75,
            Self::Water => 15,
        }
    }

    pub const fn fallback_name(self) -> &'static str {
        match self {
            Self::Pool => "Pool Tool",
            Self::Water => "Water Tool",
        }
    }
}

/// Complete Collection 1.91 roof pitch preset (`Res_CPanel.h` `3900..=3904`,
/// `HOUS 0` float pitches `0.0`, `0.4`, `0.6`, `1.0`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoofPitchPreset {
    Flat,
    Shallow,
    #[default]
    Medium,
    Steep,
}

impl RoofPitchPreset {
    pub const ALL: [Self; 4] = [Self::Flat, Self::Shallow, Self::Medium, Self::Steep];

    pub const fn resource_id(self) -> i32 {
        match self {
            Self::Flat => id::SUBTOOL_ROOF_FLAT,
            Self::Shallow => id::SUBTOOL_ROOF_SHALLOW,
            Self::Medium => id::SUBTOOL_ROOF_MEDIUM,
            Self::Steep => id::SUBTOOL_ROOF_STEEP,
        }
    }

    pub const fn pitch_f32(self) -> f32 {
        match self {
            Self::Flat => 0.0,
            Self::Shallow => 0.4,
            Self::Medium => 0.6,
            Self::Steep => 1.0,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat Roof",
            Self::Shallow => "Shallow Roof Pitch",
            Self::Medium => "Medium Roof Pitch",
            Self::Steep => "Steep Roof Pitch",
        }
    }
}

/// One Terrain subtool catalog entry loaded from `GameData/Build.iff STR# 128`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainSubtoolEntry {
    pub subtool: TerrainSubtool,
    pub resource_id: i32,
    pub price: i32,
    pub name: String,
    pub description: String,
}

/// One Water/Pool subtool catalog entry loaded from `GameData/Build.iff STR# 133`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaterSubtoolEntry {
    pub subtool: WaterSubtool,
    pub resource_id: i32,
    pub price: i32,
    pub name: String,
    pub description: String,
}

/// One enumerated roof pattern from `GameData/Roofs/*.bmp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoofPatternEntry {
    /// Zero-based index inside [`BuildCatalog::roof_patterns`].
    pub index: usize,
    /// Texture stem without `.bmp` extension (e.g. `"r1_"`, `"MMSuburban1Roof"`).
    pub stem: String,
    /// Human-readable display label.
    pub name: String,
    /// Source `.bmp` path inside the install's `GameData/Roofs/`.
    pub path: PathBuf,
    /// Decoded 32×32 swatch image, if readable.
    pub swatch: Option<Image>,
}

/// Click action produced by Build Mode Part 2 subtool/roof controls and
/// narrow-viewport tool-strip pagination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BuildSubtoolAction {
    Terrain(TerrainSubtool),
    Water(WaterSubtool),
    RoofPitch(RoofPitchPreset),
    RoofPattern(usize),
    ToolPage(i32),
}

/// Interactive state of the Build Mode panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BuildState {
    /// Active Build Mode tool button, or `None` right after entering Build Mode.
    pub tool: Option<BuildTool>,
    /// Zero-based page index in the right-hand catalog strip.
    pub page: usize,
    /// Currently selected item within the active tool's catalog.
    pub selected: Option<BuildItemId>,
    /// Last selected wall/fence style (defaults to `1` Standard Wall).
    pub last_wall_style: u16,
    /// Last selected floor pattern ID, if any.
    pub last_floor: Option<u16>,
    /// Last selected wallpaper pattern ID, if any.
    pub last_wallpaper: Option<u16>,
    /// Active Terrain subtool (`Raise`, `Lower`, `Level`, or `Grass`).
    pub terrain_subtool: TerrainSubtool,
    /// Active Water/Pool subtool (`Pool` or `Water`).
    pub water_subtool: WaterSubtool,
    /// Active Roof pitch preset (`Flat`, `Shallow`, `Medium`, or `Steep`).
    pub roof_pitch: RoofPitchPreset,
    /// Active Roof pattern index into [`BuildCatalog::roof_patterns`].
    pub roof_pattern: usize,
    /// Active tool-strip page when viewport width is below 620 px.
    pub tool_page: usize,
}

impl BuildState {
    pub fn reset(&mut self) {
        *self = Self {
            last_wall_style: 1,
            ..Self::default()
        };
    }

    /// Selects a Build Mode tool and resets paging, restoring or defaulting
    /// the tool's active catalog item (`0x501197`..`0x50149d`).
    pub fn select_tool(&mut self, tool: BuildTool, catalog: &BuildCatalog) {
        self.tool = Some(tool);
        self.page = 0;
        self.selected = match tool {
            BuildTool::Wall => {
                let style = if self.last_wall_style == 0 {
                    1
                } else {
                    self.last_wall_style
                };
                self.last_wall_style = style;
                Some(BuildItemId::WallStyle(style))
            }
            BuildTool::Floor => {
                let id = self.last_floor.or_else(|| catalog.default_floor());
                self.last_floor = id;
                id.map(BuildItemId::Floor)
            }
            BuildTool::Wallpaper => {
                let id = self.last_wallpaper.or_else(|| catalog.default_wallpaper());
                self.last_wallpaper = id;
                id.map(BuildItemId::Wallpaper)
            }
            BuildTool::Roof => {
                if !catalog.roof_patterns.is_empty() {
                    self.roof_pattern = self.roof_pattern.min(catalog.roof_patterns.len() - 1);
                } else {
                    self.roof_pattern = 0;
                }
                None
            }
            _ => None,
        };
    }

    /// Selects a catalog item inside the active tool.
    pub fn select_item(&mut self, item: BuildItemId) {
        self.selected = Some(item);
        match item {
            BuildItemId::WallStyle(style) => self.last_wall_style = style,
            BuildItemId::Floor(id) => self.last_floor = Some(id),
            BuildItemId::Wallpaper(id) => self.last_wallpaper = Some(id),
            BuildItemId::Object(_) => {}
        }
    }

    /// Applies a [`BuildSubtoolAction`] (Terrain subtool, Water/Pool subtool,
    /// Roof pitch preset, Roof pattern selection, or narrow-viewport tool page)
    /// and returns `true` when state changed.
    pub fn apply_subtool(&mut self, action: BuildSubtoolAction, catalog: &BuildCatalog) -> bool {
        match action {
            BuildSubtoolAction::Terrain(sub) => {
                let changed = self.tool != Some(BuildTool::Terrain)
                    || self.terrain_subtool != sub
                    || self.selected.is_some();
                self.tool = Some(BuildTool::Terrain);
                self.terrain_subtool = sub;
                self.selected = None;
                changed
            }
            BuildSubtoolAction::Water(sub) => {
                let changed = self.tool != Some(BuildTool::Water)
                    || self.water_subtool != sub
                    || self.selected.is_some();
                self.tool = Some(BuildTool::Water);
                self.water_subtool = sub;
                self.selected = None;
                changed
            }
            BuildSubtoolAction::RoofPitch(pitch) => {
                let changed = self.tool != Some(BuildTool::Roof) || self.roof_pitch != pitch;
                self.tool = Some(BuildTool::Roof);
                self.roof_pitch = pitch;
                self.selected = None;
                changed
            }
            BuildSubtoolAction::RoofPattern(idx) => {
                if idx >= catalog.roof_patterns.len() {
                    return false;
                }
                let changed = self.tool != Some(BuildTool::Roof) || self.roof_pattern != idx;
                self.tool = Some(BuildTool::Roof);
                self.roof_pattern = idx;
                self.selected = None;
                changed
            }
            BuildSubtoolAction::ToolPage(delta) => {
                let next = if delta < 0 {
                    self.tool_page.saturating_sub(delta.unsigned_abs() as usize)
                } else {
                    self.tool_page.saturating_add(delta as usize)
                };
                if next != self.tool_page {
                    self.tool_page = next;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Advances the catalog strip page by `delta`.
    pub fn step_page(&mut self, delta: i32) -> bool {
        let next = if delta < 0 {
            self.page.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            self.page.saturating_add(delta as usize)
        };
        if next != self.page {
            self.page = next;
            true
        } else {
            false
        }
    }
}

/// One entry in the Build Mode catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildCatalogItem {
    pub id: BuildItemId,
    pub tool: BuildTool,
    pub price: i32,
    pub name: String,
    pub description: String,
    /// Native sort order override for built-in Wall/Fence tools (`0x501c80`).
    sort_rank: i32,
    raw_name: Vec<u8>,
}

/// Complete Build Mode catalog: walls/fences, floors, wallpapers, doors,
/// windows, stairs, fireplaces, plants, terrain subtools, water/pool subtools,
/// roof patterns/pitches, and tool tooltips.
#[derive(Debug, Clone, Default)]
pub struct BuildCatalog {
    items: Vec<BuildCatalogItem>,
    tool_tips: Vec<String>,
    terrain_subtools: Vec<TerrainSubtoolEntry>,
    water_subtools: Vec<WaterSubtoolEntry>,
    roof_patterns: Vec<RoofPatternEntry>,
    /// Price lookup tables for `sims_vm::BuildPrices`.
    pub style_prices: BTreeMap<u16, i32>,
    pub floor_prices: BTreeMap<u16, i32>,
    pub wall_prices: BTreeMap<u16, i32>,
}

impl BuildCatalog {
    /// Builds the full Build Mode catalog from `install`, `index`, `catalog`,
    /// built-in floor/wall IDs, and plug-in `.flr`/`.wll` maps.
    pub fn load(
        install: &Install,
        index: &Index,
        catalog: &Catalog,
        builtin_floors: &[u16],
        builtin_walls: &[u16],
        floor_plugins: &BTreeMap<u16, SurfacePlugin>,
        wall_plugins: &BTreeMap<u16, SurfacePlugin>,
    ) -> Self {
        let build_tables = load_build_tables(install);
        let tool_tips = load_tool_tips(install);
        let mut items = Vec::new();
        let mut style_prices = BTreeMap::new();
        let mut floor_prices = BTreeMap::new();
        let mut wall_prices = BTreeMap::new();

        let str_triplet =
            |table_id: u16, entry_1based: usize| -> Option<(i32, String, Vec<u8>, String)> {
                let table = build_tables.get(&table_id)?;
                let base = entry_1based.checked_sub(1)? * 3;
                let p_entry = table.entry(base, US_ENGLISH)?;
                let price = atoi(&p_entry.raw_text);
                let name_entry = table.entry(base + 1, US_ENGLISH);
                let name = name_entry.map(|e| e.text.clone()).unwrap_or_default();
                let raw_name = name_entry.map(|e| e.raw_text.clone()).unwrap_or_default();
                let desc = table
                    .entry(base + 2, US_ENGLISH)
                    .map(|e| e.text.clone())
                    .unwrap_or_default();
                Some((price, name, raw_name, desc))
            };

        // 1. Built-in Wall & Fence styles (`STR# 129`, `0x4cd464`..`0x4cd608`).
        for (rank, &(style_id, entry_idx, _pat, fallback_price, fallback_name)) in
            BUILTIN_WALL_STYLES.iter().enumerate()
        {
            let (price, name, raw_name, description) = str_triplet(STR_WALLS, entry_idx)
                .unwrap_or_else(|| {
                    (
                        fallback_price,
                        fallback_name.to_owned(),
                        fallback_name.as_bytes().to_vec(),
                        String::new(),
                    )
                });
            style_prices.insert(style_id, price);
            items.push(BuildCatalogItem {
                id: BuildItemId::WallStyle(style_id),
                tool: BuildTool::Wall,
                price,
                name,
                description,
                sort_rank: rank as i32 - 1024,
                raw_name,
            });
        }
        // Wall style 22/23 (doors/windows cut-outs) inherit standard wall price.
        let std_price = style_prices.get(&1).copied().unwrap_or(70);
        style_prices.insert(22, std_price);
        style_prices.insert(23, std_price);

        // 2. Built-in & Plug-in Floors (`STR# 130` & `STR# 134`).
        let (def_floor_price, def_floor_name, def_floor_raw, def_floor_desc) =
            str_triplet(STR_PLUGIN_FLOOR, 1).unwrap_or_else(|| {
                (
                    20,
                    "User Created Floor".to_owned(),
                    b"User Created Floor".to_vec(),
                    String::new(),
                )
            });
        let mut floor_items = Vec::new();
        for &key in builtin_floors {
            // Native road/sidewalk built-in tiles (`9..=13`) are excluded from
            // the player Build catalog (`0x4a140b`).
            if (9..=13).contains(&key) {
                continue;
            }
            if let Some((price, name, raw_name, description)) =
                str_triplet(STR_FLOORS, usize::from(key))
            {
                if name.is_empty() {
                    continue;
                }
                floor_prices.insert(key, price);
                floor_items.push(BuildCatalogItem {
                    id: BuildItemId::Floor(key),
                    tool: BuildTool::Floor,
                    price,
                    name,
                    description,
                    sort_rank: 0,
                    raw_name,
                });
            }
        }
        for (&key, plugin) in floor_plugins {
            let price = plugin.price.unwrap_or(def_floor_price);
            let name = plugin
                .name
                .clone()
                .unwrap_or_else(|| def_floor_name.clone());
            let raw_name = plugin
                .raw_name
                .clone()
                .unwrap_or_else(|| def_floor_raw.clone());
            let description = plugin
                .description
                .clone()
                .unwrap_or_else(|| def_floor_desc.clone());
            floor_prices.insert(key, price);
            floor_items.push(BuildCatalogItem {
                id: BuildItemId::Floor(key),
                tool: BuildTool::Floor,
                price,
                name,
                description,
                sort_rank: 0,
                raw_name,
            });
        }
        floor_items.sort_by(|a, b| {
            a.price
                .cmp(&b.price)
                .then_with(|| a.raw_name.cmp(&b.raw_name))
                .then_with(|| a.id.cmp(&b.id))
        });
        items.extend(floor_items);

        // 3. Built-in & Plug-in Wallpapers (`STR# 131` & `STR# 135`).
        let (def_wall_price, def_wall_name, def_wall_raw, def_wall_desc) =
            str_triplet(STR_PLUGIN_WALL, 1).unwrap_or_else(|| {
                (
                    5,
                    "User Created Wallpaper".to_owned(),
                    b"User Created Wallpaper".to_vec(),
                    String::new(),
                )
            });
        let mut wall_items = Vec::new();
        for &key in builtin_walls {
            if let Some((price, name, raw_name, description)) =
                str_triplet(STR_WALLPAPERS, usize::from(key))
            {
                if name.is_empty() {
                    continue;
                }
                wall_prices.insert(key, price);
                wall_items.push(BuildCatalogItem {
                    id: BuildItemId::Wallpaper(key),
                    tool: BuildTool::Wallpaper,
                    price,
                    name,
                    description,
                    sort_rank: 0,
                    raw_name,
                });
            }
        }
        for (&key, plugin) in wall_plugins {
            // Skip fence pattern range `248..=255` (`0x4c5be8`).
            if (248..=255).contains(&key) {
                continue;
            }
            let price = plugin.price.unwrap_or(def_wall_price);
            let name = plugin.name.clone().unwrap_or_else(|| def_wall_name.clone());
            let raw_name = plugin
                .raw_name
                .clone()
                .unwrap_or_else(|| def_wall_raw.clone());
            let description = plugin
                .description
                .clone()
                .unwrap_or_else(|| def_wall_desc.clone());
            wall_prices.insert(key, price);
            wall_items.push(BuildCatalogItem {
                id: BuildItemId::Wallpaper(key),
                tool: BuildTool::Wallpaper,
                price,
                name,
                description,
                sort_rank: 0,
                raw_name,
            });
        }
        wall_items.sort_by(|a, b| {
            a.price
                .cmp(&b.price)
                .then_with(|| a.raw_name.cmp(&b.raw_name))
                .then_with(|| a.id.cmp(&b.id))
        });
        items.extend(wall_items);

        // 4. Build Mode Catalog Objects (Doors, Windows, Columns, Stairs,
        // Fireplaces, Plants, Pool objects — `0x4cbb08`..`0x4cbb58`).
        let mut by_resource: BTreeMap<usize, Vec<(u32, u16)>> = BTreeMap::new();
        for object in &catalog.objects {
            by_resource
                .entry(object.resource)
                .or_default()
                .push((object.guid, object.chunk));
        }
        let mut obj_items = Vec::new();
        for (resource_idx, entries) in by_resource {
            let Some(resource) = index.resources.get(resource_idx) else {
                continue;
            };
            let Ok(bytes) = resource.read() else {
                continue;
            };
            let Ok(iff) = Iff::parse(bytes) else {
                continue;
            };
            for (guid, chunk_id) in entries {
                let Some(chunk) = iff.get(b"OBJD", chunk_id) else {
                    continue;
                };
                let data = iff.data(chunk);
                let master = read_i16(data, OBJD_MASTER);
                let sub_index = read_i16(data, OBJD_SUB_INDEX);
                if master != 0 && sub_index != -1 {
                    continue;
                }
                if read_i16(data, OBJD_TYPE) == PERSON_TYPE {
                    continue;
                }
                let build_type = read_i16(data, OBJD_BUILD_TYPE);
                let Some(tool) = tool_for_build_mode_type(build_type) else {
                    continue;
                };
                let graphics = read_i16(data, OBJD_GRAPHICS);
                let ttab = read_i16(data, OBJD_TTAB);
                if master == 0 && graphics == 0 && ttab <= 0 {
                    continue;
                }
                let ctss_id = read_i16(data, OBJD_CTSS);
                let (mut name, description, mut raw_name) = read_ctss(&iff, ctss_id);
                if name.is_empty() && !chunk.label.is_empty() {
                    name = chunk.label.clone();
                    raw_name = chunk.label.as_bytes().to_vec();
                }
                let price = i32::from(read_i16(data, OBJD_PRICE));
                obj_items.push(BuildCatalogItem {
                    id: BuildItemId::Object(guid),
                    tool,
                    price,
                    name,
                    description,
                    sort_rank: 0,
                    raw_name,
                });
            }
        }
        obj_items.sort_by(|a, b| {
            a.price
                .cmp(&b.price)
                .then_with(|| a.raw_name.cmp(&b.raw_name))
                .then_with(|| a.id.cmp(&b.id))
        });
        items.extend(obj_items);

        // 5. Terrain subtools (`STR# 128`: Raise, Lower, Level, Grass).
        let terrain_subtools: Vec<TerrainSubtoolEntry> = TerrainSubtool::ALL
            .into_iter()
            .map(|subtool| {
                let (price, name, _raw, description) =
                    str_triplet(STR_TERRAIN, subtool.str_entry()).unwrap_or_else(|| {
                        (
                            1,
                            subtool.fallback_name().to_owned(),
                            subtool.fallback_name().as_bytes().to_vec(),
                            String::new(),
                        )
                    });
                TerrainSubtoolEntry {
                    subtool,
                    resource_id: subtool.resource_id(),
                    price,
                    name,
                    description,
                }
            })
            .collect();

        // 6. Water & Swimming Pool subtools (`STR# 133`: Pool Tool, Water Tool).
        let water_subtools: Vec<WaterSubtoolEntry> = WaterSubtool::ALL
            .into_iter()
            .map(|subtool| {
                let (price, name, _raw, description) = str_triplet(STR_POOL, subtool.str_entry())
                    .unwrap_or_else(|| {
                        (
                            subtool.fallback_price(),
                            subtool.fallback_name().to_owned(),
                            subtool.fallback_name().as_bytes().to_vec(),
                            String::new(),
                        )
                    });
                WaterSubtoolEntry {
                    subtool,
                    resource_id: subtool.resource_id(),
                    price,
                    name,
                    description,
                }
            })
            .collect();

        // 7. Roof patterns (`GameData/Roofs/*.bmp`).
        let roof_patterns = load_roof_patterns(install);

        Self {
            items,
            tool_tips,
            terrain_subtools,
            water_subtools,
            roof_patterns,
            style_prices,
            floor_prices,
            wall_prices,
        }
    }

    /// Terrain subtool entries (`Raise`, `Lower`, `Level`, `Grass`) loaded from
    /// `GameData/Build.iff STR# 128`.
    pub fn terrain_subtools(&self) -> &[TerrainSubtoolEntry] {
        &self.terrain_subtools
    }

    /// Water & Swimming Pool subtool entries (`Pool Tool`, `Water Tool`) loaded
    /// from `GameData/Build.iff STR# 133`.
    pub fn water_subtools(&self) -> &[WaterSubtoolEntry] {
        &self.water_subtools
    }

    /// Enumerated roof patterns from `GameData/Roofs/*.bmp`.
    pub fn roof_patterns(&self) -> &[RoofPatternEntry] {
        &self.roof_patterns
    }

    /// Looks up a roof pattern entry by zero-based index.
    pub fn roof_pattern(&self, index: usize) -> Option<&RoofPatternEntry> {
        self.roof_patterns.get(index)
    }

    /// Finds a roof pattern index by texture stem (case-insensitive, optional `.bmp`).
    pub fn roof_pattern_index(&self, stem: &str) -> Option<usize> {
        let trimmed = stem.trim();
        let bytes = trimmed.as_bytes();
        let needle = if bytes.len() >= 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".bmp") {
            &trimmed[..trimmed.len() - 4]
        } else {
            trimmed
        };
        self.roof_patterns
            .iter()
            .position(|r| r.stem.eq_ignore_ascii_case(needle))
    }

    /// Price of a Terrain subtool (`GameData/Build.iff STR# 128`, default `$1`).
    pub fn terrain_price(&self, subtool: TerrainSubtool) -> i32 {
        self.terrain_subtools
            .iter()
            .find(|e| e.subtool == subtool)
            .map(|e| e.price)
            .unwrap_or(1)
    }

    /// Price per tile of the Swimming Pool tool (`STR# 133[1]`, default `$75`).
    pub fn pool_price(&self) -> i32 {
        self.water_subtools
            .iter()
            .find(|e| e.subtool == WaterSubtool::Pool)
            .map(|e| e.price)
            .unwrap_or(75)
    }

    /// Price per tile of the Pond/Water tool (`STR# 133[2]`, default `$15`).
    pub fn water_price(&self) -> i32 {
        self.water_subtools
            .iter()
            .find(|e| e.subtool == WaterSubtool::Water)
            .map(|e| e.price)
            .unwrap_or(15)
    }

    /// Creates a synthetic catalog populated with roof patterns (and default
    /// Terrain/Water subtools) for unit tests without a full game install.
    pub fn with_roof_patterns(mut self, stems: &[&str]) -> Self {
        self.roof_patterns = stems
            .iter()
            .enumerate()
            .map(|(index, &stem)| RoofPatternEntry {
                index,
                stem: stem.to_owned(),
                name: format_roof_name(stem),
                path: PathBuf::from(format!("GameData/Roofs/{stem}.bmp")),
                swatch: None,
            })
            .collect();
        self.ensure_default_subtools();
        self
    }

    fn ensure_default_subtools(&mut self) {
        if self.terrain_subtools.is_empty() {
            self.terrain_subtools = TerrainSubtool::ALL
                .into_iter()
                .map(|subtool| TerrainSubtoolEntry {
                    subtool,
                    resource_id: subtool.resource_id(),
                    price: 1,
                    name: subtool.fallback_name().to_owned(),
                    description: String::new(),
                })
                .collect();
        }
        if self.water_subtools.is_empty() {
            self.water_subtools = WaterSubtool::ALL
                .into_iter()
                .map(|subtool| WaterSubtoolEntry {
                    subtool,
                    resource_id: subtool.resource_id(),
                    price: subtool.fallback_price(),
                    name: subtool.fallback_name().to_owned(),
                    description: String::new(),
                })
                .collect();
        }
    }

    /// Items belonging to `tool` in native `cWinArch` display order.
    pub fn items_for(&self, tool: BuildTool) -> Vec<&BuildCatalogItem> {
        let mut list: Vec<&BuildCatalogItem> =
            self.items.iter().filter(|i| i.tool == tool).collect();
        if tool == BuildTool::Wall {
            list.sort_by(|a, b| {
                a.sort_rank
                    .cmp(&b.sort_rank)
                    .then_with(|| a.price.cmp(&b.price))
                    .then_with(|| a.raw_name.cmp(&b.raw_name))
                    .then_with(|| a.id.cmp(&b.id))
            });
        }
        list
    }

    /// Looks up a single Build catalog entry by its [`BuildItemId`].
    pub fn get(&self, id: BuildItemId) -> Option<&BuildCatalogItem> {
        self.items.iter().find(|i| i.id == id)
    }

    /// Default Floor ID (slot 0 of the Floor catalog).
    pub fn default_floor(&self) -> Option<u16> {
        self.items_for(BuildTool::Floor)
            .first()
            .and_then(|item| match item.id {
                BuildItemId::Floor(k) => Some(k),
                _ => None,
            })
    }

    /// Default Wallpaper ID (slot 0 of the Wallpaper catalog).
    pub fn default_wallpaper(&self) -> Option<u16> {
        self.items_for(BuildTool::Wallpaper)
            .first()
            .and_then(|item| match item.id {
                BuildItemId::Wallpaper(k) => Some(k),
                _ => None,
            })
    }

    /// Localized tooltip string for `tool` (`GameData/UIText.iff STR# 139`).
    pub fn tool_tip(&self, tool: BuildTool) -> &str {
        let idx = TOOL_BUTTONS
            .iter()
            .position(|(t, _, _, _)| *t == tool)
            .unwrap_or(0);
        self.tool_tips
            .get(idx)
            .filter(|s| !s.is_empty())
            .map(String::as_str)
            .unwrap_or(TOOL_BUTTONS[idx].3)
    }
}

/// Maps OBJD word 69 (`build_mode_type`) to its Build Mode tool (`0x4cbb08`).
fn tool_for_build_mode_type(build_type: i16) -> Option<BuildTool> {
    match build_type {
        3 => Some(BuildTool::Doors),
        4 => Some(BuildTool::Windows),
        5 => Some(BuildTool::Stairs),
        6 => Some(BuildTool::Plants),
        7 => Some(BuildTool::Fireplace),
        8 => Some(BuildTool::Wall),
        9 => Some(BuildTool::Water),
        _ => None,
    }
}

/// Cached 45×45 swatch thumbnails for floors, wallpapers, and wall/fence styles.
#[derive(Debug, Default)]
pub struct BuildSwatches {
    swatches: BTreeMap<BuildItemId, Image>,
}

impl BuildSwatches {
    pub fn get(&self, id: BuildItemId) -> Option<&Image> {
        self.swatches.get(&id)
    }

    pub fn insert(&mut self, id: BuildItemId, image: Image) {
        self.swatches.insert(id, image);
    }

    pub fn contains(&self, id: BuildItemId) -> bool {
        self.swatches.contains_key(&id)
    }
}

/// One laid-out interactive widget on the Build Mode strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildWidget {
    pub rect: Rect,
    pub kind: BuildWidgetKind,
    pub command: Option<Command>,
    pub tooltip: Option<String>,
}

/// How a [`BuildWidget`] draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildWidgetKind {
    Button {
        id: i32,
        selected: bool,
        enabled: bool,
    },
    Slot {
        item: BuildItemId,
        selected: bool,
        affordable: bool,
    },
}

/// Hover/selected popup card for a Build Mode catalog entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPopup {
    pub rect: Rect,
    pub item: BuildItemId,
    pub name: String,
    pub price: i32,
    pub description: String,
}

/// One laid-out Build Mode Part 2 subtool or roof pattern/pitch control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildSubtoolWidget {
    pub rect: Rect,
    pub action: BuildSubtoolAction,
    pub resource_id: Option<i32>,
    pub roof_pattern: Option<usize>,
    pub selected: bool,
    pub affordable: bool,
    pub name: String,
    pub price: i32,
    pub description: String,
}

/// Hover popup card for a Build Mode Part 2 subtool or roof pattern/pitch control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildSubtoolPopup {
    pub rect: Rect,
    pub action: BuildSubtoolAction,
    pub resource_id: Option<i32>,
    pub roof_pattern: Option<usize>,
    pub name: String,
    pub price: i32,
    pub description: String,
}

/// Laid-out Build Mode strip widgets, Part 2 subtool/roof controls, and
/// optional popup cards.
#[derive(Debug, Clone, Default)]
pub struct BuildLayout {
    pub widgets: Vec<BuildWidget>,
    pub subtools: Vec<BuildSubtoolWidget>,
    pub popup: Option<BuildPopup>,
    pub subtool_popup: Option<BuildSubtoolPopup>,
    pub page: usize,
    pub total_pages: usize,
    pub tool_page: usize,
    pub total_tool_pages: usize,
}

impl BuildLayout {
    pub fn covers(&self, point: [i32; 2]) -> bool {
        self.widgets
            .iter()
            .any(|w| w.rect.contains(point[0], point[1]))
            || self
                .subtools
                .iter()
                .any(|w| w.rect.contains(point[0], point[1]))
    }

    pub fn press(&self, point: [i32; 2]) -> Option<Command> {
        self.widgets
            .iter()
            .rev()
            .find(|w| w.rect.contains(point[0], point[1]))
            .and_then(|w| w.command.clone())
    }

    /// Hit-tests Build Mode Part 2 subtool/roof/narrow-tool-page controls.
    pub fn press_subtool(&self, point: [i32; 2]) -> Option<BuildSubtoolAction> {
        self.subtools
            .iter()
            .rev()
            .find(|w| w.rect.contains(point[0], point[1]))
            .map(|w| w.action)
    }
}

/// Lays out the Build Mode strip (`cWinArch`: Undo/Redo, 12 tool buttons,
/// right-hand 2×5 catalog strip, Terrain/Water/Roof subtools & patterns, and
/// optional popup card). On viewports narrower than 620 px, tool-button
/// columns paginate cleanly to the left of the catalog strip with zero overlap.
#[allow(clippy::too_many_arguments)]
pub fn layout(
    size: [i32; 2],
    catalog: &BuildCatalog,
    state: &BuildState,
    art: &Art,
    available_funds: i32,
    can_undo: bool,
    can_redo: bool,
    pointer: Option<[i32; 2]>,
) -> BuildLayout {
    let [width, height] = size.map(|v| v.clamp(0, 1 << 15));
    let strip_y = height - STRIP;
    let btn_size = |id: i32| -> [i32; 2] {
        art.cell(id, 4, 0)
            .map(|img| [img.width as i32, img.height as i32])
            .unwrap_or_else(|| match id {
                id::UNDO | id::REDO => [30, 24],
                _ => FALLBACK_BTN,
            })
    };
    let arrow_size = |id: i32| -> [i32; 2] {
        art.cell(id, 4, 0)
            .map(|img| [img.width as i32, img.height as i32])
            .unwrap_or([9, 49])
    };
    let pitch_size = |id: i32| -> [i32; 2] {
        art.cell(id, 4, 0)
            .map(|img| [img.width as i32, img.height as i32])
            .unwrap_or([26, 17])
    };

    let mut widgets = Vec::new();
    let mut subtools = Vec::new();

    // Undo / Redo buttons (`0x5005a3`, `0x50065b`).
    let [uw, uh] = btn_size(id::UNDO);
    let [rw, rh] = btn_size(id::REDO);
    let undo_right = UNDO_POS[0] + uw.max(rw);
    if UNDO_POS[0] + uw <= width {
        widgets.push(BuildWidget {
            rect: Rect::new(UNDO_POS[0], strip_y + UNDO_POS[1], uw, uh),
            kind: BuildWidgetKind::Button {
                id: id::UNDO,
                selected: false,
                enabled: can_undo,
            },
            command: can_undo.then_some(Command::Undo),
            tooltip: Some("Undo".into()),
        });
    }
    if REDO_POS[0] + rw <= width {
        widgets.push(BuildWidget {
            rect: Rect::new(REDO_POS[0], strip_y + REDO_POS[1], rw, rh),
            kind: BuildWidgetKind::Button {
                id: id::REDO,
                selected: false,
                enabled: can_redo,
            },
            command: can_redo.then_some(Command::Redo),
            tooltip: Some("Redo".into()),
        });
    }

    // Determine right-hand catalog/subtool grid geometry using measured art
    // sizes so narrow viewports never overlap tools, swatches, or page arrows.
    let cell = CELL as i32;
    let [prev_aw, prev_ah] = arrow_size(id::PREV_ARROW);
    let [next_aw, next_ah] = arrow_size(id::NEXT_ARROW);
    let is_roof = state.tool == Some(BuildTool::Roof);
    let has_right_grid = match state.tool {
        Some(BuildTool::Terrain) => true,
        Some(BuildTool::Water) => true,
        Some(BuildTool::Roof) => true,
        Some(tool) => !catalog.items_for(tool).is_empty(),
        None => false,
    };

    let max_pitch_w = RoofPitchPreset::ALL
        .iter()
        .map(|p| pitch_size(p.resource_id())[0])
        .max()
        .unwrap_or(26);
    let roof_pitch_col_w = max_pitch_w + 4;
    let roof_extra = if is_roof { roof_pitch_col_w } else { 0 };
    let native_tools_right = TOOL_BUTTONS
        .iter()
        .map(|&(_, res_id, pos, _)| pos[0] + btn_size(res_id)[0])
        .max()
        .unwrap_or(538);

    let wide_next_arrow_x = (width - NEXT_ARROW_MARGIN)
        .min((width - next_aw).max(0))
        .max(0);
    let wide_grid_right = (width - GRID_RIGHT_MARGIN)
        .min((wide_next_arrow_x - 1).max(0))
        .max(0);
    let wide_fits = width >= NARROW_WIDTH
        && native_tools_right <= width
        && (!has_right_grid
            || (native_tools_right < PREV_ARROW_X
                && PREV_ARROW_X + prev_aw <= GRID_LEFT
                && wide_grid_right - GRID_LEFT >= cell + roof_extra));
    let narrow = !wide_fits;

    let (grid_left, grid_right, prev_arrow_x, next_arrow_x) = if !narrow {
        (
            if has_right_grid { GRID_LEFT } else { width },
            wide_grid_right,
            PREV_ARROW_X,
            wide_next_arrow_x,
        )
    } else {
        let next_x = (width - next_aw - 2).max(0);
        let right = (next_x - 2).max(0);
        let tools_left = (undo_right + 4).max(TOOL_STRIP_LEFT);
        let min_grid_left = undo_right + prev_aw + 3;
        let min_compact_tools = COMPACT_TOOL_COLS * 16 + (COMPACT_TOOL_COLS - 1) * TOOL_COL_GAP;
        let left = if !has_right_grid {
            width
        } else {
            let two_col = right - cell * 2 - roof_extra;
            let one_col = right - cell - roof_extra;
            if two_col >= tools_left + min_compact_tools + prev_aw + 6 {
                two_col
            } else if one_col >= min_grid_left {
                one_col
            } else {
                min_grid_left.min(right)
            }
        };
        let prev_x = (left - prev_aw - 2).max(0);
        (left, right, prev_x, next_x)
    };

    // 12 Build Tool buttons (`0x6f2bf0`): exact native coordinates on wide
    // viewports; on narrow viewports compact all 12 tools into a non-overlapping
    // grid strictly left of `prev_arrow_x` so every tool stays directly
    // reachable through `BuildLayout::press` without hiding behind a pager.
    let mut tool_page = 0usize;
    let mut total_tool_pages = 1usize;
    if !narrow {
        for &(tool, res_id, pos, _) in &TOOL_BUTTONS {
            let [w, h] = btn_size(res_id);
            widgets.push(BuildWidget {
                rect: Rect::new(pos[0], strip_y + pos[1], w, h),
                kind: BuildWidgetKind::Button {
                    id: res_id,
                    selected: state.tool == Some(tool),
                    enabled: true,
                },
                command: Some(Command::BuildTool(tool)),
                tooltip: Some(catalog.tool_tip(tool).to_owned()),
            });
        }
    } else {
        let tools_left = (undo_right + 4).max(TOOL_STRIP_LEFT);
        let tools_right = if has_right_grid {
            (prev_arrow_x - 3).max(tools_left)
        } else {
            (width - 4).max(tools_left)
        };
        let avail_w = (tools_right - tools_left).max(0);
        let col_widths: [i32; 6] = std::array::from_fn(|ci| {
            let (top_i, bot_i) = TOOL_COLUMNS[ci];
            btn_size(TOOL_BUTTONS[top_i].1)[0].max(btn_size(TOOL_BUTTONS[bot_i].1)[0])
        });
        let total_6x2_w: i32 = col_widths.iter().sum::<i32>() + TOOL_COL_GAP * 5;
        let min_4x3_w =
            COMPACT_TOOL_COLS * COMPACT_TOOL_MIN_W + (COMPACT_TOOL_COLS - 1) * TOOL_COL_GAP;

        if total_6x2_w <= avail_w {
            let mut x = tools_left;
            for (ci, &cw) in col_widths.iter().enumerate() {
                let (top_i, bot_i) = TOOL_COLUMNS[ci];
                for &(idx, row_y) in &[(top_i, 4), (bot_i, 52)] {
                    let (tool, res_id, _, _) = TOOL_BUTTONS[idx];
                    let [w, h] = btn_size(res_id);
                    let h = h.min(45);
                    if x + w <= tools_right {
                        widgets.push(BuildWidget {
                            rect: Rect::new(x, strip_y + row_y, w, h),
                            kind: BuildWidgetKind::Button {
                                id: res_id,
                                selected: state.tool == Some(tool),
                                enabled: true,
                            },
                            command: Some(Command::BuildTool(tool)),
                            tooltip: Some(catalog.tool_tip(tool).to_owned()),
                        });
                    }
                }
                x += cw + TOOL_COL_GAP;
            }
        } else if avail_w >= min_4x3_w {
            let row_gap = 2;
            let btn_w = (avail_w - (COMPACT_TOOL_COLS - 1) * TOOL_COL_GAP) / COMPACT_TOOL_COLS;
            let btn_h = (STRIP - 4 - (COMPACT_TOOL_ROWS - 1) * row_gap) / COMPACT_TOOL_ROWS;
            for (idx, &(tool, res_id, _, _)) in TOOL_BUTTONS.iter().enumerate() {
                let col = (idx as i32) % COMPACT_TOOL_COLS;
                let row = (idx as i32) / COMPACT_TOOL_COLS;
                let x = tools_left + col * (btn_w + TOOL_COL_GAP);
                let y = strip_y + 2 + row * (btn_h + row_gap);
                widgets.push(BuildWidget {
                    rect: Rect::new(x, y, btn_w, btn_h),
                    kind: BuildWidgetKind::Button {
                        id: res_id,
                        selected: state.tool == Some(tool),
                        enabled: true,
                    },
                    command: Some(Command::BuildTool(tool)),
                    tooltip: Some(catalog.tool_tip(tool).to_owned()),
                });
            }
        } else if avail_w >= COMPACT_TOOL_MIN_W + next_aw + 2 {
            // Ultra-narrow fallback (< ~430 px): paginate compact 3-row columns
            // without ever dropping a tool or overflowing `tools_right`.
            let usable_w = avail_w - next_aw - 2;
            let cols_per_page =
                ((usable_w + TOOL_COL_GAP) / (COMPACT_TOOL_MIN_W + TOOL_COL_GAP)).max(1) as usize;
            let per_page = cols_per_page * (COMPACT_TOOL_ROWS as usize);
            total_tool_pages = TOOL_BUTTONS.len().div_ceil(per_page).max(1);
            tool_page = state.tool_page % total_tool_pages;
            let start = tool_page * per_page;
            let col_w = ((usable_w - (cols_per_page as i32 - 1) * TOOL_COL_GAP)
                / cols_per_page as i32)
                .max(1);
            let row_gap = 2;
            let btn_h = (STRIP - 4 - (COMPACT_TOOL_ROWS - 1) * row_gap) / COMPACT_TOOL_ROWS;
            for (slot, &(tool, res_id, _, _)) in
                TOOL_BUTTONS.iter().skip(start).take(per_page).enumerate()
            {
                let col = (slot / COMPACT_TOOL_ROWS as usize) as i32;
                let row = (slot % COMPACT_TOOL_ROWS as usize) as i32;
                let x = tools_left + col * (col_w + TOOL_COL_GAP);
                let y = strip_y + 2 + row * (btn_h + row_gap);
                widgets.push(BuildWidget {
                    rect: Rect::new(x, y, col_w, btn_h),
                    kind: BuildWidgetKind::Button {
                        id: res_id,
                        selected: state.tool == Some(tool),
                        enabled: true,
                    },
                    command: Some(Command::BuildTool(tool)),
                    tooltip: Some(catalog.tool_tip(tool).to_owned()),
                });
            }
            if total_tool_pages > 1 {
                subtools.push(BuildSubtoolWidget {
                    rect: Rect::new(tools_right - next_aw, strip_y + ARROW_Y, next_aw, next_ah),
                    action: BuildSubtoolAction::ToolPage(1),
                    resource_id: Some(id::NEXT_ARROW),
                    roof_pattern: None,
                    selected: false,
                    affordable: true,
                    name: "More Build Tools".to_owned(),
                    price: 0,
                    description: "Cycle Build Mode tool strip page.".to_owned(),
                });
            }
        }
    }

    let mut page = state.page;
    let mut total_pages = 0;

    match state.tool {
        Some(BuildTool::Terrain) => {
            let entries: Vec<TerrainSubtoolEntry> = if catalog.terrain_subtools.is_empty() {
                TerrainSubtool::ALL
                    .into_iter()
                    .map(|subtool| TerrainSubtoolEntry {
                        subtool,
                        resource_id: subtool.resource_id(),
                        price: 1,
                        name: subtool.fallback_name().to_owned(),
                        description: String::new(),
                    })
                    .collect()
            } else {
                catalog.terrain_subtools.clone()
            };
            let cols = ((grid_right - grid_left) / cell).max(0) as usize;
            if cols > 0 {
                let per_page = cols * GRID_ROWS;
                total_pages = entries.len().div_ceil(per_page);
                page = if total_pages > 0 {
                    page.min(total_pages - 1)
                } else {
                    0
                };
                let start = page * per_page;
                for (slot, entry) in entries.iter().skip(start).take(per_page).enumerate() {
                    let col = (slot % cols) as i32;
                    let row = (slot / cols) as i32;
                    let rect = Rect::new(
                        grid_left + col * cell,
                        strip_y + GRID_TOP + row * cell,
                        cell,
                        cell,
                    );
                    subtools.push(BuildSubtoolWidget {
                        rect,
                        action: BuildSubtoolAction::Terrain(entry.subtool),
                        resource_id: Some(entry.resource_id),
                        roof_pattern: None,
                        selected: state.terrain_subtool == entry.subtool,
                        affordable: entry.price <= available_funds,
                        name: entry.name.clone(),
                        price: entry.price,
                        description: entry.description.clone(),
                    });
                }
                push_page_arrows(
                    &mut widgets,
                    page,
                    start + per_page < entries.len(),
                    prev_arrow_x,
                    next_arrow_x,
                    undo_right,
                    grid_left,
                    grid_right,
                    width,
                    strip_y,
                    [prev_aw, prev_ah],
                    [next_aw, next_ah],
                );
            }
        }
        Some(BuildTool::Water) => {
            let water_entries: Vec<WaterSubtoolEntry> = if catalog.water_subtools.is_empty() {
                WaterSubtool::ALL
                    .into_iter()
                    .map(|subtool| WaterSubtoolEntry {
                        subtool,
                        resource_id: subtool.resource_id(),
                        price: subtool.fallback_price(),
                        name: subtool.fallback_name().to_owned(),
                        description: String::new(),
                    })
                    .collect()
            } else {
                catalog.water_subtools.clone()
            };
            let obj_items = catalog.items_for(BuildTool::Water);
            let total_entries = water_entries.len() + obj_items.len();
            let cols = ((grid_right - grid_left) / cell).max(0) as usize;
            if cols > 0 {
                let per_page = cols * GRID_ROWS;
                total_pages = total_entries.div_ceil(per_page);
                page = if total_pages > 0 {
                    page.min(total_pages - 1)
                } else {
                    0
                };
                let start = page * per_page;
                for slot in 0..per_page {
                    let global_idx = start + slot;
                    if global_idx >= total_entries {
                        break;
                    }
                    let col = (slot % cols) as i32;
                    let row = (slot / cols) as i32;
                    let rect = Rect::new(
                        grid_left + col * cell,
                        strip_y + GRID_TOP + row * cell,
                        cell,
                        cell,
                    );
                    if global_idx < water_entries.len() {
                        let entry = &water_entries[global_idx];
                        subtools.push(BuildSubtoolWidget {
                            rect,
                            action: BuildSubtoolAction::Water(entry.subtool),
                            resource_id: Some(entry.resource_id),
                            roof_pattern: None,
                            selected: state.selected.is_none()
                                && state.water_subtool == entry.subtool,
                            affordable: entry.price <= available_funds,
                            name: entry.name.clone(),
                            price: entry.price,
                            description: entry.description.clone(),
                        });
                    } else {
                        let item = obj_items[global_idx - water_entries.len()];
                        widgets.push(BuildWidget {
                            rect,
                            kind: BuildWidgetKind::Slot {
                                item: item.id,
                                selected: state.selected == Some(item.id),
                                affordable: item.price <= available_funds,
                            },
                            command: Some(Command::BuildItem(item.id)),
                            tooltip: None,
                        });
                    }
                }
                push_page_arrows(
                    &mut widgets,
                    page,
                    start + per_page < total_entries,
                    prev_arrow_x,
                    next_arrow_x,
                    undo_right,
                    grid_left,
                    grid_right,
                    width,
                    strip_y,
                    [prev_aw, prev_ah],
                    [next_aw, next_ah],
                );
            }
        }
        Some(BuildTool::Roof) => {
            // Left pitch column (Flat, Shallow, Medium, Steep) fitted cleanly
            // within the strip height using measured art dimensions.
            if grid_left + max_pitch_w <= grid_right {
                let raw_heights: [i32; 4] =
                    std::array::from_fn(|i| pitch_size(RoofPitchPreset::ALL[i].resource_id())[1]);
                let sum_h: i32 = raw_heights.iter().sum();
                let (mut py, gap, cap_h) = if sum_h + 9 <= STRIP - 10 {
                    (strip_y + 6, 3, i32::MAX)
                } else if sum_h + 3 <= STRIP - 4 {
                    (strip_y + 2, ((STRIP - 4 - sum_h) / 3).max(1), i32::MAX)
                } else {
                    (strip_y + 2, 1, ((STRIP - 4 - 3) / 4).max(1))
                };
                for (i, preset) in RoofPitchPreset::ALL.into_iter().enumerate() {
                    let [pw, _] = pitch_size(preset.resource_id());
                    let ph = raw_heights[i].min(cap_h);
                    subtools.push(BuildSubtoolWidget {
                        rect: Rect::new(grid_left, py, pw.min(max_pitch_w), ph),
                        action: BuildSubtoolAction::RoofPitch(preset),
                        resource_id: Some(preset.resource_id()),
                        roof_pattern: None,
                        selected: state.roof_pitch == preset,
                        affordable: true,
                        name: preset.label().to_owned(),
                        price: 0,
                        description: format!("Set roof slope pitch to {:.1}.", preset.pitch_f32()),
                    });
                    py += ph + gap;
                }
            }
            // Paginated `GameData/Roofs/*.bmp` pattern swatches strictly within
            // `pattern_left..grid_right` (never overflowing into `next_arrow_x`).
            let pattern_left = grid_left + roof_pitch_col_w;
            let patterns = catalog.roof_patterns();
            let cols = ((grid_right - pattern_left) / cell).max(0) as usize;
            if !patterns.is_empty() && cols > 0 {
                let per_page = cols * GRID_ROWS;
                total_pages = patterns.len().div_ceil(per_page);
                page = if total_pages > 0 {
                    page.min(total_pages - 1)
                } else {
                    0
                };
                let start = page * per_page;
                for (slot, entry) in patterns.iter().skip(start).take(per_page).enumerate() {
                    let col = (slot % cols) as i32;
                    let row = (slot / cols) as i32;
                    let rect = Rect::new(
                        pattern_left + col * cell,
                        strip_y + GRID_TOP + row * cell,
                        cell,
                        cell,
                    );
                    subtools.push(BuildSubtoolWidget {
                        rect,
                        action: BuildSubtoolAction::RoofPattern(entry.index),
                        resource_id: None,
                        roof_pattern: Some(entry.index),
                        selected: state.roof_pattern == entry.index,
                        affordable: true,
                        name: entry.name.clone(),
                        price: 0,
                        description: format!("Roof texture ({})", entry.stem),
                    });
                }
                push_page_arrows(
                    &mut widgets,
                    page,
                    start + per_page < patterns.len(),
                    prev_arrow_x,
                    next_arrow_x,
                    undo_right,
                    grid_left,
                    grid_right,
                    width,
                    strip_y,
                    [prev_aw, prev_ah],
                    [next_aw, next_ah],
                );
            }
        }
        Some(tool) => {
            let items = catalog.items_for(tool);
            let cols = ((grid_right - grid_left) / cell).max(0) as usize;
            if !items.is_empty() && cols > 0 {
                let per_page = cols * GRID_ROWS;
                total_pages = items.len().div_ceil(per_page);
                if total_pages > 0 {
                    page = page.min(total_pages - 1);
                } else {
                    page = 0;
                }
                let start = page * per_page;
                for (slot, item) in items.iter().skip(start).take(per_page).enumerate() {
                    let col = (slot % cols) as i32;
                    let row = (slot / cols) as i32;
                    let rect = Rect::new(
                        grid_left + col * cell,
                        strip_y + GRID_TOP + row * cell,
                        cell,
                        cell,
                    );
                    widgets.push(BuildWidget {
                        rect,
                        kind: BuildWidgetKind::Slot {
                            item: item.id,
                            selected: state.selected == Some(item.id),
                            affordable: item.price <= available_funds,
                        },
                        command: Some(Command::BuildItem(item.id)),
                        tooltip: None,
                    });
                }
                push_page_arrows(
                    &mut widgets,
                    page,
                    start + per_page < items.len(),
                    prev_arrow_x,
                    next_arrow_x,
                    undo_right,
                    grid_left,
                    grid_right,
                    width,
                    strip_y,
                    [prev_aw, prev_ah],
                    [next_aw, next_ah],
                );
            }
        }
        None => {}
    }

    // Show popup card for hovered catalog slot (when hovering the strip).
    let hovered_item = pointer.and_then(|[px, py]| {
        widgets.iter().rev().find_map(|w| {
            if !w.rect.contains(px, py) {
                return None;
            }
            match w.kind {
                BuildWidgetKind::Slot { item, .. } => Some(item),
                BuildWidgetKind::Button { .. } => None,
            }
        })
    });
    let popup = hovered_item.and_then(|id| catalog.get(id)).map(|entry| {
        let rect = popup_rect(width, strip_y, &entry.description, art);
        BuildPopup {
            rect,
            item: entry.id,
            name: entry.name.clone(),
            price: entry.price,
            description: entry.description.clone(),
        }
    });

    // Show popup card for hovered Part 2 subtool or roof pattern/pitch control.
    let subtool_popup = if popup.is_none() {
        pointer.and_then(|[px, py]| {
            subtools
                .iter()
                .rev()
                .find(|w| {
                    w.rect.contains(px, py) && !matches!(w.action, BuildSubtoolAction::ToolPage(_))
                })
                .map(|w| BuildSubtoolPopup {
                    rect: popup_rect(width, strip_y, &w.description, art),
                    action: w.action,
                    resource_id: w.resource_id,
                    roof_pattern: w.roof_pattern,
                    name: w.name.clone(),
                    price: w.price,
                    description: w.description.clone(),
                })
        })
    } else {
        None
    };

    BuildLayout {
        widgets,
        subtools,
        popup,
        subtool_popup,
        page,
        total_pages,
        tool_page,
        total_tool_pages,
    }
}

#[allow(clippy::too_many_arguments)]
fn push_page_arrows(
    widgets: &mut Vec<BuildWidget>,
    page: usize,
    has_next: bool,
    prev_arrow_x: i32,
    next_arrow_x: i32,
    undo_right: i32,
    grid_left: i32,
    grid_right: i32,
    width: i32,
    strip_y: i32,
    prev_size: [i32; 2],
    next_size: [i32; 2],
) {
    if page > 0 && prev_arrow_x >= undo_right && prev_arrow_x + prev_size[0] <= grid_left {
        widgets.push(BuildWidget {
            rect: Rect::new(prev_arrow_x, strip_y + ARROW_Y, prev_size[0], prev_size[1]),
            kind: BuildWidgetKind::Button {
                id: id::PREV_ARROW,
                selected: false,
                enabled: true,
            },
            command: Some(Command::BuildPage(-1)),
            tooltip: None,
        });
    }
    if has_next && next_arrow_x >= grid_right && next_arrow_x + next_size[0] <= width {
        widgets.push(BuildWidget {
            rect: Rect::new(next_arrow_x, strip_y + ARROW_Y, next_size[0], next_size[1]),
            kind: BuildWidgetKind::Button {
                id: id::NEXT_ARROW,
                selected: false,
                enabled: true,
            },
            command: Some(Command::BuildPage(1)),
            tooltip: None,
        });
    }
}

fn popup_rect(width: i32, strip_y: i32, description: &str, art: &Art) -> Rect {
    let card_x = 240.min(width);
    let card_w = (width - card_x).max(0);
    let max_w = (card_w - 100).max(40);
    let card_h = if let Some(font) = art.font(10) {
        let desc_lines = if description.is_empty() {
            0
        } else {
            font.wrap(description, max_w).len() as i32
        };
        let needed = 36 + desc_lines * (font.line_height + 2) + 10;
        needed.clamp(80, strip_y.max(80))
    } else {
        96.min(strip_y.max(0))
    };
    Rect::new(card_x, (strip_y - card_h).max(0), card_w, card_h)
}

/// Draws the Build Mode panel (`cWinArch`) widgets, swatches, and popup card.
pub fn draw(
    frame: &mut Frame,
    art: &Art,
    thumbnails: &Thumbnails,
    swatches: &BuildSwatches,
    layout: &BuildLayout,
    pointer: Option<[i32; 2]>,
) {
    draw_with_catalog(frame, art, thumbnails, swatches, None, layout, pointer);
}

/// Like [`draw`], also rendering decoded `GameData/Roofs/*.bmp` roof pattern
/// swatches from `catalog` when `BuildTool::Roof` is open.
pub fn draw_with_catalog(
    frame: &mut Frame,
    art: &Art,
    thumbnails: &Thumbnails,
    swatches: &BuildSwatches,
    catalog: Option<&BuildCatalog>,
    layout: &BuildLayout,
    pointer: Option<[i32; 2]>,
) {
    let strip_y = frame.height() as i32 - STRIP;
    // Draw Build Mode toothpick dividers if loaded (`0x1330`, `0x1331`).
    if let Some(left) = art.image(id::TOOTHPICK_LEFT) {
        frame.draw_all(left, [266, strip_y + 2]);
    }
    let native_wide = layout
        .widgets
        .iter()
        .any(|w| matches!(w.command, Some(Command::BuildTool(BuildTool::Hand))) && w.rect.x == 502);
    if native_wide
        && frame.width() as i32 >= NARROW_WIDTH
        && let Some(right) = art.image(id::TOOTHPICK_RIGHT)
    {
        frame.draw_all(right, [535, strip_y + 2]);
    }

    for widget in &layout.widgets {
        let hovered = pointer.is_some_and(|[px, py]| widget.rect.contains(px, py));
        match widget.kind {
            BuildWidgetKind::Button {
                id,
                selected,
                enabled,
            } => {
                let cell = if !enabled {
                    3
                } else if selected {
                    1
                } else if hovered {
                    2
                } else {
                    0
                };
                if let Some(img) = art.cell(id, 4, cell) {
                    draw_image_fitted(frame, &img, widget.rect);
                }
            }
            BuildWidgetKind::Slot {
                item,
                selected,
                affordable,
            } => {
                let cell = if !affordable {
                    3
                } else if selected {
                    1
                } else if hovered {
                    2
                } else {
                    0
                };
                if let Some(bg) = art.cell(TEMPLATE, 4, cell) {
                    frame.draw_all(&bg, [widget.rect.x, widget.rect.y]);
                }
                match item {
                    BuildItemId::Object(guid) => {
                        if let Some(thumb) =
                            thumbnails.item(guid).and_then(|i| i.thumbnail.as_ref())
                        {
                            let half = if thumb.width > CELL {
                                thumb.width / 2
                            } else {
                                thumb.width
                            };
                            let src_x = if (selected || hovered) && thumb.width > CELL {
                                half
                            } else {
                                0
                            };
                            let dx = (CELL as i32 - half as i32) / 2;
                            let dy = (CELL as i32 - thumb.height as i32) / 2;
                            frame.draw(
                                thumb,
                                Rect::new(src_x as i32, 0, half as i32, thumb.height as i32),
                                [widget.rect.x + dx, widget.rect.y + dy],
                            );
                        } else if let Some(icon) = thumbnails.icon(art, guid) {
                            frame.draw_all(&icon, [widget.rect.x, widget.rect.y]);
                        }
                    }
                    BuildItemId::WallStyle(_)
                    | BuildItemId::Floor(_)
                    | BuildItemId::Wallpaper(_) => {
                        if let Some(swatch) = swatches.get(item) {
                            let dx = (CELL as i32 - swatch.width as i32) / 2;
                            let dy = (CELL as i32 - swatch.height as i32) / 2;
                            frame.draw_all(swatch, [widget.rect.x + dx, widget.rect.y + dy]);
                        }
                    }
                }
            }
        }
    }

    for sub in &layout.subtools {
        let hovered = pointer.is_some_and(|[px, py]| sub.rect.contains(px, py));
        let cell = if !sub.affordable {
            3
        } else if sub.selected {
            1
        } else if hovered {
            2
        } else {
            0
        };
        if let Some(pattern_idx) = sub.roof_pattern {
            if let Some(bg) = art
                .cell(id::ROOF_PATTERN_TEMPLATE, 4, cell)
                .or_else(|| art.cell(TEMPLATE, 4, cell))
            {
                frame.draw_all(&bg, [sub.rect.x, sub.rect.y]);
            }
            if let Some(swatch) = catalog
                .and_then(|c| c.roof_pattern(pattern_idx))
                .and_then(|r| r.swatch.as_ref())
            {
                let dx = (sub.rect.width - swatch.width as i32) / 2;
                let dy = (sub.rect.height - swatch.height as i32) / 2;
                frame.draw_all(swatch, [sub.rect.x + dx, sub.rect.y + dy]);
            }
        } else if let Some(res_id) = sub.resource_id {
            let is_square_slot = sub.rect.width == CELL as i32 && sub.rect.height == CELL as i32;
            if is_square_slot && let Some(bg) = art.cell(TEMPLATE, 4, cell) {
                frame.draw_all(&bg, [sub.rect.x, sub.rect.y]);
            }
            if let Some(img) = art.cell(res_id, 4, cell) {
                draw_image_fitted(frame, &img, sub.rect);
            }
        }
    }

    if let Some(card) = &layout.popup {
        draw_popup_card(
            frame,
            art,
            card.rect,
            &card.name,
            card.price,
            &card.description,
            |f, x, y| match card.item {
                BuildItemId::Object(guid) => {
                    if let Some(icon) = thumbnails.icon(art, guid) {
                        f.draw_all(&icon, [x, y]);
                    }
                }
                other => {
                    if let Some(swatch) = swatches.get(other) {
                        f.draw_all(swatch, [x + 4, y + 4]);
                    }
                }
            },
        );
    } else if let Some(card) = &layout.subtool_popup {
        draw_popup_card(
            frame,
            art,
            card.rect,
            &card.name,
            card.price,
            &card.description,
            |f, x, y| {
                if let Some(idx) = card.roof_pattern {
                    if let Some(swatch) = catalog
                        .and_then(|c| c.roof_pattern(idx))
                        .and_then(|r| r.swatch.as_ref())
                    {
                        f.draw_all(swatch, [x + 10, y + 10]);
                    }
                } else if let Some(res_id) = card.resource_id
                    && let Some(img) = art.cell(res_id, 4, 0)
                {
                    f.draw_all(&img, [x + 4, y + 4]);
                }
            },
        );
    }
}

fn draw_image_fitted(frame: &mut Frame, img: &Image, rect: Rect) {
    if rect.width <= 0 || rect.height <= 0 || img.width == 0 || img.height == 0 {
        return;
    }
    if img.width as i32 <= rect.width && img.height as i32 <= rect.height {
        let dx = (rect.width - img.width as i32) / 2;
        let dy = (rect.height - img.height as i32) / 2;
        frame.draw_all(img, [rect.x + dx, rect.y + dy]);
        return;
    }
    let scale = (rect.width as f32 / img.width as f32).min(rect.height as f32 / img.height as f32);
    let dst_w = ((img.width as f32 * scale).round() as u32).clamp(1, rect.width as u32);
    let dst_h = ((img.height as f32 * scale).round() as u32).clamp(1, rect.height as u32);
    let mut rgba = vec![0u8; dst_w as usize * dst_h as usize * 4];
    for dy in 0..dst_h {
        let sy = ((u64::from(dy) * u64::from(img.height)) / u64::from(dst_h))
            .min(u64::from(img.height - 1)) as u32;
        for dx in 0..dst_w {
            let sx = ((u64::from(dx) * u64::from(img.width)) / u64::from(dst_w))
                .min(u64::from(img.width - 1)) as u32;
            if let Some(px) = img.pixel(sx, sy) {
                let off = (dy as usize * dst_w as usize + dx as usize) * 4;
                rgba[off..off + 4].copy_from_slice(&px);
            }
        }
    }
    let scaled = Image {
        width: dst_w,
        height: dst_h,
        rgba,
    };
    let ox = (rect.width - dst_w as i32) / 2;
    let oy = (rect.height - dst_h as i32) / 2;
    frame.draw_all(&scaled, [rect.x + ox, rect.y + oy]);
}

fn draw_popup_card<F>(
    frame: &mut Frame,
    art: &Art,
    rect: Rect,
    name: &str,
    price: i32,
    description: &str,
    draw_icon: F,
) where
    F: FnOnce(&mut Frame, i32, i32),
{
    frame.fill(rect, [20, 32, 92]);
    let border = [120, 148, 212];
    frame.fill(Rect::new(rect.x, rect.y, rect.width, 2), border);
    frame.fill(Rect::new(rect.x, rect.y, 2, rect.height), border);
    draw_icon(frame, rect.x + 12, rect.y + 12);
    let text_x = rect.x + 84;
    if let Some(font) = art.font(12) {
        let header = format!("{name}    ${price}");
        font.draw(frame, &header, [text_x, rect.y + 12], [235, 240, 255]);
    }
    if let Some(font) = art.font(10) {
        let max_w = (rect.width - 96).max(40);
        let mut y = rect.y + 32;
        let step = font.line_height + 2;
        let bottom = rect.y + rect.height - 4;
        for line in font.wrap(description, max_w) {
            if y + font.line_height <= bottom {
                font.draw(frame, &line, [text_x, y], [210, 224, 248]);
            }
            y += step;
        }
    }
}

/// Enumerates `GameData/Roofs/*.bmp` from `install` in deterministic order
/// (base-game `r1_..r4_` first, then remaining patterns sorted
/// case-insensitively by stem) and decodes each 32×32 swatch BMP.
#[allow(clippy::disallowed_methods)]
pub fn load_roof_patterns(install: &Install) -> Vec<RoofPatternEntry> {
    let Ok(dir) = install.file("GameData/Roofs") else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !ext.eq_ignore_ascii_case("bmp") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if stem.is_empty() {
            continue;
        }
        found.push((stem.to_owned(), path));
    }
    let rank = |stem: &str| -> (u8, String) {
        match stem.to_ascii_lowercase().as_str() {
            "r1_" => (0, stem.to_ascii_lowercase()),
            "r2_" => (1, stem.to_ascii_lowercase()),
            "r3_" => (2, stem.to_ascii_lowercase()),
            "r4_" => (3, stem.to_ascii_lowercase()),
            _ => (4, stem.to_ascii_lowercase()),
        }
    };
    found.sort_by(|a, b| rank(&a.0).cmp(&rank(&b.0)).then_with(|| a.0.cmp(&b.0)));
    found
        .into_iter()
        .enumerate()
        .map(|(index, (stem, path))| {
            let swatch = std::fs::read(&path)
                .ok()
                .and_then(|bytes| bitmap::bmp(&bytes).ok());
            RoofPatternEntry {
                index,
                name: format_roof_name(&stem),
                stem,
                path,
                swatch,
            }
        })
        .collect()
}

fn format_roof_name(stem: &str) -> String {
    match stem.to_ascii_lowercase().as_str() {
        "r1_" => "Classic Shingle (Light)".to_owned(),
        "r2_" => "Classic Shingle (Slate)".to_owned(),
        "r3_" => "Classic Shingle (Dark)".to_owned(),
        "r4_" => "Terracotta Tile".to_owned(),
        "gravel1" => "Gravel Roof".to_owned(),
        "tar1" => "Tar Roof".to_owned(),
        _ => stem.trim_end_matches('_').to_owned(),
    }
}

fn load_build_tables(install: &Install) -> BTreeMap<u16, StringTable> {
    install
        .file("GameData/Build.iff")
        .ok()
        .and_then(|path| Iff::parse(std::fs::read(path).ok()?).ok())
        .map(|iff| {
            [
                STR_TERRAIN,
                STR_WALLS,
                STR_FLOORS,
                STR_WALLPAPERS,
                STR_POOL,
                STR_PLUGIN_FLOOR,
                STR_PLUGIN_WALL,
            ]
            .into_iter()
            .filter_map(|id| {
                let chunk = iff.get(b"STR#", id)?;
                Some((
                    id,
                    StringTable::parse_chunk(iff.data(chunk), iff.resource_flags(chunk)).ok()?,
                ))
            })
            .collect()
        })
        .unwrap_or_default()
}

fn load_tool_tips(install: &Install) -> Vec<String> {
    let Ok(path) = install.file("GameData/UIText.iff") else {
        return Vec::new();
    };
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(iff) = Iff::parse(bytes) else {
        return Vec::new();
    };
    let Some(chunk) = iff.get(b"STR#", 139) else {
        return Vec::new();
    };
    let Ok(table) = StringTable::parse(iff.data(chunk)) else {
        return Vec::new();
    };
    table
        .entries
        .iter()
        .filter(|e| e.language == US_ENGLISH)
        .map(|e| e.text.clone())
        .collect()
}

fn read_i16(data: &[u8], offset: usize) -> i16 {
    if offset + 2 > data.len() {
        return 0;
    }
    i16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_ctss(iff: &Iff, ctss_id: i16) -> (String, String, Vec<u8>) {
    let Ok(id) = u16::try_from(ctss_id) else {
        return (String::new(), String::new(), Vec::new());
    };
    if let Some(chunk) = iff.get(b"CATS", id) {
        let data = iff.data(chunk);
        let name_end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
        let raw_name = data[..name_end].to_vec();
        let name = sims_formats::strings::decode(&raw_name, US_ENGLISH);
        let description = if name_end + 1 < data.len() {
            let rest = &data[name_end + 1..];
            let desc_end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
            sims_formats::strings::decode(&rest[..desc_end], US_ENGLISH)
        } else {
            String::new()
        };
        return (name, description, raw_name);
    }
    let Some(chunk) = iff.get(b"CTSS", id) else {
        return (String::new(), String::new(), Vec::new());
    };
    let Ok(table) = StringTable::parse(iff.data(chunk)) else {
        return (String::new(), String::new(), Vec::new());
    };
    let pick = |index: usize| {
        table
            .entries
            .iter()
            .filter(|e| e.language == US_ENGLISH)
            .nth(index)
            .or_else(|| table.entries.get(index))
    };
    let (name, raw_name) = pick(0)
        .map(|e| (e.text.clone(), e.raw_text.clone()))
        .unwrap_or_default();
    let description = pick(1).map(|e| e.text.clone()).unwrap_or_default();
    (name, description, raw_name)
}
