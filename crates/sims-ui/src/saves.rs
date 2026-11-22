//! Save & rotating autosave slot picker dialog, styled with Complete
//! Collection 1.91's native 9-slice dialog chrome (`3008`, `17`, `5107`).
//!
//! Per `docs/vision.md` (**Saves**):
//! - Saves are stored in REDB per `(neighborhood, lot)` with a manual save and
//!   rotating autosave slots.
//! - A missing, corrupt, or unsupported save reports a clear, non-destructive
//!   error and never replaces or deletes previous saves.
//!
//! Like every other `sims-ui` module, this module has zero persistence or
//! simulation coupling: callers pass a read-only [`SaveManagerSnapshot`] and
//! execute returned [`SaveManagerCommand`] values against the save store.

use crate::Result;
use crate::art::Art;
use crate::dialog::{BACKGROUND, BUTTON};
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::neighborhood::{self, art_id::POPUP_TILER};
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;
use std::cmp::Reverse;

/// `UIGraphics` resource IDs used by [`SaveManagerDialog`]:
/// `PopupInfoTiles.bmp` (`3008`), `WinBtn.bmp` (`17`), and
/// `HotspotPopupTiler.TGA` (`5107`).
pub const RESOURCES: [i32; 3] = [BACKGROUND, BUTTON, POPUP_TILER];

/// Loads the native 1.91 9-slice frames, button strip, and bitmap fonts used
/// by [`SaveManagerDialog`].
pub fn load_art(ui: &UiGraphics, language: u8) -> Result<Art> {
    Art::load(ui, language, &RESOURCES)
}

/// Standard 1.91 UI text colors (`0x6f2a94..0x6f2aa4`).
const TEXT_NORMAL: [u8; 3] = [195, 205, 205];
const TEXT_HOVER: [u8; 3] = [255, 255, 255];
const TEXT_SELECTED: [u8; 3] = [0, 255, 255];
const TEXT_DISABLED: [u8; 3] = [0x40, 0x5d, 0x5f];
const TEXT_WARNING: [u8; 3] = [255, 214, 102];
const TEXT_ERROR: [u8; 3] = [255, 118, 118];
const TEXT_OK: [u8; 3] = [120, 238, 160];

const DIALOG_FALLBACK_BG: [u8; 3] = [24, 34, 112];
const DIALOG_FALLBACK_BORDER: [u8; 3] = [96, 132, 224];
const BANNER_FALLBACK_BG: [u8; 3] = [68, 22, 34];
const BANNER_BORDER: [u8; 3] = [224, 112, 112];
const LIST_BG: [u8; 3] = [12, 20, 74];
const LIST_BORDER: [u8; 3] = [66, 96, 176];
const ROW_SELECTED_BG: [u8; 3] = [26, 70, 148];
const ROW_HOVER_BG: [u8; 3] = [22, 42, 112];
const ROW_ALT_BG: [u8; 3] = [15, 24, 84];
const ROW_CORRUPT_BG: [u8; 3] = [56, 20, 36];
const BADGE_INTACT_BG: [u8; 3] = [22, 74, 48];
const BADGE_CORRUPT_BG: [u8; 3] = [96, 24, 30];
const BADGE_UNSUPPORTED_BG: [u8; 3] = [92, 66, 18];

/// Minimum row height in pixels for one save slot entry.
const SLOT_ROW_HEIGHT: i32 = 36;
/// Width of the vertical scroll button strip when slots overflow.
const SCROLL_BTN_WIDTH: i32 = 22;

/// Whether a save slot is a manual save or one of Track 3's rotating REDB
/// autosave slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveSlotKind {
    Manual,
    Autosave { rotation_index: u32 },
}

/// Checksum & schema health of a save slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveHealth {
    /// Checksum verified and schema supported.
    Intact,
    /// Checksum mismatch or unreadable REDB payload.
    Corrupt { reason: String },
    /// Save schema version is newer or unsupported by this build.
    Unsupported { schema_version: u32, reason: String },
}

impl SaveHealth {
    /// Returns `true` when this slot can be loaded.
    pub const fn is_loadable(&self) -> bool {
        matches!(self, Self::Intact)
    }

    /// Short badge label shown on the slot row.
    pub const fn badge_label(&self) -> &'static str {
        match self {
            Self::Intact => "Intact",
            Self::Corrupt { .. } => "Corrupt",
            Self::Unsupported { .. } => "Unsupported",
        }
    }

    /// Detail reason for non-intact slots.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Intact => None,
            Self::Corrupt { reason } | Self::Unsupported { reason, .. } => {
                (!reason.trim().is_empty()).then_some(reason.as_str())
            }
        }
    }
}

/// Read-only summary of one manual or rotating autosave slot for a
/// `(neighborhood, lot)` pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveSlotSummary {
    /// Opaque stable slot identifier passed back in [`SaveManagerCommand`].
    pub slot_id: String,
    pub neighborhood: u8,
    pub lot: u16,
    /// Human-readable slot label (e.g. `"Manual Save"` or `"Autosave #3"`).
    pub label: String,
    pub kind: SaveSlotKind,
    /// In-game simulation day (1-based).
    pub sim_day: u32,
    /// In-game hour (`0..=23`).
    pub sim_hour: u8,
    /// In-game minute (`0..=59`).
    pub sim_minute: u8,
    /// Simulation tick stamped into the snapshot.
    pub tick: u64,
    /// Optional wall-clock Unix timestamp (seconds) for ordering ties.
    pub saved_at_epoch_secs: Option<u64>,
    /// Optional household name stored in the save header.
    pub household_name: Option<String>,
    pub health: SaveHealth,
}

impl SaveSlotSummary {
    /// Formats the simulation day, time, and tick (e.g. `"Day 4 14:30 · Tick 12800"`).
    pub fn sim_time_label(&self) -> String {
        let hour = self.sim_hour.min(23);
        let minute = self.sim_minute.min(59);
        format!(
            "Day {} {:02}:{:02} | Tick {}",
            self.sim_day, hour, minute, self.tick
        )
    }
}

/// Read-only save manager view model consumed by [`SaveManagerDialog`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveManagerSnapshot {
    pub neighborhood: u8,
    pub lot: u16,
    /// Display name for the lot (e.g. `"Neighborhood 1 · Lot 1 (Goth)"`).
    pub lot_label: String,
    pub slots: Vec<SaveSlotSummary>,
    /// Optional non-destructive error from a recent load/save attempt.
    pub active_error: Option<String>,
}

impl SaveManagerSnapshot {
    /// Returns slots ordered with manual save(s) first (newest first), followed
    /// by rotating autosave slots ordered newest-first by tick, timestamp,
    /// rotation index, and slot ID.
    pub fn ordered_slots(&self) -> Vec<&SaveSlotSummary> {
        let mut ordered: Vec<&SaveSlotSummary> = self.slots.iter().collect();
        ordered.sort_by_key(|slot| {
            let group = match slot.kind {
                SaveSlotKind::Manual => 0u8,
                SaveSlotKind::Autosave { .. } => 1u8,
            };
            let rot = match slot.kind {
                SaveSlotKind::Manual => 0u32,
                SaveSlotKind::Autosave { rotation_index } => rotation_index,
            };
            (
                group,
                Reverse(slot.tick),
                Reverse(slot.saved_at_epoch_secs.unwrap_or(0)),
                Reverse(rot),
                slot.slot_id.as_str(),
            )
        });
        ordered
    }

    /// Returns the newest intact slot across manual saves and rotating
    /// autosaves, ordered by highest tick then timestamp.
    pub fn newest_intact(&self) -> Option<&SaveSlotSummary> {
        self.slots
            .iter()
            .filter(|slot| slot.health.is_loadable())
            .max_by(|a, b| {
                a.tick
                    .cmp(&b.tick)
                    .then_with(|| {
                        a.saved_at_epoch_secs
                            .unwrap_or(0)
                            .cmp(&b.saved_at_epoch_secs.unwrap_or(0))
                    })
                    .then_with(|| {
                        let am = matches!(a.kind, SaveSlotKind::Manual);
                        let bm = matches!(b.kind, SaveSlotKind::Manual);
                        am.cmp(&bm)
                    })
                    .then_with(|| b.slot_id.cmp(&a.slot_id))
            })
    }

    /// Returns `true` if any slot is corrupt or unsupported, or an active
    /// non-destructive save/load error is present.
    pub fn has_corrupt_or_error(&self) -> bool {
        self.active_error
            .as_deref()
            .is_some_and(|e| !e.trim().is_empty())
            || self.slots.iter().any(|slot| !slot.health.is_loadable())
    }

    /// Non-destructive warning banner message per `docs/vision.md`, present
    /// whenever any slot is corrupt/unsupported or `active_error` is set.
    pub fn warning_banner_text(&self) -> Option<String> {
        let (summary, preservation) = self.warning_banner_lines()?;
        Some(format!("{summary} {preservation}"))
    }

    /// Two-line warning banner `(summary, preservation_reassurance)` so the
    /// non-destructive preservation guarantee always renders on its own
    /// dedicated line even at compact viewports or with long error messages.
    pub fn warning_banner_lines(&self) -> Option<(String, &'static str)> {
        const PRESERVED: &str = "Existing saves were preserved.";

        let corrupt_count = self
            .slots
            .iter()
            .filter(|s| matches!(s.health, SaveHealth::Corrupt { .. }))
            .count();
        let unsupported_count = self
            .slots
            .iter()
            .filter(|s| matches!(s.health, SaveHealth::Unsupported { .. }))
            .count();

        if let Some(err) = self
            .active_error
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some((err.to_owned(), PRESERVED));
        }
        if corrupt_count > 0 && unsupported_count > 0 {
            return Some((
                format!("{corrupt_count} corrupt, {unsupported_count} unsupported save(s)."),
                PRESERVED,
            ));
        }
        if corrupt_count > 0 {
            return Some((
                format!("{corrupt_count} corrupt save slot(s) detected."),
                PRESERVED,
            ));
        }
        if unsupported_count > 0 {
            return Some((
                format!("{unsupported_count} unsupported save schema(s) detected."),
                PRESERVED,
            ));
        }
        None
    }
}

/// Typed commands emitted by [`SaveManagerDialog`] for the host save store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveManagerCommand {
    /// Load the selected intact save slot.
    LoadSlot { slot_id: String },
    /// Delete the selected save slot (caller may confirm or remove from REDB).
    DeleteSlot { slot_id: String },
    /// Fall back to the newest intact save slot for this `(neighborhood, lot)`.
    FallbackToNewestIntact { slot_id: String },
    /// Dismiss the save manager dialog without making changes.
    Cancel,
}

/// Interactive hit targets inside [`SaveManagerDialog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveManagerHit {
    SlotRow(usize),
    ScrollUp,
    ScrollDown,
    LoadButton,
    FallbackButton,
    DeleteButton,
    CancelButton,
}

/// Computed screen-space geometry for [`SaveManagerDialog`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveManagerLayout {
    pub dialog: Rect,
    pub title_bar: Rect,
    pub warning_banner: Option<Rect>,
    pub list_area: Rect,
    /// Visible slot rows `(ordered_slot_index, row_rect)`.
    pub visible_rows: Vec<(usize, Rect)>,
    pub scroll_up: Option<Rect>,
    pub scroll_down: Option<Rect>,
    pub load_btn: Rect,
    pub fallback_btn: Rect,
    pub delete_btn: Rect,
    pub cancel_btn: Rect,
}

impl SaveManagerLayout {
    /// Returns the interactive control under `(x, y)`, if any.
    pub fn hit(&self, x: i32, y: i32) -> Option<SaveManagerHit> {
        if self.scroll_up.is_some_and(|r| r.contains(x, y)) {
            return Some(SaveManagerHit::ScrollUp);
        }
        if self.scroll_down.is_some_and(|r| r.contains(x, y)) {
            return Some(SaveManagerHit::ScrollDown);
        }
        for &(idx, rect) in &self.visible_rows {
            if rect.contains(x, y) {
                return Some(SaveManagerHit::SlotRow(idx));
            }
        }
        if self.load_btn.contains(x, y) {
            return Some(SaveManagerHit::LoadButton);
        }
        if self.fallback_btn.contains(x, y) {
            return Some(SaveManagerHit::FallbackButton);
        }
        if self.delete_btn.contains(x, y) {
            return Some(SaveManagerHit::DeleteButton);
        }
        if self.cancel_btn.contains(x, y) {
            return Some(SaveManagerHit::CancelButton);
        }
        None
    }
}

/// Interactive Save & Rotating Autosave Slot Picker dialog controller.
#[derive(Debug, Clone, Default)]
pub struct SaveManagerDialog {
    open: bool,
    selected_slot_id: Option<String>,
    scroll_offset: usize,
    follow_selection: bool,
    hovered: Option<SaveManagerHit>,
    pressed: Option<SaveManagerHit>,
    pressed_slot_id: Option<String>,
}

impl SaveManagerDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Opens the dialog and resets transient pointer/scroll state.
    pub fn open(&mut self) {
        self.open = true;
        self.scroll_offset = 0;
        self.follow_selection = false;
        self.hovered = None;
        self.pressed = None;
        self.pressed_slot_id = None;
    }

    /// Closes the dialog and clears transient pointer state.
    pub fn close(&mut self) {
        self.open = false;
        self.follow_selection = false;
        self.hovered = None;
        self.pressed = None;
        self.pressed_slot_id = None;
    }

    /// Explicitly selects a slot by ID.
    pub fn select_slot(&mut self, slot_id: impl Into<String>) {
        self.selected_slot_id = Some(slot_id.into());
        self.follow_selection = true;
    }

    /// Resolves the currently selected slot against `snapshot`, falling back to
    /// the first ordered slot when the stored ID is absent.
    pub fn selected_slot<'a>(
        &self,
        snapshot: &'a SaveManagerSnapshot,
    ) -> Option<&'a SaveSlotSummary> {
        let ordered = snapshot.ordered_slots();
        if let Some(id) = self.selected_slot_id.as_deref()
            && let Some(&slot) = ordered.iter().find(|s| s.slot_id == id)
        {
            return Some(slot);
        }
        ordered.first().copied()
    }

    fn selected_index(&self, ordered: &[&SaveSlotSummary]) -> Option<usize> {
        if ordered.is_empty() {
            return None;
        }
        if let Some(id) = self.selected_slot_id.as_deref()
            && let Some(idx) = ordered.iter().position(|s| s.slot_id == id)
        {
            return Some(idx);
        }
        Some(0)
    }

    fn effective_scroll(&self, ordered: &[&SaveSlotSummary], capacity: usize) -> usize {
        let max_scroll = ordered.len().saturating_sub(capacity);
        let mut scroll = self.scroll_offset.min(max_scroll);
        if self.follow_selection
            && let Some(idx) = self.selected_index(ordered)
        {
            if idx < scroll {
                scroll = idx;
            } else if idx >= scroll + capacity {
                scroll = (idx + 1).saturating_sub(capacity).min(max_scroll);
            }
        }
        scroll
    }

    /// Moves selection up by one slot in display order.
    pub fn select_prev(&mut self, snapshot: &SaveManagerSnapshot) -> bool {
        let ordered = snapshot.ordered_slots();
        let Some(idx) = self.selected_index(&ordered) else {
            return false;
        };
        if idx == 0 {
            return false;
        }
        let next = idx - 1;
        self.selected_slot_id = Some(ordered[next].slot_id.clone());
        self.follow_selection = true;
        if next < self.scroll_offset {
            self.scroll_offset = next;
        }
        true
    }

    /// Moves selection down by one slot in display order.
    pub fn select_next(&mut self, snapshot: &SaveManagerSnapshot) -> bool {
        let ordered = snapshot.ordered_slots();
        let Some(idx) = self.selected_index(&ordered) else {
            return false;
        };
        if idx + 1 >= ordered.len() {
            return false;
        }
        let next = idx + 1;
        self.selected_slot_id = Some(ordered[next].slot_id.clone());
        self.follow_selection = true;
        true
    }

    /// Scrolls the slot list by `delta_rows` (positive scrolls down).
    pub fn scroll(
        &mut self,
        delta_rows: i32,
        frame_size: [u32; 2],
        snapshot: &SaveManagerSnapshot,
    ) -> bool {
        let ordered = snapshot.ordered_slots();
        let layout = self.layout(frame_size, snapshot);
        let capacity = (layout.list_area.height / SLOT_ROW_HEIGHT).max(1) as usize;
        let max_scroll = ordered.len().saturating_sub(capacity);
        let current = self.effective_scroll(&ordered, capacity);
        self.follow_selection = false;
        let next = if delta_rows >= 0 {
            current.saturating_add(delta_rows as usize).min(max_scroll)
        } else {
            current.saturating_sub(delta_rows.unsigned_abs() as usize)
        };
        if next != self.scroll_offset {
            self.scroll_offset = next;
            true
        } else {
            false
        }
    }

    /// Returns whether the Load button is enabled for `snapshot`.
    pub fn can_load(&self, snapshot: &SaveManagerSnapshot) -> bool {
        self.selected_slot(snapshot)
            .is_some_and(|slot| slot.health.is_loadable())
    }

    /// Returns whether the Fallback-to-Newest-Intact button is enabled.
    pub fn can_fallback(&self, snapshot: &SaveManagerSnapshot) -> bool {
        snapshot.newest_intact().is_some()
            && (snapshot.has_corrupt_or_error()
                || self
                    .selected_slot(snapshot)
                    .is_some_and(|s| !s.health.is_loadable()))
    }

    /// Returns whether the Delete button is enabled.
    pub fn can_delete(&self, snapshot: &SaveManagerSnapshot) -> bool {
        self.selected_slot(snapshot).is_some()
    }

    /// Handles `Enter` key submission: loads the selected intact slot, or falls
    /// back to the newest intact slot when the selected slot is corrupt.
    pub fn submit(&self, snapshot: &SaveManagerSnapshot) -> Option<SaveManagerCommand> {
        if !self.open {
            return None;
        }
        if let Some(slot) = self.selected_slot(snapshot)
            && slot.health.is_loadable()
        {
            return Some(SaveManagerCommand::LoadSlot {
                slot_id: slot.slot_id.clone(),
            });
        }
        if self.can_fallback(snapshot)
            && let Some(intact) = snapshot.newest_intact()
        {
            return Some(SaveManagerCommand::FallbackToNewestIntact {
                slot_id: intact.slot_id.clone(),
            });
        }
        None
    }

    /// Handles `Escape` key press: closes the dialog and emits `Cancel`.
    pub fn escape(&mut self) -> Option<SaveManagerCommand> {
        if !self.open {
            return None;
        }
        self.close();
        Some(SaveManagerCommand::Cancel)
    }

    /// Computes responsive dialog geometry centered inside `frame_size`.
    pub fn layout(
        &self,
        frame_size: [u32; 2],
        snapshot: &SaveManagerSnapshot,
    ) -> SaveManagerLayout {
        let fw = i32::try_from(frame_size[0]).unwrap_or(800).max(1);
        let fh = i32::try_from(frame_size[1]).unwrap_or(600).max(1);

        let margin = 12;
        let max_w = (fw - margin * 2).max(220).min(fw);
        let max_h = (fh - margin * 2).max(190).min(fh);
        let width = 540.min(max_w);
        let height = 400.min(max_h);
        let x = ((fw - width) / 2).max(0);
        let y = ((fh - height) / 2).max(0);
        let dialog = Rect::new(x, y, width, height);

        let pad = 16;
        let inner_x = dialog.x + pad;
        let inner_w = (dialog.width - pad * 2).max(120);

        let title_bar = Rect::new(inner_x, dialog.y + 10, inner_w, 20);

        let footer_h = if dialog.height < 250 { 28 } else { 30 };
        let footer_y = dialog.y + dialog.height - footer_h - 12;
        let gap = if inner_w < 360 { 4 } else { 6 };
        let avail_btn_w = (inner_w - gap * 3).max(160);
        // Reserve enough width for "Newest Intact" (97px in 10pt shipped font + padding).
        let fallback_w = 108
            .max((avail_btn_w * 38) / 100)
            .min((avail_btn_w - 132).max(108));
        let rem_w = (avail_btn_w - fallback_w).max(90);
        let load_w = (rem_w * 30) / 100;
        let delete_w = (rem_w * 34) / 100;
        let cancel_w = (rem_w - load_w - delete_w).max(30);
        let load_btn = Rect::new(inner_x, footer_y, load_w, footer_h);
        let fallback_btn = Rect::new(
            load_btn.x + load_btn.width + gap,
            footer_y,
            fallback_w,
            footer_h,
        );
        let delete_btn = Rect::new(
            fallback_btn.x + fallback_btn.width + gap,
            footer_y,
            delete_w,
            footer_h,
        );
        let cancel_btn = Rect::new(
            delete_btn.x + delete_btn.width + gap,
            footer_y,
            cancel_w,
            footer_h,
        );

        let mut cursor_y = title_bar.y + title_bar.height + 4;
        let warning_banner = if snapshot.warning_banner_lines().is_some() {
            let banner_h = 40;
            let rect = Rect::new(inner_x, cursor_y, inner_w, banner_h);
            cursor_y = rect.y + rect.height + 4;
            Some(rect)
        } else {
            None
        };

        let list_h = (footer_y - cursor_y - 8).max(SLOT_ROW_HEIGHT);
        let list_area = Rect::new(inner_x, cursor_y, inner_w, list_h);

        let ordered = snapshot.ordered_slots();
        let ordered_len = ordered.len();
        let capacity = (list_area.height / SLOT_ROW_HEIGHT).max(1) as usize;
        let needs_scroll = ordered_len > capacity;
        let scroll = self.effective_scroll(&ordered, capacity);

        let row_w = if needs_scroll {
            (list_area.width - SCROLL_BTN_WIDTH - 4).max(60)
        } else {
            (list_area.width - 4).max(60)
        };

        let mut visible_rows = Vec::new();
        for vis in 0..capacity {
            let idx = scroll + vis;
            if idx >= ordered_len {
                break;
            }
            let ry = list_area.y + 2 + vis as i32 * SLOT_ROW_HEIGHT;
            let rh = (SLOT_ROW_HEIGHT - 2).min(list_area.y + list_area.height - 2 - ry);
            if rh > 0 {
                visible_rows.push((idx, Rect::new(list_area.x + 2, ry, row_w, rh)));
            }
        }

        let (scroll_up, scroll_down) = if needs_scroll {
            let sx = list_area.x + list_area.width - SCROLL_BTN_WIDTH - 1;
            let sh = (list_area.height / 2 - 3).clamp(16, 26);
            (
                Some(Rect::new(sx, list_area.y + 2, SCROLL_BTN_WIDTH, sh)),
                Some(Rect::new(
                    sx,
                    list_area.y + list_area.height - sh - 2,
                    SCROLL_BTN_WIDTH,
                    sh,
                )),
            )
        } else {
            (None, None)
        };

        SaveManagerLayout {
            dialog,
            title_bar,
            warning_banner,
            list_area,
            visible_rows,
            scroll_up,
            scroll_down,
            load_btn,
            fallback_btn,
            delete_btn,
            cancel_btn,
        }
    }

    /// Updates hover state for pointer coordinates `(x, y)`.
    pub fn pointer_move(
        &mut self,
        frame_size: [u32; 2],
        snapshot: &SaveManagerSnapshot,
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

    /// Handles pointer press; returns `true` when the dialog captured the event.
    pub fn pointer_down(
        &mut self,
        frame_size: [u32; 2],
        snapshot: &SaveManagerSnapshot,
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
        self.pressed_slot_id = match hit {
            Some(SaveManagerHit::LoadButton | SaveManagerHit::DeleteButton) => {
                self.selected_slot(snapshot).map(|s| s.slot_id.clone())
            }
            Some(SaveManagerHit::FallbackButton) => {
                snapshot.newest_intact().map(|s| s.slot_id.clone())
            }
            Some(SaveManagerHit::SlotRow(idx)) => {
                snapshot.ordered_slots().get(idx).map(|s| s.slot_id.clone())
            }
            _ => None,
        };
        hit.is_some() || layout.dialog.contains(x, y)
    }

    /// Handles pointer release, emitting a [`SaveManagerCommand`] when an
    /// enabled action button is activated.
    pub fn pointer_up(
        &mut self,
        frame_size: [u32; 2],
        snapshot: &SaveManagerSnapshot,
        x: i32,
        y: i32,
    ) -> Option<SaveManagerCommand> {
        if !self.open {
            return None;
        }
        let pressed = self.pressed.take()?;
        let pressed_slot_id = self.pressed_slot_id.take();
        let layout = self.layout(frame_size, snapshot);
        let released = layout.hit(x, y);
        self.hovered = released;
        if released != Some(pressed) {
            return None;
        }
        match pressed {
            SaveManagerHit::SlotRow(idx) => {
                let ordered = snapshot.ordered_slots();
                if let Some(slot) = ordered.get(idx)
                    && pressed_slot_id.as_deref() == Some(slot.slot_id.as_str())
                {
                    self.selected_slot_id = Some(slot.slot_id.clone());
                    self.follow_selection = false;
                }
                None
            }
            SaveManagerHit::ScrollUp => {
                self.scroll(-1, frame_size, snapshot);
                None
            }
            SaveManagerHit::ScrollDown => {
                self.scroll(1, frame_size, snapshot);
                None
            }
            SaveManagerHit::LoadButton => {
                let expected = pressed_slot_id?;
                self.selected_slot(snapshot)
                    .filter(|slot| slot.slot_id == expected && slot.health.is_loadable())
                    .map(|slot| SaveManagerCommand::LoadSlot {
                        slot_id: slot.slot_id.clone(),
                    })
            }
            SaveManagerHit::FallbackButton => {
                let expected = pressed_slot_id?;
                self.can_fallback(snapshot)
                    .then(|| snapshot.newest_intact())
                    .flatten()
                    .filter(|slot| slot.slot_id == expected)
                    .map(|slot| SaveManagerCommand::FallbackToNewestIntact {
                        slot_id: slot.slot_id.clone(),
                    })
            }
            SaveManagerHit::DeleteButton => {
                let expected = pressed_slot_id?;
                self.selected_slot(snapshot)
                    .filter(|slot| slot.slot_id == expected)
                    .map(|slot| SaveManagerCommand::DeleteSlot {
                        slot_id: slot.slot_id.clone(),
                    })
            }
            SaveManagerHit::CancelButton => {
                self.close();
                Some(SaveManagerCommand::Cancel)
            }
        }
    }

    fn button_state(&self, target: SaveManagerHit, enabled: bool) -> ButtonState {
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

    /// Draws the save & rotating autosave picker dialog into `frame`.
    pub fn draw(&self, frame: &mut Frame, snapshot: &SaveManagerSnapshot, art: &Art) {
        if !self.open {
            return;
        }
        let layout = self.layout([frame.width(), frame.height()], snapshot);
        draw_dialog_frame(frame, art, layout.dialog);

        let title_font = art.font(12).or_else(|| art.font(10));
        let body_font = art.font(10).or_else(|| art.font(12));
        let small_font = art.font(9).or_else(|| art.font(10));

        // Header title.
        if let Some(font) = title_font {
            let title = if snapshot.lot_label.trim().is_empty() {
                format!(
                    "Saves & Autosaves — N{} Lot {}",
                    snapshot.neighborhood, snapshot.lot
                )
            } else {
                format!("Saves & Autosaves — {}", snapshot.lot_label)
            };
            draw_clipped_text(
                frame,
                font,
                &title,
                [layout.title_bar.x, layout.title_bar.y + 2],
                layout.title_bar.width,
                TEXT_HOVER,
            );
        }

        // Non-destructive warning banner when corrupt/unsupported saves or active_error exist.
        if let (Some(banner), Some((summary, preserved))) =
            (layout.warning_banner, snapshot.warning_banner_lines())
        {
            draw_warning_banner(frame, art, banner);
            if let Some(font) = small_font {
                let text_w = (banner.width - 14).max(20);
                let lh = font.line_height.clamp(12, 18);
                draw_clipped_text(
                    frame,
                    font,
                    &summary,
                    [banner.x + 7, banner.y + 3],
                    text_w,
                    TEXT_WARNING,
                );
                draw_clipped_text(
                    frame,
                    font,
                    preserved,
                    [banner.x + 7, banner.y + 3 + lh],
                    text_w,
                    TEXT_HOVER,
                );
            }
        }

        // Slot list container.
        frame.fill(layout.list_area, LIST_BG);
        draw_outline(frame, layout.list_area, LIST_BORDER);

        let ordered = snapshot.ordered_slots();
        let selected_idx = self.selected_index(&ordered);

        if ordered.is_empty() {
            if let Some(font) = body_font {
                draw_clipped_text(
                    frame,
                    font,
                    "No saves or autosaves found for this lot.",
                    [layout.list_area.x + 10, layout.list_area.y + 10],
                    (layout.list_area.width - 20).max(20),
                    TEXT_NORMAL,
                );
            }
        } else {
            for &(idx, row_rect) in &layout.visible_rows {
                let Some(slot) = ordered.get(idx) else {
                    continue;
                };
                let is_selected = selected_idx == Some(idx);
                let is_hovered = self.hovered == Some(SaveManagerHit::SlotRow(idx));
                let bg = if is_selected {
                    ROW_SELECTED_BG
                } else if !slot.health.is_loadable() {
                    ROW_CORRUPT_BG
                } else if is_hovered {
                    ROW_HOVER_BG
                } else if idx % 2 == 1 {
                    ROW_ALT_BG
                } else {
                    LIST_BG
                };
                frame.fill(row_rect, bg);
                if is_selected {
                    draw_outline(frame, row_rect, TEXT_SELECTED);
                }

                // Health badge on the right side of the row (wide enough for "Unsupported").
                let badge_w = 88.min((row_rect.width / 2).max(84));
                let badge_rect = Rect::new(
                    row_rect.x + row_rect.width - badge_w - 6,
                    row_rect.y + 6,
                    badge_w,
                    (row_rect.height - 12).clamp(14, 20),
                );
                let (badge_bg, badge_fg) = match slot.health {
                    SaveHealth::Intact => (BADGE_INTACT_BG, TEXT_OK),
                    SaveHealth::Corrupt { .. } => (BADGE_CORRUPT_BG, TEXT_ERROR),
                    SaveHealth::Unsupported { .. } => (BADGE_UNSUPPORTED_BG, TEXT_WARNING),
                };
                frame.fill(badge_rect, badge_bg);
                draw_outline(frame, badge_rect, badge_fg);
                if let Some(font) = small_font {
                    let label = slot.health.badge_label();
                    let tw = font.width(label).min((badge_rect.width - 6).max(8));
                    let tx = badge_rect.x + ((badge_rect.width - tw) / 2).max(3);
                    let ty = badge_rect.y + ((badge_rect.height - font.line_height) / 2).max(1);
                    draw_clipped_text(
                        frame,
                        font,
                        label,
                        [tx, ty],
                        (badge_rect.width - 6).max(8),
                        badge_fg,
                    );
                }

                // Slot label + household + sim time/tick.
                let text_w = (badge_rect.x - row_rect.x - 14).max(20);
                let kind_tag = match slot.kind {
                    SaveSlotKind::Manual => "[Manual]",
                    SaveSlotKind::Autosave { .. } => "[Auto]",
                };
                let headline = match slot.household_name.as_deref() {
                    Some(hh) if !hh.trim().is_empty() => {
                        format!("{kind_tag} {} — {hh}", slot.label)
                    }
                    _ => format!("{kind_tag} {}", slot.label),
                };
                if let Some(font) = body_font {
                    draw_clipped_text(
                        frame,
                        font,
                        &headline,
                        [row_rect.x + 8, row_rect.y + 3],
                        text_w,
                        if is_selected {
                            TEXT_SELECTED
                        } else if slot.health.is_loadable() {
                            TEXT_HOVER
                        } else {
                            TEXT_ERROR
                        },
                    );
                }
                if let Some(font) = small_font {
                    let subline = match slot.health.detail() {
                        Some(reason) => format!("{} · {reason}", slot.sim_time_label()),
                        None => slot.sim_time_label(),
                    };
                    draw_clipped_text(
                        frame,
                        font,
                        &subline,
                        [row_rect.x + 8, row_rect.y + 18],
                        text_w,
                        if slot.health.is_loadable() {
                            TEXT_NORMAL
                        } else {
                            TEXT_WARNING
                        },
                    );
                }
            }
        }

        // Scroll buttons when overflowing.
        let capacity = (layout.list_area.height / SLOT_ROW_HEIGHT).max(1) as usize;
        let max_scroll = ordered.len().saturating_sub(capacity);
        let eff_scroll = self.effective_scroll(&ordered, capacity);
        if let Some(up) = layout.scroll_up {
            draw_button(
                frame,
                art,
                up,
                "^",
                self.button_state(SaveManagerHit::ScrollUp, eff_scroll > 0),
            );
        }
        if let Some(down) = layout.scroll_down {
            draw_button(
                frame,
                art,
                down,
                "v",
                self.button_state(SaveManagerHit::ScrollDown, eff_scroll < max_scroll),
            );
        }

        // Footer action buttons.
        draw_button(
            frame,
            art,
            layout.load_btn,
            "Load",
            self.button_state(SaveManagerHit::LoadButton, self.can_load(snapshot)),
        );
        draw_button(
            frame,
            art,
            layout.fallback_btn,
            "Newest Intact",
            self.button_state(SaveManagerHit::FallbackButton, self.can_fallback(snapshot)),
        );
        draw_button(
            frame,
            art,
            layout.delete_btn,
            "Delete",
            self.button_state(SaveManagerHit::DeleteButton, self.can_delete(snapshot)),
        );
        draw_button(
            frame,
            art,
            layout.cancel_btn,
            "Cancel",
            self.button_state(SaveManagerHit::CancelButton, true),
        );
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

fn draw_dialog_frame(frame: &mut Frame, art: &Art, rect: Rect) {
    if let Some(tiler) = art.image(BACKGROUND)
        && tiler.width >= 3
        && tiler.height >= 3
    {
        neighborhood::draw_nine_slice(frame, tiler, rect);
    } else {
        frame.fill(rect, DIALOG_FALLBACK_BG);
        draw_outline(frame, rect, DIALOG_FALLBACK_BORDER);
    }
}

fn draw_warning_banner(frame: &mut Frame, art: &Art, rect: Rect) {
    if let Some(tiler) = art.image(POPUP_TILER)
        && tiler.width >= 3
        && tiler.height >= 3
    {
        neighborhood::draw_nine_slice(frame, tiler, rect);
        // Subtle red tint + border to emphasize non-destructive warning.
        for y in rect.y..rect.y.saturating_add(rect.height) {
            for x in rect.x..rect.x.saturating_add(rect.width) {
                frame.blend(x, y, BANNER_FALLBACK_BG, 140);
            }
        }
    } else {
        frame.fill(rect, BANNER_FALLBACK_BG);
    }
    draw_outline(frame, rect, BANNER_BORDER);
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
        draw_outline(frame, rect, LIST_BORDER);
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
