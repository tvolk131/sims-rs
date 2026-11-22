//! Family Bin ("Select a Family") screen, laid out and drawn as Complete
//! Collection 1.91 does.
//!
//! Coordinates, artwork IDs, colors, and interactions come directly from
//! `UIGraphics/Res_Nbhd.h`, `Res_Nbhd.RT`, `GameData/UIText.iff` (`STR#` 128),
//! and the 1.91 disassembly of `cWinPickFamily` (`0x563284..0x5642c2`),
//! `PickFamilyList` (`0x543918..0x544220`), and `PickFamilyItem`
//! (`0x5429a6..0x543918`).
//!
//! This module only consumes read-only [`FamilyEntry`] values and emits
//! [`FamilyBinCommand`]s, keeping it cleanly decoupled from REDB persistence
//! and neighborhood map rendering.

use crate::art::Art;
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::neighborhood::{draw_outline, draw_tooltip, format_simoleons};
use crate::{Error, Result};
use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// `GameData/UIText.iff` string table for the Family Bin (`0x56354c`).
pub const STRINGS: u16 = 128;

/// `UIGraphics/Res_Nbhd.h` resource identifiers used by the Family Bin screen.
pub mod art_id {
    /// `Nbhd/PickBkg.bmp` (800×600, standard neighborhood background).
    pub const PICK_BKG: i32 = 5041;
    /// `Nbhd/PickBkgMagic.bmp` (800×600, Magic Town background with MagiCoins column).
    pub const PICK_BKG_MAGIC: i32 = 5043;
    /// `Nbhd/FamilyPickItemBkg.bmp` (690×204 = 3 vertical states of 690×68).
    pub const ITEM_BKG: i32 = 5019;
    /// `Nbhd/ScrollBar.BMP` (300×30, scrollbar widget strip).
    pub const SCROLLBAR: i32 = 5018;
    /// `Nbhd/PAFAddFamily.bmp` (368×62, 4 cells of 92×62).
    pub const BTN_CREATE: i32 = 5103;
    /// `Nbhd/PAFDeleteFamily.bmp` (368×62, 4 cells of 92×62).
    pub const BTN_DELETE: i32 = 5105;
    /// `Nbhd/PAFMoveIn.bmp` (656×62, 4 cells of 164×62).
    pub const BTN_MOVE_IN: i32 = 5106;
    /// `Nbhd/NbhdTileBtn.BMP` (364×62, tiled Cancel button artwork).
    pub const BTN_DEFAULT: i32 = 5301;
}

/// Every `UIGraphics` resource ID used by the Family Bin screen.
pub const RESOURCES: [i32; 8] = [
    art_id::PICK_BKG,
    art_id::PICK_BKG_MAGIC,
    art_id::ITEM_BKG,
    art_id::SCROLLBAR,
    art_id::BTN_CREATE,
    art_id::BTN_DELETE,
    art_id::BTN_MOVE_IN,
    art_id::BTN_DEFAULT,
];

/// Native 1.91 screen size (800×600).
pub const NATIVE_WIDTH: i32 = 800;
pub const NATIVE_HEIGHT: i32 = 600;
/// Number of family rows visible simultaneously (`0x543c63`).
pub const VISIBLE_ROWS: usize = 6;
/// Native dimensions of one family card row (`FamilyPickItemBkg.bmp`: 690×204 / 3).
pub const ROW_WIDTH: i32 = 690;
pub const ROW_HEIGHT: i32 = 68;
/// Maximum member portraits drawn on one family card (`0x542c78`).
pub const MAX_PORTRAITS: usize = 8;

/// Standard light text (`0x6f2a94`).
const TEXT_NORMAL: [u8; 3] = [195, 205, 205];
/// Hovered row text (`0x6f2a98`).
const TEXT_HOVER: [u8; 3] = [255, 255, 255];
/// Selected row text (`0x6f2a9c`).
const TEXT_SELECTED: [u8; 3] = [0, 255, 255];
/// Drop shadow color (`0x6f2aa4`).
const TEXT_SHADOW: [u8; 3] = [0, 0, 82];
/// Letterbox / fallback background blue.
const BKG_FALLBACK: [u8; 3] = [0, 8, 72];
/// Portrait border highlight (light steel blue, matching `10-family-selected.png`).
const PORTRAIT_FRAME: [u8; 3] = [180, 190, 220];
/// Portrait interior background.
const PORTRAIT_BG: [u8; 3] = [8, 16, 68];
/// Scrollbar track / thumb fallback colors.
const SCROLL_TRACK_BG: [u8; 3] = [12, 22, 86];
const SCROLL_THUMB_BG: [u8; 3] = [66, 118, 192];
const SCROLL_THUMB_HOVER: [u8; 3] = [96, 158, 232];
const SCROLL_ARROW_BG: [u8; 3] = [36, 68, 148];

/// Loads all Family Bin bitmaps and fonts (including `variablesans_16.ffn` for
/// the title header at `0x563672`) from `ui`.
pub fn load_art(ui: &UiGraphics, language: u8) -> Result<Art> {
    let mut art = Art::load(ui, language, &RESOURCES)?;
    if let Ok(bytes) = ui.read("Fonts/variablesans_16.ffn")
        && let Ok(parsed) = sims_formats::ffn::Font::parse(&bytes)
    {
        art.insert_font(16, Font::new(parsed));
    }
    Ok(art)
}

/// Display variant of the Family Bin (`[esi+0x159]` in `0x563497`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FamilyBinMode {
    /// Standard neighborhood family picker (`PickBkg.bmp`: Net Worth & Friends).
    #[default]
    Standard,
    /// Magic Town family picker (`PickBkgMagic.bmp`: adds MagiCoins column).
    MagicTown,
}

/// One Sim portrait shown inside a family row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FamilyMemberPortrait {
    pub name: String,
    pub portrait: Option<Image>,
}

/// Read-only household entry shown in the Family Bin list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FamilyEntry {
    pub id: u32,
    pub name: String,
    pub funds: i32,
    pub friends: i32,
    pub magic_coins: i32,
    pub members: Vec<FamilyMemberPortrait>,
}

/// Read-only view data supplied to [`FamilyBin`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FamilyBinView<'a> {
    pub mode: FamilyBinMode,
    pub families: &'a [FamilyEntry],
}

/// Bottom action buttons on the Family Bin screen (`0x563755..0x5637a2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FamilyBinButton {
    Cancel,
    CreateNewFamily,
    DeleteFamily,
    MoveInFamily,
}

impl FamilyBinButton {
    pub const ALL: [Self; 4] = [
        Self::Cancel,
        Self::CreateNewFamily,
        Self::DeleteFamily,
        Self::MoveInFamily,
    ];

    /// Native 800×600 button rectangle (`0x5632b3..0x5632f9` + `0x563cf9..0x563e16`).
    pub const fn native_rect(self) -> Rect {
        match self {
            // Centered on x=200 (`0xc8`), y=529 (`0x211`), 120×62 (measured in
            // 10-family-selected.png: x=141..260, y=531..590).
            Self::Cancel => Rect::new(140, 529, 120, 62),
            Self::CreateNewFamily => Rect::new(322, 530, 92, 62),
            Self::DeleteFamily => Rect::new(414, 530, 92, 62),
            Self::MoveInFamily => Rect::new(506, 530, 164, 62),
        }
    }

    pub const fn art_id(self) -> i32 {
        match self {
            Self::Cancel => art_id::BTN_DEFAULT,
            Self::CreateNewFamily => art_id::BTN_CREATE,
            Self::DeleteFamily => art_id::BTN_DELETE,
            Self::MoveInFamily => art_id::BTN_MOVE_IN,
        }
    }

    pub const fn is_enabled(self, has_selection: bool) -> bool {
        match self {
            Self::Cancel | Self::CreateNewFamily => true,
            Self::DeleteFamily | Self::MoveInFamily => has_selection,
        }
    }
}

/// Column header icons with hover tooltips (`0x56383a..0x5639cd`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColumnHeader {
    NetWorth,
    Friends,
    MagicCoins,
}

/// Commands emitted by [`FamilyBin`] interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FamilyBinCommand {
    /// Highlighted a household row (single click).
    SelectFamily(u32),
    /// Activated "Create New Family".
    CreateNewFamily,
    /// Activated "Move In Family" (button click or row double-click).
    MoveInFamily(u32),
    /// Activated "Delete Family" for the currently selected household.
    DeleteFamily(u32),
    /// Cancelled and returned to the neighborhood screen.
    Cancel,
}

/// Localized strings read from `GameData/UIText.iff` `STR# 128`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FamilyBinStrings {
    pub entries: Vec<String>,
}

impl FamilyBinStrings {
    /// Loads `STR# 128` from the install's `GameData/UIText.iff`.
    pub fn load(install: &Install, language: u8) -> Result<Self> {
        let path = install
            .file("GameData/UIText.iff")
            .map_err(Error::Content)?;
        let bytes = std::fs::read(&path).map_err(Error::Io)?;
        Self::from_iff_bytes(bytes, language)
    }

    /// Parses `STR# 128` from raw `UIText.iff` bytes.
    pub fn from_iff_bytes(bytes: Vec<u8>, language: u8) -> Result<Self> {
        let iff = Iff::parse(bytes).map_err(Error::Format)?;
        let Some(chunk) = iff.get(b"STR#", STRINGS) else {
            return Ok(Self::default());
        };
        let table = StringTable::parse(iff.data(chunk)).map_err(Error::Format)?;
        let entries = (0..table.entries.len())
            .filter_map(|i| {
                table
                    .get(i, language)
                    .or_else(|| table.get(i, 1))
                    .map(str::to_owned)
            })
            .collect();
        Ok(Self { entries })
    }

    fn get(&self, index: usize) -> Option<&str> {
        self.entries
            .get(index)
            .filter(|s| !s.is_empty())
            .map(String::as_str)
    }

    pub fn title(&self) -> &str {
        self.get(8).unwrap_or("")
    }

    pub fn cancel_label(&self) -> &str {
        self.get(5).or_else(|| self.get(9)).unwrap_or("")
    }

    pub fn button_tooltip(&self, button: FamilyBinButton) -> Option<&str> {
        match button {
            // `0x563a21`: Cancel tooltip uses STR# 128 index 15 ("Back to Neighborhood").
            FamilyBinButton::Cancel => self.get(15).or_else(|| self.get(5)),
            // `0x56375d`: Create button uses index 2 (or 16).
            FamilyBinButton::CreateNewFamily => self.get(2).or_else(|| self.get(16)),
            FamilyBinButton::MoveInFamily => self.get(3),
            FamilyBinButton::DeleteFamily => self.get(4),
        }
    }

    pub fn header_tooltip(&self, header: ColumnHeader) -> Option<&str> {
        match header {
            ColumnHeader::NetWorth => self.get(10).or_else(|| self.get(0)),
            ColumnHeader::Friends => self.get(11).or_else(|| self.get(1)),
            ColumnHeader::MagicCoins => self.get(17),
        }
    }
}

/// Laid-out scrollbar geometry when `families.len() > visible_rows`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbarLayout {
    pub track: Rect,
    pub up_arrow: Rect,
    pub down_arrow: Rect,
    pub thumb: Rect,
}

/// Laid-out Family Bin screen geometry for a given viewport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyBinLayout {
    pub frame_size: [i32; 2],
    /// 800×600 background origin (centered on larger screens).
    pub panel_rect: Rect,
    /// Title rectangle across the top.
    pub title_rect: Rect,
    /// Number of visible family slots (`6` at native height, fewer if very short).
    pub visible_rows: usize,
    /// Visible row rectangles (length == `visible_rows`, pairwise disjoint).
    pub row_rects: Vec<Rect>,
    /// Optional scrollbar layout when `families.len() > visible_rows`.
    pub scrollbar: Option<ScrollbarLayout>,
    /// Column header tooltip hit zones.
    pub headers: Vec<(ColumnHeader, Rect)>,
    /// Bottom action button rectangles (length == 4, pairwise disjoint).
    pub buttons: [(FamilyBinButton, Rect); 4],
}

impl FamilyBinLayout {
    /// Computes the Family Bin layout for a viewport of `width` × `height`.
    pub fn compute(
        width: u32,
        height: u32,
        family_count: usize,
        scroll_offset: usize,
        mode: FamilyBinMode,
    ) -> Self {
        let w = i32::try_from(width).unwrap_or(i32::MAX).max(0);
        let h = i32::try_from(height).unwrap_or(i32::MAX).max(0);
        let empty_buttons = [
            (FamilyBinButton::Cancel, Rect::default()),
            (FamilyBinButton::CreateNewFamily, Rect::default()),
            (FamilyBinButton::DeleteFamily, Rect::default()),
            (FamilyBinButton::MoveInFamily, Rect::default()),
        ];
        if w == 0 || h == 0 {
            return Self {
                frame_size: [w, h],
                panel_rect: Rect::default(),
                title_rect: Rect::default(),
                visible_rows: 0,
                row_rects: Vec::new(),
                scrollbar: None,
                headers: Vec::new(),
                buttons: empty_buttons,
            };
        }

        if w >= NATIVE_WIDTH && h >= NATIVE_HEIGHT {
            let ox = (w - NATIVE_WIDTH) / 2;
            let oy = (h - NATIVE_HEIGHT) / 2;
            let panel_rect = Rect::new(ox, oy, NATIVE_WIDTH, NATIVE_HEIGHT);
            let title_rect = Rect::new(ox, oy + 6, NATIVE_WIDTH, 28);

            let list_x = ox + 41;
            let list_y = oy + 76;
            let row_rects: Vec<Rect> = (0..VISIBLE_ROWS)
                .map(|i| {
                    Rect::new(
                        list_x,
                        list_y + (i as i32) * ROW_HEIGHT,
                        ROW_WIDTH,
                        ROW_HEIGHT,
                    )
                })
                .collect();

            let scrollbar = if family_count > VISIBLE_ROWS {
                let sb_x = list_x + ROW_WIDTH + 6;
                let sb_h = (VISIBLE_ROWS as i32) * ROW_HEIGHT;
                Some(compute_scrollbar(
                    Rect::new(sb_x, list_y, 24, sb_h),
                    family_count,
                    VISIBLE_ROWS,
                    scroll_offset,
                ))
            } else {
                None
            };

            let magic = mode == FamilyBinMode::MagicTown;
            let mut headers = vec![
                (
                    ColumnHeader::NetWorth,
                    Rect::new(ox + if magic { 563 } else { 586 }, oy + 42, 26, 23),
                ),
                (
                    ColumnHeader::Friends,
                    Rect::new(ox + if magic { 637 } else { 662 }, oy + 42, 26, 23),
                ),
            ];
            if magic {
                headers.push((
                    ColumnHeader::MagicCoins,
                    Rect::new(ox + 690, oy + 42, 26, 23),
                ));
            }

            let buttons = [
                (
                    FamilyBinButton::Cancel,
                    FamilyBinButton::Cancel.native_rect().offset(ox, oy),
                ),
                (
                    FamilyBinButton::CreateNewFamily,
                    FamilyBinButton::CreateNewFamily
                        .native_rect()
                        .offset(ox, oy),
                ),
                (
                    FamilyBinButton::DeleteFamily,
                    FamilyBinButton::DeleteFamily.native_rect().offset(ox, oy),
                ),
                (
                    FamilyBinButton::MoveInFamily,
                    FamilyBinButton::MoveInFamily.native_rect().offset(ox, oy),
                ),
            ];

            return Self {
                frame_size: [w, h],
                panel_rect,
                title_rect,
                visible_rows: VISIBLE_ROWS,
                row_rects,
                scrollbar,
                headers,
                buttons,
            };
        }

        // Responsive compact layout for viewports narrower than 800 px or
        // shorter than 600 px: guarantees zero overlap between title, rows,
        // scrollbar, and bottom buttons.
        let panel_rect = Rect::new(0, 0, w, h);
        let title_h = 26.min(h / 8).max(16);
        let title_rect = Rect::new(0, 4, w, title_h);
        let header_h = 20;
        let btn_h = 46.min((h / 6).max(24));
        let bottom_pad = 8;
        let list_top = (title_rect.y + title_rect.height + header_h + 4).min(h);
        let list_bottom = (h - btn_h - bottom_pad * 2).max(list_top);
        let list_avail_h = (list_bottom - list_top).max(0);

        let visible_rows = if list_avail_h <= 0 {
            0
        } else {
            ((list_avail_h / 52) as usize).clamp(1, VISIBLE_ROWS)
        };
        let row_h = if visible_rows > 0 {
            (list_avail_h / visible_rows as i32).clamp(1, ROW_HEIGHT)
        } else {
            0
        };
        let needs_scroll = family_count > visible_rows;
        let sb_w = if needs_scroll { 20 } else { 0 };
        let margin_x = 12.min(w / 20);
        let row_w = (w - margin_x * 2 - sb_w - if needs_scroll { 6 } else { 0 }).max(40);
        let list_x = margin_x;

        let row_rects: Vec<Rect> = (0..visible_rows)
            .map(|i| Rect::new(list_x, list_top + (i as i32) * row_h, row_w, row_h))
            .collect();

        let scrollbar = if needs_scroll {
            Some(compute_scrollbar(
                Rect::new(
                    list_x + row_w + 6,
                    list_top,
                    sb_w,
                    row_h * visible_rows as i32,
                ),
                family_count,
                visible_rows,
                scroll_offset,
            ))
        } else {
            None
        };

        let magic = mode == FamilyBinMode::MagicTown;
        let hy = (list_top - header_h).max(title_rect.y + title_rect.height);
        let funds_x = list_x + (row_w * 72) / 100;
        let friends_x = list_x + (row_w * 86) / 100;
        let mut headers = vec![
            (ColumnHeader::NetWorth, Rect::new(funds_x, hy, 24, 18)),
            (ColumnHeader::Friends, Rect::new(friends_x, hy, 24, 18)),
        ];
        if magic {
            let magic_x = list_x + (row_w * 93) / 100;
            headers.push((ColumnHeader::MagicCoins, Rect::new(magic_x, hy, 22, 18)));
        }

        // Distribute the 4 bottom buttons evenly with 6 px gaps.
        let gap = 6;
        let usable_w = (w - margin_x * 2 - gap * 3).max(40);
        let weights = [120, 92, 92, 164];
        let total_weight: i32 = weights.iter().sum();
        let btn_y = (h - btn_h - bottom_pad).max(list_bottom + 2);
        let mut bx = margin_x;
        let mut btn_rects = [Rect::default(); 4];
        for (i, &wt) in weights.iter().enumerate() {
            let bw = if i == 3 {
                (w - margin_x - bx).max(10)
            } else {
                ((usable_w * wt) / total_weight).max(10)
            };
            btn_rects[i] = Rect::new(bx, btn_y, bw, btn_h.min((h - btn_y).max(0)));
            bx += bw + gap;
        }

        let buttons = [
            (FamilyBinButton::Cancel, btn_rects[0]),
            (FamilyBinButton::CreateNewFamily, btn_rects[1]),
            (FamilyBinButton::DeleteFamily, btn_rects[2]),
            (FamilyBinButton::MoveInFamily, btn_rects[3]),
        ];

        Self {
            frame_size: [w, h],
            panel_rect,
            title_rect,
            visible_rows,
            row_rects,
            scrollbar,
            headers,
            buttons,
        }
    }

    pub fn hit_button(&self, x: i32, y: i32) -> Option<FamilyBinButton> {
        self.buttons
            .iter()
            .find(|(_, r)| r.contains(x, y))
            .map(|&(b, _)| b)
    }

    pub fn hit_row(&self, x: i32, y: i32) -> Option<usize> {
        self.row_rects.iter().position(|r| r.contains(x, y))
    }

    pub fn hit_header(&self, x: i32, y: i32) -> Option<ColumnHeader> {
        self.headers
            .iter()
            .find(|(_, r)| r.contains(x, y))
            .map(|&(h, _)| h)
    }

    pub fn button_rect(&self, button: FamilyBinButton) -> Option<Rect> {
        self.buttons
            .iter()
            .find(|&&(b, _)| b == button)
            .map(|&(_, r)| r)
    }
}

fn compute_scrollbar(
    outer: Rect,
    total_items: usize,
    visible_rows: usize,
    scroll_offset: usize,
) -> ScrollbarLayout {
    let arrow_h = outer.width.min(outer.height / 4).max(12);
    let up_arrow = Rect::new(outer.x, outer.y, outer.width, arrow_h);
    let down_arrow = Rect::new(
        outer.x,
        outer.y + outer.height - arrow_h,
        outer.width,
        arrow_h,
    );
    let track = Rect::new(
        outer.x,
        up_arrow.y + up_arrow.height,
        outer.width,
        (outer.height - arrow_h * 2).max(1),
    );
    let max_scroll = total_items.saturating_sub(visible_rows).max(1);
    let clamped_off = scroll_offset.min(max_scroll);
    let thumb_h =
        ((i64::from(track.height) * visible_rows as i64) / total_items.max(1) as i64) as i32;
    let thumb_h = thumb_h.clamp(20.min(track.height), track.height);
    let travel = (track.height - thumb_h).max(0);
    let thumb_y = track.y + ((i64::from(travel) * clamped_off as i64) / max_scroll as i64) as i32;
    let thumb = Rect::new(track.x + 2, thumb_y, (track.width - 4).max(4), thumb_h);
    ScrollbarLayout {
        track,
        up_arrow,
        down_arrow,
        thumb,
    }
}

/// Interactive state for the Family Bin ("Select a Family") screen.
#[derive(Debug, Clone, Default)]
pub struct FamilyBin {
    selected: Option<usize>,
    scroll_offset: usize,
    hovered_row: Option<usize>,
    hovered_button: Option<FamilyBinButton>,
    hovered_header: Option<ColumnHeader>,
    pressed_button: Option<FamilyBinButton>,
    dragging_thumb: bool,
    drag_grab_y: i32,
}

impl FamilyBin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub fn selected_family<'a>(&self, families: &'a [FamilyEntry]) -> Option<&'a FamilyEntry> {
        self.selected.and_then(|i| families.get(i))
    }

    pub fn select(&mut self, index: Option<usize>, family_count: usize) {
        self.selected = index.filter(|&i| i < family_count);
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Clamps selection and scroll offset when the underlying family list changes.
    pub fn sync(&mut self, family_count: usize, visible_rows: usize) {
        if self.selected.is_some_and(|i| i >= family_count) {
            self.selected = None;
        }
        let max_scroll = family_count.saturating_sub(visible_rows.max(1));
        if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }
    }

    /// Scrolls the family list by `delta` rows (positive scrolls down).
    pub fn scroll(&mut self, delta: i32, family_count: usize, visible_rows: usize) {
        let max_scroll = family_count.saturating_sub(visible_rows.max(1));
        let next = (self.scroll_offset as i64)
            .saturating_add(i64::from(delta))
            .clamp(0, max_scroll as i64) as usize;
        self.scroll_offset = next;
    }

    /// Clears hover/press/drag state when the pointer leaves the window.
    pub fn clear_pointer(&mut self) {
        self.hovered_row = None;
        self.hovered_button = None;
        self.hovered_header = None;
        self.pressed_button = None;
        self.dragging_thumb = false;
    }

    /// Handles pointer motion at `(x, y)` (including active scrollbar thumb drag).
    pub fn hover(&mut self, x: i32, y: i32, frame_size: [u32; 2], view: &FamilyBinView<'_>) {
        let layout = FamilyBinLayout::compute(
            frame_size[0],
            frame_size[1],
            view.families.len(),
            self.scroll_offset,
            view.mode,
        );
        self.sync(view.families.len(), layout.visible_rows);

        if self.dragging_thumb
            && let Some(sb) = layout.scrollbar
        {
            let max_scroll = view.families.len().saturating_sub(layout.visible_rows);
            let travel = (sb.track.height - sb.thumb.height).max(1);
            let rel = (y - self.drag_grab_y - sb.track.y).clamp(0, travel);
            self.scroll_offset = ((i64::from(rel) * max_scroll as i64 + i64::from(travel) / 2)
                / i64::from(travel)) as usize;
            self.scroll_offset = self.scroll_offset.min(max_scroll);
            return;
        }

        self.hovered_button = layout.hit_button(x, y);
        self.hovered_header = layout.hit_header(x, y);
        self.hovered_row = layout
            .hit_row(x, y)
            .map(|slot| self.scroll_offset + slot)
            .filter(|&idx| idx < view.families.len());
    }

    /// Handles a mouse press at `(x, y)`. Row selection and scrollbar paging
    /// take effect immediately; action buttons commit on matching release.
    pub fn press(
        &mut self,
        x: i32,
        y: i32,
        frame_size: [u32; 2],
        view: &FamilyBinView<'_>,
    ) -> Option<FamilyBinCommand> {
        let layout = FamilyBinLayout::compute(
            frame_size[0],
            frame_size[1],
            view.families.len(),
            self.scroll_offset,
            view.mode,
        );
        self.sync(view.families.len(), layout.visible_rows);
        let has_selection = self.selected.is_some();

        if let Some(btn) = layout.hit_button(x, y) {
            self.hovered_button = Some(btn);
            if btn.is_enabled(has_selection) {
                self.pressed_button = Some(btn);
            }
            return None;
        }

        if let Some(sb) = layout.scrollbar {
            if sb.up_arrow.contains(x, y) {
                self.scroll(-1, view.families.len(), layout.visible_rows);
                return None;
            }
            if sb.down_arrow.contains(x, y) {
                self.scroll(1, view.families.len(), layout.visible_rows);
                return None;
            }
            if sb.thumb.contains(x, y) {
                self.dragging_thumb = true;
                self.drag_grab_y = y - sb.thumb.y;
                return None;
            }
            if sb.track.contains(x, y) {
                let page = layout.visible_rows.max(1) as i32;
                if y < sb.thumb.y {
                    self.scroll(-page, view.families.len(), layout.visible_rows);
                } else {
                    self.scroll(page, view.families.len(), layout.visible_rows);
                }
                return None;
            }
        }

        if let Some(slot) = layout.hit_row(x, y) {
            let idx = self.scroll_offset + slot;
            if let Some(family) = view.families.get(idx) {
                self.selected = Some(idx);
                self.hovered_row = Some(idx);
                return Some(FamilyBinCommand::SelectFamily(family.id));
            }
        }

        None
    }

    /// Handles a double-click at `(x, y)` on a populated row, starting Move-In
    /// for that family.
    pub fn double_click(
        &mut self,
        x: i32,
        y: i32,
        frame_size: [u32; 2],
        view: &FamilyBinView<'_>,
    ) -> Option<FamilyBinCommand> {
        let layout = FamilyBinLayout::compute(
            frame_size[0],
            frame_size[1],
            view.families.len(),
            self.scroll_offset,
            view.mode,
        );
        self.sync(view.families.len(), layout.visible_rows);
        let slot = layout.hit_row(x, y)?;
        let idx = self.scroll_offset + slot;
        let family = view.families.get(idx)?;
        self.selected = Some(idx);
        Some(FamilyBinCommand::MoveInFamily(family.id))
    }

    /// Completes a mouse release at `(x, y)`, emitting a [`FamilyBinCommand`]
    /// if released on the same enabled action button that was pressed.
    pub fn release(
        &mut self,
        x: i32,
        y: i32,
        frame_size: [u32; 2],
        view: &FamilyBinView<'_>,
    ) -> Option<FamilyBinCommand> {
        self.dragging_thumb = false;
        let pressed = self.pressed_button.take();
        let layout = FamilyBinLayout::compute(
            frame_size[0],
            frame_size[1],
            view.families.len(),
            self.scroll_offset,
            view.mode,
        );
        self.sync(view.families.len(), layout.visible_rows);
        let hit = layout.hit_button(x, y);
        self.hovered_button = hit;
        let has_selection = self.selected.is_some();
        let btn = pressed.filter(|&b| Some(b) == hit && b.is_enabled(has_selection))?;
        match btn {
            FamilyBinButton::Cancel => Some(FamilyBinCommand::Cancel),
            FamilyBinButton::CreateNewFamily => Some(FamilyBinCommand::CreateNewFamily),
            FamilyBinButton::DeleteFamily => self
                .selected_family(view.families)
                .map(|f| FamilyBinCommand::DeleteFamily(f.id)),
            FamilyBinButton::MoveInFamily => self
                .selected_family(view.families)
                .map(|f| FamilyBinCommand::MoveInFamily(f.id)),
        }
    }

    /// Handles the Escape key (closes the Family Bin cleanly).
    pub fn cancel(&mut self) -> FamilyBinCommand {
        self.clear_pointer();
        FamilyBinCommand::Cancel
    }

    /// Draws the entire Family Bin screen into `frame`.
    pub fn draw(
        &self,
        frame: &mut Frame,
        view: &FamilyBinView<'_>,
        art: &Art,
        strings: &FamilyBinStrings,
    ) -> FamilyBinLayout {
        let layout = FamilyBinLayout::compute(
            frame.width(),
            frame.height(),
            view.families.len(),
            self.scroll_offset,
            view.mode,
        );
        let has_selection = self.selected.is_some_and(|i| i < view.families.len());

        // 1. Background fill and `PickBkg.bmp` / `PickBkgMagic.bmp`.
        frame.fill(
            Rect::new(0, 0, layout.frame_size[0], layout.frame_size[1]),
            BKG_FALLBACK,
        );
        let bkg_id = match view.mode {
            FamilyBinMode::Standard => art_id::PICK_BKG,
            FamilyBinMode::MagicTown => art_id::PICK_BKG_MAGIC,
        };
        if let Some(bkg) = art.image(bkg_id) {
            if layout.panel_rect.width >= bkg.width as i32
                && layout.panel_rect.height >= bkg.height as i32
            {
                frame.draw_all(bkg, [layout.panel_rect.x, layout.panel_rect.y]);
            } else {
                draw_scaled(
                    frame,
                    bkg,
                    Rect::new(0, 0, bkg.width as i32, bkg.height as i32),
                    layout.panel_rect,
                );
            }
        }

        // 2. Centered title header (`SELECT A FAMILY`, `0x563672..0x56372f`).
        let title = strings.title();
        if !title.is_empty()
            && let Some(font) = art
                .font(16)
                .or_else(|| art.font(14))
                .or_else(|| art.font(12))
        {
            let tw = font.width(title);
            let tx = layout.title_rect.x + (layout.title_rect.width - tw) / 2;
            let ty = layout.title_rect.y + (layout.title_rect.height - font.line_height) / 2;
            font.draw(frame, title, [tx, ty.max(0)], TEXT_NORMAL);
        }

        // 3. Family rows (populated + empty slot frames, matching 09-nb-family-bin.png).
        let magic = view.mode == FamilyBinMode::MagicTown;
        for (slot, &row_rect) in layout.row_rects.iter().enumerate() {
            let family_idx = self.scroll_offset + slot;
            let family = view.families.get(family_idx);
            let is_selected = family.is_some() && self.selected == Some(family_idx);
            let is_hovered = family.is_some() && self.hovered_row == Some(family_idx);
            let row_state = if is_selected {
                2
            } else if is_hovered {
                1
            } else {
                0
            };
            draw_row_background(frame, art, row_state, row_rect);

            if let Some(entry) = family {
                let ink = if is_selected {
                    TEXT_SELECTED
                } else if is_hovered {
                    TEXT_HOVER
                } else {
                    TEXT_NORMAL
                };
                draw_family_row_contents(frame, art, entry, row_rect, magic, ink);
            }
        }

        // 4. Scrollbar (visible whenever `families.len() > visible_rows`).
        if let Some(sb) = layout.scrollbar {
            draw_scrollbar(frame, art, sb, self.dragging_thumb);
        }

        // 5. Four bottom action buttons.
        for &(btn, rect) in &layout.buttons {
            let enabled = btn.is_enabled(has_selection);
            let cell = if !enabled {
                3
            } else if self.pressed_button == Some(btn) {
                1
            } else if self.hovered_button == Some(btn) {
                2
            } else {
                0
            };
            if btn == FamilyBinButton::Cancel {
                draw_cancel_button(frame, art, strings.cancel_label(), cell, rect);
            } else {
                draw_four_cell_button(frame, art, btn.art_id(), cell, rect);
            }
        }

        // 6. Hover tooltip over buttons or column headers.
        if let Some(btn) = self.hovered_button
            && let Some(tip) = strings.button_tooltip(btn)
            && let Some(rect) = layout.button_rect(btn)
        {
            draw_tooltip(frame, art, tip, [rect.x + rect.width / 2, rect.y - 18]);
        } else if let Some(header) = self.hovered_header
            && let Some(tip) = strings.header_tooltip(header)
            && let Some((_, rect)) = layout.headers.iter().find(|&&(h, _)| h == header)
        {
            draw_tooltip(
                frame,
                art,
                tip,
                [rect.x + rect.width / 2, rect.y + rect.height + 4],
            );
        }

        layout
    }
}

fn draw_row_background(frame: &mut Frame, art: &Art, state: u32, dest: Rect) {
    if dest.width <= 0 || dest.height <= 0 {
        return;
    }
    if let Some(bkg) = art.image(art_id::ITEM_BKG)
        && bkg.height >= 3
        && bkg.width > 0
    {
        let ch = (bkg.height / 3) as i32;
        let src = Rect::new(0, ch * (state.min(2) as i32), bkg.width as i32, ch);
        if dest.width == bkg.width as i32 && dest.height == ch {
            frame.draw(bkg, src, [dest.x, dest.y]);
        } else {
            draw_scaled(frame, bkg, src, dest);
        }
    } else {
        let fill = match state {
            2 => [48, 88, 128],
            1 => [22, 42, 112],
            _ => [6, 12, 64],
        };
        frame.fill(dest, fill);
        draw_outline(
            frame,
            dest,
            if state == 2 {
                TEXT_SELECTED
            } else {
                [90, 102, 148]
            },
        );
    }
}

fn draw_family_row_contents(
    frame: &mut Frame,
    art: &Art,
    entry: &FamilyEntry,
    row: Rect,
    magic: bool,
    ink: [u8; 3],
) {
    let native = row.width >= ROW_WIDTH && row.height >= ROW_HEIGHT;

    // Family surname at `(20, 0)` in native layout (`0x5429bb..0x5429cc`),
    // font `variablesans_12` (`0x542da4`), with drop shadow `0x6f2aa4`.
    if let Some(font) = art.font(12).or_else(|| art.font(10)) {
        let nx = row.x + if native { 20 } else { 10 };
        let ny = row.y + 2;
        font.draw(frame, &entry.name, [nx + 1, ny + 1], TEXT_SHADOW);
        font.draw(frame, &entry.name, [nx, ny], ink);
    }

    // Up to 8 portraits at `(110, 14)`, 48 px stride, 45×45 (`0x5429a6`, `0x542ccd`).
    let p_size = if native {
        45
    } else {
        (row.height - 18).clamp(18, 42)
    };
    let p_stride = p_size + 3;
    let p_start_x = row.x + if native { 110 } else { (row.width * 18) / 100 };
    let p_y = row.y + if native { 16 } else { row.height - p_size - 3 };
    let funds_col_x = row.x
        + if native {
            if magic { 494 } else { 526 }
        } else {
            (row.width * 72) / 100
        };

    for (i, member) in entry.members.iter().take(MAX_PORTRAITS).enumerate() {
        let px = p_start_x + (i as i32) * p_stride;
        if px + p_size > funds_col_x - 8 {
            break;
        }
        let prect = Rect::new(px, p_y, p_size, p_size);
        frame.fill(prect, PORTRAIT_BG);
        if let Some(img) = &member.portrait {
            let inner = Rect::new(px + 2, p_y + 2, (p_size - 4).max(1), (p_size - 4).max(1));
            draw_scaled(
                frame,
                img,
                Rect::new(0, 0, img.width as i32, img.height as i32),
                inner,
            );
        }
        draw_outline(frame, prect, PORTRAIT_FRAME);
    }

    // Net Worth (`$20,000`), Friends (`0`), and optional MagiCoins (`0x5429cd..0x542a62`).
    if let Some(font) = art.font(10).or_else(|| art.font(12)) {
        let ty = row.y
            + if native {
                25
            } else {
                (row.height - font.line_height) / 2
            };
        let funds_str = format_simoleons(entry.funds);
        let funds_x = funds_col_x + if native { 22 } else { 0 };
        font.draw(frame, &funds_str, [funds_x + 1, ty + 1], TEXT_SHADOW);
        font.draw(frame, &funds_str, [funds_x, ty], ink);

        let friends_str = entry.friends.to_string();
        let friends_x = row.x
            + if native {
                if magic { 606 } else { 620 }
            } else {
                (row.width * 86) / 100
            };
        font.draw(frame, &friends_str, [friends_x + 1, ty + 1], TEXT_SHADOW);
        font.draw(frame, &friends_str, [friends_x, ty], ink);

        if magic {
            let magic_str = entry.magic_coins.to_string();
            let magic_x = row.x + if native { 649 } else { (row.width * 93) / 100 };
            font.draw(frame, &magic_str, [magic_x + 1, ty + 1], TEXT_SHADOW);
            font.draw(frame, &magic_str, [magic_x, ty], ink);
        }
    }
}

fn draw_scrollbar(frame: &mut Frame, art: &Art, sb: ScrollbarLayout, dragging: bool) {
    frame.fill(sb.track, SCROLL_TRACK_BG);
    draw_outline(frame, sb.track, [72, 92, 152]);
    frame.fill(sb.up_arrow, SCROLL_ARROW_BG);
    frame.fill(sb.down_arrow, SCROLL_ARROW_BG);
    draw_outline(frame, sb.up_arrow, PORTRAIT_FRAME);
    draw_outline(frame, sb.down_arrow, PORTRAIT_FRAME);

    // Draw simple crisp up/down chevrons (or ScrollBar.BMP arrows when available).
    if let Some(strip) = art.image(art_id::SCROLLBAR)
        && strip.width >= 30
        && strip.height > 0
    {
        // `ScrollBar.BMP` is 300×30 (10 cells of 30×30): cell 0 up, cell 4 down.
        let cw = (strip.width / 10) as i32;
        let ch = strip.height as i32;
        draw_scaled(frame, strip, Rect::new(0, 0, cw, ch), sb.up_arrow);
        draw_scaled(frame, strip, Rect::new(cw * 4, 0, cw, ch), sb.down_arrow);
    }

    let thumb_color = if dragging {
        SCROLL_THUMB_HOVER
    } else {
        SCROLL_THUMB_BG
    };
    frame.fill(sb.thumb, thumb_color);
    draw_outline(frame, sb.thumb, PORTRAIT_FRAME);
}

fn draw_cancel_button(frame: &mut Frame, art: &Art, label: &str, cell: u32, dest: Rect) {
    if dest.width <= 0 || dest.height <= 0 {
        return;
    }
    if let Some(img) = art.image(art_id::BTN_DEFAULT)
        && img.width >= 4
        && img.height > 0
    {
        // `NbhdTileBtn.BMP` (364×62) holds 4 horizontal states of 91×62; split
        // each state into left cap (30 px), stretchable center (31 px), and
        // right cap (30 px) so wide labels stretch cleanly.
        let cw = (img.width / 4) as i32;
        let ch = img.height as i32;
        let sx = cw * (cell.min(3) as i32);
        let cap = (cw / 3).min(dest.width / 2);
        let mid_src_w = (cw - cap * 2).max(1);
        let mid_dst_w = (dest.width - cap * 2).max(0);

        draw_scaled(
            frame,
            img,
            Rect::new(sx, 0, cap, ch),
            Rect::new(dest.x, dest.y, cap, dest.height),
        );
        if mid_dst_w > 0 {
            draw_scaled(
                frame,
                img,
                Rect::new(sx + cap, 0, mid_src_w, ch),
                Rect::new(dest.x + cap, dest.y, mid_dst_w, dest.height),
            );
        }
        draw_scaled(
            frame,
            img,
            Rect::new(sx + cw - cap, 0, cap, ch),
            Rect::new(dest.x + dest.width - cap, dest.y, cap, dest.height),
        );
    } else {
        let color = match cell {
            1 => [0, 160, 210],
            2 => [56, 128, 200],
            _ => [24, 92, 160],
        };
        frame.fill(dest, color);
        draw_outline(frame, dest, PORTRAIT_FRAME);
    }

    if !label.is_empty()
        && let Some(font) = art.font(12).or_else(|| art.font(10))
    {
        let tw = font.width(label);
        let tx = dest.x + (dest.width - tw) / 2;
        let ty = dest.y + (dest.height - font.line_height) / 2;
        let ink = match cell {
            1 => TEXT_SELECTED,
            2 => TEXT_HOVER,
            _ => TEXT_NORMAL,
        };
        font.draw(frame, label, [tx + 1, ty + 1], TEXT_SHADOW);
        font.draw(frame, label, [tx, ty], ink);
    }
}

fn draw_four_cell_button(frame: &mut Frame, art: &Art, id: i32, cell: u32, dest: Rect) {
    if dest.width <= 0 || dest.height <= 0 {
        return;
    }
    if let Some(img) = art.image(id)
        && img.width >= 4
        && img.height > 0
    {
        let cw = (img.width / 4) as i32;
        let ch = img.height as i32;
        let src = Rect::new(cw * (cell.min(3) as i32), 0, cw, ch);
        if dest.width == cw && dest.height == ch {
            frame.draw(img, src, [dest.x, dest.y]);
        } else {
            draw_scaled(frame, img, src, dest);
        }
    } else {
        let color = match cell {
            1 => [0, 160, 210],
            2 => [56, 128, 200],
            3 => [24, 48, 92],
            _ => [24, 92, 160],
        };
        frame.fill(dest, color);
        draw_outline(frame, dest, PORTRAIT_FRAME);
    }
}

fn draw_scaled(frame: &mut Frame, image: &Image, src: Rect, dest: Rect) {
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
