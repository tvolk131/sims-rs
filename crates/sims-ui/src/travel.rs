//! Complete Collection 1.91 travel destination chrome for **Downtown**,
//! **Vacation Island**, **Studio Town**, and **Magic Town**.
//!
//! Every element drawn here has a native source in `Sims.exe` 1.91
//! (`cWinDowntown`, `cWinVacation`, `cWinStudiotown`, `cWinMagicland`); see
//! `docs/decisions/20261123-travel-chrome-native-audit.md`.
//!
//! Each destination constructor takes one flag byte (`cWinVacation` stores it
//! at `0x5687e8`). The neighborhood shell passes `1` (`0x4f5d7d`, `0x4f5f2c`,
//! `0x4f60db`, `0x4f628a`); lot-side lot pickers pass `0` (`0x441503`,
//! `0x441e28`, `0x44201b`, `0x4421ab`, `0x4443e8`, `0x44452f`, `0x4445a2`,
//! `0x444615`).
//!
//! - **Flag `1` (map browse, bulldoze, visit from the map):** the
//!   `banner_downtown.bmp` (`kDTBarBkg` `5422`) bar with the class's button
//!   table, drawn at the shared native position table (`0x6f53b0`,
//!   `0x6f5750`, `0x6f5f58`, `0x6f62a0`): Sims logo `(5, 0)`, Return
//!   `(323, 0)`, Bulldoze `(373, 0)`, Import `(422, 0)`, Sims.com `(762, 0)`,
//!   plus Magic Town's Move In `(200, 0)`. There is no filter row, no
//!   destination switcher, and no status text on this bar.
//! - **Flag `0` (lot-side picker):** `Downtown/dlgframe2.bmp` (`5432`) with a
//!   random `STR# 212`/`213` title and a 7-button filter row at `x = 225 +
//!   50·i, y = 20` (Studio Town: `175 + 50·i`). Only Magic Town's picker is
//!   pixel-checked against the oracle, so only Magic Town uses it here.
//!
//! All localized tooltips and dialog labels are loaded from `GameData/UIText.iff`;
//! no verbatim game prose is embedded in source.

use crate::art::Art;
use crate::dialog::{BACKGROUND as DIALOG_BACKGROUND, BUTTON as DIALOG_BUTTON, draw_nine_slice};
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::neighborhood::ExpansionFlags;
use crate::{Error, Result};
use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// `GameData/UIText.iff` string table IDs for the four expansion screens,
/// filter bar, and travel confirmation dialogs.
pub mod str_id {
    /// `ObjDialogs` default button strings (`OK`, `Cancel`, `Yes`, `No`).
    pub const DIALOG_BUTTONS: u16 = 142;
    /// `Downtown Screen Tooltips` (5 entries, `0x54a9cd`).
    pub const DOWNTOWN_TIPS: u16 = 169;
    /// `Vacation Island Screen Tooltips` (5 entries, `0x5693a5`).
    pub const VACATION_TIPS: u16 = 170;
    /// `Filter Bar` tooltips (13 entries).
    pub const FILTER_BAR: u16 = 171;
    /// `Studio Town Screen Tooltips` (5 entries, `0x564e9f`).
    pub const STUDIO_TOWN_TIPS: u16 = 173;
    /// `Magic land Screen Tooltips` (6 entries, `0x551c48`).
    pub const MAGIC_TOWN_TIPS: u16 = 174;
    /// `Go Downtown Dialog Strings` (3 entries).
    pub const GO_DOWNTOWN_DIALOG: u16 = 212;
    /// `Vacation Dialog Strings` (3 entries).
    pub const GO_VACATION_DIALOG: u16 = 213;
    /// `Visit Mode Neighborhood Strings` (2 entries).
    pub const VISIT_NEIGHBORHOOD: u16 = 260;
    /// `Visit Mode Magictown Strings` (2 entries).
    pub const VISIT_MAGIC_TOWN: u16 = 261;
}

/// `UIGraphics/Res_Nbhd.h` resource identifiers for travel destination bars,
/// filter toolbars, and travel-confirm dialogs.
pub mod art_id {
    /// `NghUI/banner_downtown.bmp` (`kDTBarBkg`, 800×52): the bar backdrop of
    /// all four destination screens (`0x54a4b8`, `0x551810`, `0x564a83`,
    /// `0x568e87`).
    pub const DOWNTOWN_BAR_BKG: i32 = 5422;
    /// `NghUI/banner_neighborhood.bmp` (800×52, Old Town picker bar).
    pub const NEIGHBORHOOD_BAR_BKG: i32 = 5420;
    /// `Downtown/Return.bmp` (`kDowntownExitToNgh`): entry 0 of every
    /// destination button table, Studio Town included (`0x6ce200`).
    pub const DOWNTOWN_RETURN: i32 = 5209;
    /// `Downtown/TheSimsLogo.bmp` (`kDowntownCredits`): entry 2 of every
    /// destination button table, one cell. Non-US languages swap in
    /// `kNghSimsLogo*` (`0x54a561..0x54a591`, `0x568f30..0x568f60`).
    pub const DOWNTOWN_LOGO: i32 = 5210;
    /// `Downtown/Bulldoze.bmp` (`kDowntownBulldoze`): entry 4 of every table.
    pub const DOWNTOWN_BULLDOZE: i32 = 5211;
    /// `Downtown/Import.bmp` (`kDowntownImport`).
    pub const DOWNTOWN_IMPORT: i32 = 5212;
    /// `VIsland/Import.bmp` (`kVacationImport`).
    pub const VACATION_IMPORT: i32 = 5214;
    /// `Studiotown/Import.bmp` (`kStudiotownImport`).
    pub const STUDIO_TOWN_IMPORT: i32 = 5218;
    /// `Magicland/Import.bmp` (`kMagictownImport`).
    pub const MAGIC_TOWN_IMPORT: i32 = 5222;
    /// `kNghInet` (Sims.com button): entry 1 of every destination table.
    pub const INET: i32 = 5306;
    /// `kNghMoveIn`: entry 5 of the Magic Town table only (`0x6cdcb0`).
    pub const MOVE_IN: i32 = 5200;
    /// Travel dialog phone / hole icons (`Res_Nbhd.h`).
    pub const PHONE_DOWNTOWN: i32 = 5426;
    pub const PHONE_VACATION: i32 = 5431;
    pub const PHONE_NEIGHBORHOOD: i32 = 5433;
    pub const PHONE_STUDIO_TOWN: i32 = 5434;
    pub const PHONE_MAGIC_TOWN: i32 = 5435;
    pub const MAGIC_HOLE_ICON: i32 = 5436;
    /// Dialog frames (`Downtown/dlgframe.bmp`, `Downtown/dlgframe2.bmp`).
    pub const DIALOG_FRAME: i32 = 5423;
    pub const DIALOG_FRAME_ALT: i32 = 5432;
    /// Community filter toggle buttons (`NghUI/Filter*Btn.bmp`).
    pub const FILTER_LODGING: i32 = 5060;
    pub const FILTER_FOOD: i32 = 5061;
    pub const FILTER_GARDENING: i32 = 5062;
    pub const FILTER_SHOPPING: i32 = 5063;
    pub const FILTER_DOG_CAT: i32 = 5064;
    pub const FILTER_SMALL_ANIMAL: i32 = 5065;
    pub const FILTER_RECREATION: i32 = 5066;
    pub const FILTER_SPA: i32 = 5067;
    pub const FILTER_MOVIE: i32 = 5068;
    pub const FILTER_MODEL: i32 = 5069;
    pub const FILTER_MUSIC: i32 = 5072;
    pub const FILTER_MAGIC: i32 = 5075;
    pub const FILTER_RIDES: i32 = 5076;
}

/// Every `UIGraphics` resource ID used by the travel destination chrome.
pub const RESOURCES: [i32; 33] = [
    DIALOG_BACKGROUND,
    DIALOG_BUTTON,
    art_id::DOWNTOWN_BAR_BKG,
    art_id::NEIGHBORHOOD_BAR_BKG,
    art_id::DOWNTOWN_RETURN,
    art_id::DOWNTOWN_LOGO,
    art_id::DOWNTOWN_BULLDOZE,
    art_id::DOWNTOWN_IMPORT,
    art_id::VACATION_IMPORT,
    art_id::STUDIO_TOWN_IMPORT,
    art_id::MAGIC_TOWN_IMPORT,
    art_id::INET,
    art_id::MOVE_IN,
    art_id::PHONE_DOWNTOWN,
    art_id::PHONE_VACATION,
    art_id::PHONE_NEIGHBORHOOD,
    art_id::PHONE_STUDIO_TOWN,
    art_id::PHONE_MAGIC_TOWN,
    art_id::MAGIC_HOLE_ICON,
    art_id::DIALOG_FRAME_ALT,
    art_id::FILTER_LODGING,
    art_id::FILTER_FOOD,
    art_id::FILTER_GARDENING,
    art_id::FILTER_SHOPPING,
    art_id::FILTER_DOG_CAT,
    art_id::FILTER_SMALL_ANIMAL,
    art_id::FILTER_RECREATION,
    art_id::FILTER_SPA,
    art_id::FILTER_MOVIE,
    art_id::FILTER_MODEL,
    art_id::FILTER_MUSIC,
    art_id::FILTER_MAGIC,
    art_id::FILTER_RIDES,
];

/// Native top control bar dimensions (`800×52`).
pub const NATIVE_WIDTH: i32 = 800;
pub const BAR_HEIGHT: i32 = 52;
/// Native destination map height. Viewports taller than this center the
/// `800×600` map and its chrome (`0x556db7..0x556ddc`: origin `[112, 84]` at
/// `1024×768`).
pub const NATIVE_HEIGHT: i32 = 600;
/// Title header height of the Magic Town picker dialog frame
/// (`kDTDlgBkg2` `5432`, `Downtown/dlgframe2.bmp`), measured against the 1.91
/// oracle (`flow-magic-hole-picker.png`): rows `0..62` are the header and the
/// transparent map hole starts at row `62`.
pub const DIALOG_HEADER_HEIGHT: i32 = 62;
/// Map hole inside `dlgframe2.bmp` relative to the map origin (columns
/// `15..789`, rows `62..585`).
pub const DIALOG_MAP_HOLE: Rect = Rect::new(15, DIALOG_HEADER_HEIGHT, 774, 523);
/// Width of one `NghUI/Filter*Btn.bmp` cell (`200×52`, four states).
const PICKER_FILTER_CELL: i32 = 50;
/// Top of the picker dialog's filter row relative to the map origin
/// (position table `0x6f54d0`: `y = 20`).
const PICKER_FILTER_Y: i32 = 20;
/// Native cell size of the bar buttons (`212 / 4 × 52`).
const BAR_BUTTON_W: i32 = 53;
const BAR_BUTTON_H: i32 = 52;
/// Native `kNghInet` cell width (`156 / 4`). At `x = 762` its last column
/// falls past the 800 px bar, so the hit rect is clipped to the bar.
const INET_W: i32 = 39;
/// Width of the one-cell Sims logo (`Downtown/TheSimsLogo.bmp`, `70×52`).
const LOGO_W: i32 = 70;

/// Loads all travel chrome bitmaps and UI fonts from `ui`.
pub fn load_art(ui: &UiGraphics, language: u8) -> Result<Art> {
    Art::load(ui, language, &RESOURCES)
}

/// Bar fill used only when `banner_downtown.bmp` is missing or the viewport
/// is narrower than the native bar.
const BAR_FALLBACK_BG: [u8; 3] = [26, 30, 74];
const BAR_FALLBACK_BORDER: [u8; 3] = [76, 110, 184];
const CHIP_BG: [u8; 3] = [30, 44, 96];
const CHIP_ACTIVE_BG: [u8; 3] = [66, 132, 218];
const CHIP_DISABLED_BG: [u8; 3] = [28, 32, 48];
const TEXT_LIGHT: [u8; 3] = [232, 240, 255];
const TEXT_GOLD: [u8; 3] = [255, 224, 112];
const TEXT_MUTED: [u8; 3] = [148, 158, 182];
const DIALOG_BG: [u8; 3] = [22, 32, 82];
const DIALOG_BORDER: [u8; 3] = [110, 158, 242];
const TOOLTIP_BG: [u8; 3] = [255, 255, 230];
const TOOLTIP_BORDER: [u8; 3] = [0, 0, 0];
const TOOLTIP_TEXT: [u8; 3] = [0, 0, 0];

/// The Complete Collection travel / sub-neighborhood destinations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Destination {
    /// Hot Date Downtown (lots `21..=30`).
    Downtown,
    /// Vacation Island (lots `40..=48`).
    VacationIsland,
    /// Unleashed Old Town community lots (`58..=68` / `zoning == 2`).
    OldTown,
    /// Superstar Studio Town (lots `81..=89`).
    StudioTown,
    /// Makin' Magic Magic Town (lots `90..=98`).
    MagicTown,
}

impl Destination {
    /// The four standalone sub-neighborhood maps in expansion order.
    pub const ALL: [Self; 4] = [
        Self::Downtown,
        Self::VacationIsland,
        Self::StudioTown,
        Self::MagicTown,
    ];

    /// Native lot ID range `(min, max)` inclusive for this destination map.
    pub const fn lot_range(self) -> (u16, u16) {
        match self {
            Self::Downtown => (21, 30),
            Self::VacationIsland => (40, 48),
            Self::OldTown => (50, 79),
            Self::StudioTown => (81, 89),
            Self::MagicTown => (90, 98),
        }
    }

    /// Expansion bit required in [`ExpansionFlags`].
    pub const fn expansion_bit(self) -> u8 {
        match self {
            Self::Downtown => ExpansionFlags::HOT_DATE,
            Self::VacationIsland => ExpansionFlags::VACATION,
            Self::OldTown => ExpansionFlags::UNLEASHED,
            Self::StudioTown => ExpansionFlags::SUPERSTAR,
            Self::MagicTown => ExpansionFlags::MAKIN_MAGIC,
        }
    }

    /// Short structural code for compact labels when `UIText.iff` is absent.
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Downtown => "Downtown",
            Self::VacationIsland => "Vacation",
            Self::OldTown => "Old Town",
            Self::StudioTown => "Studio",
            Self::MagicTown => "Magic",
        }
    }

    /// Default transport mode used when traveling from a home lot.
    pub const fn default_transport(self) -> TravelTransport {
        match self {
            Self::Downtown => TravelTransport::Taxi,
            Self::VacationIsland | Self::StudioTown => TravelTransport::Shuttle,
            Self::OldTown => TravelTransport::Trolley,
            Self::MagicTown => TravelTransport::MagicHole,
        }
    }

    /// The native bar buttons of this destination's map screen with their
    /// `800×600` draw origins, in button-table order (`0x6cd798` Downtown,
    /// `0x6ce368` Vacation Island, `0x6ce200` Studio Town, `0x6cdc9c` Magic
    /// Town; positions `0x6f53b0`, `0x6f62a0`, `0x6f5f58`, `0x6f5750`). The
    /// table index is also the screen's tooltip index.
    pub const fn bar_buttons(self) -> &'static [(TravelControl, [i32; 2])] {
        const COMMON: [(TravelControl, [i32; 2]); 5] = [
            (TravelControl::ReturnHome, [323, 0]),
            (TravelControl::Web, [762, 0]),
            (TravelControl::Credits, [5, 0]),
            (TravelControl::ImportLot, [422, 0]),
            (TravelControl::Bulldoze, [373, 0]),
        ];
        const MAGIC: [(TravelControl, [i32; 2]); 6] = [
            COMMON[0],
            COMMON[1],
            COMMON[2],
            COMMON[3],
            COMMON[4],
            (TravelControl::MoveIn, [200, 0]),
        ];
        match self {
            Self::MagicTown => &MAGIC,
            _ => &COMMON,
        }
    }

    /// Filter buttons shown in the destination picker dialog header, in the
    /// native left-to-right order. Only Magic Town uses the dialog header; the
    /// order is measured against the 1.91 oracle (`flow-magic-hole-picker.png`)
    /// and matches `cWinMagicland`'s filter setup (`0x555372..0x55567c`).
    pub const fn picker_filters(self) -> &'static [LotFilter] {
        match self {
            Self::MagicTown => &[
                LotFilter::Rides,
                LotFilter::SmallAnimal,
                LotFilter::Shopping,
                LotFilter::Gardening,
                LotFilter::Food,
                LotFilter::Recreation,
                LotFilter::Magic,
            ],
            _ => &[],
        }
    }

    /// Whether this destination's lot picker uses the full `dlgframe2.bmp`
    /// dialog frame with a title header (Magic Town) instead of the native
    /// map bar (Downtown and the rest).
    pub const fn picker_uses_dialog_frame(self) -> bool {
        matches!(self, Self::MagicTown)
    }

    /// `UIGraphics` resource ID for this destination's Import button (entry
    /// 3 of its button table).
    pub const fn import_art_id(self) -> i32 {
        match self {
            Self::Downtown | Self::OldTown => art_id::DOWNTOWN_IMPORT,
            Self::VacationIsland => art_id::VACATION_IMPORT,
            Self::StudioTown => art_id::STUDIO_TOWN_IMPORT,
            Self::MagicTown => art_id::MAGIC_TOWN_IMPORT,
        }
    }
}

/// Community / sub-neighborhood filter categories (`Res_Nbhd.h` `5060..=5076`,
/// `STR# 171`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LotFilter {
    Lodging,
    Food,
    Gardening,
    Shopping,
    DogCat,
    SmallAnimal,
    Recreation,
    Spa,
    Movie,
    Model,
    Music,
    Magic,
    Rides,
}

impl LotFilter {
    /// `UIGraphics` resource ID for this filter button (`5060..=5076`).
    pub const fn art_id(self) -> i32 {
        match self {
            Self::Lodging => art_id::FILTER_LODGING,
            Self::Food => art_id::FILTER_FOOD,
            Self::Gardening => art_id::FILTER_GARDENING,
            Self::Shopping => art_id::FILTER_SHOPPING,
            Self::DogCat => art_id::FILTER_DOG_CAT,
            Self::SmallAnimal => art_id::FILTER_SMALL_ANIMAL,
            Self::Recreation => art_id::FILTER_RECREATION,
            Self::Spa => art_id::FILTER_SPA,
            Self::Movie => art_id::FILTER_MOVIE,
            Self::Model => art_id::FILTER_MODEL,
            Self::Music => art_id::FILTER_MUSIC,
            Self::Magic => art_id::FILTER_MAGIC,
            Self::Rides => art_id::FILTER_RIDES,
        }
    }

    /// 0-based index into `UIText.iff STR# 171` (`Filter Bar`).
    pub const fn tooltip_index(self) -> usize {
        match self {
            Self::DogCat => 0,
            Self::SmallAnimal => 1,
            Self::Shopping => 2,
            Self::Gardening => 3,
            Self::Food => 4,
            Self::Recreation => 5,
            Self::Lodging => 6,
            Self::Spa => 7,
            Self::Movie => 8,
            Self::Model => 9,
            Self::Music => 10,
            Self::Magic => 11,
            Self::Rides => 12,
        }
    }
}

/// Active interaction mode on a travel destination map screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TravelBarMode {
    /// Browsing lots (or picking a lot to edit in Build/Buy).
    #[default]
    Browse,
    /// Picking a lot to visit with an active household (disables Bulldoze /
    /// Import / Move In).
    Visit,
    /// Bulldozing / resetting a community lot back to template.
    Bulldoze,
    /// Placing a family from the bin on a Magic Town lot: Move In is held
    /// down (`0x55478a`).
    MoveIn,
}

/// Read-only state rendered by [`TravelChrome`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TravelState {
    pub destination: Destination,
    pub mode: TravelBarMode,
    pub expansions: ExpansionFlags,
    /// Whether the import folder holds a lot to import: the native Import
    /// button is enabled only then (`0x56b870`).
    pub has_importable_lot: bool,
    /// Active community filter highlight in the picker dialog (if any).
    pub active_filter: Option<LotFilter>,
    /// Filters with no matching lot on the current map. The picker dialog
    /// draws these with the disabled cell and ignores clicks on them.
    pub unavailable_filters: Vec<LotFilter>,
}

impl Default for TravelState {
    fn default() -> Self {
        Self {
            destination: Destination::Downtown,
            mode: TravelBarMode::Browse,
            expansions: ExpansionFlags::ALL,
            has_importable_lot: false,
            active_filter: None,
            unavailable_filters: Vec::new(),
        }
    }
}

/// Interactive controls on the destination map bar and picker dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TravelControl {
    /// `kDowntownExitToNgh` (`5209`).
    ReturnHome,
    /// `kNghInet` (`5306`).
    Web,
    /// `kDowntownCredits` (`5210`, the Sims logo).
    Credits,
    /// Destination Import button (`5212`, `5214`, `5218`, `5222`).
    ImportLot,
    /// `kDowntownBulldoze` (`5211`).
    Bulldoze,
    /// `kNghMoveIn` (`5200`), Magic Town only.
    MoveIn,
    /// Picker dialog filter button.
    ToggleFilter(LotFilter),
}

impl TravelControl {
    /// Index into this destination's screen tooltip table (`STR# 169`,
    /// `170`, `173`, `174`), which follows button-table order.
    pub const fn screen_tooltip_index(self) -> Option<usize> {
        match self {
            Self::ReturnHome => Some(0),
            Self::Web => Some(1),
            Self::Credits => Some(2),
            Self::ImportLot => Some(3),
            Self::Bulldoze => Some(4),
            Self::MoveIn => Some(5),
            Self::ToggleFilter(_) => None,
        }
    }
}

/// Commands emitted when the player clicks a control in [`TravelChrome`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TravelCommand {
    ReturnToNeighborhood,
    ToggleBulldoze,
    ImportLot,
    OpenWeb,
    OpenCredits,
    /// Magic Town's Move In button (opens the family bin).
    OpenFamilyBin,
    SetFilter(Option<LotFilter>),
}

/// Localized strings read from `GameData/UIText.iff` for travel screens and
/// travel-confirm dialogs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TravelStrings {
    pub downtown_tips: Vec<String>,
    pub vacation_tips: Vec<String>,
    pub studio_town_tips: Vec<String>,
    pub magic_town_tips: Vec<String>,
    pub filter_tips: Vec<String>,
    pub go_downtown_dialog: Vec<String>,
    pub go_vacation_dialog: Vec<String>,
    pub visit_neighborhood: Vec<String>,
    pub visit_magic_town: Vec<String>,
    /// `UIText.iff STR# 142` default dialog button labels (`OK`, `Cancel`, `Yes`, `No`).
    pub dialog_buttons: Vec<String>,
}

impl TravelStrings {
    /// Loads all travel-related `STR#` tables from `GameData/UIText.iff`.
    pub fn load(install: &Install, language: u8) -> Result<Self> {
        let path = install
            .file("GameData/UIText.iff")
            .map_err(Error::Content)?;
        let bytes = std::fs::read(&path).map_err(Error::Io)?;
        Self::from_iff_bytes(bytes, language)
    }

    /// Parses all travel-related `STR#` tables from raw `UIText.iff` bytes.
    pub fn from_iff_bytes(bytes: Vec<u8>, language: u8) -> Result<Self> {
        let iff = Iff::parse(bytes).map_err(Error::Format)?;
        let read_table = |id: u16| -> Result<Vec<String>> {
            let Some(chunk) = iff.get(b"STR#", id) else {
                return Ok(Vec::new());
            };
            let table = StringTable::parse(iff.data(chunk)).map_err(Error::Format)?;
            Ok((0..table.entries.len())
                .filter_map(|i| {
                    table
                        .get(i, language)
                        .or_else(|| table.get(i, 1))
                        .map(str::to_owned)
                })
                .collect())
        };
        Ok(Self {
            downtown_tips: read_table(str_id::DOWNTOWN_TIPS)?,
            vacation_tips: read_table(str_id::VACATION_TIPS)?,
            studio_town_tips: read_table(str_id::STUDIO_TOWN_TIPS)?,
            magic_town_tips: read_table(str_id::MAGIC_TOWN_TIPS)?,
            filter_tips: read_table(str_id::FILTER_BAR)?,
            go_downtown_dialog: read_table(str_id::GO_DOWNTOWN_DIALOG)?,
            go_vacation_dialog: read_table(str_id::GO_VACATION_DIALOG)?,
            visit_neighborhood: read_table(str_id::VISIT_NEIGHBORHOOD)?,
            visit_magic_town: read_table(str_id::VISIT_MAGIC_TOWN)?,
            dialog_buttons: read_table(str_id::DIALOG_BUTTONS)?,
        })
    }

    /// Picker dialog title for `destination` (`UIText.iff STR# 212[2]`,
    /// "Where am I taking you?" for the Magic Town hole). The native picks
    /// one of `STR# 212[0..3]` at random (`0x551ded..0x551e0e`); the 1.91
    /// oracle caught entry 2.
    pub fn picker_title(&self, destination: Destination) -> Option<&str> {
        match destination {
            Destination::MagicTown => self.go_downtown_dialog.get(2),
            _ => None,
        }
        .map(String::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    }

    /// Localized `OK` button label from `UIText.iff STR# 142[0]`.
    pub fn dialog_ok(&self) -> &str {
        self.dialog_buttons
            .first()
            .map(String::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("OK")
    }

    /// Localized `Cancel` button label from `UIText.iff STR# 142[1]`.
    pub fn dialog_cancel(&self) -> &str {
        self.dialog_buttons
            .get(1)
            .map(String::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Cancel")
    }

    /// Localized `Yes` button label from `UIText.iff STR# 142[2]`.
    pub fn dialog_yes(&self) -> &str {
        self.dialog_buttons
            .get(2)
            .map(String::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Yes")
    }

    /// Localized `No` button label from `UIText.iff STR# 142[3]`.
    pub fn dialog_no(&self) -> &str {
        self.dialog_buttons
            .get(3)
            .map(String::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("No")
    }

    /// Tooltip string for `control` on `destination`.
    pub fn tooltip(&self, destination: Destination, control: TravelControl) -> Option<&str> {
        let screen_tips = match destination {
            Destination::Downtown | Destination::OldTown => &self.downtown_tips,
            Destination::VacationIsland => &self.vacation_tips,
            Destination::StudioTown => &self.studio_town_tips,
            Destination::MagicTown => &self.magic_town_tips,
        };
        match control {
            TravelControl::ToggleFilter(f) => {
                self.filter_tips.get(f.tooltip_index()).map(String::as_str)
            }
            other => other
                .screen_tooltip_index()
                .and_then(|i| screen_tips.get(i))
                .map(String::as_str),
        }
        .filter(|s| !s.is_empty())
    }
}

/// A laid-out interactive control with non-overlapping hit rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPlacement {
    pub control: TravelControl,
    /// Where the control's sprite is drawn (its native origin).
    pub origin: [i32; 2],
    /// Hit rectangle: the part of the sprite left visible after later
    /// controls draw over it, clipped to the bar.
    pub rect: Rect,
    pub enabled: bool,
    pub active: bool,
}

/// Computed layout for the travel destination bar or picker dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TravelLayout {
    pub frame_size: [i32; 2],
    /// Top-left of the native `800×600` map area (`[112, 84]` at `1024×768`).
    pub origin: [i32; 2],
    pub bar_rect: Rect,
    pub controls: Vec<ControlPlacement>,
    pub compact: bool,
    /// Picker dialog frame (`dlgframe2.bmp`, `800×600` at `origin`), if used.
    pub dialog_rect: Option<Rect>,
    /// Transparent map hole inside [`Self::dialog_rect`].
    pub map_hole: Option<Rect>,
    /// Area for the picker dialog title text.
    pub title_rect: Option<Rect>,
}

impl TravelLayout {
    /// Whether `(x, y)` lies on chrome that should swallow map clicks.
    pub fn blocks_map(&self, x: i32, y: i32) -> bool {
        if self.bar_rect.contains(x, y) {
            return true;
        }
        if let (Some(dialog), Some(hole)) = (self.dialog_rect, self.map_hole) {
            return dialog.contains(x, y) && !hole.contains(x, y);
        }
        self.controls.iter().any(|c| c.rect.contains(x, y))
    }

    /// Magic Town lot-picker layout (`TravelBarMode::Visit` on viewports at
    /// least `800` wide): the `dlgframe2.bmp` dialog with a title and filter
    /// row. The native dialog has no visible cancel control; Esc backs out.
    fn compute_picker_dialog(w: i32, h: i32, origin: [i32; 2], state: &TravelState) -> Self {
        let [ox, oy] = origin;
        let dialog = Rect::new(ox, oy, NATIVE_WIDTH, NATIVE_HEIGHT.min(h - oy));
        let header_h = DIALOG_HEADER_HEIGHT.min(dialog.height);
        let bar_rect = Rect::new(ox, oy, NATIVE_WIDTH, header_h);
        let filters = state.destination.picker_filters();
        let count = filters.len() as i32;
        let start = ox + (NATIVE_WIDTH - count * PICKER_FILTER_CELL) / 2;
        let controls = filters
            .iter()
            .enumerate()
            .map(|(i, &filter)| ControlPlacement {
                control: TravelControl::ToggleFilter(filter),
                origin: [start + i as i32 * PICKER_FILTER_CELL, oy + PICKER_FILTER_Y],
                rect: Rect::new(
                    start + i as i32 * PICKER_FILTER_CELL,
                    oy + PICKER_FILTER_Y,
                    PICKER_FILTER_CELL,
                    BAR_HEIGHT,
                ),
                enabled: !state.unavailable_filters.contains(&filter),
                active: state.active_filter == Some(filter),
            })
            .collect();
        Self {
            frame_size: [w, h],
            origin,
            bar_rect,
            controls,
            compact: false,
            dialog_rect: Some(dialog),
            map_hole: Some(DIALOG_MAP_HOLE.offset(ox, oy)),
            title_rect: Some(Rect::new(ox, oy + 2, NATIVE_WIDTH, PICKER_FILTER_Y)),
        }
    }

    /// Computes pairwise-disjoint control placements for `width` × `height`.
    pub fn compute(width: u32, height: u32, state: &TravelState) -> Self {
        let w = i32::try_from(width).unwrap_or(i32::MAX).max(0);
        let h = i32::try_from(height).unwrap_or(i32::MAX).max(0);
        let compact = w < NATIVE_WIDTH;

        let bar_w = if compact { w } else { NATIVE_WIDTH };
        let bar_x = if compact { 0 } else { (w - bar_w) / 2 };
        let oy = if !compact && h > NATIVE_HEIGHT {
            (h - NATIVE_HEIGHT) / 2
        } else {
            0
        };
        if !compact
            && state.mode == TravelBarMode::Visit
            && state.destination.picker_uses_dialog_frame()
        {
            return Self::compute_picker_dialog(w, h, [bar_x, oy], state);
        }
        let bar_h = BAR_HEIGHT.min(h - oy).max(0);
        let bar_rect = Rect::new(bar_x, oy, bar_w, bar_h);

        // Enable rules from `cWinVacation`'s mode switch (`0x56b19e`): in
        // Bulldoze mode Return / Sims.com / logo / Import are disabled and
        // Bulldoze is held down; Import otherwise needs an importable lot
        // (`0x56b870`). Visit mode (lot picking from a trip) keeps only
        // Return live. Move In is held down while a family is placed.
        let mode = state.mode;
        let enabled = |control: TravelControl| match (control, mode) {
            (TravelControl::ReturnHome, TravelBarMode::Visit) => true,
            (_, TravelBarMode::Visit) => false,
            (TravelControl::Bulldoze, _) => true,
            (TravelControl::ImportLot, TravelBarMode::Browse) => state.has_importable_lot,
            (TravelControl::ImportLot, _) => false,
            (TravelControl::MoveIn, TravelBarMode::Browse | TravelBarMode::MoveIn) => true,
            (TravelControl::MoveIn, _) => false,
            (_, TravelBarMode::Bulldoze) => false,
            _ => true,
        };
        let active = |control: TravelControl| {
            matches!(
                (control, mode),
                (TravelControl::Bulldoze, TravelBarMode::Bulldoze)
                    | (TravelControl::MoveIn, TravelBarMode::MoveIn)
            )
        };

        let buttons = state.destination.bar_buttons();
        let mut controls = Vec::with_capacity(buttons.len());
        if compact {
            // Narrower than the native bar: keep the native left-to-right
            // order (logo first, Sims.com last) packed edge to edge at the
            // full sprite width, dropping what does not fit.
            let mut ordered: Vec<_> = buttons.to_vec();
            ordered.sort_by_key(|&(_, [x, _])| x);
            let mut x = bar_x;
            for (control, _) in ordered {
                let cw = control_sprite_width(control);
                if x + cw > bar_x + bar_w {
                    break;
                }
                controls.push(ControlPlacement {
                    control,
                    origin: [x, oy],
                    rect: Rect::new(x, oy, cw, bar_h),
                    enabled: enabled(control),
                    active: active(control),
                });
                x += cw;
            }
        } else {
            for &(control, [x, y]) in buttons {
                controls.push(ControlPlacement {
                    control,
                    origin: [bar_x + x, oy + y],
                    rect: Rect::new(
                        bar_x + x,
                        oy + y,
                        control_sprite_width(control).min(NATIVE_WIDTH - x),
                        BAR_BUTTON_H.min(bar_h),
                    ),
                    enabled: enabled(control),
                    active: active(control),
                });
            }
            // Controls draw in table order, so a later sprite covers the
            // edge of an earlier one (Bulldoze at 373..426 over Return's
            // right edge and Import's left edge). Trim each hit rect to its
            // visible span so a click lands on the button it shows.
            for i in 0..controls.len() {
                for j in i + 1..controls.len() {
                    let top = controls[j].rect;
                    let r = &mut controls[i].rect;
                    let (left, right) = (r.x, r.x + r.width);
                    let (tl, tr) = (top.x, top.x + top.width);
                    if tl >= right || tr <= left {
                        continue;
                    }
                    if tl > left {
                        r.width = tl - left;
                    } else if tr < right {
                        r.x = tr;
                        r.width = right - tr;
                    } else {
                        r.width = 0;
                    }
                }
            }
        }

        Self {
            frame_size: [w, h],
            origin: [bar_x, oy],
            bar_rect,
            controls,
            compact,
            dialog_rect: None,
            map_hole: None,
            title_rect: None,
        }
    }

    /// Returns the control whose hit rectangle contains `(x, y)`.
    pub fn hit(&self, x: i32, y: i32) -> Option<ControlPlacement> {
        self.controls
            .iter()
            .copied()
            .find(|c| c.rect.contains(x, y))
    }
}

const fn control_sprite_width(control: TravelControl) -> i32 {
    match control {
        TravelControl::Web => INET_W,
        TravelControl::Credits => LOGO_W,
        TravelControl::ToggleFilter(_) => PICKER_FILTER_CELL,
        _ => BAR_BUTTON_W,
    }
}

/// Interactive controller for the travel destination bar and picker dialog.
#[derive(Debug, Clone, Default)]
pub struct TravelChrome {
    hovered: Option<TravelControl>,
    pressed: Option<TravelControl>,
    pointer: [i32; 2],
}

impl TravelChrome {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn hovered(&self) -> Option<TravelControl> {
        self.hovered
    }

    pub fn pressed(&self) -> Option<TravelControl> {
        self.pressed
    }

    /// Clears transient hover and press state (e.g. when the pointer leaves
    /// the window or releases outside).
    pub fn clear_pointer(&mut self) -> bool {
        let had = self.hovered.is_some() || self.pressed.is_some();
        self.hovered = None;
        self.pressed = None;
        had
    }

    /// Updates hover state for a pointer move at `(x, y)`.
    pub fn pointer_move(&mut self, x: i32, y: i32, layout: &TravelLayout) {
        self.pointer = [x, y];
        self.hovered = layout.hit(x, y).map(|c| c.control);
    }

    /// Records a mouse-down press if an enabled control lies under `(x, y)`.
    pub fn pointer_down(&mut self, x: i32, y: i32, layout: &TravelLayout) -> bool {
        self.pointer = [x, y];
        if let Some(hit) = layout.hit(x, y)
            && hit.enabled
        {
            self.pressed = Some(hit.control);
            self.hovered = Some(hit.control);
            return true;
        }
        self.pressed = None;
        false
    }

    /// Completes a click on mouse-up, returning a [`TravelCommand`] when
    /// released on the same enabled control.
    pub fn pointer_up(
        &mut self,
        x: i32,
        y: i32,
        layout: &TravelLayout,
        state: &TravelState,
    ) -> Option<TravelCommand> {
        self.pointer = [x, y];
        let hit = layout.hit(x, y);
        self.hovered = hit.map(|c| c.control);
        let pressed = self.pressed.take()?;
        let hit = hit?;
        if !hit.enabled || hit.control != pressed {
            return None;
        }
        Some(match hit.control {
            TravelControl::ReturnHome => TravelCommand::ReturnToNeighborhood,
            TravelControl::Bulldoze => TravelCommand::ToggleBulldoze,
            TravelControl::ImportLot => TravelCommand::ImportLot,
            TravelControl::Web => TravelCommand::OpenWeb,
            TravelControl::Credits => TravelCommand::OpenCredits,
            TravelControl::MoveIn => TravelCommand::OpenFamilyBin,
            TravelControl::ToggleFilter(filter) => {
                if state.active_filter == Some(filter) {
                    TravelCommand::SetFilter(None)
                } else {
                    TravelCommand::SetFilter(Some(filter))
                }
            }
        })
    }

    /// Draws the travel destination bar (or Magic Town picker dialog) and
    /// hover tooltip into `frame`.
    pub fn draw(
        &self,
        frame: &mut Frame,
        state: &TravelState,
        art: &Art,
        strings: &TravelStrings,
    ) -> TravelLayout {
        let layout = TravelLayout::compute(frame.width(), frame.height(), state);

        if let Some(dialog) = layout.dialog_rect {
            draw_picker_dialog(frame, &layout, dialog, state, art, strings);
        } else if !layout.compact
            && let Some(img) = art.image(art_id::DOWNTOWN_BAR_BKG)
        {
            frame.draw_all(img, [layout.bar_rect.x, layout.bar_rect.y]);
        } else {
            frame.fill(layout.bar_rect, BAR_FALLBACK_BG);
            draw_border(frame, layout.bar_rect, BAR_FALLBACK_BORDER);
        }

        for placement in &layout.controls {
            let is_hovered = self.hovered == Some(placement.control);
            let is_pressed = self.pressed == Some(placement.control);
            let cell = if !placement.enabled {
                3
            } else if is_pressed || placement.active {
                1
            } else if is_hovered {
                2
            } else {
                0
            };
            let sprite = control_art_id(state.destination, placement.control).and_then(|id| {
                if placement.control == TravelControl::Credits {
                    // One-cell bitmap (`0x569445`: cell count 1).
                    art.image(id).cloned()
                } else {
                    art.cell(id, 4, cell)
                }
            });
            match sprite {
                // Native sprites draw at the button's origin (the hit rect's
                // top-left), overhanging the trimmed hit rect.
                Some(sprite)
                    if !layout.compact || sprite.width as i32 <= placement.rect.width + 4 =>
                {
                    // The bar window clips its children: Sims.com at `762`
                    // loses its last column past the 800 px edge.
                    let bar_right = layout.bar_rect.x + layout.bar_rect.width;
                    let [sx, sy] = placement.origin;
                    let visible = (sprite.width as i32).min(bar_right - sx);
                    frame.draw(
                        &sprite,
                        Rect::new(0, 0, visible, sprite.height as i32),
                        [sx, sy],
                    );
                }
                _ => {
                    // Missing art: a plain chip with no invented label.
                    let bg = if !placement.enabled {
                        CHIP_DISABLED_BG
                    } else if placement.active || is_pressed {
                        CHIP_ACTIVE_BG
                    } else {
                        CHIP_BG
                    };
                    let rect = Rect::new(
                        placement.rect.x + 2,
                        placement.rect.y + 2,
                        (placement.rect.width - 4).max(0),
                        (placement.rect.height - 4).max(0),
                    );
                    frame.fill(rect, bg);
                    draw_border(frame, rect, BAR_FALLBACK_BORDER);
                }
            }
        }

        // Hover tooltip.
        if let Some(hovered) = self.hovered
            && let Some(tip) = strings.tooltip(state.destination, hovered)
            && let Some(f) = pick_font(art, 9)
        {
            draw_tooltip(frame, f, self.pointer[0], self.pointer[1] + 18, tip);
        }

        layout
    }
}

/// Draws the Magic Town picker dialog: `dlgframe2.bmp` (`5432`) over the map,
/// its header gap filled, and the `STR# 212` title.
fn draw_picker_dialog(
    frame: &mut Frame,
    layout: &TravelLayout,
    dialog: Rect,
    state: &TravelState,
    art: &Art,
    strings: &TravelStrings,
) {
    if let Some(img) = art.image(art_id::DIALOG_FRAME_ALT) {
        frame.draw_all(img, [dialog.x, dialog.y]);
        fill_dialog_header_gap(frame, img, [dialog.x, dialog.y]);
    } else {
        let bg = BAR_FALLBACK_BG;
        frame.fill(layout.bar_rect, bg);
        draw_border(frame, layout.bar_rect, BAR_FALLBACK_BORDER);
        if let Some(hole) = layout.map_hole {
            let right = hole.x + hole.width;
            let bottom = hole.y + hole.height;
            frame.fill(
                Rect::new(dialog.x, hole.y, hole.x - dialog.x, hole.height),
                bg,
            );
            frame.fill(
                Rect::new(right, hole.y, dialog.x + dialog.width - right, hole.height),
                bg,
            );
            frame.fill(
                Rect::new(
                    dialog.x,
                    bottom,
                    dialog.width,
                    dialog.y + dialog.height - bottom,
                ),
                bg,
            );
        }
    }
    if let (Some(rect), Some(title)) = (layout.title_rect, strings.picker_title(state.destination))
        && let Some(font) = pick_font(art, 12)
    {
        draw_centered_text(frame, font, rect, title, TEXT_LIGHT);
    }
}

/// The native header behind the title and filter row is a horizontal bevel
/// gradient, but `dlgframe2.bmp` leaves those columns transparent. Fill each
/// header row by interpolating between the opaque pixels bordering its gap,
/// which reproduces the oracle header within the ratchet tolerance.
fn fill_dialog_header_gap(frame: &mut Frame, img: &Image, at: [i32; 2]) {
    let rows = (DIALOG_HEADER_HEIGHT as u32).min(img.height);
    for y in 0..rows {
        let transparent = |x: u32| img.pixel(x, y).is_some_and(|p| p[3] == 0);
        let Some(first) = (0..img.width).find(|&x| transparent(x)) else {
            continue;
        };
        let Some(last) = (0..img.width).rev().find(|&x| transparent(x)) else {
            continue;
        };
        if first == 0 || last + 1 >= img.width {
            continue;
        }
        let (Some(left), Some(right)) = (img.pixel(first - 1, y), img.pixel(last + 1, y)) else {
            continue;
        };
        let span = (last + 2 - first) as i32;
        for x in first..=last {
            let t = (x + 1 - first) as i32;
            let mix = |i: usize| {
                let (l, r) = (i32::from(left[i]), i32::from(right[i]));
                (l + (r - l) * t / span) as u8
            };
            frame.fill(
                Rect::new(at[0] + x as i32, at[1] + y as i32, 1, 1),
                [mix(0), mix(1), mix(2)],
            );
        }
    }
}

fn control_art_id(destination: Destination, control: TravelControl) -> Option<i32> {
    Some(match control {
        TravelControl::ReturnHome => art_id::DOWNTOWN_RETURN,
        TravelControl::Web => art_id::INET,
        TravelControl::Credits => art_id::DOWNTOWN_LOGO,
        TravelControl::ImportLot => destination.import_art_id(),
        TravelControl::Bulldoze => art_id::DOWNTOWN_BULLDOZE,
        TravelControl::MoveIn => art_id::MOVE_IN,
        TravelControl::ToggleFilter(filter) => filter.art_id(),
    })
}

/// Transport vehicle / portal shown in the travel confirmation dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TravelTransport {
    /// Hot Date Downtown yellow cab (`Downtown/dtphone.bmp`).
    Taxi,
    /// Vacation Island / Studio Town shuttle or limo.
    Shuttle,
    /// Unleashed Old Town trolley (`NghUI/nbphone.bmp`, `5433`).
    Trolley,
    /// Makin' Magic Hole in the Ground (`Magicland/mtclownhole.bmp`).
    MagicHole,
}

impl TravelTransport {
    /// `UIGraphics` resource ID for the transport icon in the dialog.
    pub const fn icon_art_id(self, destination: Destination) -> i32 {
        match (self, destination) {
            (Self::Taxi, _) => art_id::PHONE_DOWNTOWN,
            (Self::Shuttle, Destination::VacationIsland) => art_id::PHONE_VACATION,
            (Self::Shuttle, Destination::StudioTown) => art_id::PHONE_STUDIO_TOWN,
            (Self::Shuttle, _) | (Self::Trolley, _) => art_id::PHONE_NEIGHBORHOOD,
            (Self::MagicHole, _) => art_id::MAGIC_HOLE_ICON,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Taxi => "Taxi",
            Self::Shuttle => "Shuttle",
            Self::Trolley => "Trolley",
            Self::MagicHole => "Magic Hole",
        }
    }
}

/// Read-only view-model for the Taxi / Shuttle / Magic Hole travel confirmation
/// dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TravelDialogState {
    pub destination: Destination,
    pub transport: TravelTransport,
    /// Names of household members traveling (must be non-empty to confirm).
    pub travelers: Vec<String>,
    /// Trip fare in Simoleons (e.g. $50 cab, $500 Vacation Island shuttle).
    pub fare_simoleons: i32,
    /// Optional fare in MagiCoins.
    pub fare_magic_coins: i32,
    /// Household's current Simoleon balance.
    pub household_simoleons: i32,
    /// Household's current MagiCoins balance.
    pub household_magic_coins: i32,
    /// Optional custom title overriding `UIText.iff` dialog strings.
    pub title_override: Option<String>,
}

impl TravelDialogState {
    /// Creates a standard travel-confirm dialog state for `destination`.
    pub fn for_destination(destination: Destination, travelers: Vec<String>) -> Self {
        let (transport, fare_simoleons) = match destination {
            Destination::Downtown => (TravelTransport::Taxi, 50),
            Destination::VacationIsland => (TravelTransport::Shuttle, 500),
            Destination::OldTown => (TravelTransport::Trolley, 0),
            Destination::StudioTown => (TravelTransport::Shuttle, 0),
            Destination::MagicTown => (TravelTransport::MagicHole, 0),
        };
        Self {
            destination,
            transport,
            travelers,
            fare_simoleons,
            fare_magic_coins: 0,
            household_simoleons: 0,
            household_magic_coins: 0,
            title_override: None,
        }
    }

    /// Whether the household can afford the trip and has at least one non-blank traveler.
    pub fn can_confirm(&self) -> bool {
        self.travelers.iter().any(|t| !t.trim().is_empty())
            && self.household_simoleons >= self.fare_simoleons.max(0)
            && self.household_magic_coins >= self.fare_magic_coins.max(0)
    }
}

/// Commands emitted by [`TravelDialog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TravelDialogCommand {
    Confirm {
        destination: Destination,
        transport: TravelTransport,
    },
    Cancel,
}

/// Layout of the centered travel confirmation dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TravelDialogLayout {
    pub panel_rect: Rect,
    pub icon_rect: Rect,
    pub confirm_rect: Rect,
    pub cancel_rect: Rect,
}

impl TravelDialogLayout {
    pub fn compute(width: u32, height: u32) -> Self {
        let w = i32::try_from(width).unwrap_or(i32::MAX).max(0);
        let h = i32::try_from(height).unwrap_or(i32::MAX).max(0);
        let pw = 360.min((w - 16).max(0));
        let ph = 200.min((h - 16).max(0));
        let px = (w - pw) / 2;
        let py = (h - ph) / 2;
        let panel_rect = Rect::new(px, py, pw, ph);

        let icon_size = 48.min((ph - 60).max(0)).min((pw / 4).max(0));
        let icon_rect = Rect::new(px + 16, py + 42, icon_size, icon_size);

        let btn_h = 28.min((ph / 4).max(0));
        let btn_w = ((pw - 44) / 2).clamp(0, 120);
        let btn_y = py + ph - btn_h - 14;
        let confirm_rect = Rect::new(px + 16, btn_y, btn_w, btn_h);
        let cancel_rect = Rect::new(px + pw - 16 - btn_w, btn_y, btn_w, btn_h);

        Self {
            panel_rect,
            icon_rect,
            confirm_rect,
            cancel_rect,
        }
    }
}

/// Interactive controller for the travel confirmation dialog.
#[derive(Debug, Clone, Default)]
pub struct TravelDialog {
    hovered_confirm: bool,
    hovered_cancel: bool,
    pressed_confirm: bool,
    pressed_cancel: bool,
}

impl TravelDialog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tracks hover state over Confirm and Cancel buttons.
    pub fn pointer_move(&mut self, x: i32, y: i32, layout: &TravelDialogLayout) {
        self.hovered_confirm = layout.confirm_rect.contains(x, y);
        self.hovered_cancel = layout.cancel_rect.contains(x, y);
    }

    /// Handles `Enter` (confirm if affordable) or `Escape` (cancel).
    pub fn key_press(&self, key: &str, state: &TravelDialogState) -> Option<TravelDialogCommand> {
        match key {
            "Enter" | "Return" if state.can_confirm() => Some(TravelDialogCommand::Confirm {
                destination: state.destination,
                transport: state.transport,
            }),
            "Escape" => Some(TravelDialogCommand::Cancel),
            _ => None,
        }
    }

    pub fn pointer_down(
        &mut self,
        x: i32,
        y: i32,
        layout: &TravelDialogLayout,
        state: &TravelDialogState,
    ) -> bool {
        self.pressed_confirm = state.can_confirm() && layout.confirm_rect.contains(x, y);
        self.pressed_cancel = layout.cancel_rect.contains(x, y);
        self.pressed_confirm || self.pressed_cancel
    }

    pub fn pointer_up(
        &mut self,
        x: i32,
        y: i32,
        layout: &TravelDialogLayout,
        state: &TravelDialogState,
    ) -> Option<TravelDialogCommand> {
        let was_confirm = std::mem::take(&mut self.pressed_confirm);
        let was_cancel = std::mem::take(&mut self.pressed_cancel);
        if was_confirm && state.can_confirm() && layout.confirm_rect.contains(x, y) {
            return Some(TravelDialogCommand::Confirm {
                destination: state.destination,
                transport: state.transport,
            });
        }
        if was_cancel && layout.cancel_rect.contains(x, y) {
            return Some(TravelDialogCommand::Cancel);
        }
        None
    }

    /// Draws the travel confirmation dialog into `frame`.
    pub fn draw(
        &self,
        frame: &mut Frame,
        state: &TravelDialogState,
        art: &Art,
        strings: &TravelStrings,
    ) -> TravelDialogLayout {
        let layout = TravelDialogLayout::compute(frame.width(), frame.height());
        if layout.panel_rect.width < 40 || layout.panel_rect.height < 40 {
            return layout;
        }

        // Dim background behind modal dialog, then render native 9-sliced
        // `PopupInfoTiles.bmp` (`3008`) chrome (falling back to solid fill only
        // when UI art is absent).
        fill_alpha(
            frame,
            Rect::new(0, 0, frame.width() as i32, frame.height() as i32),
            [0, 0, 0],
            120,
        );
        if let Some(tiler) = art.image(DIALOG_BACKGROUND)
            && tiler.width >= 3
            && tiler.height >= 3
        {
            draw_nine_slice(frame, tiler, layout.panel_rect);
        } else {
            frame.fill(layout.panel_rect, DIALOG_BG);
            draw_border(frame, layout.panel_rect, DIALOG_BORDER);
        }

        let title = state
            .title_override
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| match state.destination {
                Destination::Downtown | Destination::OldTown => {
                    strings.go_downtown_dialog.first().map(String::as_str)
                }
                Destination::VacationIsland => {
                    strings.go_vacation_dialog.first().map(String::as_str)
                }
                Destination::StudioTown => strings.go_downtown_dialog.get(1).map(String::as_str),
                Destination::MagicTown => strings.go_downtown_dialog.get(2).map(String::as_str),
            })
            .unwrap_or_else(|| state.destination.short_label());

        if let Some(f) = pick_font(art, 11) {
            let title_rect = Rect::new(
                layout.panel_rect.x + 12,
                layout.panel_rect.y + 10,
                (layout.panel_rect.width - 24).max(0),
                22,
            );
            draw_centered_text(frame, f, title_rect, title, TEXT_GOLD);
        }

        // Transport icon fitted cleanly inside `layout.icon_rect` (aspect-preserving).
        let icon_id = state.transport.icon_art_id(state.destination);
        if let Some(icon) = art.image(icon_id) {
            draw_fitted_image(frame, icon, layout.icon_rect);
        } else {
            frame.fill(layout.icon_rect, CHIP_BG);
            draw_border(frame, layout.icon_rect, DIALOG_BORDER);
        }

        // Summary text (transport, traveler count, fare), clipped above buttons.
        if let Some(f) = pick_font(art, 9) {
            let text_x = layout.icon_rect.x + layout.icon_rect.width + 12;
            let text_w = (layout.panel_rect.x + layout.panel_rect.width - 12 - text_x).max(0);
            let max_y =
                (layout.confirm_rect.y - 4).min(layout.panel_rect.y + layout.panel_rect.height - 4);
            if text_w > 20 && f.line_height > 0 {
                let valid_travelers = state
                    .travelers
                    .iter()
                    .filter(|t| !t.trim().is_empty())
                    .count();
                let fare_sim = state.fare_simoleons.max(0);
                let fare_mc = state.fare_magic_coins.max(0);
                let line1 = format!(
                    "{} -> {}",
                    state.transport.label(),
                    state.destination.short_label()
                );
                let line2 = format!("Travelers: {valid_travelers}");
                let line3 = if fare_mc > 0 {
                    format!("Fare: ${fare_sim} / MC {fare_mc}")
                } else {
                    format!("Fare: ${fare_sim}")
                };
                let fare_color = if state.can_confirm() {
                    TEXT_GOLD
                } else {
                    [255, 128, 128]
                };
                let lines = [
                    (line1, TEXT_LIGHT),
                    (line2, TEXT_LIGHT),
                    (line3, fare_color),
                ];
                let step = (f.line_height + 2).max(12);
                for (i, (text, color)) in lines.into_iter().enumerate() {
                    let ly = layout.icon_rect.y + i as i32 * step;
                    if ly + f.line_height <= max_y {
                        let clipped = clip_text_to_width(f, &text, text_w);
                        f.draw(frame, &clipped, [text_x, ly], color);
                    }
                }
            }
        }

        // Native 4-state `WinBtn.bmp` (`17`) Confirm (`STR# 142[0]` OK) and
        // Cancel (`STR# 142[1]` Cancel) buttons.
        let confirm_cell: u32 = if !state.can_confirm() {
            3
        } else if self.pressed_confirm && self.hovered_confirm {
            1
        } else if self.hovered_confirm {
            2
        } else {
            0
        };
        let cancel_cell: u32 = if self.pressed_cancel && self.hovered_cancel {
            1
        } else if self.hovered_cancel {
            2
        } else {
            0
        };
        draw_travel_dialog_button(
            frame,
            art,
            layout.confirm_rect,
            confirm_cell,
            strings.dialog_ok(),
            state.can_confirm(),
        );
        draw_travel_dialog_button(
            frame,
            art,
            layout.cancel_rect,
            cancel_cell,
            strings.dialog_cancel(),
            true,
        );

        layout
    }
}

fn draw_travel_dialog_button(
    frame: &mut Frame,
    art: &Art,
    rect: Rect,
    cell: u32,
    label: &str,
    enabled: bool,
) {
    if rect.width <= 0 || rect.height <= 0 {
        return;
    }
    if let Some(sheet) = art.image(DIALOG_BUTTON)
        && sheet.width >= 4
        && sheet.height > 0
    {
        let cw = (sheet.width / 4) as i32;
        let ch = sheet.height as i32;
        let sx = cw.saturating_mul(cell.min(3) as i32);
        let cap = (cw / 4).min(rect.width / 2).max(1);
        let mid_src_w = (cw - cap * 2).max(1);
        let mid_dst_w = (rect.width - cap * 2).max(0);
        blit_scaled_region(
            frame,
            sheet,
            Rect::new(sx, 0, cap, ch),
            Rect::new(rect.x, rect.y, cap, rect.height),
        );
        if mid_dst_w > 0 {
            blit_scaled_region(
                frame,
                sheet,
                Rect::new(sx + cap, 0, mid_src_w, ch),
                Rect::new(rect.x + cap, rect.y, mid_dst_w, rect.height),
            );
        }
        blit_scaled_region(
            frame,
            sheet,
            Rect::new(sx + cw - cap, 0, cap, ch),
            Rect::new(rect.x + rect.width - cap, rect.y, cap, rect.height),
        );
    } else {
        let fill = if !enabled {
            CHIP_DISABLED_BG
        } else if cell == 1 {
            [22, 112, 168]
        } else if cell == 2 {
            [64, 96, 196]
        } else {
            [38, 60, 148]
        };
        frame.fill(rect, fill);
        draw_border(frame, rect, DIALOG_BORDER);
    }
    let text_color = if !enabled {
        TEXT_MUTED
    } else if cell == 1 {
        [0, 255, 255]
    } else if cell == 2 {
        [255, 255, 255]
    } else {
        TEXT_LIGHT
    };
    if let Some(f) = pick_font(art, 10) {
        draw_centered_text(frame, f, rect, label, text_color);
    }
}

fn blit_scaled_region(frame: &mut Frame, img: &sims_formats::bitmap::Image, src: Rect, dest: Rect) {
    if src.width <= 0 || src.height <= 0 || dest.width <= 0 || dest.height <= 0 {
        return;
    }
    let fw = i32::try_from(frame.width()).unwrap_or(i32::MAX);
    let fh = i32::try_from(frame.height()).unwrap_or(i32::MAX);
    let x0 = dest.x.max(0);
    let y0 = dest.y.max(0);
    let x1 = dest.x.saturating_add(dest.width).min(fw);
    let y1 = dest.y.saturating_add(dest.height).min(fh);
    for y in y0..y1 {
        let sy = src.y
            + ((i64::from(y - dest.y) * i64::from(src.height)) / i64::from(dest.height)) as i32;
        for x in x0..x1 {
            let sx = src.x
                + ((i64::from(x - dest.x) * i64::from(src.width)) / i64::from(dest.width)) as i32;
            if let (Ok(ux), Ok(uy)) = (u32::try_from(sx), u32::try_from(sy))
                && let Some([r, g, b, a]) = img.pixel(ux, uy)
                && a > 0
            {
                frame.blend(x, y, [r, g, b], a);
            }
        }
    }
}

fn pick_font(art: &Art, preferred: u8) -> Option<&Font> {
    art.font(preferred)
        .or_else(|| art.font(10))
        .or_else(|| art.font(9))
        .or_else(|| art.font(8))
}

fn draw_border(frame: &mut Frame, rect: Rect, color: [u8; 3]) {
    if rect.width <= 0 || rect.height <= 0 {
        return;
    }
    frame.fill(Rect::new(rect.x, rect.y, rect.width, 1), color);
    frame.fill(
        Rect::new(rect.x, rect.y + rect.height - 1, rect.width, 1),
        color,
    );
    frame.fill(Rect::new(rect.x, rect.y, 1, rect.height), color);
    frame.fill(
        Rect::new(rect.x + rect.width - 1, rect.y, 1, rect.height),
        color,
    );
}

fn fill_alpha(frame: &mut Frame, rect: Rect, color: [u8; 3], alpha: u8) {
    let fw = frame.width() as i32;
    let fh = frame.height() as i32;
    let x0 = rect.x.max(0);
    let y0 = rect.y.max(0);
    let x1 = (rect.x + rect.width).min(fw);
    let y1 = (rect.y + rect.height).min(fh);
    for y in y0..y1 {
        for x in x0..x1 {
            frame.blend(x, y, color, alpha);
        }
    }
}

fn clip_text_to_width(font: &Font, text: &str, max_w: i32) -> String {
    let mut fitted = text.to_owned();
    while !fitted.is_empty() && font.width(&fitted) > max_w {
        fitted.pop();
    }
    fitted
}

fn draw_fitted_image(frame: &mut Frame, img: &sims_formats::bitmap::Image, dst: Rect) {
    if img.width == 0 || img.height == 0 || dst.width <= 0 || dst.height <= 0 {
        return;
    }
    let scale_w = dst.width as f64 / img.width as f64;
    let scale_h = dst.height as f64 / img.height as f64;
    let scale = scale_w.min(scale_h).min(1.0);
    let draw_w = ((img.width as f64 * scale).round() as i32).clamp(1, dst.width);
    let draw_h = ((img.height as f64 * scale).round() as i32).clamp(1, dst.height);
    let ox = dst.x + (dst.width - draw_w) / 2;
    let oy = dst.y + (dst.height - draw_h) / 2;
    let fw = frame.width() as i32;
    let fh = frame.height() as i32;
    let x0 = ox.max(0);
    let y0 = oy.max(0);
    let x1 = (ox + draw_w).min(fw);
    let y1 = (oy + draw_h).min(fh);
    for y in y0..y1 {
        let sy = (((y - oy) as u64 * img.height as u64) / draw_h as u64).min(img.height as u64 - 1)
            as u32;
        for x in x0..x1 {
            let sx = (((x - ox) as u64 * img.width as u64) / draw_w as u64)
                .min(img.width as u64 - 1) as u32;
            if let Some([r, g, b, a]) = img.pixel(sx, sy)
                && a > 0
            {
                frame.blend(x, y, [r, g, b], a);
            }
        }
    }
}

fn draw_centered_text(frame: &mut Frame, font: &Font, rect: Rect, text: &str, color: [u8; 3]) {
    let max_w = (rect.width - 4).max(0);
    if max_w == 0 {
        return;
    }
    let fitted = clip_text_to_width(font, text, max_w);
    let tx = rect.x + (rect.width - font.width(&fitted)) / 2;
    let ty = rect.y + (rect.height - font.line_height) / 2;
    font.draw(frame, &fitted, [tx, ty], color);
}

fn draw_tooltip(frame: &mut Frame, font: &Font, x: i32, y: i32, text: &str) {
    let pad = 4;
    let w = (font.width(text) + pad * 2).min(frame.width() as i32);
    let h = font.line_height + pad * 2;
    let tx = x.clamp(0, (frame.width() as i32 - w).max(0));
    let ty = y.clamp(0, (frame.height() as i32 - h).max(0));
    let rect = Rect::new(tx, ty, w, h);
    frame.fill(rect, TOOLTIP_BG);
    draw_border(frame, rect, TOOLTIP_BORDER);
    font.draw(frame, text, [tx + pad, ty + pad], TOOLTIP_TEXT);
}
