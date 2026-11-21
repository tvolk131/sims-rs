//! Buy Mode catalog indexing, layout, drawing, and hit-testing
//! (`cWinCatalog` at `0x5050e4`..`0x507fff`, `Catalog` at `0x4cd149`,
//! `0x4cd9a5`, and `cWinCatalogPopup` at `0x508342`).

use crate::art::Art;
use crate::command::Command;
use crate::frame::{Frame, Rect};
use crate::hud::STRIP;
use crate::thumbnail::{SIZE as CELL, TEMPLATE, Thumbnails};
use sims_content::ui_graphics::US_ENGLISH;
use sims_content::{Catalog, Index};
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// Buy Mode catalog sort order (`[ui + 0xa0]`, `0x4cea5d`, `0x507288`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BuySort {
    /// Sort by room (Living, Dining, Bedroom, Study, Kitchen, Bath, Outside, Misc).
    ByRoom,
    /// Sort by function (Seating, Surfaces, Decorative, Electronics,
    /// Appliances, Plumbing, Lighting, Miscellaneous) — the default on a
    /// residential lot (`0x4cebf4`).
    #[default]
    ByFunction,
}

/// Interactive state of the Buy Mode catalog strip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BuyState {
    pub sort: BuySort,
    /// Selected top-level category `0..8`, or `None` when the 8 top-level
    /// category icons are shown (`[ui + 0xa4] == 0`).
    pub category: Option<usize>,
    /// Selected subcategory `0..8` within `category`.
    pub subcategory: usize,
    /// Zero-based page index within the filtered catalog grid.
    pub page: usize,
    /// Currently selected catalog master GUID, if any.
    pub selected: Option<u32>,
}

impl BuyState {
    /// Resets the panel to top-level By-Function categories, as entering Buy
    /// Mode on a residential lot does (`0x4cebf4`).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Applies a Buy Mode presentation command (`BuyCategory`,
    /// `BuySubcategory`, `BuyBack`, `BuyPage`). Returns `true` if state
    /// changed.
    pub fn apply(&mut self, command: &Command) -> bool {
        match *command {
            Command::BuyCategory(cat) if cat < 8 => {
                self.category = Some(cat);
                self.subcategory = 0;
                self.page = 0;
                self.selected = None;
                true
            }
            Command::BuySubcategory(sub) if sub < 8 => {
                if self.subcategory != sub || self.page != 0 {
                    self.subcategory = sub;
                    self.page = 0;
                    self.selected = None;
                    true
                } else {
                    false
                }
            }
            Command::BuyBack => {
                if self.category.is_some() {
                    self.category = None;
                    self.page = 0;
                    self.selected = None;
                    true
                } else {
                    false
                }
            }
            Command::BuyPage(delta) => {
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
            _ => false,
        }
    }
}

/// One residential-buyable catalog entry (`0x4cd149`, `0x4cbd44`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyItem {
    pub guid: u32,
    pub price: i32,
    pub name: String,
    pub description: String,
    /// OBJD `+0x4e` (word 39): room sort bitmask (`0x4cb8e7`, `0x4cb905`).
    pub room_sort: u16,
    /// OBJD `+0x50` (word 40): function sort bitmask (`0x4cb91d`).
    pub function_sort: u16,
    /// OBJD `+0xbc` (word 94): function subsort bitmask (`0x4cb93b`).
    pub function_subsort: u16,
    /// OBJD `+0x8a` (word 69): nonzero for Build-mode catalog objects.
    pub build_mode_type: i16,
}

/// All residential-buyable objects from an install, sorted by `(price, name)`
/// as `0x4cd149` / `0x4cd9a5` orders them.
#[derive(Debug, Clone, Default)]
pub struct BuyCatalog {
    items: Vec<BuyItem>,
}

/// OBJD byte offsets read by `0x4cd149` and `0x4cb8e7..0x4cb979`.
const OBJD_GRAPHICS: usize = 0x08;
const OBJD_TTAB: usize = 0x0e;
const OBJD_TYPE: usize = 0x12;
const OBJD_MASTER: usize = 0x14;
const OBJD_SUB_INDEX: usize = 0x16;
const OBJD_PRICE: usize = 0x24;
const OBJD_ROOM_SORT: usize = 0x4e;
const OBJD_FUNC_SORT: usize = 0x50;
const OBJD_CTSS: usize = 0x52;
const OBJD_BUILD_TYPE: usize = 0x8a;
const OBJD_FUNC_SUBSORT: usize = 0xbc;

/// Object type 2 is a person (excluded at `0x4575d2`).
const PERSON_TYPE: i16 = 2;

/// Function-sort category bitmasks in UI slot order `0..8` (`0x507288`).
pub const FUNCTION_BITS: [u16; 8] = [0x01, 0x02, 0x20, 0x08, 0x04, 0x10, 0x80, 0x40];

/// Function-subsort masks in UI slot order `0..8` (`0x507288`).
/// Slots 4 and 5 are shown only for category 7 (Miscellaneous); slot 6 is `+`
/// (other) and slot 7 is `∞` (all).
pub const FUNCTION_SUB_MASKS: [u16; 8] = [0x01, 0x02, 0x04, 0x08, 0x20, 0x40, 0x90, 0xff];

/// Room-sort category bitmasks in UI slot order `0..8` (`0x507288`).
pub const ROOM_BITS: [u16; 8] = [0x08, 0x20, 0x02, 0x80, 0x01, 0x04, 0x10, 0x40];

/// UIGraphics bitmap IDs from `0x6cc3fc..0x6cc684` and `0x5050e4`.
pub const PREV_ARROW: i32 = 102;
pub const NEXT_ARROW: i32 = 101;
pub const FUNC_CAT_BMPS: [i32; 8] = [150, 151, 152, 153, 154, 155, 156, 157];
pub const ROOM_CAT_BMPS: [i32; 8] = [158, 159, 160, 161, 162, 163, 164, 165];
pub const ROOM_SUB_BMPS: [i32; 8] = [210, 211, 212, 213, 214, 215, 216, 217];
pub const BACK_FUNC_BMPS: [i32; 8] = [170, 171, 172, 173, 174, 175, 176, 177];
pub const BACK_ROOM_BMPS: [i32; 8] = [180, 181, 182, 183, 184, 185, 186, 187];
pub const SUBCAT_MISC_PETS: i32 = 1574;
pub const SUBCAT_MISC_MAGIC: i32 = 1575;
pub const SUBCAT_OTHER: i32 = 1580;
pub const SUBCAT_ALL: i32 = 1581;

/// Screen-x and strip-relative-y button origins from `0x6f3510..0x6f361c`.
const FUNC_CAT_POS: [[i32; 2]; 8] = [
    [242, 8],
    [279, 16],
    [322, 1],
    [369, 4],
    [241, 61],
    [285, 59],
    [322, 55],
    [364, 64],
];

const ROOM_CAT_POS: [[i32; 2]; 8] = [
    [242, 7],
    [281, 3],
    [323, 7],
    [361, 4],
    [239, 53],
    [284, 59],
    [322, 55],
    [364, 64],
];

const SUBCAT_POS: [[i32; 2]; 8] = [
    [251, 10],
    [289, 10],
    [327, 10],
    [365, 10],
    [251, 58],
    [289, 58],
    [327, 58],
    [365, 58],
];

/// Grid geometry inside the 100-px bottom strip (`0x507288`): left = `0x19f`,
/// top = `5`, right = `width - 12`, cell = `45 × 45`.
const GRID_LEFT: i32 = 0x19f;
const GRID_TOP: i32 = 5;
const GRID_RIGHT_MARGIN: i32 = 12;
const GRID_ROWS: usize = 2;

/// Back button x coordinate (`220 - 2`, `0x506765` line `0x507040`).
const BACK_X: i32 = 218;
/// Prev/Next page arrow positions (`0x6f3610` = `(404, 26)`).
const PREV_ARROW_X: i32 = 404;
const ARROW_Y: i32 = 26;
const NEXT_ARROW_MARGIN: i32 = 24;

/// Default button fallback size when UIGraphics art is not loaded (for
/// headless layout tests).
const FALLBACK_BTN: [i32; 2] = [36, 36];

/// All UIGraphics bitmap IDs used by the Buy Mode catalog strip (for
/// `Art::load_optional`).
pub fn resource_ids() -> Vec<i32> {
    let mut ids = vec![
        PREV_ARROW,
        NEXT_ARROW,
        SUBCAT_MISC_PETS,
        SUBCAT_MISC_MAGIC,
        SUBCAT_OTHER,
        SUBCAT_ALL,
    ];
    ids.extend(FUNC_CAT_BMPS);
    ids.extend(ROOM_CAT_BMPS);
    ids.extend(ROOM_SUB_BMPS);
    ids.extend(BACK_FUNC_BMPS);
    ids.extend(BACK_ROOM_BMPS);
    for cat_bmp in FUNC_CAT_BMPS {
        for sub in 0..4 {
            ids.push(cat_bmp * 10 + sub);
        }
    }
    ids
}

impl BuyCatalog {
    /// Builds the residential Buy Mode catalog from `index` and `catalog`
    /// (`0x4cd149`, `0x4cd9a5`).
    pub fn load(index: &Index, catalog: &Catalog) -> Self {
        let mut items = Vec::new();
        // Cache parsed IFF files per resource index so multi-OBJD archives
        // are parsed once.
        let mut by_resource: std::collections::BTreeMap<usize, Vec<(u32, u16)>> =
            std::collections::BTreeMap::new();
        for (&guid, &idx) in &catalog.by_guid {
            let obj = &catalog.objects[idx];
            if i16::try_from(obj.kind).unwrap_or(0) == PERSON_TYPE {
                continue;
            }
            by_resource
                .entry(obj.resource)
                .or_default()
                .push((guid, obj.chunk));
        }

        for (res_idx, entries) in by_resource {
            let Some(resource) = index.resources.get(res_idx) else {
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
                // 0x4575b8: skip non-master tiles of multi-tile objects.
                if master != 0 && sub_index != -1 {
                    continue;
                }
                let kind = read_i16(data, OBJD_TYPE);
                if kind == PERSON_TYPE {
                    continue;
                }
                // 0x4cb905: residential Buy Mode requires room_sort != 0.
                let room_sort = read_i16(data, OBJD_ROOM_SORT) as u16;
                if room_sort == 0 {
                    continue;
                }
                // 0x457588 / 0x4575a1: needs graphics (master != 0 ||
                // base_graphic_id != 0) or an interaction table.
                let graphics = read_i16(data, OBJD_GRAPHICS);
                let ttab = read_i16(data, OBJD_TTAB);
                if master == 0 && graphics == 0 && ttab <= 0 {
                    continue;
                }
                let price = i32::from(read_i16(data, OBJD_PRICE));
                let function_sort = read_i16(data, OBJD_FUNC_SORT) as u16;
                let function_subsort = read_i16(data, OBJD_FUNC_SUBSORT) as u16;
                let build_mode_type = read_i16(data, OBJD_BUILD_TYPE);
                let ctss_id = read_i16(data, OBJD_CTSS);
                let (mut name, description, mut raw_name) = read_ctss(&iff, ctss_id);
                // 0x457d3b: when the catalog string table is missing or its
                // name entry is empty, fall back to the OBJD chunk label.
                if name.is_empty() && !chunk.label.is_empty() {
                    name = chunk.label.clone();
                    raw_name = cp1252_bytes(&chunk.label);
                }
                items.push((
                    raw_name,
                    BuyItem {
                        guid,
                        price,
                        name,
                        description,
                        room_sort,
                        function_sort,
                        function_subsort,
                        build_mode_type,
                    },
                ));
            }
        }

        // 0x4cd9a5 -> 0x430100 -> memcmp (0x6363e0): sort by price ascending,
        // then raw CTSS catalog bytes (Windows-1252) ascending, then GUID.
        items.sort_by(|(raw_a, a), (raw_b, b)| {
            a.price
                .cmp(&b.price)
                .then_with(|| raw_a.cmp(raw_b))
                .then_with(|| a.guid.cmp(&b.guid))
        });

        Self {
            items: items.into_iter().map(|(_, item)| item).collect(),
        }
    }

    /// Constructs a catalog from explicit items (for unit tests).
    pub fn from_items(mut items: Vec<BuyItem>) -> Self {
        items.sort_by(|a, b| {
            a.price
                .cmp(&b.price)
                .then_with(|| cp1252_bytes(&a.name).cmp(&cp1252_bytes(&b.name)))
                .then_with(|| a.guid.cmp(&b.guid))
        });
        Self { items }
    }

    /// Looks up an item by its catalog master GUID.
    pub fn get(&self, guid: u32) -> Option<&BuyItem> {
        self.items.iter().find(|item| item.guid == guid)
    }

    /// Every residential-buyable item in catalog sort order.
    pub fn all(&self) -> &[BuyItem] {
        &self.items
    }

    /// Items matching the current `(sort, category, subcategory)` selection
    /// (`0x507288`, `0x4cb93b`, `0x4cb8e7`, `0x4cb91d`).
    pub fn filtered(&self, state: &BuyState) -> Vec<&BuyItem> {
        let Some(cat) = state.category else {
            return Vec::new();
        };
        if cat >= 8 || state.subcategory >= 8 {
            return Vec::new();
        }
        match state.sort {
            BuySort::ByFunction => {
                let cat_bit = FUNCTION_BITS[cat];
                let sub_mask = FUNCTION_SUB_MASKS[state.subcategory];
                self.items
                    .iter()
                    .filter(|item| {
                        (item.function_sort & cat_bit) != 0
                            && ((item.function_subsort & sub_mask) != 0
                                || (sub_mask == 0xff && item.function_subsort == 0))
                    })
                    .collect()
            }
            BuySort::ByRoom => {
                let room_bit = ROOM_BITS[cat];
                let func_bit = FUNCTION_BITS[state.subcategory];
                self.items
                    .iter()
                    .filter(|item| {
                        (item.room_sort & room_bit) != 0 && (item.function_sort & func_bit) != 0
                    })
                    .collect()
            }
        }
    }
}

/// One laid-out element of the Buy Mode catalog strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyWidget {
    pub rect: Rect,
    pub kind: BuyWidgetKind,
    pub command: Option<Command>,
}

/// How a [`BuyWidget`] draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuyWidgetKind {
    /// A 4-cell UIGraphics button (normal, down/selected, hover, disabled).
    Button {
        id: i32,
        selected: bool,
        enabled: bool,
    },
    /// A 45×45 catalog thumbnail cell (`kCatalogTemplate` 4300 + object thumb).
    Slot {
        guid: u32,
        selected: bool,
        affordable: bool,
    },
}

/// Laid-out Buy Mode strip widgets and optional popup card.
#[derive(Debug, Clone, Default)]
pub struct BuyLayout {
    pub widgets: Vec<BuyWidget>,
    pub popup: Option<PopupCard>,
    pub page: usize,
    pub total_pages: usize,
}

/// Selected item's popup description card (`cWinCatalogPopup`, `0x508342`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopupCard {
    pub rect: Rect,
    pub guid: u32,
    pub name: String,
    pub price: i32,
    pub description: String,
}

impl BuyLayout {
    /// Whether any interactive Buy Mode strip widget covers `point`.
    /// The popup card is informational and mouse-transparent so moving into
    /// the house view immediately activates the placement tool (`0x487d3d`).
    pub fn covers(&self, point: [i32; 2]) -> bool {
        self.widgets
            .iter()
            .any(|w| w.rect.contains(point[0], point[1]))
    }

    /// Command emitted by pressing `point` inside the Buy Mode strip, if any.
    pub fn press(&self, point: [i32; 2]) -> Option<Command> {
        self.widgets
            .iter()
            .rev()
            .find(|w| w.rect.contains(point[0], point[1]))
            .and_then(|w| w.command.clone())
    }
}

/// Lays out the Buy Mode strip widgets and popup card for a canvas of `size`
/// (`0x506765`, `0x507288`, `0x508342`).
pub fn layout(
    size: [i32; 2],
    catalog: &BuyCatalog,
    state: &BuyState,
    art: &Art,
    available_funds: i32,
) -> BuyLayout {
    let [width, height] = size.map(|v| v.clamp(0, 1 << 15));
    let strip_y = height - STRIP;
    let btn_size = |id: i32| -> [i32; 2] {
        art.cell(id, 4, 0)
            .map(|img| [img.width as i32, img.height as i32])
            .unwrap_or(FALLBACK_BTN)
    };
    let mut widgets = Vec::new();
    let mut page = state.page;
    let mut total_pages = 0;

    match state.category {
        None => {
            let (bmps, coords) = match state.sort {
                BuySort::ByFunction => (&FUNC_CAT_BMPS, &FUNC_CAT_POS),
                BuySort::ByRoom => (&ROOM_CAT_BMPS, &ROOM_CAT_POS),
            };
            for i in 0..8 {
                let id = bmps[i];
                let [w, h] = btn_size(id);
                widgets.push(BuyWidget {
                    rect: Rect::new(coords[i][0], strip_y + coords[i][1], w, h),
                    kind: BuyWidgetKind::Button {
                        id,
                        selected: false,
                        enabled: true,
                    },
                    command: Some(Command::BuyCategory(i)),
                });
            }
        }
        Some(cat) if cat < 8 => {
            let back_id = match state.sort {
                BuySort::ByFunction => BACK_FUNC_BMPS[cat],
                BuySort::ByRoom => BACK_ROOM_BMPS[cat],
            };
            let [bw, bh] = btn_size(back_id);
            widgets.push(BuyWidget {
                rect: Rect::new(BACK_X, strip_y + (STRIP - bh) / 2, bw, bh),
                kind: BuyWidgetKind::Button {
                    id: back_id,
                    selected: false,
                    enabled: true,
                },
                command: Some(Command::BuyBack),
            });

            for sub in 0..8 {
                let bmp = match state.sort {
                    BuySort::ByFunction => match sub {
                        0..=3 => Some(FUNC_CAT_BMPS[cat] * 10 + sub as i32),
                        4 if cat == 7 => Some(SUBCAT_MISC_PETS),
                        5 if cat == 7 => Some(SUBCAT_MISC_MAGIC),
                        4 | 5 => None,
                        6 => Some(SUBCAT_OTHER),
                        _ => Some(SUBCAT_ALL),
                    },
                    BuySort::ByRoom => Some(ROOM_SUB_BMPS[sub]),
                };
                let Some(id) = bmp else {
                    continue;
                };
                let [w, h] = btn_size(id);
                widgets.push(BuyWidget {
                    rect: Rect::new(SUBCAT_POS[sub][0], strip_y + SUBCAT_POS[sub][1], w, h),
                    kind: BuyWidgetKind::Button {
                        id,
                        selected: state.subcategory == sub,
                        enabled: true,
                    },
                    command: Some(Command::BuySubcategory(sub)),
                });
            }

            let items = catalog.filtered(state);
            let grid_right = width - GRID_RIGHT_MARGIN;
            let cell = CELL as i32;
            let cols = ((grid_right - GRID_LEFT) / cell).max(1) as usize;
            let per_page = cols * GRID_ROWS;
            total_pages = if items.is_empty() {
                0
            } else {
                items.len().div_ceil(per_page)
            };
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
                widgets.push(BuyWidget {
                    rect,
                    kind: BuyWidgetKind::Slot {
                        guid: item.guid,
                        selected: state.selected == Some(item.guid),
                        affordable: item.price <= available_funds,
                    },
                    command: Some(Command::BuyItem(item.guid)),
                });
            }

            if page > 0 {
                let [aw, ah] = btn_size(PREV_ARROW);
                widgets.push(BuyWidget {
                    rect: Rect::new(PREV_ARROW_X, strip_y + ARROW_Y, aw, ah),
                    kind: BuyWidgetKind::Button {
                        id: PREV_ARROW,
                        selected: false,
                        enabled: true,
                    },
                    command: Some(Command::BuyPage(-1)),
                });
            }
            if start + per_page < items.len() {
                let [aw, ah] = btn_size(NEXT_ARROW);
                widgets.push(BuyWidget {
                    rect: Rect::new(width - NEXT_ARROW_MARGIN, strip_y + ARROW_Y, aw, ah),
                    kind: BuyWidgetKind::Button {
                        id: NEXT_ARROW,
                        selected: false,
                        enabled: true,
                    },
                    command: Some(Command::BuyPage(1)),
                });
            }
        }
        _ => {}
    }

    let popup = state
        .selected
        .and_then(|guid| catalog.get(guid))
        .map(|item| {
            let card_x = 240.min(width);
            let card_w = (width - card_x).max(0);
            let card_h = 127;
            PopupCard {
                rect: Rect::new(card_x, (strip_y - card_h).max(0), card_w, card_h),
                guid: item.guid,
                name: item.name.clone(),
                price: item.price,
                description: item.description.clone(),
            }
        });

    BuyLayout {
        widgets,
        popup,
        page,
        total_pages,
    }
}

/// Draws the Buy Mode strip widgets and selected item popup card into `frame`.
pub fn draw(
    frame: &mut Frame,
    art: &Art,
    thumbnails: &Thumbnails,
    layout: &BuyLayout,
    pointer: Option<[i32; 2]>,
) {
    for widget in &layout.widgets {
        let hovered = pointer.is_some_and(|[px, py]| widget.rect.contains(px, py));
        match widget.kind {
            BuyWidgetKind::Button {
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
            BuyWidgetKind::Slot {
                guid,
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
                if let Some(thumb) = thumbnails.item(guid).and_then(|i| i.thumbnail.as_ref()) {
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
        if let Some(icon) = thumbnails.icon(art, card.guid) {
            frame.draw_all(&icon, [card.rect.x + 12, card.rect.y + 12]);
        }
        let text_x = card.rect.x + 178;
        if let Some(font) = art.font(12) {
            let header = format!("{}    ${}", card.name, card.price);
            font.draw(frame, &header, [text_x, card.rect.y + 12], [235, 240, 255]);
        }
        if let Some(font) = art.font(10) {
            let max_w = (card.rect.width - 190).max(40);
            let mut y = card.rect.y + 32;
            for line in font.wrap(&card.description, max_w).into_iter().take(5) {
                font.draw(frame, &line, [text_x, y], [210, 224, 248]);
                y += font.line_height + 2;
            }
        }
    }
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
    let Some(chunk) = iff.get(b"CTSS", id).or_else(|| iff.get(b"CATS", id)) else {
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

/// Encodes a Unicode string to Windows-1252 bytes for `BuyCatalog::from_items`
/// so synthetic tests match native `0x430100` `memcmp` ordering on extended
/// characters such as `™` (`0x99`) vs `©` (`0xA9`).
fn cp1252_bytes(text: &str) -> Vec<u8> {
    const HIGH: [(char, u8); 27] = [
        ('\u{20AC}', 0x80),
        ('\u{201A}', 0x82),
        ('\u{0192}', 0x83),
        ('\u{201E}', 0x84),
        ('\u{2026}', 0x85),
        ('\u{2020}', 0x86),
        ('\u{2021}', 0x87),
        ('\u{02C6}', 0x88),
        ('\u{2030}', 0x89),
        ('\u{0160}', 0x8A),
        ('\u{2039}', 0x8B),
        ('\u{0152}', 0x8C),
        ('\u{017D}', 0x8E),
        ('\u{2018}', 0x91),
        ('\u{2019}', 0x92),
        ('\u{201C}', 0x93),
        ('\u{201D}', 0x94),
        ('\u{2022}', 0x95),
        ('\u{2013}', 0x96),
        ('\u{2014}', 0x97),
        ('\u{02DC}', 0x98),
        ('\u{2122}', 0x99),
        ('\u{0161}', 0x9A),
        ('\u{203A}', 0x9B),
        ('\u{0153}', 0x9C),
        ('\u{017E}', 0x9E),
        ('\u{0178}', 0x9F),
    ];
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let code = ch as u32;
        if code <= 0x7F || (0xA0..=0xFF).contains(&code) {
            out.push(code as u8);
        } else if let Some(&(_, byte)) = HIGH.iter().find(|&&(c, _)| c == ch) {
            out.push(byte);
        } else {
            let mut buf = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_catalog() -> BuyCatalog {
        BuyCatalog::from_items(vec![
            BuyItem {
                guid: 0x1111_1111,
                price: 200,
                name: "Fancy Chair".into(),
                description: "Comfy.".into(),
                room_sort: 0x20,        // Dining Room
                function_sort: 0x01,    // Seating
                function_subsort: 0x01, // Dining Chairs
                build_mode_type: 0,
            },
            BuyItem {
                guid: 0x26bf_bb29,
                price: 80,
                name: "Werkbunnst All Purpose Chair".into(),
                description: "Minimal.".into(),
                room_sort: 0x20,
                function_sort: 0x01,
                function_subsort: 0x01,
                build_mode_type: 0,
            },
            BuyItem {
                guid: 0x2222_2222,
                price: 450,
                name: "Comfy Sofa".into(),
                description: "Soft.".into(),
                room_sort: 0x08,        // Living Room
                function_sort: 0x01,    // Seating
                function_subsort: 0x04, // Sofas
                build_mode_type: 0,
            },
        ])
    }

    #[test]
    fn buy_catalog_sorts_by_price_and_filters_subcategories() {
        let cat = sample_catalog();
        let mut state = BuyState::default();
        assert!(state.apply(&Command::BuyCategory(0))); // Seating -> Dining Chairs (sub 0)
        let dining: Vec<u32> = cat.filtered(&state).iter().map(|i| i.guid).collect();
        assert_eq!(dining, [0x26bf_bb29, 0x1111_1111]);

        assert!(state.apply(&Command::BuySubcategory(2))); // Sofas
        let sofas: Vec<u32> = cat.filtered(&state).iter().map(|i| i.guid).collect();
        assert_eq!(sofas, [0x2222_2222]);

        assert!(state.apply(&Command::BuySubcategory(7))); // All Seating (0xff)
        let all: Vec<u32> = cat.filtered(&state).iter().map(|i| i.guid).collect();
        assert_eq!(all, [0x26bf_bb29, 0x1111_1111, 0x2222_2222]);

        // Equal-price items compare raw Windows-1252 CTSS bytes (`0x4cd9a5`
        // -> `0x430100` -> `memcmp` at `0x6363e0`), so `Zoo` precedes `apple`
        // and `™` (`0x99`) precedes `©` (`0xA9`).
        let tied = BuyCatalog::from_items(vec![
            BuyItem {
                guid: 1,
                price: 100,
                name: "apple".into(),
                description: String::new(),
                room_sort: 1,
                function_sort: 1,
                function_subsort: 1,
                build_mode_type: 0,
            },
            BuyItem {
                guid: 2,
                price: 100,
                name: "Zoo".into(),
                description: String::new(),
                room_sort: 1,
                function_sort: 1,
                function_subsort: 1,
                build_mode_type: 0,
            },
            BuyItem {
                guid: 3,
                price: 100,
                name: "Lamp©".into(),
                description: String::new(),
                room_sort: 1,
                function_sort: 1,
                function_subsort: 1,
                build_mode_type: 0,
            },
            BuyItem {
                guid: 4,
                price: 100,
                name: "Lamp™".into(),
                description: String::new(),
                room_sort: 1,
                function_sort: 1,
                function_subsort: 1,
                build_mode_type: 0,
            },
        ]);
        let names: Vec<&str> = tied.all().iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Lamp™", "Lamp©", "Zoo", "apple"]);
    }

    #[test]
    fn buy_layout_hits_recorded_seating_and_first_slot_coords() {
        let cat = sample_catalog();
        let art = Art::default();
        let mut state = BuyState::default();
        let top = layout([800, 600], &cat, &state, &art, 2542);
        // Recorded click (258, 524) hits Seating (category 0).
        assert_eq!(top.press([258, 524]), Some(Command::BuyCategory(0)));

        state.apply(&Command::BuyCategory(0));
        let seating = layout([800, 600], &cat, &state, &art, 2542);
        // Recorded click (437, 523) hits slot 0 (Werkbunnst Chair, $80).
        assert_eq!(
            seating.press([437, 523]),
            Some(Command::BuyItem(0x26bf_bb29))
        );
    }
}
