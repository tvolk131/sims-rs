//! Neighborhood screen chrome: the top control bar and the lot hover
//! ("house info") popup card, laid out and drawn as Complete Collection 1.91
//! does.
//!
//! Layout and artwork IDs come directly from `UIGraphics/Res_Nbhd.h`,
//! `Res_Nbhd.RT`, `GameData/UIText.iff` (`STR#` 134, 151, 225), and the 1.91
//! disassembly of `cWinCommunityNeighborhood` (`0x556531..0x557881`,
//! `0x55b2ab..0x55d120`) and the lot hotspot popup (`0x54f756..0x551060`).
//!
//! Like the rest of `sims-ui`, this module is decoupled from persistence and
//! world rendering: callers pass read-only view structs ([`NeighborhoodState`],
//! [`LotInfo`]) and handle returned [`NeighborhoodCommand`] values.

use crate::art::Art;
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::{Error, Result};
use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// `GameData/UIText.iff` string table for lot hover popup messages (`0x54f7db`).
pub const POPUP_STRINGS: u16 = 134;
/// `GameData/UIText.iff` string table for top bar tooltips (`0x557431`).
pub const BAR_STRINGS: u16 = 151;
/// `GameData/UIText.iff` string table for missing-expansion notices (`0x557413`).
pub const EXPANSION_STRINGS: u16 = 225;

/// `UIGraphics/Res_Nbhd.h` resource identifiers used by the neighborhood bar
/// and lot hover popup card.
pub mod art_id {
    /// `NghUI/banner_neighborhood.bmp` (800×52).
    pub const BAR_BKG: i32 = 5420;
    /// `NghUI/TheSimsLogo.bmp` (70×52, 1 cell).
    pub const SIMS_LOGO: i32 = 5421;
    /// `NghUI/Previous.bmp` (104×52, 4 cells of 26×52).
    pub const PREV_NEIGHBORHOOD: i32 = 5204;
    /// `NghUI/Next.bmp` (104×52, 4 cells of 26×52).
    pub const NEXT_NEIGHBORHOOD: i32 = 5205;
    /// `NghUI/Current.bmp` (23×30).
    pub const CURRENT_BADGE: i32 = 5206;
    /// `NghUI/MoveIn.bmp` (212×52, 4 cells of 53×52).
    pub const SELECT_FAMILY: i32 = 5200;
    /// `NghUI/Import.bmp` (212×52, 4 cells of 53×52).
    pub const IMPORT_FAMILY: i32 = 5207;
    /// `NghUI/Bulldoze.bmp` (212×52, 4 cells of 53×52).
    pub const BULLDOZE: i32 = 5201;
    /// `NghUI/Rezone.bmp` (212×52, 4 cells of 53×52).
    pub const REZONE: i32 = 5215;
    /// `NghUI/Downtown.bmp` (212×52, 4 cells of 53×52).
    pub const DOWNTOWN: i32 = 5208;
    /// `NghUI/Vacation.bmp` (212×52, 4 cells of 53×52).
    pub const VACATION: i32 = 5213;
    /// `NghUI/Studiotown.bmp` (212×52, 4 cells of 53×52).
    pub const STUDIO_TOWN: i32 = 5216;
    /// `NghUI/MagicLand.bmp` (212×52, 4 cells of 53×52).
    pub const MAGIC_TOWN: i32 = 5221;
    /// `NghUI/Exit.bmp` (212×52, 4 cells of 53×52).
    pub const QUIT: i32 = 5202;
    /// `NghUI/InetBtn.bmp` (156×52, 4 cells of 39×52).
    pub const WEB: i32 = 5306;
    /// `Nbhd/CreditBtnEn.bmp` (232×58, 4 cells of 58×58).
    pub const CREDITS: i32 = 5315;
    /// `Nbhd/HotspotPopupTiler.TGA` (111×111, 3×3 tiles of 37×37).
    pub const POPUP_TILER: i32 = 5107;
}

/// Every `UIGraphics` resource ID required to draw the neighborhood chrome and
/// lot hover popup card.
pub const RESOURCES: [i32; 17] = [
    art_id::BAR_BKG,
    art_id::SIMS_LOGO,
    art_id::PREV_NEIGHBORHOOD,
    art_id::NEXT_NEIGHBORHOOD,
    art_id::CURRENT_BADGE,
    art_id::SELECT_FAMILY,
    art_id::IMPORT_FAMILY,
    art_id::BULLDOZE,
    art_id::REZONE,
    art_id::DOWNTOWN,
    art_id::VACATION,
    art_id::STUDIO_TOWN,
    art_id::MAGIC_TOWN,
    art_id::QUIT,
    art_id::WEB,
    art_id::CREDITS,
    art_id::POPUP_TILER,
];

/// Native 1.91 neighborhood screen dimensions (800×600).
pub const NATIVE_WIDTH: i32 = 800;
pub const NATIVE_HEIGHT: i32 = 600;
/// Top control bar height (`NghUI/banner_neighborhood.bmp`).
pub const BAR_HEIGHT: i32 = 52;
/// Minimum and maximum neighborhood slot numbers in Complete Collection.
pub const MIN_NEIGHBORHOOD: u8 = 1;
pub const MAX_NEIGHBORHOOD: u8 = 8;

/// Standard light UI text (`0x6f2a94`: RGB(195, 205, 205)).
const TEXT_NORMAL: [u8; 3] = [195, 205, 205];
/// Pure white for the current neighborhood number (`0x556dbf`).
const TEXT_WHITE: [u8; 3] = [255, 255, 255];
/// Portrait border tint on the lot hover popup card.
const PORTRAIT_BORDER: [u8; 3] = [32, 64, 176];
/// Classic Windows/Sims hover tooltip fill (`#FFFFE6`), border, and ink.
const TOOLTIP_BG: [u8; 3] = [255, 255, 230];
const TOOLTIP_BORDER: [u8; 3] = [0, 0, 0];
const TOOLTIP_TEXT: [u8; 3] = [0, 0, 0];
/// Fallback top bar background fill when `BAR_BKG` art is absent.
const BAR_FALLBACK_BG: [u8; 3] = [24, 34, 112];
/// Fallback popup tint when `POPUP_TILER` art is absent.
const POPUP_FALLBACK_BG: [u8; 3] = [16, 26, 96];

/// Popup text padding on all four sides (`0x5507ea`: `0x19` = 25 px).
const POPUP_PAD: i32 = 25;
/// Minimum and maximum text column widths (`0x5507a2..0x5507bc`: `0x96..=0x140`).
const POPUP_MIN_TEXT_W: i32 = 150;
const POPUP_MAX_TEXT_W: i32 = 320;
/// Member portrait thumbnail size on the lot hover popup (`0x550b0e`: `0x19` = 25 px).
const POPUP_PORTRAIT_SIZE: i32 = 25;
/// Vertical gap between popup text and portraits (`0x55082d`: `0xa` = 10 px).
const POPUP_PORTRAIT_GAP: i32 = 10;

/// Loads all neighborhood chrome bitmaps and fonts from `ui`.
pub fn load_art(ui: &UiGraphics, language: u8) -> Result<Art> {
    Art::load(ui, language, &RESOURCES)
}

/// Installed expansion bitmask matching `[0x6e48bc]+0x66` in 1.91
/// (`0x5575bb..0x5576f1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpansionFlags(pub u8);

impl ExpansionFlags {
    pub const LIVIN_LARGE: u8 = 0x01;
    pub const HOUSE_PARTY: u8 = 0x02;
    pub const HOT_DATE: u8 = 0x04;
    pub const VACATION: u8 = 0x08;
    pub const UNLEASHED: u8 = 0x20;
    pub const SUPERSTAR: u8 = 0x40;
    pub const MAKIN_MAGIC: u8 = 0x80;
    pub const ALL: Self = Self(0xff);

    pub const fn has(self, bit: u8) -> bool {
        self.0 & bit != 0
    }
}

impl Default for ExpansionFlags {
    fn default() -> Self {
        Self::ALL
    }
}

/// Active neighborhood interaction mode (`[esi+0xd4]` in `0x55b2ab`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NeighborhoodMode {
    /// Normal neighborhood browsing and entering occupied lots.
    #[default]
    Normal,
    /// Placing a chosen family onto a vacant residential lot.
    MoveIn {
        family_name: String,
        family_funds: i32,
        family_magic_coins: i32,
    },
    /// Evicting an occupied household or demolishing a built house.
    Bulldoze,
    /// Rezoning a lot between residential and community (Unleashed).
    Rezone,
    /// Choosing a community lot to visit.
    Visit,
}

/// Lot zoning / category used to pick the 1.91 popup description (`0x550d85`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LotKind {
    #[default]
    Residential,
    Community,
    Tutorial,
    MagicResidential,
    MagicCommunity,
}

/// Household summary displayed in the lot hover card for an occupied lot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LotOccupant {
    pub family_name: String,
    pub funds: i32,
    pub house_value: i32,
    pub net_worth: i32,
    pub friends: i32,
    pub magic_coins: i32,
    /// Member head portraits (drawn scaled/cropped to 25×25 along the bottom
    /// of the popup card, matching `0x550b02..0x550c53`).
    pub portraits: Vec<Image>,
}

/// Read-only lot metadata for the hover / house info popup card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LotInfo {
    pub lot_number: u16,
    /// Explicit address override; when `None`, formatted from `lot_number`
    /// using the 1.91 street-name rules in `STR# 134`.
    pub address: Option<String>,
    pub house_name: String,
    pub house_description: String,
    pub kind: LotKind,
    /// Whether a house structure is currently built on a vacant lot (selects
    /// "house" vs "lot" messaging in Move-In and Bulldoze modes).
    pub has_house: bool,
    /// Purchase cost of the vacant lot/house in Simoleons.
    pub house_cost: i32,
    /// Additional MagiCoin cost for Magic Town residential lots (90..=92).
    pub magic_cost: i32,
    /// Occupying family, if any.
    pub occupant: Option<LotOccupant>,
    /// Screen-space anchor near the hovered lot; the popup positions itself
    /// beside this point while clamping inside the viewport below the top bar.
    pub anchor: [i32; 2],
}

/// Read-only view state rendered by [`NeighborhoodChrome`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeighborhoodState {
    /// Active neighborhood slot (`1..=8`).
    pub neighborhood_number: u8,
    pub mode: NeighborhoodMode,
    pub expansions: ExpansionFlags,
    /// Whether an importable family exists in `UserData/Import` (disables the
    /// Import button when `false`, matching 1.91's dimmed button).
    pub has_importable_family: bool,
    /// Whether to draw the Credits billboard button on the bottom-left island.
    pub show_credits_button: bool,
    /// Currently hovered lot, if any, driving the House Info popup card.
    pub hovered_lot: Option<LotInfo>,
}

impl Default for NeighborhoodState {
    fn default() -> Self {
        Self {
            neighborhood_number: MIN_NEIGHBORHOOD,
            mode: NeighborhoodMode::Normal,
            expansions: ExpansionFlags::ALL,
            has_importable_family: false,
            show_credits_button: true,
            hovered_lot: None,
        }
    }
}

impl NeighborhoodState {
    /// Clamps `neighborhood_number` to `1..=8`.
    pub fn clamped_neighborhood(&self) -> u8 {
        self.neighborhood_number
            .clamp(MIN_NEIGHBORHOOD, MAX_NEIGHBORHOOD)
    }

    /// Returns the previous neighborhood number, wrapping `1 -> 8`.
    pub fn previous_neighborhood(&self) -> u8 {
        let n = self.clamped_neighborhood();
        if n <= MIN_NEIGHBORHOOD {
            MAX_NEIGHBORHOOD
        } else {
            n - 1
        }
    }

    /// Returns the next neighborhood number, wrapping `8 -> 1`.
    pub fn next_neighborhood(&self) -> u8 {
        let n = self.clamped_neighborhood();
        if n >= MAX_NEIGHBORHOOD {
            MIN_NEIGHBORHOOD
        } else {
            n + 1
        }
    }
}

/// Buttons on the neighborhood chrome (`0x557459..0x557789`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BarButton {
    PreviousNeighborhood,
    NextNeighborhood,
    SelectFamily,
    ImportFamily,
    Bulldoze,
    Rezone,
    Downtown,
    VacationIsland,
    StudioTown,
    MagicTown,
    Quit,
    Web,
    Credits,
}

impl BarButton {
    /// All top-bar buttons in left-to-right order (excluding the Credits
    /// billboard at the bottom of the map).
    pub const TOP_BAR: [Self; 12] = [
        Self::PreviousNeighborhood,
        Self::NextNeighborhood,
        Self::SelectFamily,
        Self::ImportFamily,
        Self::Bulldoze,
        Self::Rezone,
        Self::Downtown,
        Self::VacationIsland,
        Self::StudioTown,
        Self::MagicTown,
        Self::Quit,
        Self::Web,
    ];

    /// `UIGraphics` resource ID for this button's 4-cell strip.
    pub const fn art_id(self) -> i32 {
        match self {
            Self::PreviousNeighborhood => art_id::PREV_NEIGHBORHOOD,
            Self::NextNeighborhood => art_id::NEXT_NEIGHBORHOOD,
            Self::SelectFamily => art_id::SELECT_FAMILY,
            Self::ImportFamily => art_id::IMPORT_FAMILY,
            Self::Bulldoze => art_id::BULLDOZE,
            Self::Rezone => art_id::REZONE,
            Self::Downtown => art_id::DOWNTOWN,
            Self::VacationIsland => art_id::VACATION,
            Self::StudioTown => art_id::STUDIO_TOWN,
            Self::MagicTown => art_id::MAGIC_TOWN,
            Self::Quit => art_id::QUIT,
            Self::Web => art_id::WEB,
            Self::Credits => art_id::CREDITS,
        }
    }

    /// Default native cell size `(width, height)` in pixels.
    pub const fn native_size(self) -> [i32; 2] {
        match self {
            Self::PreviousNeighborhood | Self::NextNeighborhood => [26, 52],
            Self::Web => [39, 52],
            Self::Credits => [58, 58],
            _ => [53, 52],
        }
    }

    /// Native 800×600 draw origin `(x, y)` from `0x556531..0x55660a`.
    pub const fn native_origin(self) -> [i32; 2] {
        match self {
            Self::PreviousNeighborhood => [106, 0],
            Self::NextNeighborhood => [141, 0],
            Self::SelectFamily => [200, 0],
            Self::ImportFamily => [250, 0],
            Self::Bulldoze => [300, 0],
            Self::Rezone => [350, 0],
            Self::Downtown => [400, 0],
            Self::VacationIsland => [450, 0],
            Self::StudioTown => [500, 0],
            Self::MagicTown => [550, 0],
            Self::Quit => [600, 0],
            Self::Web => [762, 0],
            Self::Credits => [195, 540],
        }
    }

    /// 0-based entry index into `UIText.iff STR# 151` (`0x5574f5..0x557565`).
    pub const fn tooltip_index(self) -> usize {
        match self {
            Self::SelectFamily => 0,
            Self::Bulldoze => 1,
            Self::Quit => 2,
            Self::Web => 3,
            Self::PreviousNeighborhood => 4,
            Self::NextNeighborhood => 5,
            Self::ImportFamily => 6,
            Self::Downtown => 7,
            Self::Credits => 8,
            Self::VacationIsland => 9,
            Self::Rezone => 10,
            Self::StudioTown => 11,
            Self::MagicTown => 12,
        }
    }

    /// Expansion bit required for this button to be enabled, paired with its
    /// 0-based missing-expansion notice index in `UIText.iff STR# 225`.
    pub const fn required_expansion(self) -> Option<(u8, usize)> {
        match self {
            Self::Downtown => Some((ExpansionFlags::HOT_DATE, 0)),
            Self::VacationIsland => Some((ExpansionFlags::VACATION, 1)),
            Self::Rezone => Some((ExpansionFlags::UNLEASHED, 2)),
            Self::StudioTown => Some((ExpansionFlags::SUPERSTAR, 3)),
            Self::MagicTown => Some((ExpansionFlags::MAKIN_MAGIC, 4)),
            _ => None,
        }
    }

    /// Whether this button is currently enabled for `state`.
    pub fn is_enabled(self, state: &NeighborhoodState) -> bool {
        if let Some((bit, _)) = self.required_expansion()
            && !state.expansions.has(bit)
        {
            return false;
        }
        match self {
            Self::ImportFamily => state.has_importable_family,
            // In Bulldoze or Rezone mode, 1.91 disables the other bar buttons
            // except the active toggle (`0x55b432..0x55b520`).
            Self::Bulldoze => !matches!(state.mode, NeighborhoodMode::Rezone),
            Self::Rezone => !matches!(state.mode, NeighborhoodMode::Bulldoze),
            _ => !matches!(
                state.mode,
                NeighborhoodMode::Bulldoze | NeighborhoodMode::Rezone
            ),
        }
    }

    /// Whether this button is drawn in its active/selected toggle state.
    pub fn is_toggled(self, state: &NeighborhoodState) -> bool {
        match self {
            Self::SelectFamily => matches!(state.mode, NeighborhoodMode::MoveIn { .. }),
            Self::Bulldoze => matches!(state.mode, NeighborhoodMode::Bulldoze),
            Self::Rezone => matches!(state.mode, NeighborhoodMode::Rezone),
            _ => false,
        }
    }
}

/// Commands emitted when the player activates a neighborhood chrome button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeighborhoodCommand {
    PreviousNeighborhood,
    NextNeighborhood,
    OpenFamilyBin,
    CancelMoveIn,
    ImportFamily,
    ToggleBulldoze,
    ToggleRezone,
    OpenDowntown,
    OpenVacationIsland,
    OpenStudioTown,
    OpenMagicTown,
    Quit,
    OpenWeb,
    OpenCredits,
}

/// Localized strings read from `GameData/UIText.iff` (`STR#` 134, 151, 225).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NeighborhoodStrings {
    /// `STR# 151`: top-bar button tooltips.
    pub bar: Vec<String>,
    /// `STR# 134`: lot hover / house info popup templates.
    pub popup: Vec<String>,
    /// `STR# 225`: missing-expansion tooltips.
    pub expansions: Vec<String>,
}

impl NeighborhoodStrings {
    /// Loads `STR#` 134, 151, and 225 from the install's `GameData/UIText.iff`
    /// in `language` (1 = US English).
    pub fn load(install: &Install, language: u8) -> Result<Self> {
        let path = install
            .file("GameData/UIText.iff")
            .map_err(Error::Content)?;
        let bytes = std::fs::read(&path).map_err(Error::Io)?;
        Self::from_iff_bytes(bytes, language)
    }

    /// Parses `STR#` 134, 151, and 225 from raw `UIText.iff` bytes.
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
            bar: read_table(BAR_STRINGS)?,
            popup: read_table(POPUP_STRINGS)?,
            expansions: read_table(EXPANSION_STRINGS)?,
        })
    }

    /// Tooltip string for `button`, reflecting missing-expansion state if any.
    pub fn button_tooltip(&self, button: BarButton, state: &NeighborhoodState) -> Option<&str> {
        if let Some((bit, exp_idx)) = button.required_expansion()
            && !state.expansions.has(bit)
            && let Some(msg) = self.expansions.get(exp_idx).filter(|s| !s.is_empty())
        {
            return Some(msg.as_str());
        }
        self.bar
            .get(button.tooltip_index())
            .filter(|s| !s.is_empty())
            .map(String::as_str)
    }

    fn popup_str(&self, index: usize) -> Option<&str> {
        self.popup
            .get(index)
            .filter(|s| !s.is_empty())
            .map(String::as_str)
    }
}

/// Laid-out position and non-overlapping hit rectangle for a single button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonPlacement {
    pub button: BarButton,
    /// Where the button artwork is drawn.
    pub draw_rect: Rect,
    /// Strict non-overlapping click / hover hit rectangle.
    pub hit_rect: Rect,
}

/// Complete layout for the neighborhood top control bar at a given viewport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarLayout {
    pub frame_size: [i32; 2],
    /// Horizontal offset of the 800 px bar inside wider viewports.
    pub bar_rect: Rect,
    /// Optional placement of `TheSimsLogo.bmp` (omitted on very narrow frames).
    pub logo_rect: Option<Rect>,
    /// Placement of the current neighborhood number badge (`Current.bmp`).
    pub badge_rect: Rect,
    /// Laid-out interactive buttons (guaranteed pairwise disjoint `hit_rect`s).
    pub buttons: Vec<ButtonPlacement>,
}

impl BarLayout {
    /// Computes the bar layout for a viewport of `width` × `height`.
    ///
    /// At `width >= 800`, every control matches 1.91's exact pixel coordinates
    /// (`0x556531..0x55660a`), centered horizontally on wider screens. Adjacent
    /// 53 px circular badges at 50 px stride use 50 px hit boxes so hit targets
    /// never overlap. Below `800` px, controls pack evenly into the available
    /// width with strictly disjoint hit boxes.
    pub fn compute(width: u32, height: u32, state: &NeighborhoodState, art: &Art) -> Self {
        let w = i32::try_from(width).unwrap_or(i32::MAX).max(0);
        let h = i32::try_from(height).unwrap_or(i32::MAX).max(0);
        if w == 0 || h == 0 {
            return Self {
                frame_size: [w, h],
                bar_rect: Rect::default(),
                logo_rect: None,
                badge_rect: Rect::default(),
                buttons: Vec::new(),
            };
        }

        let bar_h = BAR_HEIGHT.min(h);
        let mut buttons = Vec::with_capacity(13);

        let (bar_rect, logo_rect, badge_rect) = if w >= NATIVE_WIDTH {
            let ox = (w - NATIVE_WIDTH) / 2;
            let bar_rect = Rect::new(ox, 0, NATIVE_WIDTH, bar_h);
            let logo_rect = Some(Rect::new(ox + 5, 0, 70, bar_h));
            // 0x556e2b..0x556e8c: `Current.bmp` (23×30) is loaded solely to size
            // `[0x6f5890..0x6f589c]` for centering the neighborhood number text.
            // `[0x6cdddc]` = 124 places the 23 px box between Previous (106..132)
            // and Next (141..167), putting `'1'` bit-exact at `x=133..=137, y=12..=23`.
            let (bw, bh) = art
                .image(art_id::CURRENT_BADGE)
                .map_or((23, 30), |img| (img.width as i32, img.height as i32));
            let badge_rect = Rect::new(ox + 124, 3, bw, bh.min(bar_h));

            for &button in &BarButton::TOP_BAR {
                let [bx, by] = button.native_origin();
                let [bw, bh] = button_cell_size(art, button);
                let draw_rect = Rect::new(ox + bx, by, bw, bh.min(bar_h));
                // Adjacent 53 px action badges sit on a 50 px stride; trim 1 px
                // left and 2 px right of the transparent glow margin on the
                // hit box so neighboring hit rects never overlap.
                let hit_rect = if bw == 53 && (200..=600).contains(&bx) {
                    Rect::new(ox + bx + 1, by, 50, bh.min(bar_h))
                } else {
                    let clamped_w = bw.min((ox + NATIVE_WIDTH - (ox + bx)).max(0));
                    Rect::new(ox + bx, by, clamped_w, bh.min(bar_h))
                };
                buttons.push(ButtonPlacement {
                    button,
                    draw_rect,
                    hit_rect,
                });
            }
            (bar_rect, logo_rect, badge_rect)
        } else {
            // Compact non-overlapping layout for widths < 800 px.
            let bar_rect = Rect::new(0, 0, w, bar_h);
            let show_logo = w >= 680;
            let logo_w = if show_logo { 70 } else { 0 };
            let logo_rect = show_logo.then(|| Rect::new(2, 0, logo_w, bar_h));
            let start_x = if show_logo { 74 } else { 4 };
            let badge_w = 22;
            let nav_w = 22.min(((w - start_x) / 16).max(12));
            let prev_x = start_x;
            let badge_x = prev_x + nav_w + 1;
            let next_x = badge_x + badge_w + 1;
            let badge_rect = Rect::new(badge_x, 3, badge_w, 30.min(bar_h));

            let prev_rect = Rect::new(prev_x, 0, nav_w, bar_h);
            let next_rect = Rect::new(next_x, 0, nav_w, bar_h);
            buttons.push(ButtonPlacement {
                button: BarButton::PreviousNeighborhood,
                draw_rect: prev_rect,
                hit_rect: prev_rect,
            });
            buttons.push(ButtonPlacement {
                button: BarButton::NextNeighborhood,
                draw_rect: next_rect,
                hit_rect: next_rect,
            });

            let action_buttons = &BarButton::TOP_BAR[2..];
            let actions_start = next_x + nav_w + 4;
            let avail = (w - actions_start - 2).max(0);
            let count = action_buttons.len() as i32;
            if avail > 0 && count > 0 {
                let slot = (avail / count).max(1);
                for (i, &button) in action_buttons.iter().enumerate() {
                    let x = actions_start + (i as i32) * slot;
                    let slot_w = if i as i32 == count - 1 {
                        (w - x - 2).max(1).min(slot)
                    } else {
                        slot
                    };
                    if x >= w {
                        break;
                    }
                    let clamped_w = slot_w.min(w - x);
                    let r = Rect::new(x, 0, clamped_w, bar_h);
                    buttons.push(ButtonPlacement {
                        button,
                        draw_rect: r,
                        hit_rect: r,
                    });
                }
            }
            (bar_rect, logo_rect, badge_rect)
        };

        if state.show_credits_button && h >= bar_h + 64 {
            let [cw, ch] = button_cell_size(art, BarButton::Credits);
            let ox = if w >= NATIVE_WIDTH {
                (w - NATIVE_WIDTH) / 2
            } else {
                0
            };
            let oy = if h >= NATIVE_HEIGHT {
                (h - NATIVE_HEIGHT) / 2
            } else {
                0
            };
            let cx = if w >= NATIVE_WIDTH {
                ox + 195
            } else {
                (w / 4).min(w - cw).max(0)
            };
            let cy = if h >= NATIVE_HEIGHT {
                oy + 540
            } else {
                (h - ch - 2).max(bar_h)
            };
            let rect = Rect::new(cx, cy, cw.min(w - cx), ch.min(h - cy));
            if rect.width > 0 && rect.height > 0 {
                buttons.push(ButtonPlacement {
                    button: BarButton::Credits,
                    draw_rect: rect,
                    hit_rect: rect,
                });
            }
        }

        Self {
            frame_size: [w, h],
            bar_rect,
            logo_rect,
            badge_rect,
            buttons,
        }
    }

    /// Returns the button whose hit box contains `(x, y)`, if any.
    pub fn hit(&self, x: i32, y: i32) -> Option<BarButton> {
        self.buttons
            .iter()
            .find(|p| p.hit_rect.contains(x, y))
            .map(|p| p.button)
    }

    /// Looks up the placement for `button`.
    pub fn button(&self, button: BarButton) -> Option<ButtonPlacement> {
        self.buttons.iter().copied().find(|p| p.button == button)
    }
}

/// Laid-out geometry and formatted text for the lot hover / house info card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LotPopupLayout {
    /// Outer popup rectangle on screen (clamped inside `[0, w)` × `[BAR_HEIGHT, h)`).
    pub rect: Rect,
    /// Inner text area (`rect` inset by 25 px).
    pub text_rect: Rect,
    /// Wrapped lines to render inside `text_rect`.
    pub lines: Vec<String>,
    /// Screen-space rectangles for each 25×25 occupant portrait.
    pub portrait_rects: Vec<Rect>,
    /// Whether the popup represents an unaffordable lot in Move-In mode.
    pub cannot_afford: bool,
}

impl LotPopupLayout {
    /// Formats and lays out the 1.91 lot hover popup card (`0x54fa2a..0x550c62`).
    pub fn compute(
        lot: &LotInfo,
        state: &NeighborhoodState,
        frame_size: [u32; 2],
        art: &Art,
        strings: &NeighborhoodStrings,
    ) -> Self {
        let (text, cannot_afford) = format_lot_popup_text(lot, state, strings);
        let font = art.font(10).or_else(|| art.font(12));
        let line_height = font.map_or(14, |f| f.line_height.max(1));

        let fw = i32::try_from(frame_size[0]).unwrap_or(i32::MAX).max(0);
        let fh = i32::try_from(frame_size[1]).unwrap_or(i32::MAX).max(0);
        let top_limit = BAR_HEIGHT.min(fh);
        let avail_h = (fh - top_limit).max(0);

        let first_line = text.lines().next().unwrap_or("");
        let max_text_w = (fw - POPUP_PAD.min(fw / 4) * 2).clamp(1, POPUP_MAX_TEXT_W);
        let min_text_w = POPUP_MIN_TEXT_W.min(max_text_w);
        let mut text_w = font
            .map_or((first_line.len() as i32) * 7, |f| f.width(first_line))
            .clamp(min_text_w, max_text_w);

        let portrait_count = lot.occupant.as_ref().map_or(0, |occ| occ.portraits.len());

        // 1.91 aspect-ratio widening loop (`0x5507a2..0x550876`): up to 2 tries,
        // widening `text_w = (text_w * 3) / 2` whenever `total_h >= (total_w * 2) / 3`.
        let mut lines = Vec::new();
        let mut total_w = 0;
        let mut total_h = 0;

        for attempt in 0..=2 {
            text_w = text_w.clamp(min_text_w, max_text_w);
            lines = break_popup_lines(&text, text_w, font);
            let text_h = (lines.len() as i32).max(1).saturating_mul(line_height);
            let pad_x = POPUP_PAD.min((fw - text_w).max(0) / 2);
            let pad_y = POPUP_PAD.min(avail_h / 4);
            total_w = text_w + pad_x * 2;
            total_h = text_h + pad_y * 2;
            if portrait_count > 0 {
                let per_row = ((total_w - pad_x * 2) / POPUP_PORTRAIT_SIZE).max(1) as usize;
                total_h += POPUP_PORTRAIT_GAP;
                if portrait_count <= per_row {
                    total_h += POPUP_PORTRAIT_SIZE;
                } else {
                    total_h += POPUP_PORTRAIT_SIZE * 2;
                }
            }
            if total_h < (total_w * 2) / 3 || attempt == 2 {
                break;
            }
            text_w = (text_w * 3) / 2;
        }

        let clamped_w = total_w.min(fw);
        let clamped_h = total_h.min(avail_h);

        // Position beside the lot anchor, clamping cleanly inside viewport and
        // below the top bar so it never occludes chrome buttons.
        let max_x = (fw - clamped_w).max(0);
        let max_y = (fh - clamped_h).max(top_limit);
        let x = lot.anchor[0].clamp(0, max_x);
        let y = lot.anchor[1].clamp(top_limit, max_y);
        let rect = Rect::new(x, y, clamped_w, clamped_h);
        let pad_x = POPUP_PAD.min(clamped_w / 4);
        let pad_y = POPUP_PAD.min(clamped_h / 4);
        let inner_w = (rect.width - pad_x * 2).max(0);
        if inner_w > 0 && inner_w < text_w {
            lines = break_popup_lines(&text, inner_w, font);
        }

        let mut portrait_rects = Vec::with_capacity(portrait_count);
        let mut reserved_portrait_h = 0;
        if portrait_count > 0 && inner_w >= POPUP_PORTRAIT_SIZE && rect.height > pad_y * 2 {
            let eff_per_row = (inner_w / POPUP_PORTRAIT_SIZE).max(1) as usize;
            let want_rows = if portrait_count > eff_per_row { 2 } else { 1 };
            let min_text_h = line_height.min((rect.height - pad_y * 2).max(0));
            let room_for_portraits =
                (rect.height - pad_y * 2 - min_text_h - POPUP_PORTRAIT_GAP).max(0);
            let fit_rows = (room_for_portraits / POPUP_PORTRAIT_SIZE).min(want_rows);
            if fit_rows > 0 {
                reserved_portrait_h = fit_rows * POPUP_PORTRAIT_SIZE + POPUP_PORTRAIT_GAP;
                let bottom = rect.y + rect.height - pad_y;
                let row0_y = bottom - fit_rows * POPUP_PORTRAIT_SIZE;
                for i in 0..portrait_count.min(eff_per_row * fit_rows as usize) {
                    let row = (i / eff_per_row) as i32;
                    let col = (i % eff_per_row) as i32;
                    let px = rect.x + pad_x + col * POPUP_PORTRAIT_SIZE;
                    let py = row0_y + row * POPUP_PORTRAIT_SIZE;
                    portrait_rects.push(Rect::new(
                        px,
                        py,
                        POPUP_PORTRAIT_SIZE,
                        POPUP_PORTRAIT_SIZE,
                    ));
                }
            }
        }

        let text_rect = Rect::new(
            rect.x + pad_x,
            rect.y + pad_y,
            inner_w,
            (rect.height - pad_y * 2 - reserved_portrait_h).max(0),
        );

        Self {
            rect,
            text_rect,
            lines,
            portrait_rects,
            cannot_afford,
        }
    }
}

/// Interactive state for the neighborhood screen chrome.
#[derive(Debug, Clone, Default)]
pub struct NeighborhoodChrome {
    hovered: Option<BarButton>,
    pressed: Option<BarButton>,
    pointer: [i32; 2],
}

impl NeighborhoodChrome {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn hovered(&self) -> Option<BarButton> {
        self.hovered
    }

    pub fn pressed(&self) -> Option<BarButton> {
        self.pressed
    }

    /// Clears hover and press state (e.g., when the pointer leaves the window).
    pub fn clear_pointer(&mut self) {
        self.hovered = None;
        self.pressed = None;
    }

    /// Updates hover tracking for pointer position `(x, y)`.
    pub fn hover(
        &mut self,
        x: i32,
        y: i32,
        frame_size: [u32; 2],
        state: &NeighborhoodState,
        art: &Art,
    ) -> Option<BarButton> {
        self.pointer = [x, y];
        let layout = BarLayout::compute(frame_size[0], frame_size[1], state, art);
        self.hovered = layout.hit(x, y);
        self.hovered
    }

    /// Records a mouse button press at `(x, y)`. Returns `true` if the chrome
    /// captured the click (including any click inside the top bar strip).
    pub fn press(
        &mut self,
        x: i32,
        y: i32,
        frame_size: [u32; 2],
        state: &NeighborhoodState,
        art: &Art,
    ) -> bool {
        self.pointer = [x, y];
        let layout = BarLayout::compute(frame_size[0], frame_size[1], state, art);
        let hit = layout.hit(x, y);
        self.hovered = hit;
        if let Some(btn) = hit {
            if btn.is_enabled(state) {
                self.pressed = Some(btn);
            }
            return true;
        }
        layout.bar_rect.contains(x, y)
    }

    /// Completes a mouse button release at `(x, y)`, emitting a
    /// [`NeighborhoodCommand`] when released over the same enabled button that
    /// was pressed.
    pub fn release(
        &mut self,
        x: i32,
        y: i32,
        frame_size: [u32; 2],
        state: &NeighborhoodState,
        art: &Art,
    ) -> Option<NeighborhoodCommand> {
        self.pointer = [x, y];
        let pressed = self.pressed.take();
        let layout = BarLayout::compute(frame_size[0], frame_size[1], state, art);
        let hit = layout.hit(x, y);
        self.hovered = hit;
        let btn = pressed.filter(|&b| Some(b) == hit && b.is_enabled(state))?;
        Some(command_for_button(btn, state))
    }

    /// Draws the neighborhood top bar, current neighborhood `1..=8` badge,
    /// active/hovered/disabled controls, optional lot hover popup card, and
    /// hover tooltip into `frame`.
    pub fn draw(
        &self,
        frame: &mut Frame,
        state: &NeighborhoodState,
        art: &Art,
        strings: &NeighborhoodStrings,
    ) -> BarLayout {
        let layout = BarLayout::compute(frame.width(), frame.height(), state, art);
        if layout.bar_rect.width == 0 || layout.bar_rect.height == 0 {
            return layout;
        }

        // 1. Top bar banner background.
        if let Some(bg) = art.image(art_id::BAR_BKG) {
            if layout.bar_rect.width >= bg.width as i32 {
                frame.draw(
                    bg,
                    Rect::new(0, 0, bg.width as i32, layout.bar_rect.height),
                    [layout.bar_rect.x, layout.bar_rect.y],
                );
            } else {
                draw_stretched_horizontal(frame, bg, layout.bar_rect);
            }
        } else {
            frame.fill(layout.bar_rect, BAR_FALLBACK_BG);
        }

        // 2. Decorative "The Sims" logo (`TheSimsLogo.bmp`, 1 cell).
        if let Some(logo_rect) = layout.logo_rect
            && let Some(logo) = art.image(art_id::SIMS_LOGO)
        {
            frame.draw(
                logo,
                Rect::new(0, 0, logo_rect.width, logo_rect.height),
                [logo_rect.x, logo_rect.y],
            );
        }

        // 3. Current neighborhood number (`1..=8`, `0x55d062..0x55d102`).
        //    `cWinNeighborhood::DoDraw` never blits `Current.bmp` (`[esi+0x138]`);
        //    it only uses its 23×30 dimensions to center the white digit directly
        //    over `banner_neighborhood.bmp` between the Previous and Next arrows.
        let num_text = state.clamped_neighborhood().to_string();
        if let Some(font) = art.font(12).or_else(|| art.font(10)) {
            let tw = font.width(&num_text);
            let tx = layout.badge_rect.x + (layout.badge_rect.width / 2) - 1 - tw / 2;
            let ty =
                layout.badge_rect.y + (layout.badge_rect.height / 2) - 1 - font.line_height / 2;
            font.draw(frame, &num_text, [tx, ty], TEXT_WHITE);
        }

        // 4. All interactive bar buttons + Credits billboard.
        let compact_width = layout.frame_size[0] < NATIVE_WIDTH;
        for placement in &layout.buttons {
            let btn = placement.button;
            let enabled = btn.is_enabled(state);
            let cell = if !enabled {
                3
            } else if btn.is_toggled(state) || self.pressed == Some(btn) {
                1
            } else if self.hovered == Some(btn) {
                2
            } else {
                0
            };
            draw_four_cell_button(
                frame,
                art,
                btn.art_id(),
                cell,
                placement.draw_rect,
                compact_width,
            );
        }

        // 5. Lot hover / House Info popup card.
        if let Some(lot) = &state.hovered_lot {
            draw_hovered_lot_popup(frame, lot, state, art, strings);
        }

        // 6. Hover tooltip over top-bar buttons (drawn last so it stays visible).
        if let Some(hovered) = self.hovered
            && let Some(tip) = strings.button_tooltip(hovered, state)
            && let Some(placement) = layout.button(hovered)
        {
            let anchor_y = if placement.hit_rect.y > i32::try_from(frame.height()).unwrap_or(0) / 2
            {
                placement.hit_rect.y - 20
            } else {
                placement.hit_rect.y + placement.hit_rect.height + 4
            };
            draw_tooltip(
                frame,
                art,
                tip,
                [
                    placement.hit_rect.x + placement.hit_rect.width / 2,
                    anchor_y,
                ],
            );
        }

        layout
    }
}

/// Draws the native House Info / Community Lot hover popup card into `frame`
/// for `lot`, reusing the same [`LotPopupLayout`] geometry as
/// [`NeighborhoodChrome::draw`].
pub fn draw_hovered_lot_popup(
    frame: &mut Frame,
    lot: &LotInfo,
    state: &NeighborhoodState,
    art: &Art,
    strings: &NeighborhoodStrings,
) {
    let popup = LotPopupLayout::compute(lot, state, [frame.width(), frame.height()], art, strings);
    draw_lot_popup(frame, &popup, lot, art);
}

fn command_for_button(button: BarButton, state: &NeighborhoodState) -> NeighborhoodCommand {
    match button {
        BarButton::PreviousNeighborhood => NeighborhoodCommand::PreviousNeighborhood,
        BarButton::NextNeighborhood => NeighborhoodCommand::NextNeighborhood,
        BarButton::SelectFamily => {
            if matches!(state.mode, NeighborhoodMode::MoveIn { .. }) {
                NeighborhoodCommand::CancelMoveIn
            } else {
                NeighborhoodCommand::OpenFamilyBin
            }
        }
        BarButton::ImportFamily => NeighborhoodCommand::ImportFamily,
        BarButton::Bulldoze => NeighborhoodCommand::ToggleBulldoze,
        BarButton::Rezone => NeighborhoodCommand::ToggleRezone,
        BarButton::Downtown => NeighborhoodCommand::OpenDowntown,
        BarButton::VacationIsland => NeighborhoodCommand::OpenVacationIsland,
        BarButton::StudioTown => NeighborhoodCommand::OpenStudioTown,
        BarButton::MagicTown => NeighborhoodCommand::OpenMagicTown,
        BarButton::Quit => NeighborhoodCommand::Quit,
        BarButton::Web => NeighborhoodCommand::OpenWeb,
        BarButton::Credits => NeighborhoodCommand::OpenCredits,
    }
}

fn button_cell_size(art: &Art, button: BarButton) -> [i32; 2] {
    if let Some(img) = art.image(button.art_id())
        && img.width >= 4
        && img.height > 0
    {
        return [(img.width / 4) as i32, img.height as i32];
    }
    button.native_size()
}

fn draw_four_cell_button(
    frame: &mut Frame,
    art: &Art,
    id: i32,
    cell: u32,
    dest: Rect,
    preserve_aspect: bool,
) {
    if dest.width <= 0 || dest.height <= 0 {
        return;
    }
    if let Some(img) = art.image(id)
        && img.width >= 4
        && img.height > 0
    {
        let cw = (img.width / 4) as i32;
        let ch = img.height as i32;
        let sx = cw.saturating_mul(cell.min(3) as i32);
        if dest.width == cw && dest.height == ch {
            frame.draw(img, Rect::new(sx, 0, cw, ch), [dest.x, dest.y]);
        } else if preserve_aspect {
            // Preserve aspect ratio within `dest` for compact viewports < 800 px
            // so circular neighborhood buttons stay round instead of squishing
            // into vertical slivers.
            let (fit_w, fit_h) = if i64::from(dest.width) * i64::from(ch)
                <= i64::from(dest.height) * i64::from(cw)
            {
                let fw = dest.width;
                let fh = ((i64::from(dest.width) * i64::from(ch) + i64::from(cw) / 2)
                    / i64::from(cw))
                .clamp(1, i64::from(dest.height)) as i32;
                (fw, fh)
            } else {
                let fh = dest.height;
                let fw = ((i64::from(dest.height) * i64::from(cw) + i64::from(ch) / 2)
                    / i64::from(ch))
                .clamp(1, i64::from(dest.width)) as i32;
                (fw, fh)
            };
            let fit_dest = Rect::new(
                dest.x + (dest.width - fit_w) / 2,
                dest.y + (dest.height - fit_h) / 2,
                fit_w,
                fit_h,
            );
            draw_scaled_region(frame, img, Rect::new(sx, 0, cw, ch), fit_dest);
        } else {
            draw_scaled_region(frame, img, Rect::new(sx, 0, cw, ch), dest);
        }
    } else {
        let color = match cell {
            1 => [0, 180, 220],
            2 => [80, 120, 210],
            3 => [48, 56, 88],
            _ => [40, 64, 156],
        };
        frame.fill(dest, color);
    }
}

fn draw_stretched_horizontal(frame: &mut Frame, image: &Image, dest: Rect) {
    if image.width == 0 || image.height == 0 || dest.width <= 0 || dest.height <= 0 {
        return;
    }
    draw_scaled_region(
        frame,
        image,
        Rect::new(0, 0, image.width as i32, image.height as i32),
        dest,
    );
}

fn draw_scaled_region(frame: &mut Frame, image: &Image, src: Rect, dest: Rect) {
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
                && let Some([r, g, b, a]) = image.pixel(ux, uy)
            {
                frame.blend(x, y, [r, g, b], a);
            }
        }
    }
}

/// Draws the 1.91 9-slice alpha popup (`HotspotPopupTiler.TGA`, 111×111 ->
/// 3×3 tiles of 37×37, `0x5509cf..0x550c62`) plus formatted lines and portraits.
fn draw_lot_popup(frame: &mut Frame, popup: &LotPopupLayout, lot: &LotInfo, art: &Art) {
    if popup.rect.width <= 0 || popup.rect.height <= 0 {
        return;
    }
    if let Some(tiler) = art.image(art_id::POPUP_TILER)
        && tiler.width >= 3
        && tiler.height >= 3
    {
        draw_nine_slice(frame, tiler, popup.rect);
    } else {
        for y in popup.rect.y..popup.rect.y.saturating_add(popup.rect.height) {
            for x in popup.rect.x..popup.rect.x.saturating_add(popup.rect.width) {
                frame.blend(x, y, POPUP_FALLBACK_BG, 210);
            }
        }
    }

    if let Some(font) = art.font(10).or_else(|| art.font(12)) {
        let lh = font.line_height.max(1);
        let max_y = popup.text_rect.y.saturating_add(popup.text_rect.height);
        for (i, line) in popup.lines.iter().enumerate() {
            let y = popup
                .text_rect
                .y
                .saturating_add((i as i32).saturating_mul(lh));
            if y.saturating_add(lh) > max_y {
                break;
            }
            font.draw(frame, line, [popup.text_rect.x, y], TEXT_NORMAL);
        }
    }

    if let Some(occ) = &lot.occupant {
        for (img, &rect) in occ.portraits.iter().zip(&popup.portrait_rects) {
            draw_scaled_region(
                frame,
                img,
                Rect::new(0, 0, img.width as i32, img.height as i32),
                rect,
            );
            draw_outline(frame, rect, PORTRAIT_BORDER);
        }
    }
}

/// Renders a 3×3 tiled image across `dest`, dividing the source evenly into
/// three horizontal and three vertical bands (`0x5509cf..0x550a20`).
pub(crate) fn draw_nine_slice(frame: &mut Frame, tiler: &Image, dest: Rect) {
    let tw = (tiler.width / 3) as i32;
    let th = (tiler.height / 3) as i32;
    if tw <= 0 || th <= 0 || dest.width <= 0 || dest.height <= 0 {
        return;
    }
    let left_w = tw.min(dest.width / 2);
    let right_w = tw.min(dest.width - left_w);
    let mid_w = (dest.width - left_w - right_w).max(0);
    let top_h = th.min(dest.height / 2);
    let bot_h = th.min(dest.height - top_h);
    let mid_h = (dest.height - top_h - bot_h).max(0);

    let sx = [0, tw, (tiler.width as i32) - right_w];
    let sy = [0, th, (tiler.height as i32) - bot_h];
    let dx = [dest.x, dest.x + left_w, dest.x + left_w + mid_w];
    let dy = [dest.y, dest.y + top_h, dest.y + top_h + mid_h];
    let dw = [left_w, mid_w, right_w];
    let dh = [top_h, mid_h, bot_h];

    // Corners
    frame.draw(tiler, Rect::new(sx[0], sy[0], dw[0], dh[0]), [dx[0], dy[0]]);
    frame.draw(tiler, Rect::new(sx[2], sy[0], dw[2], dh[0]), [dx[2], dy[0]]);
    frame.draw(tiler, Rect::new(sx[0], sy[2], dw[0], dh[2]), [dx[0], dy[2]]);
    frame.draw(tiler, Rect::new(sx[2], sy[2], dw[2], dh[2]), [dx[2], dy[2]]);

    // Top and bottom edges (tiled horizontally)
    let mut x = 0;
    while x < mid_w {
        let seg = tw.min(mid_w - x);
        frame.draw(
            tiler,
            Rect::new(sx[1], sy[0], seg, dh[0]),
            [dx[1] + x, dy[0]],
        );
        frame.draw(
            tiler,
            Rect::new(sx[1], sy[2], seg, dh[2]),
            [dx[1] + x, dy[2]],
        );
        x += seg;
    }

    // Left and right edges (tiled vertically)
    let mut y = 0;
    while y < mid_h {
        let seg = th.min(mid_h - y);
        frame.draw(
            tiler,
            Rect::new(sx[0], sy[1], dw[0], seg),
            [dx[0], dy[1] + y],
        );
        frame.draw(
            tiler,
            Rect::new(sx[2], sy[1], dw[2], seg),
            [dx[2], dy[1] + y],
        );
        y += seg;
    }

    // Center fill (tiled in both axes)
    let mut cy = 0;
    while cy < mid_h {
        let seg_h = th.min(mid_h - cy);
        let mut cx = 0;
        while cx < mid_w {
            let seg_w = tw.min(mid_w - cx);
            frame.draw(
                tiler,
                Rect::new(sx[1], sy[1], seg_w, seg_h),
                [dx[1] + cx, dy[1] + cy],
            );
            cx += seg_w;
        }
        cy += seg_h;
    }
}

pub(crate) fn draw_outline(frame: &mut Frame, rect: Rect, color: [u8; 3]) {
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

/// Draws a classic 1.91 hover tooltip box (`#FFFFE6` fill, 1 px black border,
/// black `variablesans_10` ink), clamped cleanly inside `frame`.
pub(crate) fn draw_tooltip(frame: &mut Frame, art: &Art, text: &str, anchor: [i32; 2]) {
    if text.is_empty() {
        return;
    }
    let Some(font) = art.font(10).or_else(|| art.font(12)) else {
        return;
    };
    let pad_x = 4;
    let pad_y = 2;
    let tw = font.width(text);
    let w = tw + pad_x * 2 + 2;
    let h = font.line_height + pad_y * 2 + 2;
    let fw = i32::try_from(frame.width()).unwrap_or(i32::MAX);
    let fh = i32::try_from(frame.height()).unwrap_or(i32::MAX);
    let x = (anchor[0] - w / 2).clamp(0, (fw - w).max(0));
    let y = anchor[1].clamp(0, (fh - h).max(0));
    let rect = Rect::new(x, y, w.min(fw), h.min(fh));
    frame.fill(rect, TOOLTIP_BG);
    draw_outline(frame, rect, TOOLTIP_BORDER);
    font.draw(
        frame,
        text,
        [rect.x + 1 + pad_x, rect.y + 1 + pad_y],
        TOOLTIP_TEXT,
    );
}

/// Formats a Simoleon integer with `$` prefix (which renders as the Simoleon
/// glyph in `variablesans_*.ffn`) and thousands separators.
pub fn format_simoleons(amount: i32) -> String {
    let negative = amount < 0;
    let abs = amount.unsigned_abs().to_string();
    let mut grouped = String::with_capacity(abs.len() + abs.len() / 3 + 2);
    if negative {
        grouped.push('-');
    }
    grouped.push('$');
    let len = abs.len();
    for (i, ch) in abs.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    grouped
}

/// Formats the lot's street address from `lot_number` using the 1.91 `STR# 134`
/// ranges (`0x54faaa..0x54ffe4`).
pub fn format_lot_address(lot: &LotInfo, strings: &NeighborhoodStrings) -> String {
    if let Some(addr) = &lot.address {
        return addr.clone();
    }
    let num = lot.lot_number.to_string();
    let template_idx = match lot.lot_number {
        21..=23 => 13, // Wright Way
        24..=31 => 14, // Main Street
        40..=42 => 15, // Old Volcano Way
        43..=49 => 16, // Lagoon Moon Road
        81..=83 => 24, // Sunrise Boulevard
        84..=89 => 25, // Studio Town Drive
        93..=99 => 27, // Calamity Forest
        _ => 7,        // Sim Lane (1..=10, 50..=80, 90..=92)
    };
    if let Some(tmpl) = strings.popup_str(template_idx) {
        format_template(tmpl, &[&num])
    } else {
        num
    }
}

/// Formats the full popup body for `lot` under `state` using `STR# 134`
/// (`0x550422..0x550723`, `0x550d85..0x551060`). Also returns whether the
/// hovered lot is unaffordable in Move-In mode.
pub fn format_lot_popup_text(
    lot: &LotInfo,
    state: &NeighborhoodState,
    strings: &NeighborhoodStrings,
) -> (String, bool) {
    let address = format_lot_address(lot, strings);
    let mut cannot_afford = false;

    let body = match (&lot.kind, &lot.occupant, &state.mode) {
        // Tutorial lot (`0x550deb..0x550e10`).
        (LotKind::Tutorial, _, NeighborhoodMode::MoveIn { .. }) => {
            strings.popup_str(8).unwrap_or("Tutorial").to_owned()
        }
        (LotKind::Tutorial, _, NeighborhoodMode::Bulldoze) => {
            strings.popup_str(12).unwrap_or("Tutorial").to_owned()
        }
        (LotKind::Tutorial, _, _) => strings.popup_str(6).unwrap_or("Tutorial").to_owned(),

        // Community lots (`0x550344`, `0x550dc2`, `0x550e34`, `0x550e6e`).
        (LotKind::Community | LotKind::MagicCommunity, _, NeighborhoodMode::Visit) => {
            format_or_fallback(
                strings.popup_str(21),
                &[&lot.house_name, &lot.house_description],
            )
        }
        (LotKind::Community | LotKind::MagicCommunity, _, NeighborhoodMode::MoveIn { .. }) => {
            cannot_afford = true;
            format_or_fallback(
                strings.popup_str(18),
                &[&lot.house_name, &lot.house_description],
            )
        }
        (LotKind::Community | LotKind::MagicCommunity, _, NeighborhoodMode::Rezone) => {
            format_or_fallback(
                strings.popup_str(19),
                &[&lot.house_name, &lot.house_description],
            )
        }
        (LotKind::Community | LotKind::MagicCommunity, _, NeighborhoodMode::Bulldoze) => {
            let idx = if lot.has_house { 10 } else { 11 };
            strings.popup_str(idx).unwrap_or("").to_owned()
        }
        (LotKind::Community | LotKind::MagicCommunity, _, NeighborhoodMode::Normal) => {
            format_or_fallback(
                strings.popup_str(17),
                &[&lot.house_name, &lot.house_description],
            )
        }

        // Occupied Magic residential lots (`0x550efe..0x551053`).
        (LotKind::MagicResidential, Some(occ), mode) => {
            let net = format_simoleons(occ.net_worth);
            let mc = occ.magic_coins.to_string();
            let fr = occ.friends.to_string();
            let idx = if matches!(mode, NeighborhoodMode::Bulldoze | NeighborhoodMode::Rezone) {
                35
            } else {
                34
            };
            format_or_fallback(strings.popup_str(idx), &[&occ.family_name, &net, &mc, &fr])
        }

        // Occupied standard residential lots (`0x550433`, `0x550edd..0x550eec`).
        (LotKind::Residential, Some(occ), mode) => {
            let net = format_simoleons(occ.net_worth);
            let fr = occ.friends.to_string();
            let idx = if matches!(mode, NeighborhoodMode::Bulldoze | NeighborhoodMode::Rezone) {
                9
            } else {
                0
            };
            format_or_fallback(strings.popup_str(idx), &[&occ.family_name, &net, &fr])
        }

        // Vacant residential lots in Visit mode (`0x550dde`, `0x550f46`).
        (LotKind::MagicResidential, None, NeighborhoodMode::Visit) => format_or_fallback(
            strings.popup_str(28),
            &[&lot.house_name, &lot.house_description],
        ),
        (LotKind::Residential, None, NeighborhoodMode::Visit) => format_or_fallback(
            strings.popup_str(20),
            &[&lot.house_name, &lot.house_description],
        ),

        // Vacant residential lots in Bulldoze / Rezone (`0x550e47..0x550e58`).
        (_, None, NeighborhoodMode::Rezone) => format_or_fallback(
            strings.popup_str(19),
            &[&lot.house_name, &lot.house_description],
        ),
        (_, None, NeighborhoodMode::Bulldoze) => {
            let idx = if lot.has_house { 10 } else { 11 };
            strings.popup_str(idx).unwrap_or("").to_owned()
        }

        // Vacant Magic residential lots in Move-In mode (`0x550ff5..0x551026`).
        (
            LotKind::MagicResidential,
            None,
            NeighborhoodMode::MoveIn {
                family_name,
                family_funds,
                family_magic_coins,
            },
        ) => {
            let cost = format_simoleons(lot.house_cost);
            let mcost = lot.magic_cost.to_string();
            let has_money = *family_funds >= lot.house_cost;
            let has_magic = *family_magic_coins >= lot.magic_cost;
            match (has_money, has_magic) {
                (true, true) => format_or_fallback(
                    strings.popup_str(31),
                    &[&lot.house_name, &lot.house_description, &cost, &mcost],
                ),
                (true, false) => {
                    cannot_afford = true;
                    let fmc = family_magic_coins.to_string();
                    format_or_fallback(
                        strings.popup_str(36),
                        &[
                            &lot.house_name,
                            &lot.house_description,
                            &cost,
                            &mcost,
                            family_name,
                            &fmc,
                        ],
                    )
                }
                (false, false) => {
                    cannot_afford = true;
                    let fmc = family_magic_coins.to_string();
                    format_or_fallback(
                        strings.popup_str(37),
                        &[
                            &lot.house_name,
                            &lot.house_description,
                            &cost,
                            &mcost,
                            family_name,
                            &fmc,
                        ],
                    )
                }
                (false, true) => {
                    cannot_afford = true;
                    let ff = format_simoleons(*family_funds);
                    format_or_fallback(
                        strings.popup_str(38),
                        &[
                            &lot.house_name,
                            &lot.house_description,
                            &cost,
                            &mcost,
                            family_name,
                            &ff,
                        ],
                    )
                }
            }
        }

        // Vacant standard residential lots in Move-In mode (`0x550eb5..0x550edb`).
        (
            LotKind::Residential,
            None,
            NeighborhoodMode::MoveIn {
                family_name,
                family_funds,
                ..
            },
        ) => {
            let cost = format_simoleons(lot.house_cost);
            if *family_funds >= lot.house_cost {
                let remaining = format_simoleons(family_funds.saturating_sub(lot.house_cost));
                let idx = if lot.has_house { 3 } else { 5 };
                format_or_fallback(
                    strings.popup_str(idx),
                    &[&lot.house_name, &lot.house_description, &cost, &remaining],
                )
            } else {
                cannot_afford = true;
                let funds = format_simoleons(*family_funds);
                let idx = if lot.has_house { 2 } else { 4 };
                format_or_fallback(
                    strings.popup_str(idx),
                    &[
                        &lot.house_name,
                        &lot.house_description,
                        &cost,
                        family_name,
                        &funds,
                    ],
                )
            }
        }

        // Vacant Magic residential lots in Normal mode (`0x550faa`).
        (LotKind::MagicResidential, None, NeighborhoodMode::Normal) => {
            let cost = format_simoleons(lot.house_cost);
            let mcost = lot.magic_cost.to_string();
            format_or_fallback(
                strings.popup_str(29),
                &[&lot.house_name, &lot.house_description, &cost, &mcost],
            )
        }

        // Vacant standard residential lots in Normal mode (`0x5504af`).
        (LotKind::Residential, None, NeighborhoodMode::Normal) => {
            let cost = format_simoleons(lot.house_cost);
            format_or_fallback(
                strings.popup_str(1),
                &[&lot.house_name, &lot.house_description, &cost],
            )
        }
    };

    let full = if body.is_empty() {
        address
    } else {
        format!("{address}\n\n{body}")
    };
    (full, cannot_afford)
}

fn format_or_fallback(template: Option<&str>, args: &[&str]) -> String {
    match template {
        Some(tmpl) => format_template(tmpl, args),
        None => args
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Substitutes `%s` and `%d` printf placeholders sequentially without panicking
/// on missing or extra arguments, normalizing CRLF to LF.
fn format_template(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut chars = template.chars().peekable();
    let mut arg_idx = 0usize;
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            continue;
        }
        if ch == '%'
            && let Some(&next) = chars.peek()
        {
            match next {
                's' | 'd' | 'i' | 'u' => {
                    chars.next();
                    if let Some(val) = args.get(arg_idx) {
                        out.push_str(val);
                    }
                    arg_idx += 1;
                    continue;
                }
                '%' => {
                    chars.next();
                    out.push('%');
                    continue;
                }
                _ => {}
            }
        }
        out.push(ch);
    }
    out
}

fn break_popup_lines(text: &str, width: i32, font: Option<&Font>) -> Vec<String> {
    let mut out = Vec::new();
    for raw_line in text.split('\n') {
        if raw_line.is_empty() {
            out.push(String::new());
            continue;
        }
        if let Some(f) = font {
            out.extend(f.break_lines(raw_line, width));
        } else {
            out.push(raw_line.to_owned());
        }
    }
    out
}
