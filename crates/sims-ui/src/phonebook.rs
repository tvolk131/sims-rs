//! Native Complete Collection 1.91 Phonebook / Call Neighbor dialog
//! (`cWinPhoneBook`, `Dialog` kind 10, `0x5221d0`).
//!
//! Composites `UIGraphics.far`'s native `cpanel\Backgrounds\phonebookbkg.bmp`
//! (`620×356`, ID `80`) and `cpanel\Backgrounds\phoneicon.bmp` (`33×23`, ID `81`)
//! on viewports `>= 620×356`, falling back cleanly to 9-sliced
//! `PopupInfoTiles.bmp` (`3008`) on compact viewports or when native assets are
//! absent.

use crate::art::Art;
use crate::dialog::{BACKGROUND, BUTTON, draw_nine_slice};
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::hud;
use crate::Result;
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;

/// `UIGraphics/Res_CPanel.h` `kPhoneBookBkg` (`cpanel/Backgrounds/phonebookbkg.bmp`, `620×356`).
pub const PHONE_BOOK_BKG: i32 = 80;
/// `UIGraphics/Res_CPanel.h` `kPhoneBookIcon` (`cpanel/Backgrounds/phoneicon.bmp`, `33×23`).
pub const PHONE_BOOK_ICON: i32 = 81;

/// Native Phonebook bitmap dimensions (`620×356`).
pub const NATIVE_WIDTH: i32 = 620;
pub const NATIVE_HEIGHT: i32 = 356;

/// Required 9-slice / button fallback resources for [`PhoneBookDialog`].
pub const RESOURCES: [i32; 2] = [BACKGROUND, BUTTON];
/// Optional native Phonebook & portrait-well artwork (`80`, `81`, `4992`).
pub const OPTIONAL_RESOURCES: [i32; 3] = [PHONE_BOOK_BKG, PHONE_BOOK_ICON, hud::id::SIM_STUB];

/// Loads all Phonebook bitmaps and UI fonts from `ui`.
pub fn load_art(ui: &UiGraphics, language: u8) -> Result<Art> {
    Art::load_with_optional(ui, language, &RESOURCES, &OPTIONAL_RESOURCES)
}

const TEXT_NORMAL: [u8; 3] = [195, 205, 205];
const TEXT_HOVER: [u8; 3] = [255, 255, 255];
const TEXT_SELECTED: [u8; 3] = [0, 255, 255];
const TEXT_DISABLED: [u8; 3] = [0x40, 0x5d, 0x5f];
const TEXT_MUTED: [u8; 3] = [138, 160, 198];
const TEXT_GOLD: [u8; 3] = [255, 224, 130];

const FALLBACK_BG: [u8; 3] = [0, 0, 82];
const FALLBACK_BORDER: [u8; 3] = [96, 132, 224];
const CARD_BG: [u8; 3] = [10, 18, 86];
const CARD_HOVER_BG: [u8; 3] = [22, 44, 126];
const CARD_SELECTED_BG: [u8; 3] = [24, 72, 148];
const CARD_BORDER: [u8; 3] = [56, 86, 164];
const PORTRAIT_BG: [u8; 3] = [6, 12, 54];
const BAR_TRACK_BG: [u8; 3] = [8, 12, 44];
const BAR_TRACK_BORDER: [u8; 3] = [64, 88, 148];
const BAR_POS_DAILY: [u8; 3] = [52, 214, 96];
const BAR_POS_LIFE: [u8; 3] = [86, 184, 255];
const BAR_NEG: [u8; 3] = [224, 68, 68];

/// One call-eligible neighbor entry displayed in [`PhoneBookDialog`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneBookEntry {
    pub neighbor_id: i16,
    pub name: String,
    pub daily: i16,
    pub lifetime: i16,
    /// Optional 45×45 neighbor portrait (`BMP_` 2002 / `Faces::face`).
    pub portrait: Option<Image>,
    /// Whether this neighbor is a family friend (`daily >= 50` or `lifetime >= 50`).
    pub is_friend: bool,
}

/// Read-only view-model passed to [`PhoneBookDialog`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhoneBookSnapshot {
    pub title: String,
    pub caller_name: Option<String>,
    pub entries: Vec<PhoneBookEntry>,
    pub cancel_label: String,
    pub prev_label: String,
    pub next_label: String,
}

/// Commands emitted by [`PhoneBookDialog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhoneBookCommand {
    Call(i16),
    Cancel,
}

/// Interactive hit targets inside [`PhoneBookDialog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhoneBookHit {
    Entry(usize),
    PrevPage,
    NextPage,
    Cancel,
}

/// Computed screen-space geometry for [`PhoneBookDialog`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneBookLayout {
    pub dialog: Rect,
    pub native_art: bool,
    pub icon_rect: Rect,
    pub title_rect: Rect,
    /// Visible entries on the active page: `(global_entry_index, card_rect)`.
    pub entry_slots: Vec<(usize, Rect)>,
    pub per_page: usize,
    pub prev_btn: Rect,
    pub page_label: Rect,
    pub next_btn: Rect,
    pub cancel_btn: Rect,
}

impl PhoneBookLayout {
    /// Returns the interactive control under `(x, y)`, if any.
    pub fn hit(&self, x: i32, y: i32) -> Option<PhoneBookHit> {
        for &(idx, rect) in &self.entry_slots {
            if rect.contains(x, y) {
                return Some(PhoneBookHit::Entry(idx));
            }
        }
        if self.prev_btn.contains(x, y) {
            return Some(PhoneBookHit::PrevPage);
        }
        if self.next_btn.contains(x, y) {
            return Some(PhoneBookHit::NextPage);
        }
        if self.cancel_btn.contains(x, y) {
            return Some(PhoneBookHit::Cancel);
        }
        None
    }
}

/// Interactive native Phonebook / Call Neighbor modal controller (`cWinPhoneBook`).
#[derive(Debug, Clone, Default)]
pub struct PhoneBookDialog {
    open: bool,
    page: usize,
    selected_index: Option<usize>,
    hovered: Option<PhoneBookHit>,
    pressed: Option<PhoneBookHit>,
}

impl PhoneBookDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn page(&self) -> usize {
        self.page
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    /// Opens the Phonebook modal on page 0.
    pub fn open(&mut self) {
        self.open = true;
        self.page = 0;
        self.selected_index = Some(0);
        self.hovered = None;
        self.pressed = None;
    }

    /// Closes the Phonebook modal and clears pointer state.
    pub fn close(&mut self) {
        self.open = false;
        self.hovered = None;
        self.pressed = None;
    }

    fn effective_per_page(frame_size: [u32; 2]) -> (bool, usize, usize) {
        let fw = i32::try_from(frame_size[0]).unwrap_or(800).max(1);
        let fh = i32::try_from(frame_size[1]).unwrap_or(600).max(1);
        if fw >= NATIVE_WIDTH && fh >= NATIVE_HEIGHT {
            (true, 2, 4)
        } else {
            let cols = if fw >= 520 { 2 } else { 1 };
            let rows = if fh >= 320 { 4 } else if fh >= 260 { 3 } else { 2 };
            (false, cols, rows)
        }
    }

    /// Number of pages for `snapshot` at `frame_size`.
    pub fn page_count(&self, frame_size: [u32; 2], snapshot: &PhoneBookSnapshot) -> usize {
        let (_, cols, rows) = Self::effective_per_page(frame_size);
        let per_page = (cols * rows).max(1);
        snapshot.entries.len().div_ceil(per_page).max(1)
    }

    fn effective_page(&self, frame_size: [u32; 2], snapshot: &PhoneBookSnapshot) -> usize {
        let count = self.page_count(frame_size, snapshot);
        self.page.min(count.saturating_sub(1))
    }

    pub fn can_prev(&self, frame_size: [u32; 2], snapshot: &PhoneBookSnapshot) -> bool {
        self.effective_page(frame_size, snapshot) > 0
    }

    pub fn can_next(&self, frame_size: [u32; 2], snapshot: &PhoneBookSnapshot) -> bool {
        self.effective_page(frame_size, snapshot) + 1 < self.page_count(frame_size, snapshot)
    }

    pub fn prev_page(&mut self, frame_size: [u32; 2], snapshot: &PhoneBookSnapshot) -> bool {
        if !self.open {
            return false;
        }
        let cur = self.effective_page(frame_size, snapshot);
        if cur == 0 {
            return false;
        }
        self.page = cur - 1;
        let (_, cols, rows) = Self::effective_per_page(frame_size);
        let start = self.page * cols * rows;
        self.selected_index = (start < snapshot.entries.len()).then_some(start);
        true
    }

    pub fn next_page(&mut self, frame_size: [u32; 2], snapshot: &PhoneBookSnapshot) -> bool {
        if !self.open {
            return false;
        }
        let cur = self.effective_page(frame_size, snapshot);
        let total = self.page_count(frame_size, snapshot);
        if cur + 1 >= total {
            return false;
        }
        self.page = cur + 1;
        let (_, cols, rows) = Self::effective_per_page(frame_size);
        let start = self.page * cols * rows;
        self.selected_index = (start < snapshot.entries.len()).then_some(start);
        true
    }

    pub fn select_prev(&mut self, frame_size: [u32; 2], snapshot: &PhoneBookSnapshot) -> bool {
        if !self.open || snapshot.entries.is_empty() {
            return false;
        }
        let cur = self
            .selected_index
            .unwrap_or(0)
            .min(snapshot.entries.len() - 1);
        if cur == 0 {
            return false;
        }
        let next = cur - 1;
        self.selected_index = Some(next);
        let (_, cols, rows) = Self::effective_per_page(frame_size);
        self.page = next / (cols * rows).max(1);
        true
    }

    pub fn select_next(&mut self, frame_size: [u32; 2], snapshot: &PhoneBookSnapshot) -> bool {
        if !self.open || snapshot.entries.is_empty() {
            return false;
        }
        let cur = self
            .selected_index
            .unwrap_or(0)
            .min(snapshot.entries.len() - 1);
        if cur + 1 >= snapshot.entries.len() {
            return false;
        }
        let next = cur + 1;
        self.selected_index = Some(next);
        let (_, cols, rows) = Self::effective_per_page(frame_size);
        self.page = next / (cols * rows).max(1);
        true
    }

    /// Submits the currently selected neighbor (`Enter`).
    pub fn submit(&self, snapshot: &PhoneBookSnapshot) -> Option<PhoneBookCommand> {
        if !self.open || snapshot.entries.is_empty() {
            return None;
        }
        let idx = self
            .selected_index
            .unwrap_or(0)
            .min(snapshot.entries.len() - 1);
        Some(PhoneBookCommand::Call(snapshot.entries[idx].neighbor_id))
    }

    /// Cancels and closes the Phonebook (`Escape`).
    pub fn escape(&mut self) -> Option<PhoneBookCommand> {
        if !self.open {
            return None;
        }
        self.close();
        Some(PhoneBookCommand::Cancel)
    }

    /// Computes Phonebook geometry centered inside `frame_size`.
    pub fn layout(&self, frame_size: [u32; 2], snapshot: &PhoneBookSnapshot) -> PhoneBookLayout {
        let fw = i32::try_from(frame_size[0]).unwrap_or(800).max(1);
        let fh = i32::try_from(frame_size[1]).unwrap_or(600).max(1);
        let (native_art, cols, rows) = Self::effective_per_page(frame_size);
        let per_page = (cols * rows).max(1);
        let page = self.effective_page(frame_size, snapshot);

        let (dialog, icon_rect, title_rect, col_rects, footer_y, footer_x, footer_w) =
            if native_art {
                let x = (fw - NATIVE_WIDTH) / 2;
                let y = (fh - NATIVE_HEIGHT) / 2;
                let dialog = Rect::new(x, y, NATIVE_WIDTH, NATIVE_HEIGHT);
                // Native `phonebookbkg.bmp` (620x356):
                // - Phone icon slot top-left `(10, 10, 33, 23)`
                // - Header pill `(52, 6, 560, 32)`
                // - Left beveled column `(12, 56, 288, 250)`
                // - Right beveled column `(322, 56, 288, 250)`
                // - Footer strip inside lower wells `(14, 316, 592, 28)`
                let icon_rect = Rect::new(x + 11, y + 11, 33, 23);
                let title_rect = Rect::new(x + 60, y + 8, 544, 28);
                let left_col = Rect::new(x + 14, y + 58, 284, 252);
                let right_col = Rect::new(x + 324, y + 58, 284, 252);
                (
                    dialog,
                    icon_rect,
                    title_rect,
                    vec![left_col, right_col],
                    y + 318,
                    x + 14,
                    594,
                )
            } else {
                let margin = 10;
                let w = (fw - margin * 2).clamp(240, NATIVE_WIDTH).min(fw);
                let h = (fh - margin * 2).clamp(200, NATIVE_HEIGHT).min(fh);
                let x = ((fw - w) / 2).max(0);
                let y = ((fh - h) / 2).max(0);
                let dialog = Rect::new(x, y, w, h);
                let pad = 12;
                let inner_x = x + pad;
                let inner_w = (w - pad * 2).max(120);
                let icon_rect = Rect::new(inner_x, y + 10, 33.min(inner_w / 5), 23);
                let title_rect = Rect::new(
                    icon_rect.x + icon_rect.width + 8,
                    y + 10,
                    (inner_x + inner_w - (icon_rect.x + icon_rect.width + 8)).max(60),
                    24,
                );
                let footer_h = 26;
                let footer_y = y + h - footer_h - 10;
                let body_y = y + 40;
                let body_h = (footer_y - body_y - 6).max(80);
                let col_rects = if cols == 2 {
                    let gap = 8;
                    let cw = (inner_w - gap) / 2;
                    vec![
                        Rect::new(inner_x, body_y, cw, body_h),
                        Rect::new(inner_x + cw + gap, body_y, inner_w - cw - gap, body_h),
                    ]
                } else {
                    vec![Rect::new(inner_x, body_y, inner_w, body_h)]
                };
                (dialog, icon_rect, title_rect, col_rects, footer_y, inner_x, inner_w)
            };

        let footer_h = if native_art { 28 } else { 26 };
        let gap = 8;
        let btn_w = 88.min(((footer_w - gap * 3) / 4).max(48));
        let prev_btn = Rect::new(footer_x, footer_y, btn_w, footer_h);
        let cancel_btn = Rect::new(footer_x + footer_w - btn_w, footer_y, btn_w, footer_h);
        let next_btn = Rect::new(cancel_btn.x - gap - btn_w, footer_y, btn_w, footer_h);
        let page_x = prev_btn.x + prev_btn.width + gap;
        let page_w = (next_btn.x - page_x - gap).max(32);
        let page_label = Rect::new(page_x, footer_y, page_w, footer_h);

        let mut entry_slots = Vec::new();
        let start = page * per_page;
        let row_gap = 4;
        for slot in 0..per_page {
            let global_idx = start + slot;
            if global_idx >= snapshot.entries.len() {
                break;
            }
            let col = slot / rows;
            let row = slot % rows;
            let Some(&col_rect) = col_rects.get(col) else {
                break;
            };
            let cell_h = ((col_rect.height - row_gap * (rows as i32 - 1)) / rows as i32).max(42);
            let cy = col_rect.y + row as i32 * (cell_h + row_gap);
            let clamped_h = cell_h.min(col_rect.y + col_rect.height - cy);
            if clamped_h > 0 {
                entry_slots.push((
                    global_idx,
                    Rect::new(col_rect.x, cy, col_rect.width, clamped_h),
                ));
            }
        }

        PhoneBookLayout {
            dialog,
            native_art,
            icon_rect,
            title_rect,
            entry_slots,
            per_page,
            prev_btn,
            page_label,
            next_btn,
            cancel_btn,
        }
    }

    pub fn pointer_move(
        &mut self,
        frame_size: [u32; 2],
        snapshot: &PhoneBookSnapshot,
        x: i32,
        y: i32,
    ) {
        if !self.open {
            self.hovered = None;
            return;
        }
        let layout = self.layout(frame_size, snapshot);
        self.hovered = layout.hit(x, y);
    }

    pub fn pointer_down(
        &mut self,
        frame_size: [u32; 2],
        snapshot: &PhoneBookSnapshot,
        x: i32,
        y: i32,
    ) -> bool {
        if !self.open {
            return false;
        }
        let layout = self.layout(frame_size, snapshot);
        let hit = layout.hit(x, y);
        self.hovered = hit;
        self.pressed = hit;
        if let Some(PhoneBookHit::Entry(idx)) = hit {
            self.selected_index = Some(idx);
        }
        hit.is_some() || layout.dialog.contains(x, y)
    }

    pub fn pointer_up(
        &mut self,
        frame_size: [u32; 2],
        snapshot: &PhoneBookSnapshot,
        x: i32,
        y: i32,
    ) -> Option<PhoneBookCommand> {
        if !self.open {
            return None;
        }
        let pressed = self.pressed.take()?;
        let layout = self.layout(frame_size, snapshot);
        let released = layout.hit(x, y);
        self.hovered = released;
        if released != Some(pressed) {
            return None;
        }
        match pressed {
            PhoneBookHit::Entry(idx) => {
                let entry = snapshot.entries.get(idx)?;
                self.selected_index = Some(idx);
                Some(PhoneBookCommand::Call(entry.neighbor_id))
            }
            PhoneBookHit::PrevPage => {
                self.prev_page(frame_size, snapshot);
                None
            }
            PhoneBookHit::NextPage => {
                self.next_page(frame_size, snapshot);
                None
            }
            PhoneBookHit::Cancel => {
                self.close();
                Some(PhoneBookCommand::Cancel)
            }
        }
    }

    /// Draws the native Phonebook modal into `frame`.
    pub fn draw(&self, frame: &mut Frame, snapshot: &PhoneBookSnapshot, art: &Art) {
        if !self.open {
            return;
        }
        let size = [frame.width(), frame.height()];
        let layout = self.layout(size, snapshot);

        // Backdrop frame: native 620x356 `phonebookbkg.bmp` (`80`), or 9-slice `3008` fallback.
        if layout.native_art
            && let Some(bkg) = art.image(PHONE_BOOK_BKG)
        {
            frame.draw_all(bkg, [layout.dialog.x, layout.dialog.y]);
        } else if let Some(tiler) = art.image(BACKGROUND)
            && tiler.width >= 3
            && tiler.height >= 3
        {
            draw_nine_slice(frame, tiler, layout.dialog);
        } else {
            frame.fill(layout.dialog, FALLBACK_BG);
            draw_outline(frame, layout.dialog, FALLBACK_BORDER);
        }

        // Phone icon (`phoneicon.bmp`, `81`).
        if let Some(icon) = art.image(PHONE_BOOK_ICON) {
            draw_aspect_fit(frame, icon, layout.icon_rect);
        }

        let title_font = art.font(12).or_else(|| art.font(10));
        let body_font = art.font(10).or_else(|| art.font(12));
        let small_font = art.font(9).or_else(|| art.font(10));

        // Header title.
        if let Some(font) = title_font {
            let base = if snapshot.title.trim().is_empty() {
                "Call Neighbor"
            } else {
                snapshot.title.trim()
            };
            let title = match snapshot
                .caller_name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(caller) if !base.eq_ignore_ascii_case(caller) => format!("{base} - {caller}"),
                _ => base.to_owned(),
            };
            let ty = layout.title_rect.y + ((layout.title_rect.height - font.line_height) / 2).max(0);
            draw_clipped_text(
                frame,
                font,
                &title,
                [layout.title_rect.x, ty],
                layout.title_rect.width,
                TEXT_HOVER,
            );
        }

        // Neighbor cards (45x45 portrait + name + Daily & Lifetime bars).
        if snapshot.entries.is_empty() {
            if let Some(font) = body_font {
                let msg = "No known neighbors available to call.";
                let msg_rect = Rect::new(
                    layout.dialog.x + 24,
                    layout.dialog.y + 84,
                    (layout.dialog.width - 48).max(40),
                    24,
                );
                draw_clipped_text(
                    frame,
                    font,
                    msg,
                    [msg_rect.x, msg_rect.y],
                    msg_rect.width,
                    TEXT_MUTED,
                );
            }
        } else {
            let stub = art.image(hud::id::SIM_STUB);
            for &(idx, rect) in &layout.entry_slots {
                let Some(entry) = snapshot.entries.get(idx) else {
                    continue;
                };
                let is_hovered = self.hovered == Some(PhoneBookHit::Entry(idx));
                let is_pressed =
                    is_hovered && self.pressed == Some(PhoneBookHit::Entry(idx));
                let is_selected = self.selected_index == Some(idx);

                let bg = if is_pressed || is_selected {
                    CARD_SELECTED_BG
                } else if is_hovered {
                    CARD_HOVER_BG
                } else {
                    CARD_BG
                };
                let border = if is_pressed || is_selected {
                    TEXT_SELECTED
                } else if is_hovered {
                    TEXT_HOVER
                } else {
                    CARD_BORDER
                };
                frame.fill(rect, bg);
                draw_outline(frame, rect, border);

                // 45x45 portrait well on the left side of the card.
                let p_size = (rect.height - 10).clamp(32, 45);
                let portrait_rect = Rect::new(
                    rect.x + 6,
                    rect.y + (rect.height - p_size) / 2,
                    p_size,
                    p_size,
                );
                if let Some(stub_img) = stub {
                    draw_aspect_fit(frame, stub_img, portrait_rect);
                } else {
                    frame.fill(portrait_rect, PORTRAIT_BG);
                    draw_outline(frame, portrait_rect, CARD_BORDER);
                }
                if let Some(face) = entry.portrait.as_ref() {
                    let inner = Rect::new(
                        portrait_rect.x + 1,
                        portrait_rect.y + 1,
                        (portrait_rect.width - 2).max(1),
                        (portrait_rect.height - 2).max(1),
                    );
                    draw_aspect_fit(frame, face, inner);
                }

                // Name + Daily/Lifetime relationship meters.
                let tx = portrait_rect.x + portrait_rect.width + 8;
                let tw = (rect.x + rect.width - tx - 8).max(24);
                if let Some(font) = body_font {
                    let name_color = if is_pressed || is_selected {
                        TEXT_SELECTED
                    } else if is_hovered {
                        TEXT_HOVER
                    } else if entry.is_friend {
                        TEXT_GOLD
                    } else {
                        TEXT_NORMAL
                    };
                    draw_clipped_text(frame, font, &entry.name, [tx, rect.y + 4], tw, name_color);
                }

                // Daily & Lifetime relationship meters (-100..=100).
                let bar_y1 = rect.y + 22;
                let bar_y2 = rect.y + 37;
                let bar_h = 10;
                let score_w = 34;
                let label_w = 14;
                let bar_x = tx + label_w;
                let bar_w = (tw - label_w - score_w - 4).max(36);
                if bar_y1 + bar_h <= rect.y + rect.height - 2 {
                    if let Some(font) = small_font {
                        draw_clipped_text(frame, font, "D", [tx, bar_y1 - 1], label_w, TEXT_MUTED);
                    }
                    draw_rel_bar(
                        frame,
                        Rect::new(bar_x, bar_y1, bar_w, bar_h),
                        entry.daily,
                        BAR_POS_DAILY,
                    );
                    if let Some(font) = small_font {
                        let s = format!("{}", entry.daily);
                        draw_clipped_text(
                            frame,
                            font,
                            &s,
                            [bar_x + bar_w + 4, bar_y1 - 1],
                            score_w,
                            TEXT_NORMAL,
                        );
                    }
                }
                if bar_y2 + bar_h <= rect.y + rect.height - 2 {
                    if let Some(font) = small_font {
                        draw_clipped_text(frame, font, "L", [tx, bar_y2 - 1], label_w, TEXT_MUTED);
                    }
                    draw_rel_bar(
                        frame,
                        Rect::new(bar_x, bar_y2, bar_w, bar_h),
                        entry.lifetime,
                        BAR_POS_LIFE,
                    );
                    if let Some(font) = small_font {
                        let s = format!("{}", entry.lifetime);
                        draw_clipped_text(
                            frame,
                            font,
                            &s,
                            [bar_x + bar_w + 4, bar_y2 - 1],
                            score_w,
                            TEXT_NORMAL,
                        );
                    }
                }
            }
        }

        // Footer buttons: Prev, Page X / Y, Next, Cancel.
        let prev_label = if snapshot.prev_label.trim().is_empty() {
            "Prev"
        } else {
            snapshot.prev_label.trim()
        };
        let next_label = if snapshot.next_label.trim().is_empty() {
            "Next"
        } else {
            snapshot.next_label.trim()
        };
        let cancel_label = if snapshot.cancel_label.trim().is_empty() {
            "Cancel"
        } else {
            snapshot.cancel_label.trim()
        };

        let can_prev = self.can_prev(size, snapshot);
        let can_next = self.can_next(size, snapshot);
        draw_button(
            frame,
            art,
            layout.prev_btn,
            prev_label,
            self.button_state(PhoneBookHit::PrevPage, can_prev),
        );
        draw_button(
            frame,
            art,
            layout.next_btn,
            next_label,
            self.button_state(PhoneBookHit::NextPage, can_next),
        );
        draw_button(
            frame,
            art,
            layout.cancel_btn,
            cancel_label,
            self.button_state(PhoneBookHit::Cancel, true),
        );

        if let Some(font) = body_font {
            let cur_page = self.effective_page(size, snapshot) + 1;
            let total_pages = self.page_count(size, snapshot);
            let text = format!("{cur_page} / {total_pages}");
            let tw = font.width(&text).min(layout.page_label.width);
            let tx = layout.page_label.x + ((layout.page_label.width - tw) / 2).max(0);
            let ty = layout.page_label.y + ((layout.page_label.height - font.line_height) / 2).max(2);
            draw_clipped_text(frame, font, &text, [tx, ty], layout.page_label.width, TEXT_HOVER);
        }
    }

    fn button_state(&self, target: PhoneBookHit, enabled: bool) -> ButtonState {
        if !enabled {
            ButtonState::Disabled
        } else if self.pressed == Some(target) && self.hovered == Some(target) {
            ButtonState::Selected
        } else if self.hovered == Some(target) {
            ButtonState::Hover
        } else {
            ButtonState::Normal
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonState {
    Normal,
    Selected,
    Hover,
    Disabled,
}

impl ButtonState {
    const fn cell(self) -> u32 {
        match self {
            Self::Normal => 0,
            Self::Selected => 1,
            Self::Hover => 2,
            Self::Disabled => 3,
        }
    }

    const fn text_color(self) -> [u8; 3] {
        match self {
            Self::Normal => TEXT_NORMAL,
            Self::Selected => TEXT_SELECTED,
            Self::Hover => TEXT_HOVER,
            Self::Disabled => TEXT_DISABLED,
        }
    }
}

fn draw_rel_bar(frame: &mut Frame, rect: Rect, value: i16, pos_color: [u8; 3]) {
    if rect.width <= 4 || rect.height <= 2 {
        return;
    }
    frame.fill(rect, BAR_TRACK_BG);
    draw_outline(frame, rect, BAR_TRACK_BORDER);
    let clamped = i32::from(value.clamp(-100, 100));
    let inner_x = rect.x + 1;
    let inner_y = rect.y + 1;
    let inner_w = rect.width - 2;
    let inner_h = rect.height - 2;
    let mid = inner_x + inner_w / 2;
    // Center zero tick.
    frame.fill(Rect::new(mid, inner_y, 1, inner_h), BAR_TRACK_BORDER);
    let half_w = inner_w / 2;
    if clamped > 0 {
        let fill_w = ((clamped * half_w) / 100).max(1).min(inner_x + inner_w - mid);
        frame.fill(Rect::new(mid, inner_y, fill_w, inner_h), pos_color);
    } else if clamped < 0 {
        let fill_w = (((-clamped) * half_w) / 100).max(1).min(mid - inner_x);
        frame.fill(Rect::new(mid - fill_w, inner_y, fill_w, inner_h), BAR_NEG);
    }
}

fn draw_button(frame: &mut Frame, art: &Art, rect: Rect, label: &str, state: ButtonState) {
    if rect.width <= 0 || rect.height <= 0 {
        return;
    }
    if let Some(img) = art.image(BUTTON)
        && img.width >= 4
        && img.height > 0
    {
        let cw = (img.width / 4) as i32;
        let ch = img.height as i32;
        let sx = cw.saturating_mul(state.cell() as i32);
        draw_three_slice_h(frame, img, Rect::new(sx, 0, cw, ch), rect);
    } else {
        let fill = match state {
            ButtonState::Normal => [38, 60, 148],
            ButtonState::Selected => [22, 112, 168],
            ButtonState::Hover => [64, 96, 196],
            ButtonState::Disabled => [34, 42, 74],
        };
        frame.fill(rect, fill);
        draw_outline(frame, rect, CARD_BORDER);
    }
    if let Some(font) = art.font(10).or_else(|| art.font(12)) {
        let avail_w = (rect.width - 8).max(8);
        let text_w = font.width(label).min(avail_w);
        let tx = rect.x + ((rect.width - text_w) / 2).max(4);
        let ty = rect.y + ((rect.height - font.line_height) / 2).max(2);
        draw_clipped_text(frame, font, label, [tx, ty], avail_w, state.text_color());
    }
}

fn draw_three_slice_h(frame: &mut Frame, image: &Image, src: Rect, dest: Rect) {
    let cap = (src.width / 4).min(dest.width / 2).max(1);
    let mid_src_w = (src.width - cap * 2).max(1);
    let mid_dst_w = (dest.width - cap * 2).max(0);
    draw_scaled_region(
        frame,
        image,
        Rect::new(src.x, src.y, cap, src.height),
        Rect::new(dest.x, dest.y, cap, dest.height),
    );
    if mid_dst_w > 0 {
        draw_scaled_region(
            frame,
            image,
            Rect::new(src.x + cap, src.y, mid_src_w, src.height),
            Rect::new(dest.x + cap, dest.y, mid_dst_w, dest.height),
        );
    }
    draw_scaled_region(
        frame,
        image,
        Rect::new(src.x + src.width - cap, src.y, cap, src.height),
        Rect::new(dest.x + dest.width - cap, dest.y, cap, dest.height),
    );
}

fn draw_aspect_fit(frame: &mut Frame, image: &Image, bounds: Rect) {
    if image.width == 0 || image.height == 0 || bounds.width <= 0 || bounds.height <= 0 {
        return;
    }
    let iw = i64::from(image.width);
    let ih = i64::from(image.height);
    let bw = i64::from(bounds.width);
    let bh = i64::from(bounds.height);
    let (dw, dh) = if iw * bh >= ih * bw {
        (bw, ((ih * bw) / iw).max(1))
    } else {
        (((iw * bh) / ih).max(1), bh)
    };
    let dx = bounds.x + ((bounds.width - dw as i32) / 2).max(0);
    let dy = bounds.y + ((bounds.height - dh as i32) / 2).max(0);
    let src = Rect::new(
        0,
        0,
        i32::try_from(image.width).unwrap_or(i32::MAX),
        i32::try_from(image.height).unwrap_or(i32::MAX),
    );
    draw_scaled_region(frame, image, src, Rect::new(dx, dy, dw as i32, dh as i32));
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
                && a > 0
            {
                frame.blend(x, y, [r, g, b], a);
            }
        }
    }
}

fn draw_outline(frame: &mut Frame, rect: Rect, color: [u8; 3]) {
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

fn draw_clipped_text(
    frame: &mut Frame,
    font: &Font,
    text: &str,
    at: [i32; 2],
    max_width: i32,
    color: [u8; 3],
) {
    if max_width <= 0 || text.is_empty() {
        return;
    }
    if font.width(text) <= max_width {
        font.draw(frame, text, at, color);
        return;
    }
    let ellipsis = "...";
    let ellipsis_w = font.width(ellipsis);
    if ellipsis_w > max_width {
        return;
    }
    let mut clipped = String::new();
    for ch in text.chars() {
        clipped.push(ch);
        if font.width(&clipped) + ellipsis_w > max_width {
            clipped.pop();
            break;
        }
    }
    clipped.push_str(ellipsis);
    font.draw(frame, &clipped, at, color);
}
