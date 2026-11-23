//! Complete Collection 1.91 travel destination & community sub-neighborhood
//! chrome for **Downtown**, **Vacation Island**, **Studio Town**, and
//! **Magic Town**.
//!
//! Matches the four expansion maps rendered by `sims_render::neighborhood`
//! (`cWinDowntown`, `cWinVacation`, `cWinStudiotown`, `cWinMagicland`),
//! providing:
//!
//! - Top control bar with Return-to-Neighborhood, Bulldoze, Import, destination
//!   switcher, and expansion-specific status badges (`Res_Nbhd.RT` / `Res_Nbhd.h`).
//! - Interactive community lot filter bar (`STR# 171`, `NghUI/Filter*Btn.bmp`)
//!   including Vacation Island hotel / rental / campground lodging legend,
//!   Studio Town fame star rating & lot tags, and Magic Town MagiCoins balance.
//! - Taxi / Shuttle / Magic Hole travel-confirm dialog view-model
//!   (`STR# 212`, `STR# 213`, `STR# 260`, `STR# 261`, phone/hole icons).
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
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// `GameData/UIText.iff` string table IDs for the four expansion screens,
/// filter bar, and travel confirmation dialogs.
pub mod str_id {
    /// `ObjDialogs` default button strings (`OK`, `Cancel`, `Yes`, `No`).
    pub const DIALOG_BUTTONS: u16 = 142;
    /// `Downtown Screen Tooltips` (5 entries).
    pub const DOWNTOWN_TIPS: u16 = 169;
    /// `Vacation Island Screen Tooltips` (5 entries).
    pub const VACATION_TIPS: u16 = 170;
    /// `Filter Bar` tooltips (13 entries).
    pub const FILTER_BAR: u16 = 171;
    /// `Studio Town Screen Tooltips` (5 entries).
    pub const STUDIO_TOWN_TIPS: u16 = 173;
    /// `Magic land Screen Tooltips` (6 entries).
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
    /// `NghUI/banner_downtown.bmp` (800×52).
    pub const DOWNTOWN_BAR_BKG: i32 = 5422;
    /// `NghUI/banner_neighborhood.bmp` (800×52, fallback bar background).
    pub const NEIGHBORHOOD_BAR_BKG: i32 = 5420;
    /// `Downtown/Return.bmp` (`kDowntownExitToNgh`).
    pub const DOWNTOWN_RETURN: i32 = 5209;
    /// `Downtown/TheSimsLogo.bmp` (`kDowntownCredits`).
    pub const DOWNTOWN_LOGO: i32 = 5210;
    /// `Downtown/Bulldoze.bmp` (`kDowntownBulldoze`).
    pub const DOWNTOWN_BULLDOZE: i32 = 5211;
    /// `Downtown/Import.bmp` (`kDowntownImport`).
    pub const DOWNTOWN_IMPORT: i32 = 5212;
    /// `VIsland/Import.bmp` (`kVacationImport`).
    pub const VACATION_IMPORT: i32 = 5214;
    /// `VIsland/vacationlogosmall.bmp` (`kNghVacationLogo`).
    pub const VACATION_LOGO: i32 = 5441;
    /// `VIsland/no_vacancy.bmp` (`kVacationNoVacancy`).
    pub const VACATION_NO_VACANCY: i32 = 5510;
    /// `Studiotown/Return.bmp` (`kStudiotownExitToNgh`).
    pub const STUDIO_TOWN_RETURN: i32 = 5217;
    /// `Studiotown/Import.bmp` (`kStudiotownImport`).
    pub const STUDIO_TOWN_IMPORT: i32 = 5218;
    /// `Studiotown/Bulldoze.bmp` (`kStudiotownBulldoze`).
    pub const STUDIO_TOWN_BULLDOZE: i32 = 5219;
    /// `Studiotown/TheSimsLogo.bmp` (`kStudiotownCredits`).
    pub const STUDIO_TOWN_LOGO: i32 = 5220;
    /// `Magicland/Import.bmp` (`kMagictownImport`).
    pub const MAGIC_TOWN_IMPORT: i32 = 5222;
    /// Destination switcher badges (`NghUI/*`).
    pub const DEST_DOWNTOWN: i32 = 5208;
    pub const DEST_VACATION: i32 = 5213;
    pub const DEST_STUDIO_TOWN: i32 = 5216;
    pub const DEST_MAGIC_TOWN: i32 = 5221;
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
    /// Filter bar backgrounds (`NghUI/Filter_Toolbar_*.bmp`).
    pub const FILTER_TOOLBAR: i32 = 5045;
    pub const FILTER_TOOLBAR_ST: i32 = 5046;
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
///
/// Note: `Res_Nbhd.RT` misspells `kStudiotownBulldoze` (`5219`) as
/// `kStudiownBulldoze`, so Studio Town falls back to [`art_id::DOWNTOWN_BULLDOZE`].
pub const RESOURCES: [i32; 41] = [
    DIALOG_BACKGROUND,
    DIALOG_BUTTON,
    art_id::DOWNTOWN_BAR_BKG,
    art_id::NEIGHBORHOOD_BAR_BKG,
    art_id::DOWNTOWN_RETURN,
    art_id::DOWNTOWN_LOGO,
    art_id::DOWNTOWN_BULLDOZE,
    art_id::DOWNTOWN_IMPORT,
    art_id::VACATION_IMPORT,
    art_id::VACATION_LOGO,
    art_id::VACATION_NO_VACANCY,
    art_id::STUDIO_TOWN_RETURN,
    art_id::STUDIO_TOWN_IMPORT,
    art_id::STUDIO_TOWN_LOGO,
    art_id::MAGIC_TOWN_IMPORT,
    art_id::DEST_DOWNTOWN,
    art_id::DEST_VACATION,
    art_id::DEST_STUDIO_TOWN,
    art_id::DEST_MAGIC_TOWN,
    art_id::PHONE_DOWNTOWN,
    art_id::PHONE_VACATION,
    art_id::PHONE_NEIGHBORHOOD,
    art_id::PHONE_STUDIO_TOWN,
    art_id::PHONE_MAGIC_TOWN,
    art_id::MAGIC_HOLE_ICON,
    art_id::DIALOG_FRAME_ALT,
    art_id::FILTER_TOOLBAR,
    art_id::FILTER_TOOLBAR_ST,
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
/// Secondary filter / legend strip height below the top bar (accommodates 52 px
/// native filter button strips on >= 800 px viewports).
pub const FILTER_STRIP_HEIGHT: i32 = 56;
/// Compact filter strip height on viewports narrower than 800 px.
pub const COMPACT_FILTER_STRIP_HEIGHT: i32 = 30;

/// Loads all travel chrome bitmaps and UI fonts from `ui`.
pub fn load_art(ui: &UiGraphics, language: u8) -> Result<Art> {
    Art::load(ui, language, &RESOURCES)
}

const BAR_BG_DOWNTOWN: [u8; 3] = [26, 30, 74];
const BAR_BG_VACATION: [u8; 3] = [20, 66, 86];
const BAR_BG_OLD_TOWN: [u8; 3] = [24, 62, 48];
const BAR_BG_STUDIO: [u8; 3] = [62, 24, 58];
const BAR_BG_MAGIC: [u8; 3] = [42, 22, 68];
const STRIP_BG: [u8; 3] = [16, 22, 50];
const STRIP_BORDER: [u8; 3] = [76, 110, 184];
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

    /// Returns the community filter tags applicable to this destination.
    pub const fn filters(self) -> &'static [LotFilter] {
        match self {
            Self::Downtown => &[LotFilter::Food, LotFilter::Shopping, LotFilter::Recreation],
            Self::VacationIsland => &[
                LotFilter::Lodging,
                LotFilter::Food,
                LotFilter::Shopping,
                LotFilter::Recreation,
            ],
            Self::OldTown => &[
                LotFilter::DogCat,
                LotFilter::SmallAnimal,
                LotFilter::Gardening,
                LotFilter::Shopping,
                LotFilter::Food,
                LotFilter::Recreation,
            ],
            Self::StudioTown => &[
                LotFilter::Music,
                LotFilter::Movie,
                LotFilter::Model,
                LotFilter::Spa,
                LotFilter::Food,
                LotFilter::Shopping,
            ],
            Self::MagicTown => &[
                LotFilter::Magic,
                LotFilter::Rides,
                LotFilter::Food,
                LotFilter::Shopping,
            ],
        }
    }

    /// Fallback bar fill RGB for this destination.
    const fn bar_fallback_bg(self) -> [u8; 3] {
        match self {
            Self::Downtown => BAR_BG_DOWNTOWN,
            Self::VacationIsland => BAR_BG_VACATION,
            Self::OldTown => BAR_BG_OLD_TOWN,
            Self::StudioTown => BAR_BG_STUDIO,
            Self::MagicTown => BAR_BG_MAGIC,
        }
    }

    /// `UIGraphics` resource ID for this destination's Return-to-Neighborhood button.
    pub const fn return_art_id(self) -> i32 {
        match self {
            Self::StudioTown => art_id::STUDIO_TOWN_RETURN,
            _ => art_id::DOWNTOWN_RETURN,
        }
    }

    /// `UIGraphics` resource ID for this destination's Bulldoze button.
    pub const fn bulldoze_art_id(self) -> i32 {
        art_id::DOWNTOWN_BULLDOZE
    }

    /// `UIGraphics` resource ID for this destination's Import button.
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

    /// Compact ASCII fallback label when `STR# 171` is unavailable.
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Lodging => "Lodge",
            Self::Food => "Food",
            Self::Gardening => "Garden",
            Self::Shopping => "Shop",
            Self::DogCat => "Pets",
            Self::SmallAnimal => "Critters",
            Self::Recreation => "Fun",
            Self::Spa => "Spa",
            Self::Movie => "Film",
            Self::Model => "Model",
            Self::Music => "Music",
            Self::Magic => "Magic",
            Self::Rides => "Rides",
        }
    }
}

/// Vacation Island lodging legend categories (Hotel / Cabin Rental / Campground).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VacationLodging {
    Hotel,
    CabinRental,
    Campground,
}

impl VacationLodging {
    pub const ALL: [Self; 3] = [Self::Hotel, Self::CabinRental, Self::Campground];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Hotel => "Hotel",
            Self::CabinRental => "Rental",
            Self::Campground => "Camp",
        }
    }

    pub const fn badge_color(self) -> [u8; 3] {
        match self {
            Self::Hotel => [216, 162, 48],
            Self::CabinRental => [74, 152, 212],
            Self::Campground => [76, 168, 92],
        }
    }
}

/// Active interaction mode on a travel destination map screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TravelBarMode {
    /// Browsing lots (or picking a lot to edit in Build/Buy).
    #[default]
    Browse,
    /// Visiting a lot with an active household (disables Bulldoze / Import).
    Visit,
    /// Bulldozing / resetting a community lot back to template.
    Bulldoze,
}

/// Read-only state rendered by [`TravelChrome`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TravelState {
    pub destination: Destination,
    pub mode: TravelBarMode,
    pub expansions: ExpansionFlags,
    pub has_importable_lot: bool,
    /// Active community filter highlight (if any).
    pub active_filter: Option<LotFilter>,
    /// Vacation Island: optional lodging legend filter (`Hotel`/`CabinRental`/`Campground`).
    pub vacation_lodging_filter: Option<VacationLodging>,
    /// Vacation Island: whether any resort lot currently shows No Vacancy.
    pub vacation_has_no_vacancy: bool,
    /// Studio Town: active Sim's fame rating in half-stars (`0..=10`, i.e. 0.0..=5.0 stars).
    pub studio_fame_half_stars: u8,
    /// Magic Town: active household's MagiCoins balance.
    pub magic_coins: i32,
    /// Active household's Simoleon balance (displayed in Visit / Magic / Vacation contexts).
    pub simoleons: i32,
}

impl Default for TravelState {
    fn default() -> Self {
        Self {
            destination: Destination::Downtown,
            mode: TravelBarMode::Browse,
            expansions: ExpansionFlags::ALL,
            has_importable_lot: false,
            active_filter: None,
            vacation_lodging_filter: None,
            vacation_has_no_vacancy: false,
            studio_fame_half_stars: 0,
            magic_coins: 0,
            simoleons: 0,
        }
    }
}

/// Interactive controls in the destination top bar and filter legend strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TravelControl {
    ReturnHome,
    Bulldoze,
    ImportLot,
    SwitchDestination(Destination),
    ToggleFilter(LotFilter),
    ToggleVacationLodging(VacationLodging),
}

/// Commands emitted when the player clicks a control in [`TravelChrome`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TravelCommand {
    ReturnToNeighborhood,
    ToggleBulldoze,
    ImportLot,
    SwitchDestination(Destination),
    SetFilter(Option<LotFilter>),
    SetVacationLodging(Option<VacationLodging>),
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
            TravelControl::ReturnHome => screen_tips.first().map(String::as_str),
            TravelControl::ImportLot => screen_tips.get(3).map(String::as_str),
            TravelControl::Bulldoze => screen_tips.get(4).map(String::as_str),
            TravelControl::ToggleFilter(f) => {
                self.filter_tips.get(f.tooltip_index()).map(String::as_str)
            }
            TravelControl::SwitchDestination(_) | TravelControl::ToggleVacationLodging(_) => None,
        }
        .filter(|s| !s.is_empty())
    }
}

/// A laid-out interactive control with non-overlapping hit rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPlacement {
    pub control: TravelControl,
    pub rect: Rect,
    pub enabled: bool,
    pub active: bool,
}

/// Computed layout for the travel destination top bar and filter legend strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TravelLayout {
    pub frame_size: [i32; 2],
    pub bar_rect: Rect,
    pub filter_strip_rect: Rect,
    pub status_badge_rect: Rect,
    pub controls: Vec<ControlPlacement>,
    pub compact: bool,
}

impl TravelLayout {
    /// Computes pairwise-disjoint control placements for `width` × `height`.
    pub fn compute(width: u32, height: u32, state: &TravelState) -> Self {
        let w = i32::try_from(width).unwrap_or(i32::MAX).max(0);
        let h = i32::try_from(height).unwrap_or(i32::MAX).max(0);
        let compact = w < NATIVE_WIDTH;

        let bar_w = if compact { w } else { NATIVE_WIDTH };
        let bar_x = if compact { 0 } else { (w - bar_w) / 2 };
        let bar_h = BAR_HEIGHT.min(h);
        let bar_rect = Rect::new(bar_x, 0, bar_w, bar_h);

        let desired_strip_h = if compact {
            COMPACT_FILTER_STRIP_HEIGHT
        } else {
            FILTER_STRIP_HEIGHT
        };
        let strip_h = desired_strip_h.min((h - bar_h).max(0));
        let filter_strip_rect = Rect::new(bar_x, bar_h, bar_w, strip_h);

        let mut controls = Vec::new();
        let is_visit = state.mode == TravelBarMode::Visit;

        // Top bar controls: ReturnHome, Bulldoze, ImportLot, + 4 destination tabs.
        let top_items: [(TravelControl, bool, bool); 7] = [
            (TravelControl::ReturnHome, true, false),
            (
                TravelControl::Bulldoze,
                !is_visit,
                state.mode == TravelBarMode::Bulldoze,
            ),
            (
                TravelControl::ImportLot,
                !is_visit && state.has_importable_lot,
                false,
            ),
            (
                TravelControl::SwitchDestination(Destination::Downtown),
                state.expansions.has(ExpansionFlags::HOT_DATE) && !is_visit,
                state.destination == Destination::Downtown,
            ),
            (
                TravelControl::SwitchDestination(Destination::VacationIsland),
                state.expansions.has(ExpansionFlags::VACATION) && !is_visit,
                state.destination == Destination::VacationIsland,
            ),
            (
                TravelControl::SwitchDestination(Destination::StudioTown),
                state.expansions.has(ExpansionFlags::SUPERSTAR) && !is_visit,
                state.destination == Destination::StudioTown,
            ),
            (
                TravelControl::SwitchDestination(Destination::MagicTown),
                state.expansions.has(ExpansionFlags::MAKIN_MAGIC) && !is_visit,
                state.destination == Destination::MagicTown,
            ),
        ];

        // Reserve right side of top bar for status badge (MagiCoins / Fame / Simoleons).
        let badge_w = if compact {
            (bar_w / 4).clamp(64, 120).min(bar_w)
        } else {
            170
        };
        let status_badge_rect = Rect::new(
            bar_x + bar_w - badge_w - 6,
            6.min(bar_h),
            (badge_w).max(0),
            (bar_h - 12).max(0),
        );

        let left_pad = if compact { 4 } else { 78 };
        let avail_top_w = (status_badge_rect.x - (bar_x + left_pad) - 6).max(0);
        let gap = if compact { 2 } else { 6 };
        let count = top_items.len() as i32;
        let btn_w = if compact {
            ((avail_top_w - gap * (count - 1)).max(0)) / count
        } else {
            53.min(((avail_top_w - gap * (count - 1)).max(0)) / count)
        };
        let btn_h = (bar_h - 6).max(0);
        let btn_y = (bar_h - btn_h) / 2;

        for (idx, (control, enabled, active)) in top_items.into_iter().enumerate() {
            let x = bar_x + left_pad + idx as i32 * (btn_w + gap);
            if btn_w > 0 && btn_h > 0 && x + btn_w <= status_badge_rect.x {
                controls.push(ControlPlacement {
                    control,
                    rect: Rect::new(x, btn_y, btn_w, btn_h),
                    enabled,
                    active,
                });
            }
        }

        // Secondary strip: filter buttons (+ Vacation Island lodging legend chips).
        if strip_h >= 14 {
            let filters = state.destination.filters();
            let extra_lodging = if state.destination == Destination::VacationIsland {
                VacationLodging::ALL.len()
            } else {
                0
            };
            let total_chips = (filters.len() + extra_lodging) as i32;
            if total_chips > 0 {
                let chip_gap = if compact { 3 } else { 6 };
                let strip_pad = if compact { 4 } else { 12 };
                let avail_w = (bar_w - strip_pad * 2).max(0);
                let max_chip_w = if compact { 58 } else { 76 };
                let chip_w = (((avail_w - chip_gap * (total_chips - 1)).max(0)) / total_chips)
                    .min(max_chip_w);
                let chip_h = if compact {
                    (strip_h - 6).max(10)
                } else {
                    (strip_h - 4).max(10)
                };
                let chip_y = filter_strip_rect.y + (strip_h - chip_h) / 2;
                let mut cursor_x = bar_x + strip_pad;

                for &filter in filters {
                    if chip_w > 0 && cursor_x + chip_w <= bar_x + bar_w {
                        controls.push(ControlPlacement {
                            control: TravelControl::ToggleFilter(filter),
                            rect: Rect::new(cursor_x, chip_y, chip_w, chip_h),
                            enabled: true,
                            active: state.active_filter == Some(filter),
                        });
                        cursor_x += chip_w + chip_gap;
                    }
                }
                if state.destination == Destination::VacationIsland {
                    for lodging in VacationLodging::ALL {
                        if chip_w > 0 && cursor_x + chip_w <= bar_x + bar_w {
                            controls.push(ControlPlacement {
                                control: TravelControl::ToggleVacationLodging(lodging),
                                rect: Rect::new(cursor_x, chip_y, chip_w, chip_h),
                                enabled: true,
                                active: state.vacation_lodging_filter == Some(lodging),
                            });
                            cursor_x += chip_w + chip_gap;
                        }
                    }
                }
            }
        }

        Self {
            frame_size: [w, h],
            bar_rect,
            filter_strip_rect,
            status_badge_rect,
            controls,
            compact,
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

/// Interactive controller for the travel destination top bar and filter strip.
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
            TravelControl::SwitchDestination(dest) => TravelCommand::SwitchDestination(dest),
            TravelControl::ToggleFilter(filter) => {
                if state.active_filter == Some(filter) {
                    TravelCommand::SetFilter(None)
                } else {
                    TravelCommand::SetFilter(Some(filter))
                }
            }
            TravelControl::ToggleVacationLodging(lodging) => {
                if state.vacation_lodging_filter == Some(lodging) {
                    TravelCommand::SetVacationLodging(None)
                } else {
                    TravelCommand::SetVacationLodging(Some(lodging))
                }
            }
        })
    }

    /// Draws the travel destination top bar, filter legend strip, status badge,
    /// and hover tooltip into `frame`.
    pub fn draw(
        &self,
        frame: &mut Frame,
        state: &TravelState,
        art: &Art,
        strings: &TravelStrings,
    ) -> TravelLayout {
        let layout = TravelLayout::compute(frame.width(), frame.height(), state);

        // Top bar backdrop.
        let bar_bg = art
            .image(art_id::DOWNTOWN_BAR_BKG)
            .or_else(|| art.image(art_id::NEIGHBORHOOD_BAR_BKG));
        if !layout.compact
            && let Some(img) = bar_bg
        {
            frame.draw_all(img, [layout.bar_rect.x, layout.bar_rect.y]);
        } else {
            frame.fill(layout.bar_rect, state.destination.bar_fallback_bg());
            draw_border(frame, layout.bar_rect, STRIP_BORDER);
        }

        // Destination emblem on left of top bar when non-compact.
        if !layout.compact {
            let logo_id = match state.destination {
                Destination::VacationIsland => art_id::VACATION_LOGO,
                Destination::StudioTown => art_id::STUDIO_TOWN_LOGO,
                _ => art_id::DOWNTOWN_LOGO,
            };
            if let Some(logo) = art.image(logo_id) {
                let ly = layout.bar_rect.y + (layout.bar_rect.height - logo.height as i32) / 2;
                frame.draw_all(logo, [layout.bar_rect.x + 4, ly.max(0)]);
            }
        }

        // Secondary filter & legend strip.
        if layout.filter_strip_rect.height > 0 {
            fill_alpha(frame, layout.filter_strip_rect, STRIP_BG, 225);
            if !layout.compact {
                let toolbar_id = if state.destination == Destination::StudioTown {
                    art_id::FILTER_TOOLBAR_ST
                } else {
                    art_id::FILTER_TOOLBAR
                };
                if let Some(toolbar) = art.image(toolbar_id) {
                    let ty = layout.filter_strip_rect.y
                        + (layout.filter_strip_rect.height - toolbar.height as i32) / 2;
                    frame.draw_all(
                        toolbar,
                        [
                            layout.filter_strip_rect.x,
                            ty.max(layout.filter_strip_rect.y),
                        ],
                    );
                }
            }
            draw_border(frame, layout.filter_strip_rect, STRIP_BORDER);
        }

        // Draw controls.
        let font = pick_font(art, if layout.compact { 8 } else { 10 });
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

            let maybe_art = control_art_id(state.destination, placement.control)
                .and_then(|id| art.cell(id, 4, cell));

            if !layout.compact
                && let Some(sprite) = maybe_art
                && sprite.width as i32 <= placement.rect.width + 6
                && sprite.height as i32 <= placement.rect.height + 6
            {
                let sx = placement.rect.x + (placement.rect.width - sprite.width as i32) / 2;
                let sy = placement.rect.y + (placement.rect.height - sprite.height as i32) / 2;
                frame.draw_all(&sprite, [sx, sy]);
            } else {
                let bg = match placement.control {
                    TravelControl::ToggleVacationLodging(lodging) => {
                        if placement.active {
                            lodging.badge_color()
                        } else {
                            CHIP_BG
                        }
                    }
                    _ if !placement.enabled => CHIP_DISABLED_BG,
                    _ if placement.active || is_pressed => CHIP_ACTIVE_BG,
                    _ => CHIP_BG,
                };
                frame.fill(placement.rect, bg);
                let border = if is_hovered && placement.enabled {
                    TEXT_GOLD
                } else {
                    STRIP_BORDER
                };
                draw_border(frame, placement.rect, border);
                if let Some(f) = font {
                    let label = control_short_label(placement.control);
                    let ink = if placement.enabled {
                        TEXT_LIGHT
                    } else {
                        TEXT_MUTED
                    };
                    draw_centered_text(frame, f, placement.rect, label, ink);
                }
            }
        }

        // Destination-specific status badge (MagiCoins / Studio Fame / Vacation Vacancy / Simoleons).
        if layout.status_badge_rect.width >= 32 && layout.status_badge_rect.height >= 16 {
            fill_alpha(frame, layout.status_badge_rect, STRIP_BG, 230);
            draw_border(frame, layout.status_badge_rect, TEXT_GOLD);
            if let Some(f) = font {
                let badge_text = format_status_badge(state);
                draw_centered_text(frame, f, layout.status_badge_rect, &badge_text, TEXT_GOLD);
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

/// Formats the destination-specific status badge (MagiCoins, Studio Fame stars,
/// Vacation availability, or Simoleon balance).
pub fn format_status_badge(state: &TravelState) -> String {
    match state.destination {
        Destination::MagicTown => format!("MC {} | ${}", state.magic_coins, state.simoleons),
        Destination::StudioTown => {
            let half = state.studio_fame_half_stars.min(10);
            format!("Fame {}.{}", half / 2, (half % 2) * 5)
        }
        Destination::VacationIsland => {
            if state.vacation_has_no_vacancy {
                format!("Full | ${}", state.simoleons)
            } else {
                format!("Vacancy | ${}", state.simoleons)
            }
        }
        Destination::Downtown | Destination::OldTown => format!("${}", state.simoleons),
    }
}

fn control_art_id(destination: Destination, control: TravelControl) -> Option<i32> {
    match control {
        TravelControl::ReturnHome => Some(destination.return_art_id()),
        TravelControl::Bulldoze => Some(destination.bulldoze_art_id()),
        TravelControl::ImportLot => Some(destination.import_art_id()),
        TravelControl::SwitchDestination(Destination::Downtown) => Some(art_id::DEST_DOWNTOWN),
        TravelControl::SwitchDestination(Destination::VacationIsland) => {
            Some(art_id::DEST_VACATION)
        }
        TravelControl::SwitchDestination(Destination::OldTown) => Some(art_id::PHONE_NEIGHBORHOOD),
        TravelControl::SwitchDestination(Destination::StudioTown) => Some(art_id::DEST_STUDIO_TOWN),
        TravelControl::SwitchDestination(Destination::MagicTown) => Some(art_id::DEST_MAGIC_TOWN),
        TravelControl::ToggleFilter(filter) => Some(filter.art_id()),
        TravelControl::ToggleVacationLodging(_) => None,
    }
}

fn control_short_label(control: TravelControl) -> &'static str {
    match control {
        TravelControl::ReturnHome => "Home",
        TravelControl::Bulldoze => "Doze",
        TravelControl::ImportLot => "Imp",
        TravelControl::SwitchDestination(d) => d.short_label(),
        TravelControl::ToggleFilter(f) => f.short_label(),
        TravelControl::ToggleVacationLodging(l) => l.label(),
    }
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

fn blit_scaled_region(
    frame: &mut Frame,
    img: &sims_formats::bitmap::Image,
    src: Rect,
    dest: Rect,
) {
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
