//! Native-styled paginated reference book modal for Spellbook (`Dialog` kind 12),
//! Cookbook (`Dialog` kind 15), and Neighborhood Credits/About.
//!
//! Styled with Complete Collection 1.91's native 9-slice chrome (`3008`, `17`,
//! `5107`). Localized labels, spell/charm/recipe entries, and credits text are
//! loaded at runtime from `GameData/magic.iff`, `GameData/Credits.iff`, and
//! `GameData/UIText.iff`; no verbatim game prose is embedded in source or tests.

use crate::art::Art;
use crate::dialog::{BACKGROUND, BUTTON};
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::neighborhood::{self, art_id::POPUP_TILER};
use crate::{Error, Result};
use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// `UIGraphics` resource IDs used by [`ReferenceBookDialog`]:
/// `PopupInfoTiles.bmp` (`3008`), `WinBtn.bmp` (`17`), and
/// `HotspotPopupTiler.TGA` (`5107`).
pub const RESOURCES: [i32; 3] = [BACKGROUND, BUTTON, POPUP_TILER];

/// `UIGraphics/Res_CPanel.h` `kMagicBookBkg` (`cpanel/Backgrounds/magicbookbkg.bmp`, `620×400`).
pub const MAGIC_BOOK_BKG: i32 = 82;
/// `UIGraphics/Res_CPanel.h` `kMagicBookOven` (`SPELLBOOK_BakersOven.bmp`, `130×90`).
pub const MAGIC_BOOK_OVEN: i32 = 90;
/// `UIGraphics/Res_CPanel.h` `kMagicBookCharm` (`SPELLBOOK_CharmMakerAdult.bmp`, `130×90`).
pub const MAGIC_BOOK_CHARM: i32 = 91;
/// `UIGraphics/Res_CPanel.h` `kMagicBookSpell` (`SPELLBOOK_WandCharger.bmp`, `130×90`).
pub const MAGIC_BOOK_SPELL: i32 = 92;
/// `UIGraphics/Res_CPanel.h` `kMagicBookKid` (`SPELLBOOK_CharmMakerKid.bmp`, `130×90`).
pub const MAGIC_BOOK_KID: i32 = 93;
/// `UIGraphics/Res_CPanel.h` `kMagicBookNectar` (`SPELLBOOK_NectarPress.bmp`, `130×90`).
pub const MAGIC_BOOK_NECTAR: i32 = 94;

/// Optional native Makin' Magic Spellbook & Cookbook bitmaps (`82`, `90..=94`).
pub const OPTIONAL_RESOURCES: [i32; 6] = [
    MAGIC_BOOK_BKG,
    MAGIC_BOOK_OVEN,
    MAGIC_BOOK_CHARM,
    MAGIC_BOOK_SPELL,
    MAGIC_BOOK_KID,
    MAGIC_BOOK_NECTAR,
];

/// Native `magicbookbkg.bmp` dimensions (`620×400`).
pub const MAGIC_BOOK_NATIVE_WIDTH: i32 = 620;
pub const MAGIC_BOOK_NATIVE_HEIGHT: i32 = 400;

/// Native category illustration shown on Spellbook & Cookbook pages (`90..=94`, `130×90`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookArtCategory {
    BakerOven,
    AdultCharm,
    AdultSpell,
    KidCharm,
    NectarPress,
}

impl BookArtCategory {
    /// `UIGraphics` resource ID (`90..=94`) for this book page's native `130×90` illustration.
    pub const fn art_id(self) -> i32 {
        match self {
            Self::BakerOven => MAGIC_BOOK_OVEN,
            Self::AdultCharm => MAGIC_BOOK_CHARM,
            Self::AdultSpell => MAGIC_BOOK_SPELL,
            Self::KidCharm => MAGIC_BOOK_KID,
            Self::NectarPress => MAGIC_BOOK_NECTAR,
        }
    }
}

/// `GameData/magic.iff` dialog labels (`STR# 1`: title templates, back/next,
/// page counter).
pub const MAGIC_BOOK_STR: u16 = 1;
/// `GameData/magic.iff` adult spell/charm descriptions (`STR# 8`).
pub const SPELL_DESC_STR: u16 = 8;
/// `GameData/magic.iff` kid charm descriptions (`STR# 9`).
pub const KID_CHARM_DESC_STR: u16 = 9;
/// `GameData/magic.iff` Nectar Bar / Baker's Oven descriptions (`STR# 10`).
pub const COOKBOOK_DESC_STR: u16 = 10;
/// `GameData/magic.iff` adult spell/charm titles (`STR# 12`).
pub const SPELL_TITLES_STR: u16 = 12;
/// `GameData/magic.iff` kid charm titles (`STR# 13`).
pub const KID_CHARM_TITLES_STR: u16 = 13;
/// `GameData/magic.iff` Nectar Bar / Baker's Oven titles (`STR# 14`).
pub const COOKBOOK_TITLES_STR: u16 = 14;
/// `GameData/UIText.iff` default dialog button labels (`STR# 142`).
pub const DIALOG_LABELS_STR: u16 = 142;
/// `GameData/UIText.iff` base credits fallback (`STR# 163`).
pub const UITEXT_CREDITS_STR: u16 = 163;
/// `GameData/Credits.iff` expansion credit tables ordered newest to base 1.0.
pub const CREDITS_TABLES: [u16; 9] = [171, 170, 169, 168, 167, 166, 165, 164, 163];

/// Loads the native 1.91 Magic Book bitmaps (`82`, `90..=94`), 9-slice frames,
/// button strip, and bitmap fonts used by [`ReferenceBookDialog`].
pub fn load_art(ui: &UiGraphics, language: u8) -> Result<Art> {
    Art::load_with_optional(ui, language, &RESOURCES, &OPTIONAL_RESOURCES)
}

const TEXT_NORMAL: [u8; 3] = [195, 205, 205];
const TEXT_HOVER: [u8; 3] = [255, 255, 255];
const TEXT_SELECTED: [u8; 3] = [0, 255, 255];
const TEXT_DISABLED: [u8; 3] = [0x40, 0x5d, 0x5f];
const TEXT_OK: [u8; 3] = [120, 238, 160];
const TEXT_MAGIC: [u8; 3] = [208, 152, 255];
const TEXT_ACCENT: [u8; 3] = [255, 224, 130];

const DIALOG_FALLBACK_BG: [u8; 3] = [24, 34, 112];
const DIALOG_FALLBACK_BORDER: [u8; 3] = [96, 132, 224];
const PAGE_BG: [u8; 3] = [12, 20, 74];
const PAGE_BORDER: [u8; 3] = [66, 96, 176];
const ENTRY_BG: [u8; 3] = [18, 28, 92];
const ENTRY_HIGHLIGHT_BG: [u8; 3] = [22, 54, 112];
const HELD_BG: [u8; 3] = [14, 24, 84];
const BADGE_OWNED_BG: [u8; 3] = [20, 76, 62];
const BADGE_MISSING_BG: [u8; 3] = [24, 30, 74];

/// Localized navigation and title templates loaded from `GameData/magic.iff`
/// (`STR# 1`) and `GameData/UIText.iff` (`STR# 142`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookStrings {
    pub spellbook_title_template: String,
    pub cookbook_title_template: String,
    pub back: String,
    pub next: String,
    pub page_counter_template: String,
    pub close: String,
}

impl Default for BookStrings {
    fn default() -> Self {
        Self {
            spellbook_title_template: "%s".into(),
            cookbook_title_template: "%s".into(),
            back: "Prev".into(),
            next: "Next".into(),
            page_counter_template: "%d / %d".into(),
            close: "OK".into(),
        }
    }
}

impl BookStrings {
    /// Loads localized book dialog labels from `GameData/magic.iff` and
    /// `GameData/UIText.iff` for `language` (1 = US English).
    pub fn load(install: &Install, language: u8) -> Result<Self> {
        let mut strings = Self::default();
        if let Ok(path) = install.file("GameData/magic.iff")
            && let Ok(bytes) = std::fs::read(&path)
            && let Ok(iff) = Iff::parse(bytes)
            && let Some(chunk) = iff.get(b"STR#", MAGIC_BOOK_STR)
            && let Ok(table) = StringTable::parse(iff.data(chunk))
        {
            if let Some(s) = read_table_entry(&table, 0, language) {
                strings.spellbook_title_template = s;
            }
            if let Some(s) = read_table_entry(&table, 1, language) {
                strings.cookbook_title_template = s;
            }
            if let Some(s) = read_table_entry(&table, 2, language) {
                strings.back = s;
            }
            if let Some(s) = read_table_entry(&table, 3, language) {
                strings.next = s;
            }
            if let Some(s) = read_table_entry(&table, 4, language) {
                strings.page_counter_template = s;
            }
        }
        if let Ok(path) = install.file("GameData/UIText.iff")
            && let Ok(bytes) = std::fs::read(&path)
            && let Ok(iff) = Iff::parse(bytes)
            && let Some(chunk) = iff.get(b"STR#", DIALOG_LABELS_STR)
            && let Ok(table) = StringTable::parse(iff.data(chunk))
            && let Some(ok) = read_table_entry(&table, 0, language)
        {
            strings.close = ok;
        }
        Ok(strings)
    }

    /// Formats the localized title for Spellbook (`is_cookbook == false`) or
    /// Cookbook (`is_cookbook == true`) using `subject` (family or Sim name).
    pub fn format_title(&self, is_cookbook: bool, subject: Option<&str>) -> String {
        let tmpl = if is_cookbook {
            &self.cookbook_title_template
        } else {
            &self.spellbook_title_template
        };
        let name = subject.map(str::trim).unwrap_or("");
        if tmpl.contains("%s") {
            let formatted = format_template(tmpl, &[name]);
            let cleaned = formatted.trim();
            if cleaned.is_empty() {
                name.to_owned()
            } else {
                cleaned.to_owned()
            }
        } else if name.is_empty() {
            tmpl.clone()
        } else {
            format!("{tmpl} - {name}")
        }
    }

    /// Formats the localized page counter (`1`-based `page` of `total`).
    pub fn format_page_counter(&self, page: usize, total: usize) -> String {
        let cur = page.to_string();
        let max = total.to_string();
        if self.page_counter_template.contains("%d") {
            format_template(&self.page_counter_template, &[&cur, &max])
        } else {
            format!("{cur} / {max}")
        }
    }
}

/// One spell, charm, or recipe loaded from `GameData/magic.iff`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagicCatalogEntry {
    pub index: usize,
    pub title: String,
    pub description: String,
    pub is_child: bool,
    pub category: BookArtCategory,
}

/// Loads the installed Spellbook (`is_cookbook == false`) or Cookbook
/// (`is_cookbook == true`) catalog directly from `GameData/magic.iff`.
pub fn load_magic_book_entries(
    install: &Install,
    is_cookbook: bool,
    language: u8,
) -> Result<Vec<MagicCatalogEntry>> {
    let path = install.file("GameData/magic.iff").map_err(Error::Content)?;
    let bytes = std::fs::read(&path).map_err(Error::Io)?;
    let iff = Iff::parse(bytes).map_err(Error::Format)?;
    let mut out = Vec::new();
    if is_cookbook {
        collect_magic_pairs(
            &iff,
            COOKBOOK_TITLES_STR,
            COOKBOOK_DESC_STR,
            language,
            false,
            true,
            &mut out,
        );
    } else {
        collect_magic_pairs(
            &iff,
            SPELL_TITLES_STR,
            SPELL_DESC_STR,
            language,
            false,
            false,
            &mut out,
        );
        collect_magic_pairs(
            &iff,
            KID_CHARM_TITLES_STR,
            KID_CHARM_DESC_STR,
            language,
            true,
            false,
            &mut out,
        );
    }
    Ok(out)
}

fn collect_magic_pairs(
    iff: &Iff,
    titles_id: u16,
    descs_id: u16,
    language: u8,
    is_child: bool,
    is_cookbook: bool,
    out: &mut Vec<MagicCatalogEntry>,
) {
    let Some(t_chunk) = iff.get(b"STR#", titles_id) else {
        return;
    };
    let Ok(titles) = StringTable::parse(iff.data(t_chunk)) else {
        return;
    };
    let descs = iff
        .get(b"STR#", descs_id)
        .and_then(|c| StringTable::parse(iff.data(c)).ok());

    // Entry 0 in each `magic.iff` table is the placeholder / intro header.
    for idx in 1..titles.entries.len() {
        let Some(title) = read_table_entry(&titles, idx, language) else {
            continue;
        };
        let lower = title.to_ascii_lowercase();
        if lower.contains("tbd")
            || lower == "not used"
            || lower == "unknown spell"
            || lower == "unknown charm"
            || lower == "unknown magic"
        {
            continue;
        }
        let desc = descs
            .as_ref()
            .and_then(|d| read_table_entry(d, idx, language))
            .unwrap_or_default();
        if desc.to_ascii_lowercase().contains("tbd") {
            continue;
        }
        let category = if is_cookbook {
            // `magic.iff STR# 14`: `1..=4` Nectar Press, `5..=9` Baker's Oven.
            if idx <= 4 {
                BookArtCategory::NectarPress
            } else {
                BookArtCategory::BakerOven
            }
        } else if is_child {
            BookArtCategory::KidCharm
        } else if idx <= 11 {
            // `magic.iff STR# 12`: `1..=11` Adult Charms, `13..=22` Adult Spells.
            BookArtCategory::AdultCharm
        } else {
            BookArtCategory::AdultSpell
        };
        out.push(MagicCatalogEntry {
            index: idx,
            title,
            description: desc,
            is_child,
            category,
        });
    }
}

/// Loads paginated credits pages from `GameData/Credits.iff` (with fallback to
/// `GameData/UIText.iff` `STR# 163`), preserving section headings without
/// embedding any game prose in source.
pub fn load_credits_pages(install: &Install, language: u8) -> Result<Vec<BookPage>> {
    const LINES_PER_PAGE: usize = 12;
    let mut pages = Vec::new();

    if let Ok(path) = install.file("GameData/Credits.iff")
        && let Ok(bytes) = std::fs::read(&path)
        && let Ok(iff) = Iff::parse(bytes)
    {
        for &table_id in &CREDITS_TABLES {
            let Some(chunk) = iff.get(b"STR#", table_id) else {
                continue;
            };
            let Ok(table) = StringTable::parse(iff.data(chunk)) else {
                continue;
            };
            let section_heading = {
                let raw = chunk.label.trim();
                (!raw.is_empty()).then(|| raw.to_owned())
            };
            let mut lines = Vec::new();
            for idx in 0..table.entries.len() {
                let Some(entry) = read_table_entry(&table, idx, language) else {
                    continue;
                };
                for part in entry.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
                    let trimmed = part.trim();
                    if !trimmed.is_empty() && trimmed != "[F]" {
                        lines.push(trimmed.to_owned());
                    }
                }
            }
            for chunk_lines in lines.chunks(LINES_PER_PAGE) {
                pages.push(BookPage {
                    heading: section_heading.clone(),
                    category: None,
                    entries: vec![BookEntry {
                        title: String::new(),
                        subtitle: None,
                        body: chunk_lines.join("\n"),
                        badges: Vec::new(),
                        highlight: false,
                    }],
                });
            }
        }
    }

    if pages.is_empty()
        && let Ok(path) = install.file("GameData/UIText.iff")
        && let Ok(bytes) = std::fs::read(&path)
        && let Ok(iff) = Iff::parse(bytes)
        && let Some(chunk) = iff.get(b"STR#", UITEXT_CREDITS_STR)
        && let Ok(table) = StringTable::parse(iff.data(chunk))
    {
        let mut lines = Vec::new();
        for idx in 0..table.entries.len() {
            let Some(entry) = read_table_entry(&table, idx, language) else {
                continue;
            };
            let trimmed = entry.trim();
            if !trimmed.is_empty() && trimmed != "[F]" {
                lines.push(trimmed.to_owned());
            }
        }
        for chunk_lines in lines.chunks(LINES_PER_PAGE) {
            pages.push(BookPage {
                heading: None,
                category: None,
                entries: vec![BookEntry {
                    title: String::new(),
                    subtitle: None,
                    body: chunk_lines.join("\n"),
                    badges: Vec::new(),
                    highlight: false,
                }],
            });
        }
    }

    Ok(pages)
}

/// Tag chip drawn inside a reference book (e.g. an ingredient or magic token
/// held by the calling Sim).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookBadge {
    pub label: String,
    pub owned: bool,
}

/// One entry card displayed on a [`BookPage`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookEntry {
    pub title: String,
    pub subtitle: Option<String>,
    pub body: String,
    pub badges: Vec<BookBadge>,
    /// Highlights the card border/fill (e.g. when the calling Sim holds
    /// matching ingredients or tokens for this spell/recipe).
    pub highlight: bool,
}

/// One page of a [`ReferenceBookSnapshot`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BookPage {
    pub heading: Option<String>,
    /// Optional native `130×90` Magic Book illustration (`90..=94`) for this page.
    pub category: Option<BookArtCategory>,
    pub entries: Vec<BookEntry>,
}

/// Read-only view model for [`ReferenceBookDialog`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferenceBookSnapshot {
    pub title: String,
    pub subtitle: Option<String>,
    /// Optional summary strip of tokens/ingredients currently held by the caller.
    pub held_summary: Vec<BookBadge>,
    pub pages: Vec<BookPage>,
    /// When `true`, renders the native `620×400` `magicbookbkg.bmp` (`82`) chrome
    /// on viewports `>= 620×400` (falling back to 9-slice on compact viewports).
    pub use_magic_book_chrome: bool,
}

/// Typed commands emitted by [`ReferenceBookDialog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookCommand {
    /// Active page index changed (`0`-based).
    PageChanged(usize),
    /// Request to scroll the current page vertically by the signed line delta
    /// (positive scrolls down, negative scrolls up).
    ScrollLines(i32),
    /// Request to close / dismiss the book modal (the host closes the dialog
    /// and answers any backing VM special panel).
    Close,
}

/// Interactive hit targets inside [`ReferenceBookDialog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookHit {
    PrevPage,
    NextPage,
    ScrollUp,
    ScrollDown,
    Close,
}

/// Computed screen-space geometry for [`ReferenceBookDialog`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookLayout {
    pub dialog: Rect,
    pub title_bar: Rect,
    pub held_bar: Option<Rect>,
    pub content_area: Rect,
    pub scroll_up_btn: Rect,
    pub scroll_down_btn: Rect,
    pub prev_btn: Rect,
    pub page_label: Rect,
    pub next_btn: Rect,
    pub close_btn: Rect,
}

impl BookLayout {
    /// Returns the interactive control under `(x, y)`, if any.
    pub fn hit(&self, x: i32, y: i32) -> Option<BookHit> {
        if self.scroll_up_btn.contains(x, y) {
            return Some(BookHit::ScrollUp);
        }
        if self.scroll_down_btn.contains(x, y) {
            return Some(BookHit::ScrollDown);
        }
        if self.prev_btn.contains(x, y) {
            return Some(BookHit::PrevPage);
        }
        if self.next_btn.contains(x, y) {
            return Some(BookHit::NextPage);
        }
        if self.close_btn.contains(x, y) {
            return Some(BookHit::Close);
        }
        None
    }
}

/// Interactive paginated Spellbook, Cookbook, and Credits reference modal.
#[derive(Debug, Clone, Default)]
pub struct ReferenceBookDialog {
    open: bool,
    page: usize,
    scroll_lines: usize,
    hovered: Option<BookHit>,
    pressed: Option<BookHit>,
}

impl ReferenceBookDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Current `0`-based page index.
    pub fn page(&self) -> usize {
        self.page
    }

    /// Current vertical line scroll offset within the active page.
    pub fn scroll_lines(&self) -> usize {
        self.scroll_lines
    }

    /// Opens the modal on page `0` at scroll offset `0`.
    pub fn open(&mut self) {
        self.open = true;
        self.page = 0;
        self.scroll_lines = 0;
        self.hovered = None;
        self.pressed = None;
    }

    /// Closes the modal and clears transient pointer state.
    pub fn close(&mut self) {
        self.open = false;
        self.scroll_lines = 0;
        self.hovered = None;
        self.pressed = None;
    }

    /// Clamps `self.page` against `snapshot.pages.len()`.
    pub fn effective_page(&self, snapshot: &ReferenceBookSnapshot) -> usize {
        if snapshot.pages.is_empty() {
            0
        } else {
            self.page.min(snapshot.pages.len() - 1)
        }
    }

    pub fn can_prev(&self, snapshot: &ReferenceBookSnapshot) -> bool {
        self.effective_page(snapshot) > 0
    }

    pub fn can_next(&self, snapshot: &ReferenceBookSnapshot) -> bool {
        let count = snapshot.pages.len();
        count > 0 && self.effective_page(snapshot) + 1 < count
    }

    /// Moves to the previous page, returning `Some(BookCommand::PageChanged)` if
    /// the page changed.
    pub fn prev_page(&mut self, snapshot: &ReferenceBookSnapshot) -> Option<BookCommand> {
        if !self.open {
            return None;
        }
        let cur = self.effective_page(snapshot);
        if cur == 0 {
            return None;
        }
        self.page = cur - 1;
        self.scroll_lines = 0;
        Some(BookCommand::PageChanged(self.page))
    }

    /// Moves to the next page, returning `Some(BookCommand::PageChanged)` if
    /// the page changed.
    pub fn next_page(&mut self, snapshot: &ReferenceBookSnapshot) -> Option<BookCommand> {
        if !self.open {
            return None;
        }
        let cur = self.effective_page(snapshot);
        if cur + 1 >= snapshot.pages.len() {
            return None;
        }
        self.page = cur + 1;
        self.scroll_lines = 0;
        Some(BookCommand::PageChanged(self.page))
    }

    /// Jumps to a specific `0`-based page clamped to `snapshot.pages.len()`.
    pub fn set_page(
        &mut self,
        index: usize,
        snapshot: &ReferenceBookSnapshot,
    ) -> Option<BookCommand> {
        if !self.open || snapshot.pages.is_empty() {
            return None;
        }
        let clamped = index.min(snapshot.pages.len() - 1);
        if clamped != self.page {
            self.page = clamped;
            self.scroll_lines = 0;
            Some(BookCommand::PageChanged(self.page))
        } else {
            None
        }
    }

    /// Maximum vertical line scroll offset for the active page at `frame_size`.
    pub fn max_scroll_lines(
        &self,
        frame_size: [u32; 2],
        snapshot: &ReferenceBookSnapshot,
        art: &Art,
    ) -> usize {
        let layout = self.layout(frame_size, snapshot);
        let page_idx = self.effective_page(snapshot);
        let Some(page) = snapshot.pages.get(page_idx) else {
            return 0;
        };
        measure_page_max_scroll(&layout, page, art)
    }

    /// Scrolls vertically within the active page by `delta_lines` (positive
    /// scrolls down), clamped to `max_scroll_lines`. Returns `true` if the
    /// visible scroll offset changed.
    pub fn scroll_vertical(
        &mut self,
        delta_lines: i32,
        frame_size: [u32; 2],
        snapshot: &ReferenceBookSnapshot,
        art: &Art,
    ) -> bool {
        if !self.open || delta_lines == 0 {
            return false;
        }
        let max_scroll = self.max_scroll_lines(frame_size, snapshot, art);
        let current = self.scroll_lines.min(max_scroll);
        let next = if delta_lines > 0 {
            current.saturating_add(delta_lines as usize).min(max_scroll)
        } else {
            current.saturating_sub(delta_lines.unsigned_abs() as usize)
        };
        if next != self.scroll_lines {
            self.scroll_lines = next;
            true
        } else {
            false
        }
    }

    /// Scrolls pages by `delta_pages` (positive advances forward).
    pub fn scroll(
        &mut self,
        delta_pages: i32,
        snapshot: &ReferenceBookSnapshot,
    ) -> Option<BookCommand> {
        if delta_pages < 0 {
            self.prev_page(snapshot)
        } else if delta_pages > 0 {
            self.next_page(snapshot)
        } else {
            None
        }
    }

    /// Emits a close request (`Escape` / `Enter`) so the host can dismiss the
    /// modal and answer any backing VM panel cleanly.
    pub fn escape(&mut self) -> Option<BookCommand> {
        if !self.open {
            return None;
        }
        Some(BookCommand::Close)
    }

    /// Computes responsive dialog geometry centered inside `frame_size`.
    pub fn layout(&self, frame_size: [u32; 2], snapshot: &ReferenceBookSnapshot) -> BookLayout {
        let fw = i32::try_from(frame_size[0]).unwrap_or(800).max(1);
        let fh = i32::try_from(frame_size[1]).unwrap_or(600).max(1);

        let margin = 12;
        let max_w = (fw - margin * 2).max(240).min(fw);
        let max_h = (fh - margin * 2).max(200).min(fh);
        let use_native_magic = snapshot.use_magic_book_chrome
            && fw >= MAGIC_BOOK_NATIVE_WIDTH
            && fh >= MAGIC_BOOK_NATIVE_HEIGHT;
        let (width, height) = if use_native_magic {
            (MAGIC_BOOK_NATIVE_WIDTH, MAGIC_BOOK_NATIVE_HEIGHT)
        } else {
            (580.min(max_w), 430.min(max_h))
        };
        let x = ((fw - width) / 2).max(0);
        let y = ((fh - height) / 2).max(0);
        let dialog = Rect::new(x, y, width, height);

        let pad = 16;
        let inner_x = dialog.x + pad;
        let inner_w = (dialog.width - pad * 2).max(120);

        let title_bar = Rect::new(inner_x, dialog.y + 12, inner_w, 22);
        let held_bar = if !snapshot.held_summary.is_empty() && dialog.height >= 260 {
            Some(Rect::new(
                inner_x,
                title_bar.y + title_bar.height + 4,
                inner_w,
                24,
            ))
        } else {
            None
        };

        let footer_h = 30;
        let footer_y = dialog.y + dialog.height - footer_h - 14;
        let gap = 8;
        let nav_w = 88.min((inner_w / 5).max(56));
        let close_w = 88.min((inner_w / 5).max(56));
        let prev_btn = Rect::new(inner_x, footer_y, nav_w, footer_h);
        let close_btn = Rect::new(inner_x + inner_w - close_w, footer_y, close_w, footer_h);
        let next_btn = Rect::new(close_btn.x - gap - nav_w, footer_y, nav_w, footer_h);
        let page_x = prev_btn.x + prev_btn.width + gap;
        let page_w = (next_btn.x - page_x - gap).max(32);
        let page_label = Rect::new(page_x, footer_y, page_w, footer_h);

        let content_y = held_bar.map_or(title_bar.y + title_bar.height + 8, |hb| {
            hb.y + hb.height + 6
        });
        let content_h = (footer_y - content_y - 10).max(60);
        let scroll_btn_w = 22;
        let scroll_gap = 4;
        let content_w = (inner_w - scroll_btn_w - scroll_gap).max(96);
        let content_area = Rect::new(inner_x, content_y, content_w, content_h);
        let scroll_x = content_area.x + content_area.width + scroll_gap;
        let scroll_btn_h = (content_h / 2 - 2).clamp(18, 28);
        let scroll_up_btn = Rect::new(scroll_x, content_y, scroll_btn_w, scroll_btn_h);
        let scroll_down_btn = Rect::new(
            scroll_x,
            content_y + content_h - scroll_btn_h,
            scroll_btn_w,
            scroll_btn_h,
        );

        BookLayout {
            dialog,
            title_bar,
            held_bar,
            content_area,
            scroll_up_btn,
            scroll_down_btn,
            prev_btn,
            page_label,
            next_btn,
            close_btn,
        }
    }

    pub fn pointer_move(
        &mut self,
        frame_size: [u32; 2],
        snapshot: &ReferenceBookSnapshot,
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
        snapshot: &ReferenceBookSnapshot,
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
        hit.is_some() || layout.dialog.contains(x, y)
    }

    pub fn pointer_up(
        &mut self,
        frame_size: [u32; 2],
        snapshot: &ReferenceBookSnapshot,
        x: i32,
        y: i32,
    ) -> Option<BookCommand> {
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
            BookHit::PrevPage => self.prev_page(snapshot),
            BookHit::NextPage => self.next_page(snapshot),
            BookHit::ScrollUp => Some(BookCommand::ScrollLines(-3)),
            BookHit::ScrollDown => Some(BookCommand::ScrollLines(3)),
            BookHit::Close => Some(BookCommand::Close),
        }
    }

    fn button_state(&self, target: BookHit, enabled: bool) -> ButtonState {
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

    /// Draws the paginated reference book modal into `frame`.
    pub fn draw(
        &self,
        frame: &mut Frame,
        snapshot: &ReferenceBookSnapshot,
        strings: &BookStrings,
        art: &Art,
    ) {
        if !self.open {
            return;
        }
        let layout = self.layout([frame.width(), frame.height()], snapshot);
        let use_native_magic = snapshot.use_magic_book_chrome
            && layout.dialog.width == MAGIC_BOOK_NATIVE_WIDTH
            && layout.dialog.height == MAGIC_BOOK_NATIVE_HEIGHT;
        if use_native_magic && let Some(bkg) = art.image(MAGIC_BOOK_BKG) {
            frame.draw_all(bkg, [layout.dialog.x, layout.dialog.y]);
        } else {
            draw_dialog_frame(frame, art, layout.dialog);
        }

        let title_font = art.font(12).or_else(|| art.font(10));
        let body_font = art.font(10).or_else(|| art.font(12));
        let small_font = art.font(9).or_else(|| art.font(10));

        // Title bar.
        if let Some(font) = title_font {
            let full_title = match snapshot
                .subtitle
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(sub) => format!("{} - {sub}", snapshot.title),
                None => snapshot.title.clone(),
            };
            draw_clipped_text(
                frame,
                font,
                &full_title,
                [layout.title_bar.x, layout.title_bar.y + 2],
                layout.title_bar.width,
                TEXT_HOVER,
            );
        }

        // Optional held-ingredients/tokens highlight strip.
        if let Some(held_rect) = layout.held_bar {
            frame.fill(held_rect, HELD_BG);
            draw_outline(frame, held_rect, PAGE_BORDER);
            if let Some(font) = small_font {
                let mut cx = held_rect.x + 6;
                let max_x = held_rect.x + held_rect.width - 6;
                let chip_h = (held_rect.height - 6).max(12);
                let chip_y = held_rect.y + (held_rect.height - chip_h) / 2;
                for badge in &snapshot.held_summary {
                    let text_w = font.width(&badge.label);
                    let chip_w = (text_w + 10).min((max_x - cx).max(0));
                    if chip_w < 18 {
                        break;
                    }
                    let chip_rect = Rect::new(cx, chip_y, chip_w, chip_h);
                    frame.fill(
                        chip_rect,
                        if badge.owned {
                            BADGE_OWNED_BG
                        } else {
                            BADGE_MISSING_BG
                        },
                    );
                    draw_outline(
                        frame,
                        chip_rect,
                        if badge.owned { TEXT_OK } else { PAGE_BORDER },
                    );
                    let ty = chip_rect.y + ((chip_rect.height - font.line_height) / 2).max(1);
                    draw_clipped_text(
                        frame,
                        font,
                        &badge.label,
                        [chip_rect.x + 5, ty],
                        (chip_rect.width - 10).max(8),
                        if badge.owned { TEXT_OK } else { TEXT_NORMAL },
                    );
                    cx += chip_w + 5;
                }
            }
        }

        // Page content panel.
        frame.fill(layout.content_area, PAGE_BG);
        draw_outline(frame, layout.content_area, PAGE_BORDER);

        let page_idx = self.effective_page(snapshot);
        let mut max_scroll = 0usize;
        if let Some(page) = snapshot.pages.get(page_idx) {
            max_scroll = measure_page_max_scroll(&layout, page, art);
            let skip_lines = self.scroll_lines.min(max_scroll);
            let mut y = layout.content_area.y + 8;
            let x = layout.content_area.x + 10;
            let content_w = (layout.content_area.width - 20).max(40);
            let max_y = layout.content_area.y + layout.content_area.height - 8;

            if let Some(heading) = page.heading.as_deref().filter(|s| !s.trim().is_empty())
                && let Some(font) = title_font
                && y + font.line_height <= max_y
            {
                draw_clipped_text(frame, font, heading, [x, y], content_w, TEXT_MAGIC);
                y += font.line_height + 6;
            }

            let entry_count = page.entries.len().max(1);
            let avail_h = (max_y - y).max(20);
            let gap = 6;
            let per_entry_h = if entry_count == 1 {
                avail_h
            } else {
                ((avail_h - gap * (entry_count as i32 - 1)) / entry_count as i32).max(48)
            };

            for entry in &page.entries {
                if y + 24 > max_y {
                    break;
                }
                let card_h = per_entry_h.min(max_y - y);
                let card = Rect::new(x, y, content_w, card_h);
                frame.fill(
                    card,
                    if entry.highlight {
                        ENTRY_HIGHLIGHT_BG
                    } else {
                        ENTRY_BG
                    },
                );
                draw_outline(
                    frame,
                    card,
                    if entry.highlight {
                        TEXT_OK
                    } else {
                        PAGE_BORDER
                    },
                );

                let mut ey = card.y + 8;
                let ex = card.x + 10;
                let emax_y = card.y + card.height - 6;
                let category_img = page
                    .category
                    .and_then(|cat| art.image(cat.art_id()))
                    .filter(|img| img.width > 0 && img.height > 0 && card.width >= 260 && card.height >= 106);
                let illus_rect = category_img.map(|img| {
                    let iw = i32::try_from(img.width).unwrap_or(130).min(130);
                    let ih = i32::try_from(img.height).unwrap_or(90).min(90);
                    Rect::new(card.x + card.width - iw - 10, card.y + 8, iw, ih)
                });
                let ew = match illus_rect {
                    Some(ir) => (ir.x - ex - 10).max(80),
                    None => (card.width - 20).max(24),
                };
                if let (Some(img), Some(ir)) = (category_img, illus_rect) {
                    let src = Rect::new(
                        0,
                        0,
                        i32::try_from(img.width).unwrap_or(i32::MAX),
                        i32::try_from(img.height).unwrap_or(i32::MAX),
                    );
                    draw_scaled_region(frame, img, src, ir);
                    draw_outline(frame, ir, PAGE_BORDER);
                }

                if !entry.title.is_empty()
                    && let Some(font) = title_font
                    && ey + font.line_height <= emax_y
                {
                    draw_clipped_text(
                        frame,
                        font,
                        &entry.title,
                        [ex, ey],
                        ew,
                        if entry.highlight {
                            TEXT_ACCENT
                        } else {
                            TEXT_HOVER
                        },
                    );
                    ey += font.line_height + 3;
                }

                if let Some(sub) = entry.subtitle.as_deref().filter(|s| !s.trim().is_empty())
                    && let Some(font) = small_font
                    && ey + font.line_height <= emax_y
                {
                    draw_clipped_text(frame, font, sub, [ex, ey], ew, TEXT_MAGIC);
                    ey += font.line_height + 4;
                }

                if !entry.badges.is_empty()
                    && let Some(font) = small_font
                    && ey + font.line_height + 6 <= emax_y
                {
                    let mut bx = ex;
                    let bmax_x = ex + ew;
                    let bh = font.line_height + 4;
                    for badge in &entry.badges {
                        let bw = (font.width(&badge.label) + 10).min((bmax_x - bx).max(0));
                        if bw < 16 {
                            break;
                        }
                        let brect = Rect::new(bx, ey, bw, bh);
                        frame.fill(
                            brect,
                            if badge.owned {
                                BADGE_OWNED_BG
                            } else {
                                BADGE_MISSING_BG
                            },
                        );
                        draw_outline(
                            frame,
                            brect,
                            if badge.owned { TEXT_OK } else { PAGE_BORDER },
                        );
                        draw_clipped_text(
                            frame,
                            font,
                            &badge.label,
                            [brect.x + 5, brect.y + 2],
                            (brect.width - 10).max(8),
                            if badge.owned { TEXT_OK } else { TEXT_NORMAL },
                        );
                        bx += bw + 5;
                    }
                    ey += bh + 6;
                }

                if !entry.body.is_empty()
                    && let Some(font) = body_font
                    && ey < emax_y
                {
                    let body_rect = Rect::new(ex, ey, ew, (emax_y - ey).max(0));
                    draw_multiline_wrapped_text(
                        frame,
                        font,
                        &entry.body,
                        body_rect,
                        skip_lines,
                        TEXT_NORMAL,
                    );
                }

                y += card_h + gap;
            }
        } else if let Some(font) = body_font {
            draw_clipped_text(
                frame,
                font,
                "—",
                [layout.content_area.x + 12, layout.content_area.y + 12],
                (layout.content_area.width - 24).max(16),
                TEXT_NORMAL,
            );
        }

        // Within-page vertical scroll buttons (enabled whenever page text wraps past card height).
        let clamped_scroll = self.scroll_lines.min(max_scroll);
        draw_button(
            frame,
            art,
            layout.scroll_up_btn,
            "^",
            self.button_state(BookHit::ScrollUp, clamped_scroll > 0),
        );
        draw_button(
            frame,
            art,
            layout.scroll_down_btn,
            "v",
            self.button_state(BookHit::ScrollDown, clamped_scroll < max_scroll),
        );

        // Footer navigation: Prev, Page X of Y, Next, Close.
        draw_button(
            frame,
            art,
            layout.prev_btn,
            &strings.back,
            self.button_state(BookHit::PrevPage, self.can_prev(snapshot)),
        );
        draw_button(
            frame,
            art,
            layout.next_btn,
            &strings.next,
            self.button_state(BookHit::NextPage, self.can_next(snapshot)),
        );
        draw_button(
            frame,
            art,
            layout.close_btn,
            &strings.close,
            self.button_state(BookHit::Close, true),
        );

        if let Some(font) = body_font {
            let total = snapshot.pages.len().max(1);
            let counter = strings.format_page_counter(page_idx + 1, total);
            let tw = font.width(&counter).min(layout.page_label.width);
            let tx = layout.page_label.x + ((layout.page_label.width - tw) / 2).max(0);
            let ty =
                layout.page_label.y + ((layout.page_label.height - font.line_height) / 2).max(2);
            draw_clipped_text(
                frame,
                font,
                &counter,
                [tx, ty],
                layout.page_label.width,
                TEXT_HOVER,
            );
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

fn read_table_entry(table: &StringTable, index: usize, language: u8) -> Option<String> {
    table
        .get(index, language)
        .or_else(|| table.get(index, 1))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn format_template(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut chars = template.chars().peekable();
    let mut idx = 0usize;
    while let Some(ch) = chars.next() {
        if ch == '%'
            && let Some(&next) = chars.peek()
        {
            match next {
                's' | 'd' | 'i' | 'u' => {
                    chars.next();
                    if let Some(arg) = args.get(idx) {
                        out.push_str(arg);
                    }
                    idx += 1;
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
        draw_outline(frame, rect, PAGE_BORDER);
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

fn wrap_multiline_lines(font: &Font, text: &str, max_width: i32) -> Vec<String> {
    if max_width <= 0 || text.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for raw_line in text.split('\n') {
        if raw_line.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in raw_line.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && font.width(&candidate) > max_width {
                lines.push(std::mem::take(&mut line));
                line.push_str(word);
            } else {
                line = candidate;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    lines
}

fn measure_page_max_scroll(layout: &BookLayout, page: &BookPage, art: &Art) -> usize {
    let title_font = art.font(12).or_else(|| art.font(10));
    let body_font = art.font(10).or_else(|| art.font(12));
    let small_font = art.font(9).or_else(|| art.font(10));
    let Some(body_font) = body_font else {
        return 0;
    };
    let lh = body_font.line_height.max(1);
    let mut y = layout.content_area.y + 8;
    let content_w = (layout.content_area.width - 20).max(40);
    let max_y = layout.content_area.y + layout.content_area.height - 8;

    if page
        .heading
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
        && let Some(font) = title_font
        && y + font.line_height <= max_y
    {
        y += font.line_height + 6;
    }

    let entry_count = page.entries.len().max(1);
    let avail_h = (max_y - y).max(20);
    let gap = 6;
    let per_entry_h = if entry_count == 1 {
        avail_h
    } else {
        ((avail_h - gap * (entry_count as i32 - 1)) / entry_count as i32).max(48)
    };

    let mut max_overflow = 0usize;
    for entry in &page.entries {
        if y + 24 > max_y {
            break;
        }
        let card_h = per_entry_h.min(max_y - y);
        let mut ey = y + 8;
        let has_illus = page
            .category
            .and_then(|cat| art.image(cat.art_id()))
            .is_some_and(|img| img.width > 0 && img.height > 0 && content_w >= 260 && card_h >= 106);
        let ew = if has_illus {
            (content_w - 130 - 30).max(80)
        } else {
            (content_w - 20).max(24)
        };
        let emax_y = y + card_h - 6;

        if !entry.title.is_empty()
            && let Some(font) = title_font
            && ey + font.line_height <= emax_y
        {
            ey += font.line_height + 3;
        }
        if entry
            .subtitle
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
            && let Some(font) = small_font
            && ey + font.line_height <= emax_y
        {
            ey += font.line_height + 4;
        }
        if !entry.badges.is_empty()
            && let Some(font) = small_font
            && ey + font.line_height + 6 <= emax_y
        {
            ey += font.line_height + 10;
        }
        if !entry.body.is_empty() && ey < emax_y {
            let body_h = (emax_y - ey).max(0);
            let visible = (body_h / lh).max(1) as usize;
            let total = wrap_multiline_lines(body_font, &entry.body, ew).len();
            max_overflow = max_overflow.max(total.saturating_sub(visible));
        }
        y += card_h + gap;
    }
    max_overflow
}

fn draw_multiline_wrapped_text(
    frame: &mut Frame,
    font: &Font,
    text: &str,
    rect: Rect,
    skip_lines: usize,
    color: [u8; 3],
) {
    if rect.width <= 0 || rect.height <= 0 {
        return;
    }
    let lh = font.line_height.max(1);
    let mut y = rect.y;
    let max_y = rect.y + rect.height;
    let lines = wrap_multiline_lines(font, text, rect.width);
    for line in lines.into_iter().skip(skip_lines) {
        if y + lh > max_y {
            return;
        }
        if !line.is_empty() {
            draw_clipped_text(frame, font, &line, [rect.x, y], rect.width, color);
        }
        y += lh;
    }
}
