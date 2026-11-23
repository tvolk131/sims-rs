//! Household Photo Album / Family Storybook modal viewer & editor (`cWinPhotoAlbum`).
//!
//! Styled with Complete Collection 1.91's native 9-slice dialog chrome
//! (`PopupInfoTiles.bmp` `3008`, `WinBtn.bmp` `17`, `HotspotPopupTiler.TGA`
//! `5107`, `CamViewAlbum.bmp` `1105`) and `cTSWinTextEdit2` text inputs.
//!
//! All user-facing chrome strings can be loaded dynamically at runtime from
//! `GameData/UIText.iff` (`STR# 140` `cammodestrs`, `STR# 141` `scrapbookstrs`,
//! `STR# 142` `DefaultDialogButtons`, `STR# 154` `MiscStrings`) via
//! [`AlbumStrings::load_from_install`], with zero verbatim game prose embedded
//! in source.
//!
//! Like every other `sims-ui` screen, this module has zero persistence or
//! simulation coupling: callers pass a read-only [`AlbumSnapshot`] (ready for
//! REDB photo storage) and execute returned [`AlbumCommand`] values in
//! `sims-game`.

use crate::art::Art;
use crate::dialog::{BACKGROUND, BUTTON, CARET_BLINK_US, draw_nine_slice};
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::hud::rgb565;
use crate::neighborhood::{art_id::POPUP_TILER, draw_outline, draw_tooltip};
use crate::{Error, Result};
use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// `GameData/UIText.iff` string table for Camera Mode tooltips (`cammodestrs`).
pub const CAM_MODE_STRINGS: u16 = 140;
/// `GameData/UIText.iff` string table for the Photo Album (`scrapbookstrs`).
pub const SCRAPBOOK_STRINGS: u16 = 141;
/// `GameData/UIText.iff` default dialog button labels (`OK`, `Cancel`, `Yes`, `No`).
pub const DIALOG_BUTTON_STRINGS: u16 = 142;
/// `GameData/UIText.iff` miscellaneous UI strings (`MiscStrings`).
pub const MISC_STRINGS: u16 = 154;

/// `UIGraphics/Res_CPanel.h` `kCamViewAlbum` (`CPanel/Buttons/CamViewAlbum.bmp`).
pub const CAM_VIEW_ALBUM: i32 = 1105;

/// `UIGraphics/Res_CPanel.h` native Scrapbook / Photo Album artwork (`1200..=1206`).
pub const SCRAP_BACK: i32 = 1200;
pub const SCRAP_DONE: i32 = 1201;
pub const SCRAP_DELETE: i32 = 1202;
pub const SCRAP_LEFT: i32 = 1203;
pub const SCRAP_RIGHT: i32 = 1204;
pub const SCRAP_HOME: i32 = 1205;
pub const SCRAP_END: i32 = 1206;

/// Native `scrapback.bmp` dimensions (`700×560`).
pub const NATIVE_WIDTH: i32 = 700;
pub const NATIVE_HEIGHT: i32 = 560;

/// All 7 native Scrapbook / Photo Album `UIGraphics` resource IDs (`1200..=1206`).
pub const SCRAP_RESOURCES: [i32; 7] = [
    SCRAP_BACK,
    SCRAP_DONE,
    SCRAP_DELETE,
    SCRAP_LEFT,
    SCRAP_RIGHT,
    SCRAP_HOME,
    SCRAP_END,
];

/// Required `UIGraphics` fallback resource IDs used by [`PhotoAlbumDialog`].
pub const RESOURCES: [i32; 4] = [BACKGROUND, BUTTON, POPUP_TILER, CAM_VIEW_ALBUM];

/// Maximum character length of a photo title.
pub const MAX_TITLE_LEN: usize = 80;
/// Maximum character length of a photo storybook caption.
pub const MAX_CAPTION_LEN: usize = 512;

/// Native 1.91 UI text colors (`0x6f2a94..0x6f2aa4`).
const TEXT_NORMAL: [u8; 3] = [195, 205, 205];
const TEXT_HOVER: [u8; 3] = [255, 255, 255];
const TEXT_SELECTED: [u8; 3] = [0, 255, 255];
const TEXT_DISABLED: [u8; 3] = [0x40, 0x5d, 0x5f];
const TEXT_MUTED: [u8; 3] = [132, 152, 186];
const TEXT_WARNING: [u8; 3] = [255, 214, 102];

/// `cTSWinTextEdit2` background fill (`RGB(0, 0, 0x52)` quantized to RGB565).
const ENTRY_FILL: [u8; 3] = rgb565(0x000a);
const ENTRY_BORDER: [u8; 3] = [88, 120, 204];
const ENTRY_FOCUS_BORDER: [u8; 3] = [0, 255, 255];
const DIALOG_FALLBACK_BG: [u8; 3] = [24, 34, 112];
const DIALOG_FALLBACK_BORDER: [u8; 3] = [96, 132, 224];
const PHOTO_MATTE_BG: [u8; 3] = [8, 12, 34];
const PHOTO_MATTE_BORDER: [u8; 3] = [110, 144, 224];
const STRIP_BG: [u8; 3] = [12, 20, 74];
const STRIP_BORDER: [u8; 3] = [66, 96, 176];
const THUMB_BG: [u8; 3] = [18, 28, 88];
const THUMB_SELECTED_BORDER: [u8; 3] = [0, 255, 255];
const THUMB_HOVER_BORDER: [u8; 3] = [210, 228, 255];
const CONFIRM_BG: [u8; 3] = [56, 24, 36];
const CONFIRM_BORDER: [u8; 3] = [228, 128, 128];

/// Loads the native 1.91 Scrapbook bitmaps (`1200..=1206`), 9-slice frames,
/// button strip, album icon, and bitmap fonts used by [`PhotoAlbumDialog`].
pub fn load_art(ui: &UiGraphics, language: u8) -> Result<Art> {
    Art::load_with_optional(ui, language, &RESOURCES, &SCRAP_RESOURCES)
}

/// Opaque REDB-ready identifier for a household photo album entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhotoId(pub u64);

/// Read-only view-model of a single photo / storybook page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumEntry {
    pub id: PhotoId,
    pub title: String,
    pub caption: String,
    /// Full-size photo bitmap (drawn aspect-preserving inside the viewport).
    pub image: Option<Image>,
    /// Optional pre-scaled thumbnail bitmap; falls back to scaling `image`.
    pub thumbnail: Option<Image>,
    /// Optional in-game day number when the photo was taken.
    pub sim_day: Option<u32>,
}

/// Read-only household album state passed into [`PhotoAlbumDialog`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumSnapshot {
    pub family_name: String,
    pub entries: Vec<AlbumEntry>,
    /// Whether the Camera Snapshot trigger button is currently available
    /// (e.g. enabled on a live lot, disabled on the neighborhood screen).
    pub can_capture_snapshot: bool,
}

/// Localized chrome strings for [`PhotoAlbumDialog`].
///
/// Use [`AlbumStrings::load_from_install`] to populate from `GameData/UIText.iff`
/// at runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumStrings {
    pub title: String,
    pub done: String,
    pub done_tooltip: String,
    pub delete: String,
    pub delete_tooltip: String,
    pub confirm_yes: String,
    pub confirm_no: String,
    pub first_tooltip: String,
    pub prev_tooltip: String,
    pub next_tooltip: String,
    pub last_tooltip: String,
    pub empty_caption: String,
    pub add_page: String,
    pub snapshot: String,
    pub move_earlier: String,
    pub move_later: String,
    pub save: String,
    pub title_label: String,
    pub caption_label: String,
}

impl Default for AlbumStrings {
    fn default() -> Self {
        Self {
            title: "Album".to_owned(),
            done: "Done".to_owned(),
            done_tooltip: "Close".to_owned(),
            delete: "Delete".to_owned(),
            delete_tooltip: "Delete".to_owned(),
            confirm_yes: "Yes".to_owned(),
            confirm_no: "No".to_owned(),
            first_tooltip: "First".to_owned(),
            prev_tooltip: "Previous".to_owned(),
            next_tooltip: "Next".to_owned(),
            last_tooltip: "Last".to_owned(),
            empty_caption: "(empty)".to_owned(),
            add_page: "Add".to_owned(),
            snapshot: "Snap".to_owned(),
            move_earlier: "Move <".to_owned(),
            move_later: "Move >".to_owned(),
            save: "Save".to_owned(),
            title_label: "Title".to_owned(),
            caption_label: "Caption".to_owned(),
        }
    }
}

impl AlbumStrings {
    /// Loads localized Photo Album strings from `GameData/UIText.iff`
    /// (`STR# 140`, `141`, `142`, and `154`). Missing entries keep their
    /// concise default fallback labels.
    pub fn load_from_install(install: &Install, language: u8) -> Result<Self> {
        let path = install
            .file("GameData/UIText.iff")
            .map_err(Error::Content)?;
        let bytes = std::fs::read(&path).map_err(Error::Io)?;
        let iff = Iff::parse(bytes).map_err(Error::Format)?;
        let mut strings = Self::default();

        let read_table = |id: u16| -> Option<StringTable> {
            let chunk = iff.get(b"STR#", id)?;
            StringTable::parse(iff.data(chunk)).ok()
        };

        if let Some(table) = read_table(CAM_MODE_STRINGS)
            && let Some(s) = lookup_clean(&table, 9, language)
        {
            strings.title = s;
        }
        if let Some(table) = read_table(SCRAPBOOK_STRINGS) {
            if let Some(s) = lookup_clean(&table, 4, language) {
                strings.done = s;
            }
            if let Some(s) = lookup_clean(&table, 5, language) {
                strings.done_tooltip = s;
            }
            if let Some(s) = lookup_clean(&table, 7, language) {
                strings.delete = s;
            }
            if let Some(s) = lookup_clean(&table, 8, language) {
                strings.delete_tooltip = s;
            }
            if let Some(s) = lookup_clean(&table, 10, language) {
                strings.prev_tooltip = s;
            }
            if let Some(s) = lookup_clean(&table, 12, language) {
                strings.first_tooltip = s;
            }
            if let Some(s) = lookup_clean(&table, 14, language) {
                strings.next_tooltip = s;
            }
            if let Some(s) = lookup_clean(&table, 16, language) {
                strings.last_tooltip = s;
            }
        }
        if let Some(table) = read_table(DIALOG_BUTTON_STRINGS) {
            if let Some(s) = lookup_clean(&table, 2, language) {
                strings.confirm_yes = s;
            }
            if let Some(s) = lookup_clean(&table, 3, language) {
                strings.confirm_no = s;
            }
        }
        if let Some(table) = read_table(MISC_STRINGS)
            && let Some(s) = lookup_clean(&table, 3, language)
        {
            strings.empty_caption = s;
        }

        Ok(strings)
    }
}

/// Strips Complete Collection hotkey/flag prefixes (`&`, `*`) from a `STR#` entry.
fn clean_ui_label(raw: &str) -> String {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix('*')
        .or_else(|| trimmed.strip_prefix('&'))
        .unwrap_or(trimmed);
    stripped.replace('&', "").trim().to_owned()
}

fn lookup_clean(table: &StringTable, index: usize, language: u8) -> Option<String> {
    let raw = table.get(index, language).or_else(|| table.get(index, 1))?;
    let cleaned = clean_ui_label(raw);
    (!cleaned.is_empty()).then_some(cleaned)
}

/// Actions emitted by [`PhotoAlbumDialog`] for the host (`sims-game` / REDB).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlbumCommand {
    /// Close the Photo Album modal.
    Close,
    /// User selected a different photo entry.
    SelectEntry(PhotoId),
    /// User clicked the Camera Snapshot button to capture a new lot screenshot.
    CaptureSnapshot,
    /// User clicked Add to append a blank storybook page.
    AddBlankPage,
    /// User confirmed deletion of an album entry.
    DeleteEntry(PhotoId),
    /// User reordered an album entry to `new_index` (`0..entries.len()`).
    MoveEntry { id: PhotoId, new_index: usize },
    /// User committed edited title and caption text for `id`.
    UpdateEntryText {
        id: PhotoId,
        title: String,
        caption: String,
    },
}

/// Keyboard input events accepted by [`PhotoAlbumDialog::handle_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumKey {
    Char(char),
    Backspace,
    Delete,
    Enter,
    Tab,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// Save shortcut (`Ctrl+S`).
    SaveShortcut,
}

/// Which text input field currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditFocus {
    Title,
    Caption,
}

/// Interactive controls inside [`PhotoAlbumDialog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlId {
    First,
    Prev,
    Next,
    Last,
    AddPage,
    CaptureSnapshot,
    MoveEarlier(PhotoId),
    MoveLater(PhotoId),
    Delete(PhotoId),
    ConfirmDeleteYes(PhotoId),
    ConfirmDeleteNo,
    Save(PhotoId),
    Done,
    TitleBox,
    CaptionBox,
    StripPrev,
    StripNext,
    Thumbnail(PhotoId),
}

/// Computed layout rectangles for a given viewport size.
#[derive(Debug, Clone)]
pub struct AlbumLayout {
    pub modal: Rect,
    /// Whether this layout uses the native `700×560` `scrapback.bmp` (`1200..=1206`) coordinates.
    pub native_scrap: bool,
    pub header_title: [i32; 2],
    pub page_counter: [i32; 2],
    pub photo_viewport: Rect,
    pub title_label_pos: [i32; 2],
    pub title_box: Rect,
    pub caption_label_pos: [i32; 2],
    pub caption_box: Rect,
    pub btn_snapshot: Rect,
    pub btn_add: Rect,
    pub btn_move_earlier: Rect,
    pub btn_move_later: Rect,
    pub btn_delete: Rect,
    pub btn_save: Rect,
    pub btn_done: Rect,
    pub confirm_bar: Rect,
    pub btn_confirm_yes: Rect,
    pub btn_confirm_no: Rect,
    pub btn_first: Rect,
    pub btn_prev: Rect,
    pub btn_next: Rect,
    pub btn_last: Rect,
    pub strip_box: Rect,
    pub strip_prev: Rect,
    pub strip_next: Rect,
    pub thumb_slots: Vec<Rect>,
}

/// Interactive Photo Album / Family Storybook modal controller.
#[derive(Debug, Clone)]
pub struct PhotoAlbumDialog {
    open: bool,
    strings: AlbumStrings,
    selected_id: Option<PhotoId>,
    /// Draft text synchronized to `selected_id`.
    draft_title: String,
    draft_caption: String,
    title_cursor: usize,
    caption_cursor: usize,
    caption_scroll_line: usize,
    dirty: bool,
    focus: Option<EditFocus>,
    /// Horizontal scroll offset (first visible index) in the thumbnail strip.
    strip_scroll: usize,
    /// Entry awaiting inline delete confirmation, if any.
    pending_delete: Option<PhotoId>,
    /// Pointer state for hover tooltips and press-bound clicks.
    pointer: Option<[i32; 2]>,
    hovered: Option<ControlId>,
    pressed: Option<ControlId>,
    /// Blink timer in microseconds (`CARET_BLINK_US` = 500 ms).
    blink_us: u64,
}

impl Default for PhotoAlbumDialog {
    fn default() -> Self {
        Self::new(AlbumStrings::default())
    }
}

impl PhotoAlbumDialog {
    /// Creates a closed Photo Album modal with the given localized strings.
    pub fn new(strings: AlbumStrings) -> Self {
        Self {
            open: false,
            strings,
            selected_id: None,
            draft_title: String::new(),
            draft_caption: String::new(),
            title_cursor: 0,
            caption_cursor: 0,
            caption_scroll_line: 0,
            dirty: false,
            focus: None,
            strip_scroll: 0,
            pending_delete: None,
            pointer: None,
            hovered: None,
            pressed: None,
            blink_us: 0,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn strings(&self) -> &AlbumStrings {
        &self.strings
    }

    pub fn set_strings(&mut self, strings: AlbumStrings) {
        self.strings = strings;
    }

    /// Opens the Photo Album modal and synchronizes selection to `snapshot`.
    pub fn open(&mut self, snapshot: &AlbumSnapshot) {
        self.open = true;
        self.pending_delete = None;
        self.pressed = None;
        self.focus = None;
        self.blink_us = 0;
        self.sync_snapshot(snapshot);
    }

    /// Closes the Photo Album modal and clears transient interaction state.
    pub fn close(&mut self) {
        self.open = false;
        self.pending_delete = None;
        self.pressed = None;
        self.hovered = None;
        self.focus = None;
    }

    /// Currently selected photo ID, if any.
    pub fn selected_id(&self) -> Option<PhotoId> {
        self.selected_id
    }

    /// Currently focused text field, if any.
    pub fn focus(&self) -> Option<EditFocus> {
        self.focus
    }

    /// Current draft title in the editor.
    pub fn draft_title(&self) -> &str {
        &self.draft_title
    }

    /// Current draft caption in the editor.
    pub fn draft_caption(&self) -> &str {
        &self.draft_caption
    }

    /// Whether the user has uncommitted edits to the selected entry's title or caption.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Entry currently awaiting inline delete confirmation, if any.
    pub fn pending_delete(&self) -> Option<PhotoId> {
        self.pending_delete
    }

    /// Advances the 500 ms caret blink timer.
    pub fn advance_time(&mut self, dt_us: u64) {
        self.blink_us = self.blink_us.wrapping_add(dt_us);
    }

    /// Advances the 500 ms caret blink timer and returns `true` only when a text
    /// field is focused and the caret visibility phase toggled.
    pub fn sync_blink(&mut self, dt_us: u64) -> bool {
        if !self.open || self.focus.is_none() {
            return false;
        }
        let prev_phase = (self.blink_us / CARET_BLINK_US) & 1;
        self.blink_us = self.blink_us.wrapping_add(dt_us);
        let next_phase = (self.blink_us / CARET_BLINK_US) & 1;
        prev_phase != next_phase
    }

    /// Exact microseconds until the Photo Album text-entry caret next flips
    /// visibility, or `None` when closed or no text field is focused.
    pub fn next_blink_us(&self) -> Option<u64> {
        if !self.open || self.focus.is_none() {
            return None;
        }
        let rem = CARET_BLINK_US - (self.blink_us % CARET_BLINK_US);
        Some(rem.max(1_000))
    }

    /// Synchronizes internal selection and draft state against a new `snapshot`
    /// (e.g., after the host adds, deletes, or reorders entries).
    pub fn sync_snapshot(&mut self, snapshot: &AlbumSnapshot) {
        if snapshot.entries.is_empty() {
            self.selected_id = None;
            self.draft_title.clear();
            self.draft_caption.clear();
            self.title_cursor = 0;
            self.caption_cursor = 0;
            self.caption_scroll_line = 0;
            self.dirty = false;
            self.pending_delete = None;
            self.strip_scroll = 0;
            return;
        }

        if let Some(pending) = self.pending_delete
            && !snapshot.entries.iter().any(|e| e.id == pending)
        {
            self.pending_delete = None;
        }

        let current_idx = self
            .selected_id
            .and_then(|id| snapshot.entries.iter().position(|e| e.id == id));

        match current_idx {
            Some(idx) => {
                if !self.dirty {
                    let entry = &snapshot.entries[idx];
                    if self.draft_title != entry.title || self.draft_caption != entry.caption {
                        self.load_draft(entry);
                    }
                }
            }
            None => {
                let first = &snapshot.entries[0];
                self.selected_id = Some(first.id);
                self.load_draft(first);
            }
        }
    }

    /// Explicitly selects an entry by `PhotoId` (discarding any uncommitted draft
    /// on the previous entry). Returns `true` if the entry exists in `snapshot`.
    pub fn select_entry(&mut self, snapshot: &AlbumSnapshot, id: PhotoId) -> bool {
        let Some(entry) = snapshot.entries.iter().find(|e| e.id == id) else {
            return false;
        };
        self.selected_id = Some(entry.id);
        self.pending_delete = None;
        self.load_draft(entry);
        true
    }

    /// Commits any unsaved title/caption edits for the currently selected entry.
    pub fn commit_edits(&mut self) -> Option<AlbumCommand> {
        if !self.dirty {
            return None;
        }
        let id = self.selected_id?;
        self.dirty = false;
        Some(AlbumCommand::UpdateEntryText {
            id,
            title: self.draft_title.clone(),
            caption: self.draft_caption.clone(),
        })
    }

    fn load_draft(&mut self, entry: &AlbumEntry) {
        self.draft_title = entry.title.clone();
        self.draft_caption = entry.caption.clone();
        self.title_cursor = self.draft_title.len();
        self.caption_cursor = self.draft_caption.len();
        self.caption_scroll_line = 0;
        self.dirty = false;
        self.blink_us = 0;
    }

    fn selected_index(&self, snapshot: &AlbumSnapshot) -> Option<usize> {
        let id = self.selected_id?;
        snapshot.entries.iter().position(|e| e.id == id)
    }

    fn ensure_strip_visible(&mut self, idx: usize, visible_slots: usize, total: usize) {
        if visible_slots == 0 || total == 0 {
            self.strip_scroll = 0;
            return;
        }
        let max_scroll = total.saturating_sub(visible_slots);
        if idx < self.strip_scroll {
            self.strip_scroll = idx;
        } else if idx >= self.strip_scroll + visible_slots {
            self.strip_scroll = idx + 1 - visible_slots;
        }
        self.strip_scroll = self.strip_scroll.min(max_scroll);
    }

    fn switch_to_index(
        &mut self,
        snapshot: &AlbumSnapshot,
        target_idx: usize,
        visible_slots: usize,
    ) -> Vec<AlbumCommand> {
        let mut cmds = Vec::new();
        let Some(entry) = snapshot.entries.get(target_idx) else {
            return cmds;
        };
        if self.selected_id == Some(entry.id) {
            return cmds;
        }
        if let Some(save) = self.commit_edits() {
            cmds.push(save);
        }
        self.selected_id = Some(entry.id);
        self.pending_delete = None;
        self.load_draft(entry);
        self.ensure_strip_visible(target_idx, visible_slots, snapshot.entries.len());
        cmds.push(AlbumCommand::SelectEntry(entry.id));
        cmds
    }

    /// Computes responsive modal geometry inside a `screen_w × screen_h` frame.
    ///
    /// On viewports `>= 700×560`, anchors the modal to native `scrapback.bmp`
    /// (`700×560`) and places the 6 native Scrapbook buttons (`1201..=1206`) at
    /// their exact `UIText.iff STR# 141` pixel coordinates. On smaller viewports,
    /// falls back cleanly to the responsive 9-slice layout.
    pub fn layout(&self, screen_w: i32, screen_h: i32) -> AlbumLayout {
        if screen_w >= NATIVE_WIDTH && screen_h >= NATIVE_HEIGHT {
            let mx = (screen_w - NATIVE_WIDTH) / 2;
            let my = (screen_h - NATIVE_HEIGHT) / 2;
            let modal = Rect {
                x: mx,
                y: my,
                width: NATIVE_WIDTH,
                height: NATIVE_HEIGHT,
            };

            // Upper beveled frame interior inside `scrapback.bmp`: `(50, 18, 600, 402)`.
            let inner_x = mx + 58;
            let inner_right = mx + 642;
            let header_y = my + 24;
            let header_title = [inner_x, header_y + 4];
            let small_btn_h = 24;
            let btn_add = Rect {
                x: inner_right - 58,
                y: header_y,
                width: 58,
                height: small_btn_h,
            };
            let btn_snapshot = Rect {
                x: btn_add.x - 6 - 68,
                y: header_y,
                width: 68,
                height: small_btn_h,
            };
            let page_counter = [btn_snapshot.x - 84, header_y + 5];

            let content_top = header_y + small_btn_h + 8;
            let content_bottom = my + 412;
            let content_h = content_bottom - content_top;
            let photo_viewport = Rect {
                x: inner_x,
                y: content_top,
                width: 342,
                height: content_h,
            };

            let right_x = photo_viewport.x + photo_viewport.width + 12;
            let right_w = (inner_right - right_x).max(160);
            let title_label_pos = [right_x, content_top];
            let title_box = Rect {
                x: right_x,
                y: content_top + 16,
                width: right_w,
                height: 26,
            };
            let caption_label_pos = [right_x, title_box.y + title_box.height + 8];
            let action_row_h = 26;
            let actions_top = content_bottom - action_row_h * 2 - 6;
            let caption_top = caption_label_pos[1] + 16;
            let caption_box = Rect {
                x: right_x,
                y: caption_top,
                width: right_w,
                height: (actions_top - 8 - caption_top).max(60),
            };
            let half_w = (right_w - 6) / 2;
            let btn_move_earlier = Rect {
                x: right_x,
                y: actions_top,
                width: half_w,
                height: action_row_h,
            };
            let btn_move_later = Rect {
                x: right_x + half_w + 6,
                y: actions_top,
                width: right_w - half_w - 6,
                height: action_row_h,
            };
            let btn_save = Rect {
                x: right_x,
                y: actions_top + action_row_h + 6,
                width: right_w,
                height: action_row_h,
            };

            // Native `STR# 141` button positions over `scrapback.bmp` recesses:
            // First `(19, 442)` 38x47, Prev `(58, 442)` 29x47,
            // Next `(614, 442)` 28x46, Last `(643, 442)` 37x46,
            // Done `(150, 512)` 130x41, Delete `(442, 511)` 130x41.
            let btn_first = Rect::new(mx + 19, my + 442, 38, 47);
            let btn_prev = Rect::new(mx + 58, my + 442, 29, 47);
            let btn_next = Rect::new(mx + 614, my + 442, 28, 46);
            let btn_last = Rect::new(mx + 643, my + 442, 37, 46);
            let btn_done = Rect::new(mx + 150, my + 512, 130, 41);
            let btn_delete = Rect::new(mx + 442, my + 511, 130, 41);

            // Middle thumbnail strip sits cleanly inside the `STR# 141` `(100, 430, 500, 72)` well.
            let strip_box = Rect::new(mx + 100, my + 430, 500, 72);
            let arrow_w = 24;
            let strip_prev = Rect::new(strip_box.x + 4, strip_box.y + 8, arrow_w, strip_box.height - 16);
            let strip_next = Rect::new(
                strip_box.x + strip_box.width - 4 - arrow_w,
                strip_box.y + 8,
                arrow_w,
                strip_box.height - 16,
            );
            let thumbs_x = strip_prev.x + strip_prev.width + 6;
            let thumbs_avail = (strip_next.x - 6 - thumbs_x).max(0);
            let thumb_w = 78;
            let thumb_gap = 6;
            let slot_count = ((thumbs_avail + thumb_gap) / (thumb_w + thumb_gap)).max(1) as usize;
            let mut thumb_slots = Vec::with_capacity(slot_count);
            for i in 0..slot_count {
                thumb_slots.push(Rect {
                    x: thumbs_x + (i as i32) * (thumb_w + thumb_gap),
                    y: strip_box.y + 7,
                    width: thumb_w,
                    height: strip_box.height - 14,
                });
            }

            // Inline delete confirmation replaces `btn_delete` cleanly without overlapping `btn_done`.
            let confirm_bar = Rect::new(mx + 392, my + 514, 230, 36);
            let confirm_btn_w = 54;
            let btn_confirm_no = Rect::new(
                confirm_bar.x + confirm_bar.width - confirm_btn_w - 4,
                confirm_bar.y + 4,
                confirm_btn_w,
                confirm_bar.height - 8,
            );
            let btn_confirm_yes = Rect::new(
                btn_confirm_no.x - confirm_btn_w - 4,
                confirm_bar.y + 4,
                confirm_btn_w,
                confirm_bar.height - 8,
            );

            return AlbumLayout {
                modal,
                native_scrap: true,
                header_title,
                page_counter,
                photo_viewport,
                title_label_pos,
                title_box,
                caption_label_pos,
                caption_box,
                btn_snapshot,
                btn_add,
                btn_move_earlier,
                btn_move_later,
                btn_delete,
                btn_save,
                btn_done,
                confirm_bar,
                btn_confirm_yes,
                btn_confirm_no,
                btn_first,
                btn_prev,
                btn_next,
                btn_last,
                strip_box,
                strip_prev,
                strip_next,
                thumb_slots,
            };
        }

        let modal_w = (screen_w - 40).clamp(560, 720).min(screen_w.max(320));
        let modal_h = (screen_h - 32).clamp(420, 552).min(screen_h.max(240));
        let mx = (screen_w - modal_w) / 2;
        let my = (screen_h - modal_h) / 2;
        let modal = Rect {
            x: mx,
            y: my,
            width: modal_w,
            height: modal_h,
        };

        let pad = 16;
        let header_y = my + 14;
        let header_title = [mx + pad, header_y + 4];

        // Top-right action buttons: [Snap] [Add] [Done]
        let btn_h = 26;
        let btn_done = Rect {
            x: mx + modal_w - pad - 68,
            y: header_y,
            width: 68,
            height: btn_h,
        };
        let btn_add = Rect {
            x: btn_done.x - 6 - 58,
            y: header_y,
            width: 58,
            height: btn_h,
        };
        let btn_snapshot = Rect {
            x: btn_add.x - 6 - 68,
            y: header_y,
            width: 68,
            height: btn_h,
        };
        let page_counter = [btn_snapshot.x - 92, header_y + 6];

        // Bottom thumbnail strip
        let strip_h = 74;
        let strip_box = Rect {
            x: mx + pad,
            y: my + modal_h - pad - strip_h,
            width: modal_w - pad * 2,
            height: strip_h,
        };
        let arrow_w = 24;
        let strip_prev = Rect {
            x: strip_box.x + 4,
            y: strip_box.y + 8,
            width: arrow_w,
            height: strip_h - 16,
        };
        let strip_next = Rect {
            x: strip_box.x + strip_box.width - 4 - arrow_w,
            y: strip_box.y + 8,
            width: arrow_w,
            height: strip_h - 16,
        };

        let thumbs_x = strip_prev.x + strip_prev.width + 6;
        let thumbs_avail = (strip_next.x - 6 - thumbs_x).max(0);
        let thumb_w = 78;
        let thumb_gap = 6;
        let slot_count = ((thumbs_avail + thumb_gap) / (thumb_w + thumb_gap)).max(1) as usize;
        let mut thumb_slots = Vec::with_capacity(slot_count);
        for i in 0..slot_count {
            thumb_slots.push(Rect {
                x: thumbs_x + (i as i32) * (thumb_w + thumb_gap),
                y: strip_box.y + 7,
                width: thumb_w,
                height: strip_h - 14,
            });
        }

        // Main content area between header and strip
        let content_top = header_y + btn_h + 12;
        let content_bottom = strip_box.y - 10;
        let content_h = (content_bottom - content_top).max(120);

        // Left: photo viewport + page nav bar below it
        let nav_h = 26;
        let left_w = ((modal_w - pad * 2 - 14) * 56) / 100;
        let photo_viewport = Rect {
            x: mx + pad,
            y: content_top,
            width: left_w,
            height: (content_h - nav_h - 8).max(80),
        };
        let nav_y = photo_viewport.y + photo_viewport.height + 8;
        let nav_btn_w = 48;
        let nav_gap = 6;
        let nav_total = nav_btn_w * 4 + nav_gap * 3;
        let nav_x = photo_viewport.x + (photo_viewport.width - nav_total).max(0) / 2;
        let btn_first = Rect {
            x: nav_x,
            y: nav_y,
            width: nav_btn_w,
            height: nav_h,
        };
        let btn_prev = Rect {
            x: btn_first.x + nav_btn_w + nav_gap,
            y: nav_y,
            width: nav_btn_w,
            height: nav_h,
        };
        let btn_next = Rect {
            x: btn_prev.x + nav_btn_w + nav_gap,
            y: nav_y,
            width: nav_btn_w,
            height: nav_h,
        };
        let btn_last = Rect {
            x: btn_next.x + nav_btn_w + nav_gap,
            y: nav_y,
            width: nav_btn_w,
            height: nav_h,
        };

        // Right: title editor, wrapped caption editor, reorder/delete/save actions
        let right_x = photo_viewport.x + photo_viewport.width + 14;
        let right_w = (mx + modal_w - pad - right_x).max(160);
        let title_label_pos = [right_x, content_top];
        let title_box = Rect {
            x: right_x,
            y: content_top + 16,
            width: right_w,
            height: 26,
        };
        let caption_label_pos = [right_x, title_box.y + title_box.height + 8];
        let action_row_h = 26;
        let actions_top = content_bottom - action_row_h * 2 - 8;
        let caption_top = caption_label_pos[1] + 16;
        let caption_box = Rect {
            x: right_x,
            y: caption_top,
            width: right_w,
            height: (actions_top - 8 - caption_top).max(60),
        };

        // Row 1 of right-side actions: [Move <] [Move >]
        let half_w = (right_w - 6) / 2;
        let btn_move_earlier = Rect {
            x: right_x,
            y: actions_top,
            width: half_w,
            height: action_row_h,
        };
        let btn_move_later = Rect {
            x: right_x + half_w + 6,
            y: actions_top,
            width: right_w - half_w - 6,
            height: action_row_h,
        };

        // Row 2 of right-side actions: [Save] [Delete] (or inline confirm bar)
        let row2_y = actions_top + action_row_h + 8;
        let btn_save = Rect {
            x: right_x,
            y: row2_y,
            width: half_w,
            height: action_row_h,
        };
        let btn_delete = Rect {
            x: right_x + half_w + 6,
            y: row2_y,
            width: right_w - half_w - 6,
            height: action_row_h,
        };

        let confirm_bar = Rect {
            x: right_x,
            y: row2_y,
            width: right_w,
            height: action_row_h,
        };
        let confirm_btn_w = 50;
        let btn_confirm_no = Rect {
            x: confirm_bar.x + confirm_bar.width - confirm_btn_w - 3,
            y: confirm_bar.y + 2,
            width: confirm_btn_w,
            height: confirm_bar.height - 4,
        };
        let btn_confirm_yes = Rect {
            x: btn_confirm_no.x - confirm_btn_w - 4,
            y: confirm_bar.y + 2,
            width: confirm_btn_w,
            height: confirm_bar.height - 4,
        };

        AlbumLayout {
            modal,
            native_scrap: false,
            header_title,
            page_counter,
            photo_viewport,
            title_label_pos,
            title_box,
            caption_label_pos,
            caption_box,
            btn_snapshot,
            btn_add,
            btn_move_earlier,
            btn_move_later,
            btn_delete,
            btn_save,
            btn_done,
            confirm_bar,
            btn_confirm_yes,
            btn_confirm_no,
            btn_first,
            btn_prev,
            btn_next,
            btn_last,
            strip_box,
            strip_prev,
            strip_next,
            thumb_slots,
        }
    }

    fn hit_test(
        &self,
        layout: &AlbumLayout,
        snapshot: &AlbumSnapshot,
        pos: [i32; 2],
    ) -> Option<ControlId> {
        if !layout.modal.contains(pos[0], pos[1]) {
            return None;
        }
        if layout.btn_done.contains(pos[0], pos[1]) {
            return Some(ControlId::Done);
        }
        if layout.btn_add.contains(pos[0], pos[1]) {
            return Some(ControlId::AddPage);
        }
        if layout.btn_snapshot.contains(pos[0], pos[1]) {
            return Some(ControlId::CaptureSnapshot);
        }
        if layout.btn_first.contains(pos[0], pos[1]) {
            return Some(ControlId::First);
        }
        if layout.btn_prev.contains(pos[0], pos[1]) {
            return Some(ControlId::Prev);
        }
        if layout.btn_next.contains(pos[0], pos[1]) {
            return Some(ControlId::Next);
        }
        if layout.btn_last.contains(pos[0], pos[1]) {
            return Some(ControlId::Last);
        }
        if layout.title_box.contains(pos[0], pos[1]) {
            return Some(ControlId::TitleBox);
        }
        if layout.caption_box.contains(pos[0], pos[1]) {
            return Some(ControlId::CaptionBox);
        }
        if let Some(pending) = self.pending_delete {
            if layout.btn_confirm_yes.contains(pos[0], pos[1]) {
                return Some(ControlId::ConfirmDeleteYes(pending));
            }
            if layout.btn_confirm_no.contains(pos[0], pos[1]) {
                return Some(ControlId::ConfirmDeleteNo);
            }
            if layout.native_scrap
                && let Some(id) = self.selected_id
                && layout.btn_save.contains(pos[0], pos[1])
            {
                return Some(ControlId::Save(id));
            }
        } else if let Some(id) = self.selected_id {
            if layout.btn_save.contains(pos[0], pos[1]) {
                return Some(ControlId::Save(id));
            }
            if layout.btn_delete.contains(pos[0], pos[1]) {
                return Some(ControlId::Delete(id));
            }
        }
        if let Some(id) = self.selected_id {
            if layout.btn_move_earlier.contains(pos[0], pos[1]) {
                return Some(ControlId::MoveEarlier(id));
            }
            if layout.btn_move_later.contains(pos[0], pos[1]) {
                return Some(ControlId::MoveLater(id));
            }
        }
        if layout.strip_prev.contains(pos[0], pos[1]) {
            return Some(ControlId::StripPrev);
        }
        if layout.strip_next.contains(pos[0], pos[1]) {
            return Some(ControlId::StripNext);
        }
        for (slot_idx, rect) in layout.thumb_slots.iter().enumerate() {
            if rect.contains(pos[0], pos[1])
                && let Some(entry) = snapshot.entries.get(self.strip_scroll + slot_idx)
            {
                return Some(ControlId::Thumbnail(entry.id));
            }
        }
        None
    }

    fn control_enabled(
        &self,
        control: ControlId,
        snapshot: &AlbumSnapshot,
        visible_slots: usize,
    ) -> bool {
        let len = snapshot.entries.len();
        let idx = self.selected_index(snapshot);
        match control {
            ControlId::Done | ControlId::AddPage => true,
            ControlId::CaptureSnapshot => snapshot.can_capture_snapshot,
            ControlId::First | ControlId::Prev => idx.is_some_and(|i| i > 0),
            ControlId::Next | ControlId::Last => idx.is_some_and(|i| i + 1 < len),
            ControlId::MoveEarlier(id) => {
                self.selected_id == Some(id) && idx.is_some_and(|i| i > 0)
            }
            ControlId::MoveLater(id) => {
                self.selected_id == Some(id) && idx.is_some_and(|i| i + 1 < len)
            }
            ControlId::Delete(id) => self.selected_id == Some(id) && idx.is_some(),
            ControlId::ConfirmDeleteYes(id) => {
                self.pending_delete == Some(id) && snapshot.entries.iter().any(|e| e.id == id)
            }
            ControlId::ConfirmDeleteNo => self.pending_delete.is_some(),
            ControlId::Save(id) => self.selected_id == Some(id) && self.dirty,
            ControlId::TitleBox | ControlId::CaptionBox => idx.is_some(),
            ControlId::StripPrev => self.strip_scroll > 0,
            ControlId::StripNext => self.strip_scroll + visible_slots < len,
            ControlId::Thumbnail(id) => snapshot.entries.iter().any(|e| e.id == id),
        }
    }

    /// Updates hover state for pointer movement.
    pub fn handle_pointer_move(
        &mut self,
        pos: [i32; 2],
        screen: [i32; 2],
        snapshot: &AlbumSnapshot,
    ) {
        if !self.open {
            return;
        }
        self.sync_snapshot(snapshot);
        self.pointer = Some(pos);
        let layout = self.layout(screen[0], screen[1]);
        self.hovered = self.hit_test(&layout, snapshot, pos);
    }

    /// Handles a mouse press inside the modal. Returns `true` if the modal
    /// consumed the click (modal blocks clicks behind it while open).
    pub fn handle_pointer_down(
        &mut self,
        pos: [i32; 2],
        screen: [i32; 2],
        snapshot: &AlbumSnapshot,
    ) -> bool {
        if !self.open {
            return false;
        }
        self.sync_snapshot(snapshot);
        self.pointer = Some(pos);
        let layout = self.layout(screen[0], screen[1]);
        let hit = self.hit_test(&layout, snapshot, pos);
        self.hovered = hit;
        if let Some(ctrl) = hit
            && self.control_enabled(ctrl, snapshot, layout.thumb_slots.len())
        {
            self.pressed = Some(ctrl);
        } else {
            self.pressed = None;
        }
        true
    }

    /// Handles a mouse release inside the modal, firing press-bound commands.
    pub fn handle_pointer_up(
        &mut self,
        pos: [i32; 2],
        screen: [i32; 2],
        snapshot: &AlbumSnapshot,
    ) -> Vec<AlbumCommand> {
        let mut cmds = Vec::new();
        if !self.open {
            self.pressed = None;
            return cmds;
        }
        self.sync_snapshot(snapshot);
        self.pointer = Some(pos);
        let layout = self.layout(screen[0], screen[1]);
        let hit = self.hit_test(&layout, snapshot, pos);
        self.hovered = hit;
        let pressed = self.pressed.take();
        let Some(ctrl) = pressed else {
            return cmds;
        };
        if hit != Some(ctrl) || !self.control_enabled(ctrl, snapshot, layout.thumb_slots.len()) {
            return cmds;
        }

        let visible_slots = layout.thumb_slots.len();
        let idx = self.selected_index(snapshot);
        match ctrl {
            ControlId::Done => {
                if let Some(save) = self.commit_edits() {
                    cmds.push(save);
                }
                self.close();
                cmds.push(AlbumCommand::Close);
            }
            ControlId::AddPage => {
                if let Some(save) = self.commit_edits() {
                    cmds.push(save);
                }
                self.pending_delete = None;
                cmds.push(AlbumCommand::AddBlankPage);
            }
            ControlId::CaptureSnapshot => {
                if let Some(save) = self.commit_edits() {
                    cmds.push(save);
                }
                self.pending_delete = None;
                cmds.push(AlbumCommand::CaptureSnapshot);
            }
            ControlId::First => {
                cmds.extend(self.switch_to_index(snapshot, 0, visible_slots));
            }
            ControlId::Prev => {
                if let Some(i) = idx
                    && i > 0
                {
                    cmds.extend(self.switch_to_index(snapshot, i - 1, visible_slots));
                }
            }
            ControlId::Next => {
                if let Some(i) = idx
                    && i + 1 < snapshot.entries.len()
                {
                    cmds.extend(self.switch_to_index(snapshot, i + 1, visible_slots));
                }
            }
            ControlId::Last => {
                if !snapshot.entries.is_empty() {
                    cmds.extend(self.switch_to_index(
                        snapshot,
                        snapshot.entries.len() - 1,
                        visible_slots,
                    ));
                }
            }
            ControlId::MoveEarlier(id) => {
                if let Some(i) = idx
                    && i > 0
                {
                    if let Some(save) = self.commit_edits() {
                        cmds.push(save);
                    }
                    self.pending_delete = None;
                    self.ensure_strip_visible(i - 1, visible_slots, snapshot.entries.len());
                    cmds.push(AlbumCommand::MoveEntry {
                        id,
                        new_index: i - 1,
                    });
                }
            }
            ControlId::MoveLater(id) => {
                if let Some(i) = idx
                    && i + 1 < snapshot.entries.len()
                {
                    if let Some(save) = self.commit_edits() {
                        cmds.push(save);
                    }
                    self.pending_delete = None;
                    self.ensure_strip_visible(i + 1, visible_slots, snapshot.entries.len());
                    cmds.push(AlbumCommand::MoveEntry {
                        id,
                        new_index: i + 1,
                    });
                }
            }
            ControlId::Delete(id) => {
                self.pending_delete = Some(id);
                self.focus = None;
            }
            ControlId::ConfirmDeleteYes(id) => {
                self.pending_delete = None;
                self.dirty = false;
                cmds.push(AlbumCommand::DeleteEntry(id));
            }
            ControlId::ConfirmDeleteNo => {
                self.pending_delete = None;
            }
            ControlId::Save(_) => {
                if let Some(save) = self.commit_edits() {
                    cmds.push(save);
                }
            }
            ControlId::TitleBox => {
                self.focus = Some(EditFocus::Title);
                self.title_cursor = self.draft_title.len();
                self.blink_us = 0;
            }
            ControlId::CaptionBox => {
                self.focus = Some(EditFocus::Caption);
                self.caption_cursor = self.draft_caption.len();
                self.blink_us = 0;
            }
            ControlId::StripPrev => {
                self.strip_scroll = self.strip_scroll.saturating_sub(1);
            }
            ControlId::StripNext => {
                let max_scroll = snapshot.entries.len().saturating_sub(visible_slots);
                self.strip_scroll = (self.strip_scroll + 1).min(max_scroll);
            }
            ControlId::Thumbnail(id) => {
                if let Some(target_idx) = snapshot.entries.iter().position(|e| e.id == id) {
                    cmds.extend(self.switch_to_index(snapshot, target_idx, visible_slots));
                }
            }
        }

        cmds
    }

    /// Handles horizontal thumbnail-strip or caption scrolling via mouse wheel.
    pub fn handle_scroll(
        &mut self,
        delta_lines: i32,
        screen: [i32; 2],
        snapshot: &AlbumSnapshot,
        art: &Art,
    ) {
        if !self.open || delta_lines == 0 {
            return;
        }
        self.sync_snapshot(snapshot);
        let layout = self.layout(screen[0], screen[1]);
        if let Some(pos) = self.pointer
            && layout.caption_box.contains(pos[0], pos[1])
        {
            let font = art.font(10);
            let max_w = (layout.caption_box.width - 12).max(24);
            let wrapped = wrap_text_ranges(&self.draft_caption, font, max_w);
            let visible_lines = caption_visible_lines(&layout.caption_box, font);
            let max_scroll = wrapped.len().saturating_sub(visible_lines);
            if delta_lines > 0 {
                self.caption_scroll_line =
                    (self.caption_scroll_line + delta_lines as usize).min(max_scroll);
            } else {
                self.caption_scroll_line = self
                    .caption_scroll_line
                    .saturating_sub(delta_lines.unsigned_abs() as usize);
            }
            return;
        }
        let max_scroll = snapshot
            .entries
            .len()
            .saturating_sub(layout.thumb_slots.len());
        if delta_lines > 0 {
            self.strip_scroll = (self.strip_scroll + delta_lines as usize).min(max_scroll);
        } else {
            self.strip_scroll = self
                .strip_scroll
                .saturating_sub(delta_lines.unsigned_abs() as usize);
        }
    }

    /// Handles keyboard input for navigation, editing, and shortcuts.
    pub fn handle_key(
        &mut self,
        key: AlbumKey,
        screen: [i32; 2],
        snapshot: &AlbumSnapshot,
        art: &Art,
    ) -> Vec<AlbumCommand> {
        let mut cmds = Vec::new();
        if !self.open {
            return cmds;
        }
        self.sync_snapshot(snapshot);
        let layout = self.layout(screen[0], screen[1]);
        let visible_slots = layout.thumb_slots.len();

        if key == AlbumKey::SaveShortcut {
            if let Some(save) = self.commit_edits() {
                cmds.push(save);
            }
            return cmds;
        }

        if let Some(focus) = self.focus {
            self.handle_edit_key(focus, key, &layout, art);
            return cmds;
        }

        // Unfocused modal navigation
        let idx = self.selected_index(snapshot);
        match key {
            AlbumKey::Escape => {
                if self.pending_delete.is_some() {
                    self.pending_delete = None;
                } else {
                    if let Some(save) = self.commit_edits() {
                        cmds.push(save);
                    }
                    self.close();
                    cmds.push(AlbumCommand::Close);
                }
            }
            AlbumKey::Tab => {
                if idx.is_some() {
                    self.focus = Some(EditFocus::Title);
                    self.blink_us = 0;
                }
            }
            AlbumKey::Left | AlbumKey::PageUp => {
                if let Some(i) = idx
                    && i > 0
                {
                    cmds.extend(self.switch_to_index(snapshot, i - 1, visible_slots));
                }
            }
            AlbumKey::Right | AlbumKey::PageDown => {
                if let Some(i) = idx
                    && i + 1 < snapshot.entries.len()
                {
                    cmds.extend(self.switch_to_index(snapshot, i + 1, visible_slots));
                }
            }
            AlbumKey::Home => {
                if !snapshot.entries.is_empty() {
                    cmds.extend(self.switch_to_index(snapshot, 0, visible_slots));
                }
            }
            AlbumKey::End => {
                if !snapshot.entries.is_empty() {
                    cmds.extend(self.switch_to_index(
                        snapshot,
                        snapshot.entries.len() - 1,
                        visible_slots,
                    ));
                }
            }
            AlbumKey::Delete => {
                if let Some(id) = self.selected_id {
                    self.pending_delete = Some(id);
                }
            }
            AlbumKey::Enter => {
                if let Some(pending) = self.pending_delete.take() {
                    self.dirty = false;
                    cmds.push(AlbumCommand::DeleteEntry(pending));
                }
            }
            _ => {}
        }

        cmds
    }

    fn handle_edit_key(
        &mut self,
        focus: EditFocus,
        key: AlbumKey,
        layout: &AlbumLayout,
        art: &Art,
    ) {
        self.blink_us = 0;
        match focus {
            EditFocus::Title => match key {
                AlbumKey::Escape => {
                    self.focus = None;
                }
                AlbumKey::Tab | AlbumKey::Enter => {
                    self.focus = Some(EditFocus::Caption);
                }
                AlbumKey::Left => {
                    self.title_cursor = prev_char_boundary(&self.draft_title, self.title_cursor);
                }
                AlbumKey::Right => {
                    self.title_cursor = next_char_boundary(&self.draft_title, self.title_cursor);
                }
                AlbumKey::Home => {
                    self.title_cursor = 0;
                }
                AlbumKey::End => {
                    self.title_cursor = self.draft_title.len();
                }
                AlbumKey::Backspace => {
                    if self.title_cursor > 0 {
                        let prev = prev_char_boundary(&self.draft_title, self.title_cursor);
                        self.draft_title.drain(prev..self.title_cursor);
                        self.title_cursor = prev;
                        self.dirty = true;
                    }
                }
                AlbumKey::Delete => {
                    if self.title_cursor < self.draft_title.len() {
                        let next = next_char_boundary(&self.draft_title, self.title_cursor);
                        self.draft_title.drain(self.title_cursor..next);
                        self.dirty = true;
                    }
                }
                AlbumKey::Char(ch)
                    if !ch.is_control() && self.draft_title.chars().count() < MAX_TITLE_LEN =>
                {
                    self.draft_title.insert(self.title_cursor, ch);
                    self.title_cursor += ch.len_utf8();
                    self.dirty = true;
                }
                _ => {}
            },
            EditFocus::Caption => {
                let font = art.font(10);
                let max_w = (layout.caption_box.width - 12).max(24);
                match key {
                    AlbumKey::Escape => {
                        self.focus = None;
                    }
                    AlbumKey::Tab => {
                        self.focus = None;
                    }
                    AlbumKey::Left => {
                        self.caption_cursor =
                            prev_char_boundary(&self.draft_caption, self.caption_cursor);
                    }
                    AlbumKey::Right => {
                        self.caption_cursor =
                            next_char_boundary(&self.draft_caption, self.caption_cursor);
                    }
                    AlbumKey::Home => {
                        let lines = wrap_text_ranges(&self.draft_caption, font, max_w);
                        let (line_idx, _) = locate_cursor(&lines, self.caption_cursor);
                        if let Some(line) = lines.get(line_idx) {
                            self.caption_cursor = line.start;
                        }
                    }
                    AlbumKey::End => {
                        let lines = wrap_text_ranges(&self.draft_caption, font, max_w);
                        let (line_idx, _) = locate_cursor(&lines, self.caption_cursor);
                        if let Some(line) = lines.get(line_idx) {
                            self.caption_cursor = line.end;
                        }
                    }
                    AlbumKey::Up => {
                        let lines = wrap_text_ranges(&self.draft_caption, font, max_w);
                        let (line_idx, col_chars) = locate_cursor(&lines, self.caption_cursor);
                        if line_idx > 0
                            && let Some(target) = lines.get(line_idx - 1)
                        {
                            self.caption_cursor = advance_chars(
                                &self.draft_caption,
                                target.start,
                                target.end,
                                col_chars,
                            );
                        }
                    }
                    AlbumKey::Down => {
                        let lines = wrap_text_ranges(&self.draft_caption, font, max_w);
                        let (line_idx, col_chars) = locate_cursor(&lines, self.caption_cursor);
                        if let Some(target) = lines.get(line_idx + 1) {
                            self.caption_cursor = advance_chars(
                                &self.draft_caption,
                                target.start,
                                target.end,
                                col_chars,
                            );
                        }
                    }
                    AlbumKey::Enter => {
                        if self.draft_caption.chars().count() < MAX_CAPTION_LEN {
                            self.draft_caption.insert(self.caption_cursor, '\n');
                            self.caption_cursor += 1;
                            self.dirty = true;
                        }
                    }
                    AlbumKey::Backspace => {
                        if self.caption_cursor > 0 {
                            let prev = prev_char_boundary(&self.draft_caption, self.caption_cursor);
                            self.draft_caption.drain(prev..self.caption_cursor);
                            self.caption_cursor = prev;
                            self.dirty = true;
                        }
                    }
                    AlbumKey::Delete => {
                        if self.caption_cursor < self.draft_caption.len() {
                            let next = next_char_boundary(&self.draft_caption, self.caption_cursor);
                            self.draft_caption.drain(self.caption_cursor..next);
                            self.dirty = true;
                        }
                    }
                    AlbumKey::Char(ch)
                        if !ch.is_control()
                            && self.draft_caption.chars().count() < MAX_CAPTION_LEN =>
                    {
                        self.draft_caption.insert(self.caption_cursor, ch);
                        self.caption_cursor += ch.len_utf8();
                        self.dirty = true;
                    }
                    _ => {}
                }
                self.ensure_caption_caret_visible(&layout.caption_box, font);
            }
        }
    }

    fn ensure_caption_caret_visible(&mut self, box_rect: &Rect, font: Option<&Font>) {
        let max_w = (box_rect.width - 12).max(24);
        let lines = wrap_text_ranges(&self.draft_caption, font, max_w);
        let (line_idx, _) = locate_cursor(&lines, self.caption_cursor);
        let visible = caption_visible_lines(box_rect, font);
        if line_idx < self.caption_scroll_line {
            self.caption_scroll_line = line_idx;
        } else if line_idx >= self.caption_scroll_line + visible {
            self.caption_scroll_line = line_idx + 1 - visible;
        }
        let max_scroll = lines.len().saturating_sub(visible);
        self.caption_scroll_line = self.caption_scroll_line.min(max_scroll);
    }

    /// Renders the Photo Album modal into `frame`.
    pub fn draw(&mut self, frame: &mut Frame, snapshot: &AlbumSnapshot, art: &Art) {
        if !self.open {
            return;
        }
        self.sync_snapshot(snapshot);
        let layout = self.layout(frame.width() as i32, frame.height() as i32);
        let visible_slots = layout.thumb_slots.len();
        let max_scroll = snapshot.entries.len().saturating_sub(visible_slots);
        self.strip_scroll = self.strip_scroll.min(max_scroll);

        // 1. Subtle modal backdrop dimming
        dim_backdrop(frame);

        // 2. Native `scrapback.bmp` (`1200`, 700x560) or 9-slice fallback (`3008`)
        if layout.native_scrap
            && let Some(scrap_bg) = art.image(SCRAP_BACK)
        {
            frame.draw_all(scrap_bg, [layout.modal.x, layout.modal.y]);
        } else if let Some(sheet) = art.image(BACKGROUND) {
            draw_nine_slice(frame, sheet, layout.modal);
        } else {
            frame.fill(layout.modal, DIALOG_FALLBACK_BG);
            draw_outline(frame, layout.modal, DIALOG_FALLBACK_BORDER);
        }

        // 3. Header: family title, page counter, top actions
        let header_text = if snapshot.family_name.is_empty() {
            self.strings.title.clone()
        } else {
            format!("{} - {}", snapshot.family_name, self.strings.title)
        };
        draw_text_clipped(
            frame,
            art.font(12).or_else(|| art.font(10)),
            layout.header_title[0],
            layout.header_title[1],
            (layout.page_counter[0] - layout.header_title[0] - 8).max(40),
            &header_text,
            TEXT_HOVER,
        );

        let counter_text = match self.selected_index(snapshot) {
            Some(i) => format!("{} / {}", i + 1, snapshot.entries.len()),
            None => "0 / 0".to_owned(),
        };
        draw_text(
            frame,
            art.font(10),
            layout.page_counter[0],
            layout.page_counter[1],
            &counter_text,
            TEXT_SELECTED,
        );

        self.draw_button(
            frame,
            art,
            layout.btn_snapshot,
            &self.strings.snapshot,
            ControlId::CaptureSnapshot,
            snapshot,
            visible_slots,
        );
        self.draw_button(
            frame,
            art,
            layout.btn_add,
            &self.strings.add_page,
            ControlId::AddPage,
            snapshot,
            visible_slots,
        );
        if layout.native_scrap {
            self.draw_scrap_button(
                frame,
                art,
                SCRAP_DONE,
                layout.btn_done,
                Some(&self.strings.done),
                ControlId::Done,
                snapshot,
                visible_slots,
            );
        } else {
            self.draw_button(
                frame,
                art,
                layout.btn_done,
                &self.strings.done,
                ControlId::Done,
                snapshot,
                visible_slots,
            );
        }

        // 4. Full-size photo viewport
        frame.fill(layout.photo_viewport, PHOTO_MATTE_BG);
        draw_outline(frame, layout.photo_viewport, PHOTO_MATTE_BORDER);
        if let Some(idx) = self.selected_index(snapshot) {
            let entry = &snapshot.entries[idx];
            let inner = inset_rect(layout.photo_viewport, 4);
            if let Some(img) = &entry.image {
                blit_aspect_fit(frame, img, inner);
            } else {
                draw_centered_text(
                    frame,
                    art.font(10),
                    inner,
                    &self.strings.empty_caption,
                    TEXT_MUTED,
                );
            }
            if let Some(day) = entry.sim_day {
                let badge = format!("Day {day}");
                draw_text(
                    frame,
                    art.font(10),
                    layout.photo_viewport.x + 8,
                    layout.photo_viewport.y + layout.photo_viewport.height - 18,
                    &badge,
                    TEXT_WARNING,
                );
            }
        } else {
            draw_centered_text(
                frame,
                art.font(10),
                layout.photo_viewport,
                &self.strings.empty_caption,
                TEXT_MUTED,
            );
        }

        // 5. Navigation buttons (`scraphome`/`scrapleft`/`scrapright`/`scrapend` in native mode)
        if layout.native_scrap {
            self.draw_scrap_button(
                frame,
                art,
                SCRAP_HOME,
                layout.btn_first,
                None,
                ControlId::First,
                snapshot,
                visible_slots,
            );
            self.draw_scrap_button(
                frame,
                art,
                SCRAP_LEFT,
                layout.btn_prev,
                None,
                ControlId::Prev,
                snapshot,
                visible_slots,
            );
            self.draw_scrap_button(
                frame,
                art,
                SCRAP_RIGHT,
                layout.btn_next,
                None,
                ControlId::Next,
                snapshot,
                visible_slots,
            );
            self.draw_scrap_button(
                frame,
                art,
                SCRAP_END,
                layout.btn_last,
                None,
                ControlId::Last,
                snapshot,
                visible_slots,
            );
        } else {
            self.draw_button(
                frame,
                art,
                layout.btn_first,
                "|<",
                ControlId::First,
                snapshot,
                visible_slots,
            );
            self.draw_button(
                frame,
                art,
                layout.btn_prev,
                "<",
                ControlId::Prev,
                snapshot,
                visible_slots,
            );
            self.draw_button(
                frame,
                art,
                layout.btn_next,
                ">",
                ControlId::Next,
                snapshot,
                visible_slots,
            );
            self.draw_button(
                frame,
                art,
                layout.btn_last,
                ">|",
                ControlId::Last,
                snapshot,
                visible_slots,
            );
        }

        // 6. Title & wrapped caption editors (`cTSWinTextEdit2`)
        draw_text(
            frame,
            art.font(10),
            layout.title_label_pos[0],
            layout.title_label_pos[1],
            &self.strings.title_label,
            TEXT_NORMAL,
        );
        self.draw_single_line_input(
            frame,
            art.font(10),
            layout.title_box,
            &self.draft_title,
            self.title_cursor,
            self.focus == Some(EditFocus::Title),
            self.selected_id.is_some(),
        );

        draw_text(
            frame,
            art.font(10),
            layout.caption_label_pos[0],
            layout.caption_label_pos[1],
            &self.strings.caption_label,
            TEXT_NORMAL,
        );
        self.draw_multiline_caption(
            frame,
            art.font(10),
            layout.caption_box,
            self.focus == Some(EditFocus::Caption),
            self.selected_id.is_some(),
        );

        // 7. Reorder, Save, Delete / inline confirmation bar
        let selected_id = self.selected_id.unwrap_or(PhotoId(0));
        self.draw_button(
            frame,
            art,
            layout.btn_move_earlier,
            &self.strings.move_earlier,
            ControlId::MoveEarlier(selected_id),
            snapshot,
            visible_slots,
        );
        self.draw_button(
            frame,
            art,
            layout.btn_move_later,
            &self.strings.move_later,
            ControlId::MoveLater(selected_id),
            snapshot,
            visible_slots,
        );

        if layout.native_scrap {
            self.draw_button(
                frame,
                art,
                layout.btn_save,
                &self.strings.save,
                ControlId::Save(selected_id),
                snapshot,
                visible_slots,
            );
            if let Some(pending) = self.pending_delete {
                frame.fill(layout.confirm_bar, CONFIRM_BG);
                draw_outline(frame, layout.confirm_bar, CONFIRM_BORDER);
                draw_text_clipped(
                    frame,
                    art.font(10),
                    layout.confirm_bar.x + 8,
                    layout.confirm_bar.y + 10,
                    (layout.btn_confirm_yes.x - layout.confirm_bar.x - 12).max(20),
                    &format!("{}?", self.strings.delete),
                    TEXT_WARNING,
                );
                self.draw_button(
                    frame,
                    art,
                    layout.btn_confirm_yes,
                    &self.strings.confirm_yes,
                    ControlId::ConfirmDeleteYes(pending),
                    snapshot,
                    visible_slots,
                );
                self.draw_button(
                    frame,
                    art,
                    layout.btn_confirm_no,
                    &self.strings.confirm_no,
                    ControlId::ConfirmDeleteNo,
                    snapshot,
                    visible_slots,
                );
            } else {
                self.draw_scrap_button(
                    frame,
                    art,
                    SCRAP_DELETE,
                    layout.btn_delete,
                    Some(&self.strings.delete),
                    ControlId::Delete(selected_id),
                    snapshot,
                    visible_slots,
                );
            }
        } else if let Some(pending) = self.pending_delete {
            frame.fill(layout.confirm_bar, CONFIRM_BG);
            draw_outline(frame, layout.confirm_bar, CONFIRM_BORDER);
            draw_text_clipped(
                frame,
                art.font(10),
                layout.confirm_bar.x + 6,
                layout.confirm_bar.y + 6,
                (layout.btn_confirm_yes.x - layout.confirm_bar.x - 10).max(20),
                &format!("{}?", self.strings.delete),
                TEXT_WARNING,
            );
            self.draw_button(
                frame,
                art,
                layout.btn_confirm_yes,
                &self.strings.confirm_yes,
                ControlId::ConfirmDeleteYes(pending),
                snapshot,
                visible_slots,
            );
            self.draw_button(
                frame,
                art,
                layout.btn_confirm_no,
                &self.strings.confirm_no,
                ControlId::ConfirmDeleteNo,
                snapshot,
                visible_slots,
            );
        } else {
            self.draw_button(
                frame,
                art,
                layout.btn_save,
                &self.strings.save,
                ControlId::Save(selected_id),
                snapshot,
                visible_slots,
            );
            self.draw_button(
                frame,
                art,
                layout.btn_delete,
                &self.strings.delete,
                ControlId::Delete(selected_id),
                snapshot,
                visible_slots,
            );
        }

        // 8. Bottom thumbnail strip
        frame.fill(layout.strip_box, STRIP_BG);
        draw_outline(frame, layout.strip_box, STRIP_BORDER);
        self.draw_button(
            frame,
            art,
            layout.strip_prev,
            "<",
            ControlId::StripPrev,
            snapshot,
            visible_slots,
        );
        self.draw_button(
            frame,
            art,
            layout.strip_next,
            ">",
            ControlId::StripNext,
            snapshot,
            visible_slots,
        );
        for (slot_idx, slot_rect) in layout.thumb_slots.iter().enumerate() {
            let entry_idx = self.strip_scroll + slot_idx;
            let Some(entry) = snapshot.entries.get(entry_idx) else {
                frame.fill(*slot_rect, THUMB_BG);
                draw_outline(frame, *slot_rect, STRIP_BORDER);
                continue;
            };
            frame.fill(*slot_rect, THUMB_BG);
            let inner = inset_rect(*slot_rect, 2);
            if let Some(thumb) = entry.thumbnail.as_ref().or(entry.image.as_ref()) {
                blit_aspect_fit(frame, thumb, inner);
            } else {
                let num = format!("#{}", entry_idx + 1);
                draw_centered_text(frame, art.font(10), inner, &num, TEXT_MUTED);
            }
            let border = if self.selected_id == Some(entry.id) {
                THUMB_SELECTED_BORDER
            } else if self.hovered == Some(ControlId::Thumbnail(entry.id)) {
                THUMB_HOVER_BORDER
            } else {
                STRIP_BORDER
            };
            draw_outline(frame, *slot_rect, border);
        }

        // 9. Hover tooltip
        if let (Some(pos), Some(ctrl)) = (self.pointer, self.hovered)
            && let Some(tip) = self.tooltip_for(ctrl)
        {
            draw_tooltip(frame, art, tip, [pos[0] + 12, pos[1] + 16]);
        }
    }

    fn tooltip_for(&self, ctrl: ControlId) -> Option<&str> {
        match ctrl {
            ControlId::Done => Some(&self.strings.done_tooltip),
            ControlId::Delete(_) => Some(&self.strings.delete_tooltip),
            ControlId::First => Some(&self.strings.first_tooltip),
            ControlId::Prev => Some(&self.strings.prev_tooltip),
            ControlId::Next => Some(&self.strings.next_tooltip),
            ControlId::Last => Some(&self.strings.last_tooltip),
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_scrap_button(
        &self,
        frame: &mut Frame,
        art: &Art,
        art_id: i32,
        rect: Rect,
        label: Option<&str>,
        control: ControlId,
        snapshot: &AlbumSnapshot,
        visible_slots: usize,
    ) {
        let enabled = self.control_enabled(control, snapshot, visible_slots);
        let pressed = enabled && self.pressed == Some(control) && self.hovered == Some(control);
        let hovered = enabled && self.hovered == Some(control);
        let (cell_idx, text_color) = if !enabled {
            (3, TEXT_DISABLED)
        } else if pressed {
            (1, TEXT_SELECTED)
        } else if hovered {
            (2, TEXT_HOVER)
        } else {
            (0, TEXT_NORMAL)
        };
        if let Some(sheet) = art.image(art_id)
            && sheet.width >= 4
            && sheet.height > 0
        {
            let cell_w = (sheet.width / 4) as i32;
            let cell_h = sheet.height as i32;
            let src = Rect::new(cell_w * cell_idx as i32, 0, cell_w, cell_h);
            frame.draw(sheet, src, [rect.x, rect.y]);
            if let Some(text) = label {
                draw_centered_text(
                    frame,
                    art.font(12).or_else(|| art.font(10)),
                    rect,
                    text,
                    text_color,
                );
            }
        } else {
            let fallback = label.unwrap_or(match control {
                ControlId::First => "|<",
                ControlId::Prev => "<",
                ControlId::Next => ">",
                ControlId::Last => ">|",
                _ => "",
            });
            self.draw_button(
                frame,
                art,
                rect,
                fallback,
                control,
                snapshot,
                visible_slots,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_button(
        &self,
        frame: &mut Frame,
        art: &Art,
        rect: Rect,
        label: &str,
        control: ControlId,
        snapshot: &AlbumSnapshot,
        visible_slots: usize,
    ) {
        let enabled = self.control_enabled(control, snapshot, visible_slots);
        let pressed = enabled && self.pressed == Some(control) && self.hovered == Some(control);
        let hovered = enabled && self.hovered == Some(control);
        let (cell_idx, text_color) = if !enabled {
            (3, TEXT_DISABLED)
        } else if pressed {
            (1, TEXT_SELECTED)
        } else if hovered {
            (2, TEXT_HOVER)
        } else {
            (0, TEXT_NORMAL)
        };

        if let Some(sheet) = art.image(BUTTON) {
            draw_win_btn_scaled(frame, sheet, cell_idx, rect);
        } else {
            let bg = match cell_idx {
                1 => [34, 62, 148],
                2 => [42, 74, 166],
                3 => [18, 28, 72],
                _ => [28, 48, 126],
            };
            frame.fill(rect, bg);
            draw_outline(frame, rect, DIALOG_FALLBACK_BORDER);
        }

        draw_centered_text(frame, art.font(10), rect, label, text_color);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_single_line_input(
        &self,
        frame: &mut Frame,
        font: Option<&Font>,
        rect: Rect,
        text: &str,
        cursor: usize,
        focused: bool,
        enabled: bool,
    ) {
        frame.fill(rect, ENTRY_FILL);
        draw_outline(
            frame,
            rect,
            if focused {
                ENTRY_FOCUS_BORDER
            } else {
                ENTRY_BORDER
            },
        );

        let inner_w = (rect.width - 10).max(8);
        let color = if enabled { TEXT_HOVER } else { TEXT_DISABLED };
        let text_y = rect.y + (rect.height - font_line_height(font)) / 2;
        draw_text_clipped(frame, font, rect.x + 5, text_y, inner_w, text, color);

        if focused && enabled && self.caret_visible() {
            let prefix = &text[..cursor.min(text.len())];
            let caret_x = (rect.x + 5 + measure_text(font, prefix)).min(rect.x + rect.width - 4);
            frame.fill(
                Rect {
                    x: caret_x,
                    y: rect.y + 4,
                    width: 1,
                    height: (rect.height - 8).max(4),
                },
                TEXT_SELECTED,
            );
        }
    }

    fn draw_multiline_caption(
        &self,
        frame: &mut Frame,
        font: Option<&Font>,
        rect: Rect,
        focused: bool,
        enabled: bool,
    ) {
        frame.fill(rect, ENTRY_FILL);
        draw_outline(
            frame,
            rect,
            if focused {
                ENTRY_FOCUS_BORDER
            } else {
                ENTRY_BORDER
            },
        );

        let line_h = font_line_height(font) + 2;
        let max_w = (rect.width - 12).max(24);
        let visible = caption_visible_lines(&rect, font);

        if self.draft_caption.is_empty() && !focused {
            draw_text_clipped(
                frame,
                font,
                rect.x + 6,
                rect.y + 5,
                max_w,
                &self.strings.empty_caption,
                TEXT_MUTED,
            );
            return;
        }

        let lines = wrap_text_ranges(&self.draft_caption, font, max_w);
        let color = if enabled { TEXT_HOVER } else { TEXT_DISABLED };
        for (row, line) in lines
            .iter()
            .skip(self.caption_scroll_line)
            .take(visible)
            .enumerate()
        {
            let slice = &self.draft_caption[line.start..line.end];
            let y = rect.y + 5 + (row as i32) * line_h;
            draw_text_clipped(frame, font, rect.x + 6, y, max_w, slice, color);
        }

        if focused && enabled && self.caret_visible() {
            let (caret_line, _) = locate_cursor(&lines, self.caption_cursor);
            if caret_line >= self.caption_scroll_line
                && caret_line < self.caption_scroll_line + visible
                && let Some(line) = lines.get(caret_line)
            {
                let clamped = self.caption_cursor.clamp(line.start, line.end);
                let prefix = &self.draft_caption[line.start..clamped];
                let caret_x =
                    (rect.x + 6 + measure_text(font, prefix)).min(rect.x + rect.width - 4);
                let caret_y =
                    rect.y + 5 + ((caret_line - self.caption_scroll_line) as i32) * line_h;
                frame.fill(
                    Rect {
                        x: caret_x,
                        y: caret_y,
                        width: 1,
                        height: (line_h - 2).max(8),
                    },
                    TEXT_SELECTED,
                );
            }
        }
    }

    fn caret_visible(&self) -> bool {
        (self.blink_us / CARET_BLINK_US).is_multiple_of(2)
    }
}

/// One wrapped visual line as a byte range into the source string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WrappedLine {
    start: usize,
    end: usize,
}

fn wrap_text_ranges(text: &str, font: Option<&Font>, max_width: i32) -> Vec<WrappedLine> {
    let mut out = Vec::new();
    let max_width = max_width.max(16);
    let mut para_start = 0usize;

    for raw_para in text.split('\n') {
        let para_end = para_start + raw_para.len();
        if raw_para.is_empty() {
            out.push(WrappedLine {
                start: para_start,
                end: para_start,
            });
        } else {
            let mut line_start = para_start;
            let mut last_space: Option<usize> = None;
            let mut cursor = para_start;
            while cursor < para_end {
                let ch = text[cursor..para_end].chars().next().unwrap();
                let next = cursor + ch.len_utf8();
                if ch == ' ' {
                    last_space = Some(cursor);
                }
                let width = measure_text(font, &text[line_start..next]);
                if width > max_width && cursor > line_start {
                    if let Some(space_idx) = last_space
                        && space_idx > line_start
                    {
                        out.push(WrappedLine {
                            start: line_start,
                            end: space_idx,
                        });
                        line_start = space_idx + 1;
                        last_space = None;
                        cursor = line_start;
                        continue;
                    }
                    out.push(WrappedLine {
                        start: line_start,
                        end: cursor,
                    });
                    line_start = cursor;
                    last_space = None;
                } else {
                    cursor = next;
                }
            }
            out.push(WrappedLine {
                start: line_start,
                end: para_end,
            });
        }
        para_start = para_end + 1;
    }

    if out.is_empty() {
        out.push(WrappedLine { start: 0, end: 0 });
    }
    out
}

fn locate_cursor(lines: &[WrappedLine], byte_cursor: usize) -> (usize, usize) {
    for (idx, line) in lines.iter().enumerate() {
        if byte_cursor >= line.start && byte_cursor <= line.end {
            return (idx, byte_cursor - line.start);
        }
    }
    let last = lines.len().saturating_sub(1);
    let col = lines.get(last).map_or(0, |l| l.end.saturating_sub(l.start));
    (last, col)
}

fn advance_chars(text: &str, start: usize, end: usize, byte_offset: usize) -> usize {
    let target = (start + byte_offset).min(end);
    let mut pos = start;
    while pos < target {
        let next = next_char_boundary(text, pos);
        if next > target {
            break;
        }
        pos = next;
    }
    pos
}

fn prev_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    if i == 0 {
        return 0;
    }
    i -= 1;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn next_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    if i >= s.len() {
        return s.len();
    }
    i += 1;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

fn font_line_height(font: Option<&Font>) -> i32 {
    font.map_or(12, |f| f.line_height.max(10))
}

fn caption_visible_lines(box_rect: &Rect, font: Option<&Font>) -> usize {
    let line_h = font_line_height(font) + 2;
    ((box_rect.height - 10) / line_h).max(1) as usize
}

fn measure_text(font: Option<&Font>, text: &str) -> i32 {
    match font {
        Some(f) => f.width(text),
        None => (text.chars().count() as i32) * 6,
    }
}

fn draw_text(frame: &mut Frame, font: Option<&Font>, x: i32, y: i32, text: &str, color: [u8; 3]) {
    if let Some(f) = font {
        f.draw(frame, text, [x, y], color);
    }
}

fn draw_text_clipped(
    frame: &mut Frame,
    font: Option<&Font>,
    x: i32,
    y: i32,
    max_width: i32,
    text: &str,
    color: [u8; 3],
) {
    let Some(f) = font else {
        return;
    };
    if f.width(text) <= max_width {
        f.draw(frame, text, [x, y], color);
        return;
    }
    let mut end = 0;
    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if f.width(&text[..next]) > max_width {
            break;
        }
        end = next;
    }
    f.draw(frame, &text[..end], [x, y], color);
}

fn draw_centered_text(
    frame: &mut Frame,
    font: Option<&Font>,
    rect: Rect,
    text: &str,
    color: [u8; 3],
) {
    let w = measure_text(font, text);
    let h = font_line_height(font);
    let x = rect.x + (rect.width - w) / 2;
    let y = rect.y + (rect.height - h) / 2;
    draw_text_clipped(
        frame,
        font,
        x.max(rect.x + 2),
        y,
        (rect.width - 4).max(4),
        text,
        color,
    );
}

fn inset_rect(r: Rect, inset: i32) -> Rect {
    Rect {
        x: r.x + inset,
        y: r.y + inset,
        width: (r.width - inset * 2).max(1),
        height: (r.height - inset * 2).max(1),
    }
}

/// Dims the existing frame pixels behind the modal.
fn dim_backdrop(frame: &mut Frame) {
    let w = frame.width() as i32;
    let h = frame.height() as i32;
    for y in 0..h {
        for x in 0..w {
            frame.blend(x, y, [0, 0, 0], 102);
        }
    }
}

/// Draws one cell of the 4-cell `WinBtn.bmp` (`17`) scaled into `dest`.
fn draw_win_btn_scaled(frame: &mut Frame, sheet: &Image, cell_idx: usize, dest: Rect) {
    let cell_w = (sheet.width / 4) as i32;
    let cell_h = sheet.height as i32;
    if cell_w <= 0 || cell_h <= 0 || dest.width <= 0 || dest.height <= 0 {
        return;
    }
    let src_x0 = (cell_idx.min(3) as u32) * (cell_w as u32);
    let fw = frame.width() as i32;
    let fh = frame.height() as i32;
    for dy in 0..dest.height {
        let py = dest.y + dy;
        if py < 0 || py >= fh {
            continue;
        }
        let sy = ((i64::from(dy) * i64::from(cell_h)) / i64::from(dest.height)) as u32;
        for dx in 0..dest.width {
            let px = dest.x + dx;
            if px < 0 || px >= fw {
                continue;
            }
            let sx = src_x0 + ((i64::from(dx) * i64::from(cell_w)) / i64::from(dest.width)) as u32;
            if let Some([r, g, b, a]) = sheet.pixel(sx, sy)
                && a != 0
            {
                frame.blend(px, py, [r, g, b], a);
            }
        }
    }
}

/// Blits `image` centered and aspect-preserving inside `dest` using nearest-neighbor sampling.
fn blit_aspect_fit(frame: &mut Frame, image: &Image, dest: Rect) {
    if image.width == 0 || image.height == 0 || dest.width <= 0 || dest.height <= 0 {
        return;
    }
    let iw = i64::from(image.width);
    let ih = i64::from(image.height);
    let dw = i64::from(dest.width);
    let dh = i64::from(dest.height);
    let (fit_w, fit_h) = if iw * dh >= ih * dw {
        (dw, ((ih * dw) / iw).max(1))
    } else {
        (((iw * dh) / ih).max(1), dh)
    };
    let ox = dest.x + ((dw - fit_w) / 2) as i32;
    let oy = dest.y + ((dh - fit_h) / 2) as i32;
    let fw = frame.width() as i32;
    let fh = frame.height() as i32;

    for dy in 0..fit_h as i32 {
        let py = oy + dy;
        if py < 0 || py >= fh {
            continue;
        }
        let sy = ((i64::from(dy) * ih) / fit_h).min(ih - 1) as u32;
        for dx in 0..fit_w as i32 {
            let px = ox + dx;
            if px < 0 || px >= fw {
                continue;
            }
            let sx = ((i64::from(dx) * iw) / fit_w).min(iw - 1) as u32;
            if let Some([r, g, b, a]) = image.pixel(sx, sy)
                && a != 0
            {
                frame.blend(px, py, [r, g, b], a);
            }
        }
    }
}
