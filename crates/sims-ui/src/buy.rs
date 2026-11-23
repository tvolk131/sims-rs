//! Buy Mode catalog indexing, layout, drawing, and hit-testing
//! (`cWinCatalog` at `0x5050e4`..`0x507fff`, `Catalog` at `0x4cd149`,
//! `0x4cd9a5`, and `cWinCatalogPopup` at `0x508342`).

use crate::art::Art;
use crate::command::Command;
use crate::dialog;
use crate::frame::{Frame, Rect};
use crate::hud::STRIP;
use crate::thumbnail::{SIZE as CELL, TEMPLATE, Thumbnails};
use sims_content::ui_graphics::US_ENGLISH;
use sims_content::{Catalog, Index, Install};
use sims_formats::behavior::InteractionTable;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// Which lot catalog mode Buy Mode filters against (`0x4cb8e7..0x4cbaa8`,
/// `0x507288`). Residential lots use By-Function / By-Room (`+0x4e`/`+0x50`),
/// while each expansion's commercial lots use its own 5-category sort word
/// (`+0xbe`, `+0xc2`, `+0xc6`, `+0xce`, `+0xd0`) plus function subcategories.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BuyZone {
    #[default]
    Residential,
    Downtown,
    Vacation,
    Community,
    StudioTown,
    MagicTown,
}

impl BuyZone {
    /// Selects the authentic Complete Collection Buy Mode catalog zone for a
    /// lot ID (`globals[10]`) and neighborhood zoning (`1` residential, `2`
    /// community).
    pub const fn for_lot(lot_id: i16, zoning: Option<i32>) -> Self {
        match lot_id {
            21..=30 => Self::Downtown,
            40..=49 => Self::Vacation,
            81..=89 => Self::StudioTown,
            93..=98 => Self::MagicTown,
            _ if matches!(zoning, Some(2))
                || (zoning.is_none() && lot_id >= 58 && lot_id <= 68) =>
            {
                Self::Community
            }
            _ => Self::Residential,
        }
    }

    /// Number of top-level category buttons for this zone (`8` on Residential,
    /// `5` on all five expansion commercial lot types).
    pub const fn category_count(self) -> usize {
        match self {
            Self::Residential => 8,
            Self::Downtown
            | Self::Vacation
            | Self::Community
            | Self::StudioTown
            | Self::MagicTown => 5,
        }
    }
}

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
    pub zone: BuyZone,
    pub sort: BuySort,
    /// Selected top-level category (`0..8` on Residential, `0..5` on commercial
    /// expansion lots), or `None` when top-level category icons are shown
    /// (`[ui + 0xa4] == 0`).
    pub category: Option<usize>,
    /// Selected subcategory `0..8` within `category`.
    pub subcategory: usize,
    /// Zero-based page index within the filtered catalog grid.
    pub page: usize,
    /// Currently selected catalog master GUID, if any.
    pub selected: Option<u32>,
}

impl BuyState {
    /// Resets the panel to top-level By-Function categories on the current zone,
    /// as entering Buy Mode does (`0x4cebf4`).
    pub fn reset(&mut self) {
        let zone = self.zone;
        *self = Self {
            zone,
            ..Self::default()
        };
    }

    /// Switches the active Buy Mode catalog zone, clearing category/selection
    /// state if the zone changed.
    pub fn set_zone(&mut self, zone: BuyZone) {
        if self.zone != zone {
            self.zone = zone;
            self.category = None;
            self.subcategory = 0;
            self.page = 0;
            self.selected = None;
        }
    }

    /// Applies a Buy Mode presentation command (`BuyCategory`,
    /// `BuySubcategory`, `BuyBack`, `BuyPage`). Returns `true` if state
    /// changed.
    pub fn apply(&mut self, command: &Command) -> bool {
        match *command {
            Command::BuyCategory(cat) if cat < self.zone.category_count() => {
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
            Command::BuyToggleSort => {
                if self.zone != BuyZone::Residential {
                    if self.category.is_some() {
                        self.category = None;
                        self.subcategory = 0;
                        self.page = 0;
                        self.selected = None;
                        return true;
                    }
                    return false;
                }
                self.sort = match self.sort {
                    BuySort::ByFunction => BuySort::ByRoom,
                    BuySort::ByRoom => BuySort::ByFunction,
                };
                self.category = None;
                self.subcategory = 0;
                self.page = 0;
                self.selected = None;
                true
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

/// Interaction-table user restriction shown in Buy Mode hover popups from
/// `GameData/UIText.iff STR# 160[14, 15, 17, 18, 19]` (`0x45775c..0x457978`,
/// `0x5086c5..0x50873e`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UsageRestriction {
    /// `STR# 160[14]`: `"Can only be used by adults"`.
    AdultsOnly,
    /// `STR# 160[15]`: `"Can only be used by kids"`.
    KidsOnly,
    /// `STR# 160[17]`: `"Can only be used by pets"`.
    PetsOnly,
    /// `STR# 160[18]`: `"Can only be used by dogs"`.
    DogsOnly,
    /// `STR# 160[19]`: `"Can only be used by cats"`.
    CatsOnly,
}

impl UsageRestriction {
    /// Zero-based index into `GameData/UIText.iff STR# 160`.
    pub const fn str160_index(self) -> usize {
        match self {
            Self::AdultsOnly => 14,
            Self::KidsOnly => 15,
            Self::PetsOnly => 17,
            Self::DogsOnly => 18,
            Self::CatsOnly => 19,
        }
    }
}

/// One Buy Mode catalog entry (`0x4cd149`, `0x4cbd44`, `0x4cb9cd..0x4cbaa8`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyItem {
    pub guid: u32,
    pub price: i32,
    pub name: String,
    pub description: String,
    /// OBJD `+0x4e` (word 39): residential room sort bitmask (`0x4cb8e7`, `0x4cb905`).
    pub room_sort: u16,
    /// OBJD `+0x50` (word 40): function sort bitmask (`0x4cb91d`).
    pub function_sort: u16,
    /// OBJD `+0xbc` (word 94): function subsort bitmask (`0x4cb93b`).
    pub function_subsort: u16,
    /// OBJD `+0xbe` (word 95): Hot Date Downtown sort bitmask (`0x4cb9cd`).
    pub downtown_sort: u16,
    /// OBJD `+0xc2` (word 97): Vacation Island sort bitmask (`0x4cb9ee`).
    pub vacation_sort: u16,
    /// OBJD `+0xc6` (word 99): Unleashed Old Town / Community sort bitmask (`0x4cba0f`).
    pub community_sort: u16,
    /// OBJD `+0xce` (word 103): Superstar Studio Town sort bitmask (`0x4cba30`).
    pub studio_sort: u16,
    /// OBJD `+0xd0` (word 104): Makin' Magic Magic Town sort bitmask (`0x4cba51`).
    pub magic_sort: u16,
    /// OBJD `+0x8a` (word 69): nonzero for Build-mode catalog objects.
    pub build_mode_type: i16,
    /// OBJD `+0xa4..+0xb0` (words 82..88): Hunger, Comfort, Hygiene, Bladder,
    /// Energy, Fun, Room motive ratings (`0x457a36` cases `0..7`).
    pub ratings: [i16; 7],
    /// OBJD `+0xb2` full 16-bit skill/activity rating flags (`0x457a36` cases
    /// `7..14`: Cooking, Mechanical, Logic, Body, Creativity, Charisma, Study,
    /// plus upper bits preserved without truncation).
    pub skill_flags: u16,
    /// TTAB user restriction (`0x45775c..0x457978`: adults/kids/pets/dogs/cats).
    pub usage_restriction: Option<UsageRestriction>,
    /// TTAB group-activity flag (`0x4579da`: `"Group Activity"`, `STR# 160[16]`).
    pub group_activity: bool,
}

impl BuyItem {
    /// Returns the catalog sort bitmask for `zone`.
    pub const fn sort_word(&self, zone: BuyZone) -> u16 {
        match zone {
            BuyZone::Residential => self.room_sort | self.function_sort,
            BuyZone::Downtown => self.downtown_sort,
            BuyZone::Vacation => self.vacation_sort,
            BuyZone::Community => self.community_sort,
            BuyZone::StudioTown => self.studio_sort,
            BuyZone::MagicTown => self.magic_sort,
        }
    }
}

/// Catalog popup rating labels loaded from `GameData/UIText.iff STR# 160`
/// (`0x508168` → `0x50845d`, all 20 entries in Complete Collection 1.91).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatingLabels {
    labels: [String; 20],
}

impl Default for RatingLabels {
    fn default() -> Self {
        Self {
            labels: [
                "Hunger: %d".into(),
                "Comfort: %d".into(),
                "Hygiene: %d".into(),
                "Bladder: %d".into(),
                "Energy: %d".into(),
                "Fun: %d".into(),
                "Room: %d".into(),
                "+ Cooking".into(),
                "+ Mechanical".into(),
                "+ Logic".into(),
                "+ Body".into(),
                "+ Creativity".into(),
                "+ Charisma".into(),
                "+ Study".into(),
                "Can only be used by adults".into(),
                "Can only be used by kids".into(),
                "Group Activity".into(),
                "Can only be used by pets".into(),
                "Can only be used by dogs".into(),
                "Can only be used by cats".into(),
            ],
        }
    }
}

impl RatingLabels {
    /// All 20 localized `STR# 160` rating & restriction labels.
    pub fn labels(&self) -> &[String; 20] {
        &self.labels
    }

    /// Reads `STR# 160` (`0xa0`) from `GameData/UIText.iff`, falling back to
    /// built-in US English strings for any missing entry.
    pub fn load(install: &Install) -> Self {
        let mut out = Self::default();
        let Ok(path) = install.file("GameData/UIText.iff") else {
            return out;
        };
        let Ok(bytes) = std::fs::read(path) else {
            return out;
        };
        let Ok(iff) = Iff::parse(bytes) else {
            return out;
        };
        let Some(chunk) = iff.get(b"STR#", 160) else {
            return out;
        };
        let Ok(table) = StringTable::parse(iff.data(chunk)) else {
            return out;
        };
        let us: Vec<&str> = table
            .entries
            .iter()
            .filter(|e| e.language == US_ENGLISH)
            .map(|e| e.text.as_str())
            .collect();
        for (i, slot) in out.labels.iter_mut().enumerate() {
            if let Some(&text) = us.get(i)
                && !text.is_empty()
            {
                *slot = text.to_owned();
            }
        }
        out
    }

    /// Formats all nonzero motive ratings, set skill flags, user restriction
    /// notice, and group-activity flag for `item` (`0x50845d..0x50878e`).
    pub fn format(&self, item: &BuyItem) -> Vec<String> {
        let mut lines = Vec::new();
        for (i, &val) in item.ratings.iter().enumerate() {
            if val != 0 {
                lines.push(self.labels[i].replace("%d", &val.to_string()));
            }
        }
        for bit in 0..7 {
            if (item.skill_flags & (1u16 << bit)) != 0 {
                lines.push(self.labels[7 + bit].clone());
            }
        }
        if let Some(restriction) = item.usage_restriction {
            lines.push(self.labels[restriction.str160_index()].clone());
        }
        if item.group_activity {
            lines.push(self.labels[16].clone());
        }
        lines
    }
}

/// Buy Mode sell/dispose prompt templates loaded from `GameData/UIText.iff STR# 136`
/// (`0x50a343`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SellLabels {
    /// Entry 0: free disposal (`can_delete && refund == 0`).
    pub dispose: String,
    /// Entry 1: paid sale (`can_delete && refund > 0`).
    pub sell: String,
    /// Entry 2: non-deletable (`!can_delete`).
    pub cannot_delete: String,
}

impl Default for SellLabels {
    fn default() -> Self {
        Self {
            dispose: "Press \"Delete\" or click here \r\nto dispose of %.".into(),
            sell: "Press \"Delete\" or click here \r\nto sell % for $.".into(),
            cannot_delete: "% cannot be deleted.".into(),
        }
    }
}

impl SellLabels {
    /// Reads `STR# 136` (`0x88`) from `GameData/UIText.iff`, falling back to
    /// built-in US English templates for any missing entry.
    pub fn load(install: &Install) -> Self {
        let mut out = Self::default();
        let Ok(path) = install.file("GameData/UIText.iff") else {
            return out;
        };
        let Ok(bytes) = std::fs::read(path) else {
            return out;
        };
        let Ok(iff) = Iff::parse(bytes) else {
            return out;
        };
        let Some(chunk) = iff.get(b"STR#", 136) else {
            return out;
        };
        let Ok(table) = StringTable::parse(iff.data(chunk)) else {
            return out;
        };
        let us: Vec<&str> = table
            .entries
            .iter()
            .filter(|e| e.language == US_ENGLISH)
            .map(|e| e.text.as_str())
            .collect();
        if let Some(&text) = us.first()
            && !text.is_empty()
        {
            out.dispose = text.to_owned();
        }
        if let Some(&text) = us.get(1)
            && !text.is_empty()
        {
            out.sell = text.to_owned();
        }
        if let Some(&text) = us.get(2)
            && !text.is_empty()
        {
            out.cannot_delete = text.to_owned();
        }
        out
    }

    /// Formats the sell/dispose/cannot-delete prompt lines for `info`
    /// (`0x50a343`, substituting `%` with the object name and `$` with
    /// `$<refund>` — rendered as `§` by the Sims UI font).
    pub fn format(&self, info: &SellInfo) -> Vec<String> {
        let template = if !info.can_delete {
            &self.cannot_delete
        } else if info.refund > 0 {
            &self.sell
        } else {
            &self.dispose
        };
        let price = format!("${}", info.refund);
        let mut rendered = String::with_capacity(template.len() + info.name.len() + price.len());
        for ch in template.chars() {
            match ch {
                '%' => rendered.push_str(&info.name),
                '$' => rendered.push_str(&price),
                other => rendered.push(other),
            }
        }
        rendered
            .split('\n')
            .map(|line| line.trim_end_matches(['\r', ' ']).to_owned())
            .filter(|line| !line.is_empty())
            .collect()
    }
}

/// Information about a picked-up existing lot object needed to build the Buy
/// Mode sell prompt (`0x487d7a` → `0x4cf90b` → `0x50a343`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SellInfo {
    /// Catalog master GUID (or object GUID fallback) for the thumbnail icon.
    pub guid: u32,
    /// Display name of the held object.
    pub name: String,
    /// Depreciated resale value (`0x43219a`).
    pub refund: i32,
    /// Whether `CanDelete` (`0x435440` / `0x423135`) permits deleting/selling.
    pub can_delete: bool,
}

/// All residential-buyable objects from an install, sorted by `(price, name)`
/// as `0x4cd149` / `0x4cd9a5` orders them.
#[derive(Debug, Clone, Default)]
pub struct BuyCatalog {
    items: Vec<BuyItem>,
    /// Maps each non-master part GUID of a multi-tile object to its master OBJD GUID.
    master_by_part: std::collections::BTreeMap<u32, u32>,
    /// Display names for non-catalog objects (e.g. `room_sort == 0`) read from CTSS/OBJD.
    fallback_names: std::collections::BTreeMap<u32, String>,
    rating_labels: RatingLabels,
    sell_labels: SellLabels,
}

/// OBJD byte offsets read by `0x4cd149`, `0x4cb8e7..0x4cbaa8`, and `0x457a36`.
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
const OBJD_RATING_BASE: usize = 0xa4;
const OBJD_RATING_SKILLS: usize = 0xb2;
const OBJD_FUNC_SUBSORT: usize = 0xbc;
const OBJD_DOWNTOWN_SORT: usize = 0xbe;
const OBJD_VACATION_SORT: usize = 0xc2;
const OBJD_COMMUNITY_SORT: usize = 0xc6;
const OBJD_STUDIO_SORT: usize = 0xce;
const OBJD_MAGIC_SORT: usize = 0xd0;

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

/// Expansion commercial category bitmasks in UI slot order `0..5`
/// (`0x6cc464..0x6cc500`, `0x4cb9cd..0x4cba6f`).
pub const COMMERCIAL_BITS: [u16; 5] = [0x01, 0x02, 0x04, 0x08, 0x80];

/// UIGraphics bitmap IDs from `0x6cc3fc..0x6cc684` and `0x5050e4`.
pub const PREV_ARROW: i32 = 102;
pub const NEXT_ARROW: i32 = 101;
pub const FUNC_CAT_BMPS: [i32; 8] = [150, 151, 152, 153, 154, 155, 156, 157];
pub const ROOM_CAT_BMPS: [i32; 8] = [158, 159, 160, 161, 162, 163, 164, 165];
pub const ROOM_SUB_BMPS: [i32; 8] = [210, 211, 212, 213, 214, 215, 216, 217];
pub const BACK_FUNC_BMPS: [i32; 8] = [170, 171, 172, 173, 174, 175, 176, 177];
pub const BACK_ROOM_BMPS: [i32; 8] = [180, 181, 182, 183, 184, 185, 186, 187];
pub const DOWNTOWN_CAT_BMPS: [i32; 5] = [200, 201, 202, 203, 204];
pub const BACK_DOWNTOWN_BMPS: [i32; 5] = [190, 191, 192, 193, 194];
pub const DOWNTOWN_SUB_BMPS: [i32; 8] = [220, 221, 222, 223, 224, 225, 226, 227];
pub const VACATION_CAT_BMPS: [i32; 5] = [230, 231, 232, 233, 234];
pub const BACK_VACATION_BMPS: [i32; 5] = [195, 196, 197, 198, 199];
pub const VACATION_SUB_BMPS: [i32; 8] = [240, 241, 242, 243, 244, 245, 246, 247];
pub const COMMUNITY_CAT_BMPS: [i32; 5] = [250, 251, 252, 253, 254];
pub const BACK_COMMUNITY_BMPS: [i32; 5] = [255, 256, 257, 258, 259];
pub const COMMUNITY_SUB_BMPS: [i32; 8] = [260, 261, 262, 263, 264, 265, 266, 267];
pub const STUDIO_CAT_BMPS: [i32; 5] = [273, 274, 276, 275, 277];
pub const BACK_STUDIO_BMPS: [i32; 5] = [268, 269, 270, 271, 272];
pub const MAGIC_CAT_BMPS: [i32; 5] = [1705, 1706, 1708, 1707, 1709];
pub const BACK_MAGIC_BMPS: [i32; 5] = [1700, 1701, 1702, 1703, 1704];
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

const COMMERCIAL_CAT_POS: [[i32; 2]; 5] = [
    ROOM_CAT_POS[0],
    ROOM_CAT_POS[1],
    ROOM_CAT_POS[2],
    ROOM_CAT_POS[3],
    ROOM_CAT_POS[7],
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
        dialog::BACKGROUND,
    ];
    ids.extend(FUNC_CAT_BMPS);
    ids.extend(ROOM_CAT_BMPS);
    ids.extend(ROOM_SUB_BMPS);
    ids.extend(BACK_FUNC_BMPS);
    ids.extend(BACK_ROOM_BMPS);
    ids.extend(DOWNTOWN_CAT_BMPS);
    ids.extend(BACK_DOWNTOWN_BMPS);
    ids.extend(DOWNTOWN_SUB_BMPS);
    ids.extend(VACATION_CAT_BMPS);
    ids.extend(BACK_VACATION_BMPS);
    ids.extend(VACATION_SUB_BMPS);
    ids.extend(COMMUNITY_CAT_BMPS);
    ids.extend(BACK_COMMUNITY_BMPS);
    ids.extend(COMMUNITY_SUB_BMPS);
    ids.extend(STUDIO_CAT_BMPS);
    ids.extend(BACK_STUDIO_BMPS);
    ids.extend(MAGIC_CAT_BMPS);
    ids.extend(BACK_MAGIC_BMPS);
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
        let mut master_by_part = std::collections::BTreeMap::new();
        let mut fallback_names = std::collections::BTreeMap::new();
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
            let mut master_guids: std::collections::BTreeMap<i16, u32> =
                std::collections::BTreeMap::new();
            let mut part_masters: Vec<(u32, i16)> = Vec::new();
            for (guid, chunk_id) in entries {
                let Some(chunk) = iff.get(b"OBJD", chunk_id) else {
                    continue;
                };
                let data = iff.data(chunk);
                let master = read_i16(data, OBJD_MASTER);
                let sub_index = read_i16(data, OBJD_SUB_INDEX);
                if master != 0 {
                    if sub_index == -1 {
                        master_guids.insert(master, guid);
                    } else {
                        part_masters.push((guid, master));
                    }
                }
                // 0x4575b8: skip non-master tiles of multi-tile objects.
                if master != 0 && sub_index != -1 {
                    continue;
                }
                let kind = read_i16(data, OBJD_TYPE);
                if kind == PERSON_TYPE {
                    continue;
                }
                let ctss_id = read_i16(data, OBJD_CTSS);
                let (mut name, description, mut raw_name) = read_ctss(&iff, ctss_id);
                // 0x457d3b: when the catalog string table is missing or its
                // name entry is empty, fall back to the OBJD chunk label.
                if name.is_empty() && !chunk.label.is_empty() {
                    name = chunk.label.clone();
                    raw_name = cp1252_bytes(&chunk.label);
                }
                // 0x4cb905..0x4cbaa8: keep the object whenever any residential
                // (`+0x4e` room or `+0x50` function) or expansion commercial
                // catalog sort word is nonzero.
                let room_sort = read_i16(data, OBJD_ROOM_SORT) as u16;
                let function_sort = read_i16(data, OBJD_FUNC_SORT) as u16;
                let downtown_sort = read_i16(data, OBJD_DOWNTOWN_SORT) as u16;
                let vacation_sort = read_i16(data, OBJD_VACATION_SORT) as u16;
                let community_sort = read_i16(data, OBJD_COMMUNITY_SORT) as u16;
                let studio_sort = read_i16(data, OBJD_STUDIO_SORT) as u16;
                let magic_sort = read_i16(data, OBJD_MAGIC_SORT) as u16;
                let any_sort = room_sort
                    | function_sort
                    | downtown_sort
                    | vacation_sort
                    | community_sort
                    | studio_sort
                    | magic_sort;
                // 0x457588 / 0x4575a1: needs graphics (master != 0 ||
                // base_graphic_id != 0) or an interaction table.
                let graphics = read_i16(data, OBJD_GRAPHICS);
                let ttab = read_i16(data, OBJD_TTAB);
                if !is_buy_catalog_eligible(any_sort, master, graphics, ttab) {
                    if !name.is_empty() {
                        fallback_names.insert(guid, name);
                    }
                    continue;
                }
                let price = i32::from(read_i16(data, OBJD_PRICE));
                let function_subsort = read_i16(data, OBJD_FUNC_SUBSORT) as u16;
                let build_mode_type = read_i16(data, OBJD_BUILD_TYPE);
                let mut ratings = [0i16; 7];
                for (i, slot) in ratings.iter_mut().enumerate() {
                    *slot = read_i16(data, OBJD_RATING_BASE + i * 2);
                }
                let skill_flags = read_i16(data, OBJD_RATING_SKILLS) as u16;
                let (usage_restriction, group_activity) = read_ttab_usage(&iff, ttab);
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
                        downtown_sort,
                        vacation_sort,
                        community_sort,
                        studio_sort,
                        magic_sort,
                        build_mode_type,
                        ratings,
                        skill_flags,
                        usage_restriction,
                        group_activity,
                    },
                ));
            }
            for (part_guid, master_id) in part_masters {
                if let Some(&master_guid) = master_guids.get(&master_id) {
                    master_by_part.insert(part_guid, master_guid);
                }
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
            master_by_part,
            fallback_names,
            rating_labels: RatingLabels::default(),
            sell_labels: SellLabels::default(),
        }
    }

    /// Attaches localized `UIText.iff STR# 160` rating labels to this catalog.
    pub fn with_rating_labels(mut self, rating_labels: RatingLabels) -> Self {
        self.rating_labels = rating_labels;
        self
    }

    /// Attaches localized `UIText.iff STR# 136` sell prompt labels to this catalog.
    pub fn with_sell_labels(mut self, sell_labels: SellLabels) -> Self {
        self.sell_labels = sell_labels;
        self
    }

    /// Constructs a catalog from explicit items (for unit tests).
    pub fn from_items(mut items: Vec<BuyItem>) -> Self {
        items.sort_by(|a, b| {
            a.price
                .cmp(&b.price)
                .then_with(|| cp1252_bytes(&a.name).cmp(&cp1252_bytes(&b.name)))
                .then_with(|| a.guid.cmp(&b.guid))
        });
        Self {
            items,
            master_by_part: std::collections::BTreeMap::new(),
            fallback_names: std::collections::BTreeMap::new(),
            rating_labels: RatingLabels::default(),
            sell_labels: SellLabels::default(),
        }
    }

    /// Looks up an item by its catalog master GUID.
    pub fn get(&self, guid: u32) -> Option<&BuyItem> {
        self.items.iter().find(|item| item.guid == guid)
    }

    /// Resolves any multi-tile part GUID to its master OBJD GUID, returning
    /// `guid` unchanged for single-tile or master GUIDs.
    pub fn master_guid(&self, guid: u32) -> u32 {
        self.master_by_part.get(&guid).copied().unwrap_or(guid)
    }

    /// Looks up the catalog master [`BuyItem`] for either a master GUID or any
    /// non-master part GUID of a multi-tile object.
    pub fn get_for_object(&self, guid: u32) -> Option<&BuyItem> {
        self.get(self.master_guid(guid))
    }

    /// Resolves the master GUID and display name of any object GUID (buyable
    /// catalog items and non-catalog objects alike).
    pub fn display_for_object(&self, guid: u32) -> (u32, String) {
        let master = self.master_guid(guid);
        if let Some(item) = self.get(master) {
            return (item.guid, item.name.clone());
        }
        let name = self
            .fallback_names
            .get(&master)
            .or_else(|| self.fallback_names.get(&guid))
            .cloned()
            .unwrap_or_default();
        (master, name)
    }

    /// Every buyable item across residential and commercial catalogs in sort order.
    pub fn all(&self) -> &[BuyItem] {
        &self.items
    }

    /// All catalog items available in `zone` (i.e. with a nonzero sort word
    /// for that zone), preserving `(price, raw_name, guid)` catalog sort order.
    pub fn items_for_zone(&self, zone: BuyZone) -> Vec<&BuyItem> {
        self.items
            .iter()
            .filter(|item| item.sort_word(zone) != 0)
            .collect()
    }

    /// Items matching the current `(zone, sort, category, subcategory)`
    /// selection (`0x507288`, `0x4cb8e7..0x4cbaa8`).
    pub fn filtered(&self, state: &BuyState) -> Vec<&BuyItem> {
        let Some(cat) = state.category else {
            return Vec::new();
        };
        if cat >= state.zone.category_count() || state.subcategory >= 8 {
            return Vec::new();
        }
        match state.zone {
            BuyZone::Residential => match state.sort {
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
            },
            zone => {
                let cat_bit = COMMERCIAL_BITS[cat];
                let func_bit = FUNCTION_BITS[state.subcategory];
                let is_misc_sub = state.subcategory == 7;
                self.items
                    .iter()
                    .filter(|item| {
                        (item.sort_word(zone) & cat_bit) != 0
                            && ((item.function_sort & func_bit) != 0
                                || (is_misc_sub && item.function_sort == 0))
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

/// Laid-out Buy Mode strip widgets, optional popup card, and optional sell
/// prompt (`0x487d7a` → `0x4cf90b` → `0x50a343`).
#[derive(Debug, Clone, Default)]
pub struct BuyLayout {
    pub widgets: Vec<BuyWidget>,
    pub popup: Option<PopupCard>,
    pub sell_prompt: Option<SellPrompt>,
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
    /// Formatted motive/skill rating entries (`0x50845d`, `STR# 160`).
    pub ratings: Vec<String>,
}

/// Active sell/dispose/cannot-delete prompt shown in the Buy Mode catalog strip
/// while holding an existing lot object (`0x487d7a` → `0x4cf90b` → `0x50a343`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SellPrompt {
    pub rect: Rect,
    pub guid: u32,
    pub name: String,
    pub refund: i32,
    pub can_delete: bool,
    pub lines: Vec<String>,
}

impl BuyLayout {
    /// Whether any interactive Buy Mode strip widget (or the active sell
    /// prompt target) covers `point`.
    /// The popup card is informational and mouse-transparent so moving into
    /// the house view immediately activates the placement tool (`0x487d3d`).
    pub fn covers(&self, point: [i32; 2]) -> bool {
        self.widgets
            .iter()
            .any(|w| w.rect.contains(point[0], point[1]))
            || self
                .sell_prompt
                .as_ref()
                .is_some_and(|p| p.rect.contains(point[0], point[1]))
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
    layout_with_sell(size, catalog, state, art, available_funds, None)
}

/// Lays out the Buy Mode strip widgets, popup card, or active Hand Tool sell
/// prompt (`0x487d7a` → `0x4cf90b` → `0x50a343`) for a canvas of `size`.
pub fn layout_with_sell(
    size: [i32; 2],
    catalog: &BuyCatalog,
    state: &BuyState,
    art: &Art,
    available_funds: i32,
    sell: Option<&SellInfo>,
) -> BuyLayout {
    let [width, height] = size.map(|v| v.clamp(0, 1 << 15));
    let strip_y = height - STRIP;
    if let Some(info) = sell {
        let prompt_x = 240.min(width);
        let prompt_w = (width - prompt_x).max(0);
        return BuyLayout {
            widgets: Vec::new(),
            popup: None,
            sell_prompt: Some(SellPrompt {
                rect: Rect::new(prompt_x, strip_y, prompt_w, STRIP),
                guid: info.guid,
                name: info.name.clone(),
                refund: info.refund,
                can_delete: info.can_delete,
                lines: catalog.sell_labels.format(info),
            }),
            page: state.page,
            total_pages: 0,
        };
    }
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
            if state.zone == BuyZone::Residential {
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
            } else {
                let bmps = commercial_cat_bmps(state.zone);
                for (i, &id) in bmps.iter().enumerate() {
                    let [w, h] = btn_size(id);
                    let pos = COMMERCIAL_CAT_POS[i];
                    widgets.push(BuyWidget {
                        rect: Rect::new(pos[0], strip_y + pos[1], w, h),
                        kind: BuyWidgetKind::Button {
                            id,
                            selected: false,
                            enabled: true,
                        },
                        command: Some(Command::BuyCategory(i)),
                    });
                }
            }
        }
        Some(cat) if cat < state.zone.category_count() => {
            let back_id = match state.zone {
                BuyZone::Residential => match state.sort {
                    BuySort::ByFunction => BACK_FUNC_BMPS[cat],
                    BuySort::ByRoom => BACK_ROOM_BMPS[cat],
                },
                zone => commercial_back_bmps(zone)[cat],
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
                let bmp = match state.zone {
                    BuyZone::Residential => match state.sort {
                        BuySort::ByFunction => match sub {
                            0..=3 => Some(FUNC_CAT_BMPS[cat] * 10 + sub as i32),
                            4 if cat == 7 => Some(SUBCAT_MISC_PETS),
                            5 if cat == 7 => Some(SUBCAT_MISC_MAGIC),
                            4 | 5 => None,
                            6 => Some(SUBCAT_OTHER),
                            _ => Some(SUBCAT_ALL),
                        },
                        BuySort::ByRoom => Some(ROOM_SUB_BMPS[sub]),
                    },
                    zone => Some(commercial_sub_bmps(zone)[sub]),
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
            let ratings = catalog.rating_labels.format(item);
            // Native `0x50845d` places each formatted STR# 160 rating on its
            // own line and expands the popup card height past 127 px to fit
            // wrapped description lines plus all rating lines.
            let max_w = (card_w - 190).max(40);
            let card_h = if let Some(font) = art.font(10) {
                let desc_lines = if item.description.is_empty() {
                    0
                } else {
                    font.wrap(&item.description, max_w).len() as i32
                };
                let rating_lines: i32 = ratings
                    .iter()
                    .map(|r| font.wrap(r, max_w).len() as i32)
                    .sum();
                let line_step = font.line_height + 2;
                let ratings_gap = if desc_lines > 0 && rating_lines > 0 {
                    4
                } else {
                    0
                };
                let needed = 32 + (desc_lines + rating_lines) * line_step + ratings_gap + 10;
                needed.max(127).min(strip_y.max(0))
            } else {
                127.min(strip_y.max(0))
            };
            PopupCard {
                rect: Rect::new(card_x, (strip_y - card_h).max(0), card_w, card_h),
                guid: item.guid,
                name: item.name.clone(),
                price: item.price,
                description: item.description.clone(),
                ratings,
            }
        });

    BuyLayout {
        widgets,
        popup,
        sell_prompt: None,
        page,
        total_pages,
    }
}

/// Draws the Buy Mode strip widgets, active sell prompt, and selected item
/// popup card into `frame`.
pub fn draw(
    frame: &mut Frame,
    art: &Art,
    thumbnails: &Thumbnails,
    layout: &BuyLayout,
    pointer: Option<[i32; 2]>,
) {
    if let Some(prompt) = &layout.sell_prompt {
        // Native `0x50a343`: draw the object's isometric preview (`0x4384c8`)
        // centered in the left portrait box (`after-sell.png`: bench spans
        // x=264..343, y=509..590 centered at (304, 550); text at x=399, y=517),
        // falling back to the catalog icon/thumbnail when absent.
        let box_x = prompt.rect.x + 8;
        let box_w = 112;
        let box_h = prompt.rect.height.max(1);
        if let Some(pic) = thumbnails.balloon_picture(prompt.guid, None, box_w, box_h) {
            let draw_w = (pic.width as i32).min(box_w);
            let draw_h = (pic.height as i32).min(box_h);
            let src_x = ((pic.width as i32 - draw_w) / 2).max(0);
            let src_y = ((pic.height as i32 - draw_h) / 2).max(0);
            let dst_x = box_x + (box_w - draw_w) / 2;
            let dst_y = prompt.rect.y + (box_h - draw_h) / 2;
            frame.draw(pic, Rect::new(src_x, src_y, draw_w, draw_h), [dst_x, dst_y]);
        } else if let Some(icon) = thumbnails.icon(art, prompt.guid) {
            let draw_h = (icon.height as i32).min(prompt.rect.height);
            let src_y = ((icon.height as i32 - draw_h) / 2).max(0);
            let dst_y = prompt.rect.y + ((prompt.rect.height - draw_h) / 2).max(0);
            frame.draw(
                &icon,
                Rect::new(0, src_y, icon.width as i32, draw_h),
                [box_x + (box_w - icon.width as i32) / 2, dst_y],
            );
        } else if let Some(thumb) = thumbnails
            .item(prompt.guid)
            .and_then(|i| i.thumbnail.as_ref())
        {
            let half = if thumb.width > CELL {
                thumb.width / 2
            } else {
                thumb.width
            };
            let dst_y = prompt.rect.y + ((prompt.rect.height - thumb.height as i32) / 2).max(0);
            frame.draw(
                thumb,
                Rect::new(0, 0, half as i32, thumb.height as i32),
                [box_x + (box_w - half as i32) / 2, dst_y],
            );
        }
        if let Some(font) = art.font(10) {
            // Native `after-sell.png`: 10pt text starts at `(399, 517)`..`526`
            // on line 1 and `(399, 536)`..`548` on line 2 (`19` px line step).
            let text_x = prompt.rect.x + 158;
            let max_w = (prompt.rect.width - 170).max(40);
            let mut wrapped = Vec::new();
            for line in &prompt.lines {
                wrapped.extend(font.wrap(line, max_w));
            }
            let line_step = 19;
            let mut y = prompt.rect.y + 12;
            for line in wrapped {
                if y + font.line_height <= prompt.rect.y + prompt.rect.height {
                    font.draw(frame, &line, [text_x, y], [235, 240, 255]);
                }
                y += line_step;
            }
        }
        return;
    }

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
        if let Some(sheet) = art.image(dialog::BACKGROUND) {
            dialog::draw_nine_slice(frame, sheet, card.rect);
        } else {
            frame.fill(card.rect, [20, 32, 92]);
            let border = [120, 148, 212];
            frame.fill(
                Rect::new(card.rect.x, card.rect.y, card.rect.width, 2),
                border,
            );
            frame.fill(
                Rect::new(
                    card.rect.x,
                    card.rect.y + card.rect.height - 2,
                    card.rect.width,
                    2,
                ),
                border,
            );
            frame.fill(
                Rect::new(card.rect.x, card.rect.y, 2, card.rect.height),
                border,
            );
            frame.fill(
                Rect::new(
                    card.rect.x + card.rect.width - 2,
                    card.rect.y,
                    2,
                    card.rect.height,
                ),
                border,
            );
        }
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
            let line_step = font.line_height + 2;
            let desc_lines = if card.description.is_empty() {
                Vec::new()
            } else {
                font.wrap(&card.description, max_w)
            };
            let mut rating_lines = Vec::new();
            for rating in &card.ratings {
                rating_lines.extend(font.wrap(rating, max_w));
            }
            let bottom = card.rect.y + card.rect.height - 4;
            for line in &desc_lines {
                if y + font.line_height <= bottom {
                    font.draw(frame, line, [text_x, y], [210, 224, 248]);
                }
                y += line_step;
            }
            if !desc_lines.is_empty() && !rating_lines.is_empty() {
                y += 4;
            }
            for line in &rating_lines {
                if y + font.line_height <= bottom {
                    font.draw(frame, line, [text_x, y], [248, 232, 168]);
                }
                y += line_step;
            }
        }
    }
}

fn commercial_cat_bmps(zone: BuyZone) -> &'static [i32; 5] {
    match zone {
        BuyZone::Downtown | BuyZone::Residential => &DOWNTOWN_CAT_BMPS,
        BuyZone::Vacation => &VACATION_CAT_BMPS,
        BuyZone::Community => &COMMUNITY_CAT_BMPS,
        BuyZone::StudioTown => &STUDIO_CAT_BMPS,
        BuyZone::MagicTown => &MAGIC_CAT_BMPS,
    }
}

fn commercial_back_bmps(zone: BuyZone) -> &'static [i32; 5] {
    match zone {
        BuyZone::Downtown | BuyZone::Residential => &BACK_DOWNTOWN_BMPS,
        BuyZone::Vacation => &BACK_VACATION_BMPS,
        BuyZone::Community => &BACK_COMMUNITY_BMPS,
        BuyZone::StudioTown => &BACK_STUDIO_BMPS,
        BuyZone::MagicTown => &BACK_MAGIC_BMPS,
    }
}

fn commercial_sub_bmps(zone: BuyZone) -> &'static [i32; 8] {
    match zone {
        BuyZone::Vacation => &VACATION_SUB_BMPS,
        BuyZone::Community => &COMMUNITY_SUB_BMPS,
        BuyZone::Downtown | BuyZone::StudioTown | BuyZone::MagicTown | BuyZone::Residential => {
            &DOWNTOWN_SUB_BMPS
        }
    }
}

/// Parses an object's `TTAB` chunk to determine its user restriction
/// (`0x45775c..0x457978`: adults/kids/dogs/cats/pets only) and group-activity
/// flag (`0x4579da`) for `GameData/UIText.iff STR# 160[14..=19]`.
fn read_ttab_usage(iff: &Iff, ttab_id: i16) -> (Option<UsageRestriction>, bool) {
    let Ok(id) = u16::try_from(ttab_id) else {
        return (None, false);
    };
    let Some(chunk) = iff.get(b"TTAB", id) else {
        return (None, false);
    };
    let Ok(table) = InteractionTable::parse(iff.data(chunk)) else {
        return (None, false);
    };
    let mut non_debug = 0usize;
    let mut adults_only = true;
    let mut kids_only = true;
    let mut dogs_only = true;
    let mut cats_only = true;
    let mut pets_only = true;
    let mut group_activity = false;
    for action in &table.interactions {
        let f = action.flags;
        // Bit 7 (0x80): debug interaction, skipped by `0x45775c..0x457a25`.
        if (f & (1 << 7)) != 0 {
            continue;
        }
        non_debug += 1;
        let no_kids = (f & (1 << 4)) != 0;
        let no_adults = (f & (1 << 6)) != 0;
        let dogs = (f & (1 << 9)) != 0;
        let cats = (f & (1 << 10)) != 0;
        if !no_kids || dogs || cats {
            adults_only = false;
        }
        if !no_adults || dogs || cats {
            kids_only = false;
        }
        if !no_adults || !no_kids || cats {
            dogs_only = false;
        }
        if !no_adults || !no_kids || dogs {
            cats_only = false;
        }
        if !no_adults || !no_kids {
            pets_only = false;
        }
        // 0x4579da -> 0x41f12c: `joining_index` references a target interaction
        // by its TTAB menu `string_index`, not its dense vector position.
        if action.joining_index >= 0
            && table
                .interactions
                .iter()
                .any(|other| other.string_index == action.joining_index as u32)
        {
            group_activity = true;
        }
    }
    if non_debug == 0 {
        return (None, false);
    }
    // Check `adults_only`, `kids_only`, then distinguish both-species `pets_only`
    // (`!(dogs_only ^ cats_only)`) from species-specific `dogs_only` / `cats_only`
    // so all three Unleashed pet restriction labels (`STR# 160[17..=19]`) remain reachable.
    let restriction = if adults_only {
        Some(UsageRestriction::AdultsOnly)
    } else if kids_only {
        Some(UsageRestriction::KidsOnly)
    } else if pets_only && !(dogs_only ^ cats_only) {
        Some(UsageRestriction::PetsOnly)
    } else if dogs_only {
        Some(UsageRestriction::DogsOnly)
    } else if cats_only {
        Some(UsageRestriction::CatsOnly)
    } else {
        None
    };
    (restriction, group_activity)
}

/// Whether an OBJD entry passes the Buy Mode catalog sort & art/interaction
/// filter (`0x4cb905..0x4cbaa8`, `0x457588`).
fn is_buy_catalog_eligible(any_sort: u16, master: i16, graphics: i16, ttab: i16) -> bool {
    any_sort != 0 && !(master == 0 && graphics == 0 && ttab <= 0)
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
    // 0x40621d..0x40635e: look up legacy `CATS` first (raw NUL-terminated name
    // followed by optional NUL-terminated description), then `CTSS`.
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

    #[allow(clippy::too_many_arguments)]
    fn make_item(
        guid: u32,
        price: i32,
        name: &str,
        description: &str,
        room_sort: u16,
        function_sort: u16,
        function_subsort: u16,
        ratings: [i16; 7],
        skill_flags: u16,
    ) -> BuyItem {
        BuyItem {
            guid,
            price,
            name: name.into(),
            description: description.into(),
            room_sort,
            function_sort,
            function_subsort,
            downtown_sort: 0,
            vacation_sort: 0,
            community_sort: 0,
            studio_sort: 0,
            magic_sort: 0,
            build_mode_type: 0,
            ratings,
            skill_flags,
            usage_restriction: None,
            group_activity: false,
        }
    }

    fn sample_catalog() -> BuyCatalog {
        BuyCatalog::from_items(vec![
            make_item(
                0x1111_1111,
                200,
                "Fancy Chair",
                "Comfy.",
                0x20, // Dining Room
                0x01, // Seating
                0x01, // Dining Chairs
                [0, 4, 0, 0, 0, 0, 1],
                0,
            ),
            make_item(
                0x26bf_bb29,
                80,
                "Werkbunnst All Purpose Chair",
                "Minimal.",
                0x20,
                0x01,
                0x01,
                [0, 2, 0, 0, 0, 0, 0],
                0,
            ),
            make_item(
                0x2222_2222,
                450,
                "Comfy Sofa",
                "Soft.",
                0x08, // Living Room
                0x01, // Seating
                0x04, // Sofas
                [0, 6, 0, 0, 3, 0, 2],
                0x04, // + Logic
            ),
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
            make_item(1, 100, "apple", "", 1, 1, 1, [0; 7], 0),
            make_item(2, 100, "Zoo", "", 1, 1, 1, [0; 7], 0),
            make_item(3, 100, "Lamp©", "", 1, 1, 1, [0; 7], 0),
            make_item(4, 100, "Lamp™", "", 1, 1, 1, [0; 7], 0),
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
        // Selecting an item populates formatted ratings on the popup card.
        state.selected = Some(0x2222_2222);
        let with_popup = layout([800, 600], &cat, &state, &art, 2542);
        let popup = with_popup.popup.expect("popup card for selected item");
        assert_eq!(
            popup.ratings,
            vec!["Comfort: 6", "Energy: 3", "Room: 2", "+ Logic"]
        );
        // Second Buy click toggles sort between ByFunction and ByRoom and
        // returns to top-level categories.
        assert!(state.apply(&Command::BuyToggleSort));
        assert_eq!(state.sort, BuySort::ByRoom);
        assert_eq!(state.category, None);
        assert_eq!(state.selected, None);
        assert!(state.apply(&Command::BuyToggleSort));
        assert_eq!(state.sort, BuySort::ByFunction);
    }

    /// Builds a minimal IFF 2.5 buffer from `(type_code, id, payload)` chunks.
    fn build_iff(chunks: &[(&[u8; 4], u16, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1");
        buf.resize(64, 0);
        for &(tag, id, payload) in chunks {
            let size = (76 + payload.len()) as u32;
            buf.extend_from_slice(tag);
            buf.extend_from_slice(&size.to_be_bytes());
            buf.extend_from_slice(&id.to_be_bytes());
            buf.extend_from_slice(&0u16.to_be_bytes()); // flags
            buf.extend_from_slice(&[0u8; 64]); // label
            buf.extend_from_slice(payload);
        }
        buf
    }

    #[test]
    fn legacy_cats_is_parsed_as_nul_strings_before_ctss() {
        // Legacy `CATS` only (`0x40621d` -> `0x40631f`): raw NUL-terminated
        // name + description in Windows-1252.
        let cats_payload = b"Legacy Lamp\x99\0Legacy description\0";
        let iff = Iff::parse(build_iff(&[(b"CATS", 2008, cats_payload)])).unwrap();
        let (name, desc, raw) = read_ctss(&iff, 2008);
        assert_eq!(name, "Legacy Lamp™");
        assert_eq!(desc, "Legacy description");
        assert_eq!(raw, b"Legacy Lamp\x99");

        // When both `CATS` and `CTSS` share the same ID, native `0x40621d`
        // uses `CATS` first.
        let mut ctss_payload = Vec::new();
        ctss_payload.extend_from_slice(&(-3i16).to_le_bytes()); // format -3
        ctss_payload.extend_from_slice(&2u16.to_le_bytes()); // 2 entries
        ctss_payload.push(US_ENGLISH);
        ctss_payload.extend_from_slice(b"CTSS Lamp\0\0");
        ctss_payload.push(US_ENGLISH);
        ctss_payload.extend_from_slice(b"CTSS description\0\0");
        let both = Iff::parse(build_iff(&[
            (b"CTSS", 2008, &ctss_payload),
            (b"CATS", 2008, cats_payload),
        ]))
        .unwrap();
        let (name, desc, _) = read_ctss(&both, 2008);
        assert_eq!(name, "Legacy Lamp™");
        assert_eq!(desc, "Legacy description");

        // With `CTSS` alone, `read_ctss` falls through to `StringTable::parse`.
        let ctss_only = Iff::parse(build_iff(&[(b"CTSS", 2008, &ctss_payload)])).unwrap();
        let (name, desc, _) = read_ctss(&ctss_only, 2008);
        assert_eq!(name, "CTSS Lamp");
        assert_eq!(desc, "CTSS description");
    }

    #[test]
    fn function_only_residential_item_is_retained_and_filtered_by_function() {
        // OBJD eligibility helper used by `BuyCatalog::load` retains an object
        // whose ONLY nonzero sort word is `function_sort` (`+0x50`) and rejects
        // an object with all sort words zero.
        let room_sort: u16 = 0;
        let function_sort: u16 = 0x0001;
        assert!(is_buy_catalog_eligible(room_sort | function_sort, 0, 1, 0));
        assert!(!is_buy_catalog_eligible(0, 0, 1, 0));

        // An object with `room_sort == 0`, `function_sort != 0`, and
        // `function_subsort != 0` must be included in Residential ByFunction
        // (`0x50765c`) and `items_for_zone(BuyZone::Residential)`.
        let item = make_item(
            0x3333_3333,
            150,
            "Function-Only Chair",
            "No room bits set",
            0,      // room_sort == 0
            0x0001, // Seating
            0x0001, // Dining chairs subsort
            [2, 0, 0, 0, 0, 0, 0],
            0,
        );
        let cat = BuyCatalog::from_items(vec![item]);
        assert_eq!(cat.items_for_zone(BuyZone::Residential).len(), 1);
        let state = BuyState {
            zone: BuyZone::Residential,
            sort: BuySort::ByFunction,
            category: Some(0),
            subcategory: 0,
            page: 0,
            selected: None,
        };
        let filtered = cat.filtered(&state);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].guid, 0x3333_3333);
    }

    /// Encodes a minimal TTAB v7 chunk from `(string_index, flags, joining_index)` tuples.
    fn build_ttab(rows: &[(u32, u32, i32)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(rows.len() as u16).to_le_bytes());
        out.extend_from_slice(&7u16.to_le_bytes()); // version 7 (no compression flag)
        for &(string_index, flags, joining_index) in rows {
            out.extend_from_slice(&4096u16.to_le_bytes()); // action
            out.extend_from_slice(&0u16.to_le_bytes()); // test
            out.extend_from_slice(&0u32.to_le_bytes()); // motive count
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&string_index.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes()); // attenuation_code
            out.extend_from_slice(&0f32.to_le_bytes()); // attenuation
            out.extend_from_slice(&50u32.to_le_bytes()); // autonomy
            out.extend_from_slice(&joining_index.to_le_bytes());
        }
        out
    }

    #[test]
    fn ttab_usage_matches_191_sparse_joining_id_and_pets_precedence() {
        // Sparse menu string IDs: `joining_index = 42` matches `string_index == 42`
        // even though `interactions.len() == 2` (`0x41f12c`).
        let sparse_ttab = build_ttab(&[(10, 0, 42), (42, 0, -1)]);
        let iff = Iff::parse(build_iff(&[(b"TTAB", 128, &sparse_ttab)])).unwrap();
        assert_eq!(read_ttab_usage(&iff, 128), (None, true));

        // Nonexistent low `joining_index = 0` must NOT match when no interaction
        // has `string_index == 0` (even though `0 < interactions.len()`).
        let bogus_join = build_ttab(&[(10, 0, 0), (20, 0, -1)]);
        let iff_bogus = Iff::parse(build_iff(&[(b"TTAB", 128, &bogus_join)])).unwrap();
        assert_eq!(read_ttab_usage(&iff_bogus, 128), (None, false));

        // Pet restriction labels (`STR# 160[17..=19]`): dog-only, cat-only, and
        // both-species Pets Only must all be reachable without shadowing each other.
        let pet_flags = (1 << 4) | (1 << 6);
        let dog_ttab = build_ttab(&[(1, pet_flags | (1 << 9), -1)]);
        let iff_dog = Iff::parse(build_iff(&[(b"TTAB", 128, &dog_ttab)])).unwrap();
        assert_eq!(
            read_ttab_usage(&iff_dog, 128),
            (Some(UsageRestriction::DogsOnly), false)
        );
        let cat_ttab = build_ttab(&[(1, pet_flags | (1 << 10), -1)]);
        let iff_cat = Iff::parse(build_iff(&[(b"TTAB", 128, &cat_ttab)])).unwrap();
        assert_eq!(
            read_ttab_usage(&iff_cat, 128),
            (Some(UsageRestriction::CatsOnly), false)
        );
        let both_pets_ttab = build_ttab(&[(1, pet_flags | (1 << 9) | (1 << 10), -1)]);
        let iff_pets = Iff::parse(build_iff(&[(b"TTAB", 128, &both_pets_ttab)])).unwrap();
        assert_eq!(
            read_ttab_usage(&iff_pets, 128),
            (Some(UsageRestriction::PetsOnly), false)
        );
    }
}
