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
use sims_formats::bitmap::Image;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use std::collections::BTreeMap;

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
const STR_WALLS: u16 = 129;
const STR_FLOORS: u16 = 130;
const STR_WALLPAPERS: u16 = 131;
const STR_PLUGIN_FLOOR: u16 = 134;
const STR_PLUGIN_WALL: u16 = 135;

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
/// windows, stairs, fireplaces, plants, and tool tooltips.
#[derive(Debug, Clone, Default)]
pub struct BuildCatalog {
    items: Vec<BuildCatalogItem>,
    tool_tips: Vec<String>,
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

        Self {
            items,
            tool_tips,
            style_prices,
            floor_prices,
            wall_prices,
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

/// Laid-out Build Mode strip widgets and optional popup card.
#[derive(Debug, Clone, Default)]
pub struct BuildLayout {
    pub widgets: Vec<BuildWidget>,
    pub popup: Option<BuildPopup>,
    pub page: usize,
    pub total_pages: usize,
}

impl BuildLayout {
    pub fn covers(&self, point: [i32; 2]) -> bool {
        self.widgets
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
}

/// Lays out the Build Mode strip (`cWinArch`: Undo/Redo, 12 tool buttons,
/// right-hand 2×5 catalog strip, and optional popup card).
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
            .unwrap_or(FALLBACK_BTN)
    };

    let mut widgets = Vec::new();

    // Undo / Redo buttons (`0x5005a3`, `0x50065b`).
    let [uw, uh] = btn_size(id::UNDO);
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
    let [rw, rh] = btn_size(id::REDO);
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

    // 12 Build Tool buttons (`0x6f2bf0`).
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

    let mut page = state.page;
    let mut total_pages = 0;

    if let Some(tool) = state.tool {
        let items = catalog.items_for(tool);
        if !items.is_empty() {
            let grid_right = width - GRID_RIGHT_MARGIN;
            let cell = CELL as i32;
            let cols = ((grid_right - GRID_LEFT) / cell).max(1) as usize;
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
                    GRID_LEFT + col * cell,
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

            if page > 0 {
                let [aw, ah] = btn_size(id::PREV_ARROW);
                widgets.push(BuildWidget {
                    rect: Rect::new(PREV_ARROW_X, strip_y + ARROW_Y, aw, ah),
                    kind: BuildWidgetKind::Button {
                        id: id::PREV_ARROW,
                        selected: false,
                        enabled: true,
                    },
                    command: Some(Command::BuildPage(-1)),
                    tooltip: None,
                });
            }
            if start + per_page < items.len() {
                let [aw, ah] = btn_size(id::NEXT_ARROW);
                widgets.push(BuildWidget {
                    rect: Rect::new(width - NEXT_ARROW_MARGIN, strip_y + ARROW_Y, aw, ah),
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
        let card_x = 240.min(width);
        let card_w = (width - card_x).max(0);
        let max_w = (card_w - 100).max(40);
        let card_h = if let Some(font) = art.font(10) {
            let desc_lines = if entry.description.is_empty() {
                0
            } else {
                font.wrap(&entry.description, max_w).len() as i32
            };
            let needed = 36 + desc_lines * (font.line_height + 2) + 10;
            needed.clamp(80, strip_y.max(80))
        } else {
            96.min(strip_y.max(0))
        };
        BuildPopup {
            rect: Rect::new(card_x, (strip_y - card_h).max(0), card_w, card_h),
            item: entry.id,
            name: entry.name.clone(),
            price: entry.price,
            description: entry.description.clone(),
        }
    });

    BuildLayout {
        widgets,
        popup,
        page,
        total_pages,
    }
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
    let strip_y = frame.height() as i32 - STRIP;
    // Draw Build Mode toothpick dividers if loaded (`0x1330`, `0x1331`).
    if let Some(left) = art.image(id::TOOTHPICK_LEFT) {
        frame.draw_all(left, [266, strip_y + 2]);
    }
    if let Some(right) = art.image(id::TOOTHPICK_RIGHT) {
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
                    frame.draw_all(&img, [widget.rect.x, widget.rect.y]);
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

    if let Some(card) = &layout.popup {
        frame.fill(card.rect, [20, 32, 92]);
        let border = [120, 148, 212];
        frame.fill(
            Rect::new(card.rect.x, card.rect.y, card.rect.width, 2),
            border,
        );
        frame.fill(
            Rect::new(card.rect.x, card.rect.y, 2, card.rect.height),
            border,
        );
        match card.item {
            BuildItemId::Object(guid) => {
                if let Some(icon) = thumbnails.icon(art, guid) {
                    frame.draw_all(&icon, [card.rect.x + 12, card.rect.y + 12]);
                }
            }
            other => {
                if let Some(swatch) = swatches.get(other) {
                    frame.draw_all(swatch, [card.rect.x + 16, card.rect.y + 16]);
                }
            }
        }
        let text_x = card.rect.x + 84;
        if let Some(font) = art.font(12) {
            let header = format!("{}    ${}", card.name, card.price);
            font.draw(frame, &header, [text_x, card.rect.y + 12], [235, 240, 255]);
        }
        if let Some(font) = art.font(10) {
            let max_w = (card.rect.width - 96).max(40);
            let mut y = card.rect.y + 32;
            let step = font.line_height + 2;
            let bottom = card.rect.y + card.rect.height - 4;
            for line in font.wrap(&card.description, max_w) {
                if y + font.line_height <= bottom {
                    font.draw(frame, &line, [text_x, y], [210, 224, 248]);
                }
                y += step;
            }
        }
    }
}

fn load_build_tables(install: &Install) -> BTreeMap<u16, StringTable> {
    install
        .file("GameData/Build.iff")
        .ok()
        .and_then(|path| Iff::parse(std::fs::read(path).ok()?).ok())
        .map(|iff| {
            [
                STR_WALLS,
                STR_FLOORS,
                STR_WALLPAPERS,
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
