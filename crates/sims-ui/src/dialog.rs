//! Message dialogs: what the Dialog primitive asks the player, drawn the
//! way the original draws it.
//!
//! The simulation keeps only a dialog's record: its caller, the
//! instruction's operands and its status (`sims_sim::Dialog`). What the
//! dialog says is presentation, so it is read here: the strings are
//! entries of the caller's code owner's `STR#` 301, picked by the
//! operands, and the icon is one of its `BMP_` chunks.
//!
//! The layout is the original's (0x523fe3, 0x524257, 0x535266), read
//! from the exe and checked against a screenshot of the original at 800
//! by 600 (the Magic Town notice on the Goth house);
//! `docs/decisions/20261115-message-dialog-layout.md` says what is read
//! and what is inferred.

use crate::art::{Art, KEY};
use crate::command::Command;
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::hud::{LABEL, rgb565};
use crate::{Error, Result};
use sims_content::{Catalog, Index, Install};
use sims_formats::bitmap::{self, Image};
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use sims_sim::{Dialog, World};
use std::cell::Cell;

/// The string table a private dialog's text comes from.
pub const STRINGS: u16 = 301;
/// The table of default button labels in `GameData/UIText.iff`: 1 OK,
/// 2 Cancel, 3 Yes, 4 No.
pub const LABELS: u16 = 142;
/// The table of localized keyboard hotkeys in `GameData/UIText.iff`
/// (`0x4f370d..0x4f374e`: 7 Yes, 8 No -> stored in `[0x6d7530]`).
pub const HOTKEYS: u16 = 161;
/// A dialog's icon is `BMP_` 5000 plus operand 1 in the caller's file
/// (icon mode 3; legacy/crates/sims-vm/src/dialog.rs `private_bitmap_id`).
pub const ICONS: u16 = 5000;
/// The shared `WinBtn.bmp` (registered at 0x4efe20, 0x4efe33 and
/// 0x5e7d90): four cells of 65 by 33 (normal, selected, hover, disabled).
pub const BUTTON: i32 = 17;
/// The dialog frame 9-slice tile sheet (`kCatalogPopupBackTiles`,
/// `PopupInfoTiles.bmp`, 36×36 = 3×3 grid of 12×12 tiles, registered at
/// CtrlMgr slot 5 at `0x4efe3b..0x4efe59` and loaded in `cTSWinMsgBox::Init`
/// at `0x523528..0x523536`).
pub const BACKGROUND: i32 = 3008;
/// The generic dialog frame 9-slice tile sheet (`kSMSystemGenDlgEdge`,
/// `GenDlg.bmp`, 30×30 = 3×3 grid of 10×10 tiles, CtrlMgr slot 9 at
/// `0x4efec9..0x4efede`, used by `cTSWinGenDlg::Init` at `0x5e8f3d`).
pub const GENERIC_FRAME: i32 = 22;
/// The portrait backing bitmap (`kPictureDialogStub`, `SimStub.BMP`,
/// 133×103, loaded at `0x523572`) drawn behind 45×45 (`0x2d × 0x2d`)
/// dialog icons (`0x524660..0x524782`, `0x524a6c..0x524ae7`).
pub const ICON_STUB: i32 = 4992;
/// The art a dialog needs besides the fonts.
pub const RESOURCES: [i32; 4] = [BUTTON, BACKGROUND, GENERIC_FRAME, ICON_STUB];
/// Half-period of the `cTSWinTextEdit2` caret blink timer in microseconds:
/// 500 ms (`0x1f4` ms at `0x5f203e`).
pub const CARET_BLINK_US: u64 = 500_000;

/// Button text colors from `CtrlMgr::SetTextColors(4)` (`0x4ff509..0x4ff538`):
/// normal `[0x6f2a94]`, selected/pressed `[0x6f2a9c]`, hover `[0x6f2a98]`,
/// disabled `[0x6f2aa0]`.
const BUTTON_SELECTED: [u8; 3] = [0, 255, 255];
const BUTTON_HOVER: [u8; 3] = [255, 255, 255];
const BUTTON_DISABLED: [u8; 3] = [0x40, 0x5d, 0x5f];
/// `cTSWinTextEdit2` background fill color (`[0x6f2aa4]`, initialized from
/// `RGB(0, 0, 0x52)` at `0x4ff3dd..0x4ff3ef` and passed to `SetColors` at
/// `0x5238cd..0x5238d3`).
const ENTRY_FILL: [u8; 3] = rgb565(0x000a);

/// The 9-slice tile sheet a modal dialog draws for its border and interior.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrameStyle {
    /// `PopupInfoTiles.bmp` (`3008`, 36×36 = 12×12 tiles, `cTSWinMsgBox`,
    /// `0x4efe3b..0x4efe59`, `0x523528`).
    #[default]
    Popup,
    /// `cTSWinGenDlg` confirmation layout (`0x5e542f..0x5e589b`, `0x5e5d46`):
    /// requests frame slot 5 at `0x5e5441`, which `0x4efe3b..0x4efe59` binds
    /// to `PopupInfoTiles.bmp` (`3008`, 36×36 = 12×12 tiles).
    Generic,
}

impl FrameStyle {
    /// UIGraphics bitmap resource ID for this frame style.
    pub const fn resource_id(self) -> i32 {
        match self {
            Self::Popup | Self::Generic => BACKGROUND,
        }
    }

    /// Fallback 9-slice tile inset when the bitmap is not loaded.
    const fn default_inset(self) -> i32 {
        INSET
    }
}

/// The text the original shows for a missing string (the old
/// prototype's `select_text`). **Unverified.**
const MISSING: &str = "Missing string!!!";

/// One dialog, ready to lay out.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// The object whose Dialog instruction asked.
    pub caller: u16,
    pub title: String,
    pub text: String,
    /// The buttons' labels in answer order: the first button, then the
    /// second and third.
    pub buttons: Vec<String>,
    pub icon: Option<Image>,
    /// The text typed into a dialog that asks for text (kind 3) or the Lot Name
    /// field of an edit-mode Save Card (`0x50da07`); None for any other dialog.
    pub entry: Option<String>,
    /// Optional second text field: the Lot Description field (`0x50db31`, max
    /// 254 chars) on an edit-mode Save Card (`cWinEditLotInfo`, `0x50d68a`).
    pub secondary_entry: Option<String>,
    /// Which 9-slice frame tile sheet (`PopupInfoTiles.bmp` 3008 or
    /// `GenDlg.bmp` 22) the dialog uses.
    pub frame: FrameStyle,
    /// Whether this dialog uses the edit-mode Save Card layout (`0x50d68a..0x50de10`,
    /// centered around `SAVE_CARD` 4994 with Lot Name and Lot Description fields).
    pub save_card: bool,
}

/// What a dialog that asks for text holds when it opens: the original
/// passes this string to its entry field (0x436c48, 0x523808).
pub const ENTRY_START: &str = "Hello world";
/// The most characters the entry field holds (0x5238a1, field +0x138,
/// checked by 0x5f334f and 0x5f33be). The original counts bytes of its
/// text encoding, the same as characters in a single-byte one.
pub const ENTRY_MOST: usize = 64;
/// Maximum characters in the Save Card's Lot Name field (`0x50db03`: `push 0x31`).
pub const SAVE_CARD_NAME_MOST: usize = 49;
/// Maximum characters in the Save Card's Lot Description field (`0x50dc50`: `push 0xfe`).
pub const SAVE_CARD_DESC_MOST: usize = 254;
/// The entry field's height (0x524420).
const ENTRY_HEIGHT: i32 = 36;

/// Types `typed` at the end of the dialog's entry, if it asks for text,
/// is not full and `typed` is not a control character. Returns whether
/// the text changed.
pub fn type_into(message: &mut Message, typed: char) -> bool {
    match &mut message.entry {
        Some(text) if !typed.is_control() && text.chars().count() < ENTRY_MOST => {
            text.push(typed);
            true
        }
        _ => false,
    }
}

/// Erases the entry's last character. Returns whether the text changed.
pub fn erase(message: &mut Message) -> bool {
    message.entry.as_mut().and_then(String::pop).is_some()
}

/// The default button labels (`STR# 142`) and localized Yes/No confirmation
/// hotkeys (`STR# 161` entries 7 and 8, loaded into `[0x6d7530]` at
/// `0x4f370d..0x4f374e`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Labels {
    labels: Vec<String>,
    yes_key: char,
    no_key: char,
}

impl Default for Labels {
    fn default() -> Self {
        Self {
            labels: Vec::new(),
            yes_key: 'y',
            no_key: 'n',
        }
    }
}

impl Labels {
    /// Reads labels (`STR# 142`) and confirmation hotkeys (`STR# 161[7..8]`)
    /// from the install's `GameData/UIText.iff`.
    pub fn load(install: &Install) -> Result<Self> {
        let path = install
            .file("GameData/UIText.iff")
            .map_err(Error::Content)?;
        let bytes = std::fs::read(&path).map_err(Error::Io)?;
        let iff = Iff::parse(bytes).map_err(Error::Format)?;
        let mut labels = Vec::new();
        if let Some(chunk) = iff.get(b"STR#", LABELS) {
            let table = StringTable::parse(iff.data(chunk)).map_err(Error::Format)?;
            labels = (0..table.entries.len())
                .filter_map(|i| table.get(i, 1).map(str::to_owned))
                .collect();
        }
        let mut yes_key = 'y';
        let mut no_key = 'n';
        if let Some(chunk) = iff.get(b"STR#", HOTKEYS)
            && let Ok(table) = StringTable::parse(iff.data(chunk))
        {
            if let Some(ch) = table.get(6, 1).and_then(|s| s.chars().next()) {
                yes_key = ch;
            }
            if let Some(ch) = table.get(7, 1).and_then(|s| s.chars().next()) {
                no_key = ch;
            }
        }
        Ok(Self {
            labels,
            yes_key,
            no_key,
        })
    }

    pub fn new(labels: Vec<String>) -> Self {
        Self {
            labels,
            yes_key: 'y',
            no_key: 'n',
        }
    }

    /// Overrides the confirmation Yes/No hotkeys (for localized tests).
    pub fn with_hotkeys(mut self, yes_key: char, no_key: char) -> Self {
        self.yes_key = yes_key;
        self.no_key = no_key;
        self
    }

    /// Localized Yes shortcut key from `UIText.iff STR# 161[7]` (`[0x6d7530]`).
    pub fn yes_key(&self) -> char {
        self.yes_key
    }

    /// Localized No shortcut key from `UIText.iff STR# 161[8]` (`[0x6d7531]`).
    pub fn no_key(&self) -> char {
        self.no_key
    }

    /// Label `n`, counting from 1 as the table's users do (1 OK, 2 Cancel,
    /// 3 Yes, 4 No in `GameData/UIText.iff STR# 142`).
    pub fn get(&self, n: usize) -> &str {
        n.checked_sub(1)
            .and_then(|i| self.labels.get(i))
            .map_or(MISSING, String::as_str)
    }
}

/// The first dialog record that waits for the player: made and not yet
/// answered.
pub fn pending(world: &World) -> Option<&Dialog> {
    world.dialogs.records.iter().find(|r| r.status <= 1)
}

impl Message {
    /// Builds a modal notice dialog with one button (for UI/host dialogs
    /// outside the simulation, with `caller == 0`).
    pub fn notice(
        title: impl Into<String>,
        text: impl Into<String>,
        ok: impl Into<String>,
    ) -> Self {
        Self {
            caller: 0,
            title: title.into(),
            text: text.into(),
            buttons: vec![ok.into()],
            icon: None,
            entry: None,
            secondary_entry: None,
            frame: FrameStyle::Popup,
            save_card: false,
        }
    }

    /// Builds a modal confirmation dialog with two or three buttons (e.g.
    /// Yes/No or Yes/No/Cancel, as `cTSWinGenDlg` creates at `0x4f290d`,
    /// `0x4f1a11`, and `0x4f25b4`).
    pub fn confirm(
        title: impl Into<String>,
        text: impl Into<String>,
        buttons: Vec<String>,
    ) -> Self {
        Self {
            caller: 0,
            title: title.into(),
            text: text.into(),
            buttons,
            icon: None,
            entry: None,
            secondary_entry: None,
            frame: FrameStyle::Popup,
            save_card: false,
        }
    }

    /// Builds a modal text-input dialog (`cTSWinTextEdit2` at `0x523820..0x5238f3`).
    pub fn prompt(
        title: impl Into<String>,
        text: impl Into<String>,
        initial: impl Into<String>,
        buttons: Vec<String>,
    ) -> Self {
        let mut initial = initial.into();
        if initial.chars().count() > ENTRY_MOST {
            initial = initial.chars().take(ENTRY_MOST).collect();
        }
        Self {
            caller: 0,
            title: title.into(),
            text: text.into(),
            buttons,
            icon: None,
            entry: Some(initial),
            secondary_entry: None,
            frame: FrameStyle::Popup,
            save_card: false,
        }
    }

    /// Builds the edit-mode Save Card dialog (`cWinEditLotInfo`,
    /// `0x50d68a..0x50de10`, `PopupInfoTiles.bmp` 3008 + `SAVE_CARD` 4994)
    /// with editable Lot Name (max 49 chars), multiline Lot Description (max
    /// 254 chars), and optional confirmation prompt (`STR# 153[5]` at
    /// `0x50dcc8..0x50dcec` on 3-button cards).
    pub fn save_card(
        lot_name: impl Into<String>,
        lot_description: impl Into<String>,
        prompt: Option<String>,
        buttons: Vec<String>,
        card_art: Option<Image>,
    ) -> Self {
        let mut name = lot_name.into();
        if name.chars().count() > SAVE_CARD_NAME_MOST {
            name = name.chars().take(SAVE_CARD_NAME_MOST).collect();
        }
        let mut desc = lot_description.into();
        if desc.chars().count() > SAVE_CARD_DESC_MOST {
            desc = desc.chars().take(SAVE_CARD_DESC_MOST).collect();
        }
        Self {
            caller: 0,
            title: String::new(),
            text: prompt.unwrap_or_default(),
            buttons,
            icon: card_art,
            entry: Some(name),
            secondary_entry: Some(desc),
            frame: FrameStyle::Popup,
            save_card: true,
        }
    }

    /// Attaches an icon or 45×45 portrait to this dialog.
    pub fn with_icon(mut self, icon: Option<Image>) -> Self {
        self.icon = icon;
        self
    }

    /// Selects the 9-slice frame style (`PopupInfoTiles.bmp` 3008 or
    /// `GenDlg.bmp` 22) for this dialog.
    pub fn with_frame(mut self, frame: FrameStyle) -> Self {
        self.frame = frame;
        self
    }

    /// Reads what `dialog` shows. None when the caller, its code owner or
    /// its file is gone, or the dialog is of a kind that is not ported
    /// (text entry and the special panels).
    pub fn read(
        world: &World,
        index: &Index,
        catalog: &Catalog,
        labels: &Labels,
        dialog: &Dialog,
    ) -> Result<Option<Self>> {
        let Some(owner) = world
            .objects
            .get(&dialog.caller)
            .and_then(|o| o.stack.last())
            .and_then(|f| u16::try_from(f.code_owner_type).ok())
            .and_then(|t| world.types.get(&t))
            .and_then(|&guid| catalog.object(guid))
            .and_then(|o| index.resources.get(o.resource))
        else {
            return Ok(None);
        };
        let iff = Iff::parse(owner.read().map_err(Error::Content)?).map_err(Error::Format)?;
        Self::from_file(&iff, labels, dialog.caller, &dialog.operands)
    }

    /// Reads what a Dialog instruction with `operands` shows, from the
    /// code owner's file `iff`.
    ///
    /// The operands, as the old prototype read them
    /// (legacy/crates/sims-vm/src/dialog.rs, `describe_using`): 0 the
    /// third button's string, 1 the icon, 2 the message, 3 the first
    /// button's, 4 the second's, 5 the kind, 6 the title, 7 flags (bits 1
    /// to 3 the icon mode). String numbers count from 1; 0 or an empty
    /// string takes the default.
    pub fn from_file(
        iff: &Iff,
        labels: &Labels,
        caller: u16,
        operands: &[u8; 8],
    ) -> Result<Option<Self>> {
        let table = match iff.get(b"STR#", STRINGS) {
            Some(chunk) => Some(StringTable::parse(iff.data(chunk)).map_err(Error::Format)?),
            None => None,
        };
        let entry = |n: u8| {
            usize::from(n)
                .checked_sub(1)
                .and_then(|i| table.as_ref()?.get(i, 1))
                .filter(|s| !s.is_empty())
        };
        let text = |n: u8, default: &str| entry(n).unwrap_or(default).to_owned();
        // From the original's button setup (0x43601c), as the old
        // prototype read it (legacy/crates/sims-vm/src/dialog/presentation.rs).
        let kind = operands[5] & 0x7f;
        let buttons = match kind {
            // Kind 3 asks for text: the first button and an entry field
            // under the text (0x436a52).
            0 | 3 | 4 => vec![text(operands[3], labels.get(1))],
            1 => vec![
                text(operands[3], labels.get(3)),
                text(operands[4], labels.get(4)),
            ],
            2 => vec![
                text(operands[3], labels.get(3)),
                text(operands[4], labels.get(4)),
                text(operands[0], labels.get(2)),
            ],
            _ => return Ok(None),
        };
        let icon = if (operands[7] >> 1) & 7 == 3 {
            match iff.get(b"BMP_", ICONS + u16::from(operands[1])) {
                Some(chunk) => {
                    let mut image = bitmap::bmp(iff.data(chunk)).map_err(Error::Format)?;
                    image.key(KEY);
                    Some(image)
                }
                None => None,
            }
        } else {
            None
        };
        Ok(Some(Self {
            caller,
            // The original falls back to the caller's catalog name
            // (the old prototype's `display_name`); not done here.
            title: text(operands[6], ""),
            text: text(operands[2], MISSING),
            buttons,
            icon,
            entry: (kind == 3).then(|| ENTRY_START.to_owned()),
            secondary_entry: None,
            frame: FrameStyle::Popup,
            save_card: false,
        }))
    }
}

/// The fonts: the title in `variablesans_14`, the rest in `_12`.
const TITLE_FONT: u8 = 14;
const BODY_FONT: u8 = 12;
/// How far in from the frame's edge the parts are laid out: a third of
/// the frame's art (0x524a31) and 9 more (0x524257). The frame's art is
/// not identified; 12 gives the 21 pixels measured.
const INSET: i32 = 12;
const PAD: i32 = 9;
/// Space between the title, the text and the buttons, and beside the
/// icon (0x524257).
const GAP: i32 = 18;
/// A button is at least 100 wide (0x52373c); its height is the art's.
const BUTTON_SIZE: [i32; 2] = [100, 33];
/// The text is at least 30 wider than each button, together (0x524257).
const BUTTON_ROOM: i32 = 30;
/// The text's least width with neither title nor icon (0x524257).
const PLAIN_WIDTH: i32 = 100;
/// The title's box is its text and 4 more across: inferred from where
/// the measured title sits, since the title is centered by its box.
const TITLE_BOX: i32 = 4;
/// The width search (0x523fe3) aims at a dialog twice as wide as tall
/// (0x523409), is done within 0.05 of it, and halves the gap until it
/// is 20 or less, for at most 10 tries.
const ASPECT: f64 = 2.0;
const CLOSE: f64 = 0.05f32 as f64;
const STEP: i32 = 20;
const TRIES: i32 = 10;
/// The dialog's width before it is first laid out (0x524001).
const FIRST_WIDTH: i32 = 300;
/// The widest text tried. The original doubles without end until the
/// dialog is wide enough; a text of many short lines never gets there.
const WIDEST: i32 = 1 << 20;

/// A laid-out dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub caller: u16,
    pub rect: Rect,
    pub icon: Option<[i32; 2]>,
    pub title: [i32; 2],
    /// Each line of the message and where it starts.
    pub lines: Vec<(String, [i32; 2])>,
    pub buttons: Vec<Rect>,
    /// The entry field of a dialog that asks for text (or Lot Name on a Save Card).
    pub entry: Option<Rect>,
    /// The Lot Description field of an edit-mode Save Card (`0x50dbb2..0x50dc50`).
    pub secondary_entry: Option<Rect>,
}

/// The message box at `width` (0x535266): the lines beside the icon,
/// those under it, and its height. `band` is the room the icon takes,
/// across and down, in whole lines; the lines that fit down it go beside
/// the icon, the rest under the band.
struct TextBox {
    beside: Vec<String>,
    below: Vec<String>,
    height: i32,
}

/// Splits `text` into the lines a box `width` wide gives, at most
/// `most` of them, and how many characters they take.
fn split(font: &Font, text: &[char], width: i32, most: usize) -> (Vec<String>, usize) {
    let (mut lines, mut at) = (Vec::new(), 0);
    while at < text.len() && lines.len() < most {
        let n = font.fit(&text[at..], width).clamp(1, text.len() - at);
        lines.push(text[at..at + n].iter().filter(|&&c| c != '\n').collect());
        at += n;
    }
    (lines, at)
}

fn text_box(font: &Font, text: &[char], width: i32, band: [i32; 2]) -> TextBox {
    let line = font.line_height.max(1);
    let fits = usize::try_from(band[1] / line).unwrap_or(0);
    let narrow = width.saturating_sub(band[0]);
    let (all, _) = split(font, text, narrow, fits.saturating_add(1));
    if all.len() <= fits {
        return TextBox {
            beside: all,
            below: Vec::new(),
            height: band[1],
        };
    }
    let (mut beside, used) = split(font, text, narrow, fits);
    let (mut below, _) = split(font, &text[used..], width, usize::MAX);
    let rows = i32::try_from(below.len()).unwrap_or(i32::MAX);
    // One line left over that fits beside the icon is drawn there, on
    // down from the others (0x534fe5).
    if below.len() == 1 && font.width(&text[used..].iter().collect::<String>()) <= narrow {
        beside.append(&mut below);
    }
    TextBox {
        beside,
        below,
        height: band[1].saturating_add(rows.saturating_mul(line)),
    }
}

/// The dialog with its text `width` wide (or the least it may be).
struct Arranged {
    text_width: i32,
    text: TextBox,
    /// The buttons' top, from the dialog's.
    buttons: i32,
    size: [i32; 2],
}

impl Arranged {
    fn ratio(&self) -> f64 {
        f64::from(self.size[0]) / f64::from(self.size[1].max(1))
    }
}

/// Lays `message` out centered in a window of `size`, as the original's
/// message dialog lays itself out (0x523fe3, 0x524257). None without
/// the fonts.
///
/// - The text is at least as wide as the title, the icon and 18
///   between them, or the icon, or 100, and as the buttons with 30 more
///   each. Its width is then searched for a dialog twice as wide as
///   tall: doubled until it is at least that, halved until it is at
///   most that, then the two halved toward each other; the closer of the
///   last two is kept.
/// - The icon is at the top left; the text's lines that fit down the
///   icon's band go beside it, the rest under the band.
/// - The buttons are spread evenly across the text's width, an equal
///   gap before, between and after them, 18 under the text.
///
/// A dialog bigger than the window is narrowed to it, loses lines from
/// its end and keeps to the window's bottom, so its buttons are on
/// screen in any window 100 a button wide and 54 high; how the original
/// handles one is **unverified**.
pub fn layout(size: [i32; 2], message: &Message, art: &Art) -> Option<Layout> {
    let (title_font, body) = (art.font(TITLE_FONT)?, art.font(BODY_FONT)?);
    let size = size.map(|n| n.clamp(0, 32768));
    if message.save_card {
        return Some(layout_save_card(size, message, body));
    }
    if message.frame == FrameStyle::Generic {
        return Some(layout_generic(size, message, body));
    }
    // 0x524a31 / 0x5e8f6a: frame inset is (frame_bmp.width / 3, frame_bmp.height / 3):
    // (12, 12) for `PopupInfoTiles.bmp` (3008, 36×36), (10, 10) for `GenDlg.bmp` (22, 30×30).
    let fallback_inset = message.frame.default_inset();
    let (inset_x, inset_y) = art
        .image(message.frame.resource_id())
        .filter(|bmp| bmp.width >= 3 && bmp.height >= 3)
        .map_or((fallback_inset, fallback_inset), |bmp| {
            (
                i32::try_from(bmp.width / 3).unwrap_or(fallback_inset),
                i32::try_from(bmp.height / 3).unwrap_or(fallback_inset),
            )
        });
    // 0x524a6c..0x524ae7: a 45×45 (`0x2d × 0x2d`) portrait icon reserves
    // `SimStub.BMP` (`ICON_STUB` = 4992, 133×103) dimensions.
    let icon = message.icon.as_ref().map(|i| {
        if i.width == 45
            && i.height == 45
            && let Some(stub) = art.image(ICON_STUB)
        {
            return [
                i32::try_from(stub.width).unwrap_or(0).min(1024),
                i32::try_from(stub.height).unwrap_or(0).min(1024),
            ];
        }
        [
            i32::try_from(i.width).unwrap_or(0).min(1024),
            i32::try_from(i.height).unwrap_or(0).min(1024),
        ]
    });
    // Line heights come from the font files: keep them sane so nothing
    // below can overflow.
    let title_line = title_font.line_height.clamp(0, 1024);
    let line = body.line_height.clamp(1, 1024);
    let title = (!message.title.is_empty())
        .then(|| title_font.width(&message.title).clamp(0, 1 << 16) + TITLE_BOX);
    let text: Vec<char> = message.text.chars().collect();
    let count = message.buttons.len().min(3);
    let buttons = i32::try_from(count).unwrap_or(0);
    let icon_width = icon.map_or(0, |[w, _]| w);
    let least = match title {
        Some(title) => title + icon_width + GAP,
        None if icon_width != 0 => icon_width,
        None => PLAIN_WIDTH,
    }
    .max(buttons * (BUTTON_SIZE[0] + BUTTON_ROOM));
    let top = inset_x + PAD;
    let top_y = inset_y + PAD;
    let text_top = match title {
        Some(_) => top_y + title_line + GAP,
        None => top_y,
    };
    // The icon's band reaches 18 under it, from where the text is first
    // put (0x523fe3), rounded to the nearest whole line, down on a tie
    // (0x5351cc). The text is first put under the title's box as it was
    // made, which is **unverified**; here it is the title's line, and
    // anything from 6 less to 16 more gives the measured dialog.
    let band = icon.map_or([0, 0], |[w, h]| {
        let reach = top + h + GAP - text_top;
        let lines = reach / line;
        let over = reach - lines * line;
        let lines = if over > (lines + 1) * line - reach {
            lines + 1
        } else {
            lines
        };
        [w + GAP, lines * line]
    });
    // The dialog's width as the last pass left it: the title is
    // centered on it (0x5243b5). It is 300 before the first (0x524001).
    let last_width = Cell::new(FIRST_WIDTH);
    let arrange_within = |width: i32, least: i32| {
        let text_width = width.clamp(least, WIDEST);
        let text = text_box(body, &text, text_width, band);
        // An entry field goes 9 under the text, as wide as it and 36 high,
        // and the buttons 18 under where they would have been and its
        // height lower (0x52440d).
        let entry_room = if message.entry.is_some() {
            ENTRY_HEIGHT + GAP
        } else {
            0
        };
        let buttons_top = text_top
            .saturating_add(text.height)
            .saturating_add(GAP + entry_room);
        let bottom = if count > 0 {
            buttons_top.saturating_add(BUTTON_SIZE[1] + PAD)
        } else {
            buttons_top
        };
        let size = [text_width + 2 * top, bottom.saturating_add(inset_y)];
        last_width.set(size[0]);
        Arranged {
            text_width,
            text,
            buttons: buttons_top,
            size,
        }
    };
    let arrange = |width: i32| arrange_within(width, least);

    // The width search (0x523fe3). The text starts with no width.
    let first = arrange(0);
    let start = first.text_width;
    let (mut wide, mut narrow) = (start, start);
    let (mut wide_ratio, mut narrow_ratio) = (first.ratio() as f32, first.ratio() as f32);
    if (first.ratio() - ASPECT).abs() > CLOSE {
        loop {
            let a = arrange(wide);
            wide_ratio = a.ratio() as f32;
            if a.ratio() >= ASPECT || a.text_width >= WIDEST {
                break;
            }
            wide = a.text_width.saturating_mul(2).min(WIDEST);
        }
        for tries in 1.. {
            let a = arrange(narrow);
            narrow_ratio = a.ratio() as f32;
            if a.ratio() <= ASPECT {
                break;
            }
            narrow = a.text_width / 2;
            if tries + 1 >= TRIES {
                break;
            }
        }
        for tries in 1.. {
            if (wide - narrow).abs() <= STEP {
                break;
            }
            let a = arrange((wide + narrow) / 2);
            if a.ratio() >= ASPECT {
                (wide, wide_ratio) = (a.text_width, a.ratio() as f32);
            } else {
                (narrow, narrow_ratio) = (a.text_width, a.ratio() as f32);
            }
            if tries + 1 >= TRIES {
                break;
            }
        }
    }
    let off = |ratio: f32| (f64::from(ratio) - ASPECT).abs();
    let mut before = last_width.get();
    let mut a = arrange(if off(wide_ratio) <= off(narrow_ratio) {
        wide
    } else {
        narrow
    });

    // A dialog bigger than the window is narrowed to it, no narrower
    // than its buttons side by side, loses lines from its end, and is
    // then kept to the window's bottom, so that its buttons are on
    // screen in any window 100 a button wide and 54 high. None of it is
    // the original's: how it handles one is **unverified**.
    let room = size[0] - 2 * top;
    if a.size[0] > size[0] || a.size[1] > size[1] {
        a = arrange_within(a.text_width.min(room), buttons * BUTTON_SIZE[0]);
        before = a.size[0];
    }
    let mut height = a.size[1];
    while height > size[1] && a.text.below.len() + a.text.beside.len() > 1 {
        if a.text.below.pop().is_none() {
            break;
        }
        height -= line;
        a.buttons -= line;
    }
    let width = a.size[0];
    let y = ((size[1] - height) / 2).min(size[1] - height);
    let rect = Rect::new((size[0] - width) / 2, y, width, height);
    let title_x = match (title, icon) {
        (Some(title), Some(_)) => {
            top - PAD + icon_width + GAP + (a.text_width - band[0] - title) / 2
        }
        (title, _) => (before - title.unwrap_or(0)) / 2,
    };
    let beside = a.text.beside.into_iter().enumerate().map(|(n, text)| {
        let y = text_top + line * n as i32;
        (text, [rect.x + top + band[0], rect.y + y])
    });
    let below = a.text.below.into_iter().enumerate().map(|(n, text)| {
        let y = text_top + band[1] + line * n as i32;
        (text, [rect.x + top, rect.y + y])
    });
    let lines = beside.chain(below).collect();
    let gap = (a.text_width - buttons * BUTTON_SIZE[0]) / (buttons + 1);
    let buttons = (0..buttons)
        .map(|n| {
            Rect::new(
                rect.x + top + gap * (n + 1) + BUTTON_SIZE[0] * n,
                rect.y + a.buttons,
                BUTTON_SIZE[0],
                BUTTON_SIZE[1],
            )
        })
        .collect();
    Some(Layout {
        caller: message.caller,
        rect,
        icon: icon.map(|_| [rect.x + top, rect.y + top_y]),
        entry: message.entry.as_ref().map(|_| {
            Rect::new(
                rect.x + top,
                rect.y + a.buttons - GAP - ENTRY_HEIGHT - GAP + PAD,
                a.text_width,
                ENTRY_HEIGHT,
            )
        }),
        secondary_entry: None,
        title: [rect.x + title_x, rect.y + top_y],
        lines,
        buttons,
    })
}

/// Lays out the edit-mode Save Card (`cWinEditLotInfo`, `0x50d8c5..0x50de10`,
/// `PopupInfoTiles.bmp` 3008 + `SAVE_CARD` 4994 + Lot Name & multiline Lot
/// Description `cTSWinTextEdit2` fields + centered 3-button confirmation prompt
/// at `0x50dfb2..0x50e056`).
fn layout_save_card(size: [i32; 2], message: &Message, font: &Font) -> Layout {
    let [card_w, card_h] = message.icon.as_ref().map_or([280, 180], |img| {
        [
            i32::try_from(img.width).unwrap_or(280).clamp(120, 1024),
            i32::try_from(img.height).unwrap_or(180).clamp(100, 1024),
        ]
    });
    let line_h = font.line_height.clamp(1, 1024);
    let has_prompt = !message.text.is_empty();
    let prompt_w = if has_prompt {
        font.width(&message.text).clamp(0, 1 << 16)
    } else {
        0
    };
    // `0x50d8d9..0x50d9e5`: expand card rect by 16 px on each side, then add
    // button height + 16 px below (3-button prompt paints above the buttons
    // within this frame at `0x50dfef..0x50e056`).
    let width = (card_w + 32)
        .max(3 * BUTTON_SIZE[0] + 20)
        .max(prompt_w + 32);
    let height = card_h + 32 + BUTTON_SIZE[1] + 16;
    let rect = Rect::new(
        ((size[0] - width) / 2).max(0),
        ((size[1] - height) / 2).max(0),
        width,
        height,
    );
    let card_x = rect.x + (width - card_w) / 2;
    let card_y = rect.y + 16;
    // `0x50da8f..0x50dac3`: Lot Name edit box (`left = card_x + 8`,
    // `right = card_x + card_w`, width `card_w - 8`, `y = 25..45`).
    let field_x = card_x + 8;
    let field_w = (card_w - 8).max(40);
    let name_rect = Rect::new(field_x, rect.y + 25, field_w, 20);
    // `0x50dbb2..0x50dc16`: multiline Lot Description edit box below Lot Name
    // (`top = card_y + 38`, `bottom = card_y + card_h - 30`, height `card_h - 68`).
    let desc_top = card_y + 38;
    let desc_h = (card_h - 68).max(36);
    let desc_rect = Rect::new(field_x, desc_top, field_w, desc_h);
    let count = message.buttons.len().min(3);
    let buttons = match count {
        0 => Vec::new(),
        1 => vec![Rect::new(
            rect.x + (width - BUTTON_SIZE[0]) / 2,
            rect.y + height - 16 - BUTTON_SIZE[1],
            BUTTON_SIZE[0],
            BUTTON_SIZE[1],
        )],
        2 => {
            let y = rect.y + height - 5 - BUTTON_SIZE[1];
            vec![
                Rect::new(rect.x + 5, y, BUTTON_SIZE[0], BUTTON_SIZE[1]),
                Rect::new(
                    rect.x + width - 5 - BUTTON_SIZE[0],
                    y,
                    BUTTON_SIZE[0],
                    BUTTON_SIZE[1],
                ),
            ]
        }
        _ => {
            // `0x50dd5d..0x50de0b`: Yes at left (x=5), No centered, Cancel at right.
            let y = rect.y + height - 5 - BUTTON_SIZE[1];
            vec![
                Rect::new(rect.x + 5, y, BUTTON_SIZE[0], BUTTON_SIZE[1]),
                Rect::new(
                    rect.x + (width - BUTTON_SIZE[0]) / 2,
                    y,
                    BUTTON_SIZE[0],
                    BUTTON_SIZE[1],
                ),
                Rect::new(
                    rect.x + width - 5 - BUTTON_SIZE[0],
                    y,
                    BUTTON_SIZE[0],
                    BUTTON_SIZE[1],
                ),
            ]
        }
    };
    // `0x50dfb2..0x50e056`: centered confirmation prompt 9 px above the buttons.
    let lines = if has_prompt {
        let btn_top = buttons
            .first()
            .map_or(rect.y + height - 16 - BUTTON_SIZE[1], |b| b.y);
        let px = rect.x + (width - prompt_w) / 2;
        let py = btn_top - line_h - 9;
        vec![(message.text.clone(), [px, py])]
    } else {
        Vec::new()
    };
    Layout {
        caller: message.caller,
        rect,
        icon: message.icon.as_ref().map(|_| [card_x, card_y]),
        title: [rect.x + 16, rect.y + 16],
        lines,
        buttons,
        entry: message.entry.as_ref().map(|_| name_rect),
        secondary_entry: message.secondary_entry.as_ref().map(|_| desc_rect),
    }
}

/// Lays out a native `cTSWinGenDlg` confirmation dialog (`0x5e542f..0x5e589b`,
/// `0x5e5d46..0x5e5eac`, `GenDlg.bmp` 22):
/// - upper-left title at `(16, 16)` in `BODY_FONT` (`variablesans_12`),
/// - horizontally centered body starting at `16 + title_h + 8`,
/// - width `>= 200`, `>= (16 + btn_w) * count + 48` (`396` for 3×100 buttons),
/// - height `btn_h + title_h + body_h + 56` (`135` for 1-line title + 1-line body).
fn layout_generic(size: [i32; 2], message: &Message, font: &Font) -> Layout {
    let line = font.line_height.clamp(1, 1024);
    let title_w = if message.title.is_empty() {
        0
    } else {
        font.width(&message.title).clamp(0, 1 << 16)
    };
    let title_h = if message.title.is_empty() { 0 } else { line };
    let mut wrap_w = (200 - 32).max(title_w);
    let chars: Vec<char> = message.text.chars().collect();
    let (mut body_lines, _) = split(font, &chars, wrap_w, usize::MAX);
    // `0x5e5586..0x5e55bb`: if body wrapped past 1 line, grow width by 3/2.
    if body_lines.len() > 1 {
        wrap_w = (wrap_w * 3) / 2;
        body_lines = split(font, &chars, wrap_w, usize::MAX).0;
    }
    // `0x5e55dd..0x5e560c`: if still > 1 line, grow width by 4/3.
    if body_lines.len() > 1 {
        wrap_w = (wrap_w * 4) / 3;
        body_lines = split(font, &chars, wrap_w, usize::MAX).0;
    }
    let body_w = body_lines.iter().map(|l| font.width(l)).max().unwrap_or(0);
    let count = message.buttons.len().min(3);
    let count_i = i32::try_from(count).unwrap_or(0);
    let btn_w = message
        .buttons
        .iter()
        .take(count)
        .map(|label| font.width(label) + 24)
        .fold(BUTTON_SIZE[0], i32::max);
    let btn_row_w = if count_i > 0 {
        (16 + btn_w) * count_i + 32 + 16
    } else {
        200
    };
    let mut width = 200.max(btn_row_w).max(title_w + 32).max(body_w + 32);
    if size[0] > 0 && width > size[0] {
        width = size[0].max(count_i * BUTTON_SIZE[0]);
        let inner = (width - 32).max(40);
        body_lines = split(font, &chars, inner, usize::MAX).0;
    }
    let body_h = line * i32::try_from(body_lines.len()).unwrap_or(0);
    // `0x5e579d`: height = btn_h + title_h + body_h + 0x38 (56).
    let height = BUTTON_SIZE[1] + title_h + body_h + 56;
    let rect = Rect::new(
        (size[0] - width) / 2,
        ((size[1] - height) / 2).min(size[1] - height),
        width,
        height,
    );
    let body_y0 = rect.y + if title_h > 0 { 16 + title_h + 8 } else { 16 };
    let inner_w = (width - 32).max(0);
    let lines = body_lines
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let lw = font.width(&text);
            let lx = rect.x + 16 + (inner_w - lw) / 2;
            let ly = body_y0 + line * i32::try_from(i).unwrap_or(0);
            (text, [lx, ly])
        })
        .collect();
    let gap = if count_i > 0 {
        ((width - 32 - count_i * btn_w) / (count_i + 1)).max(16)
    } else {
        16
    };
    let btn_y = rect.y + height - 16 - BUTTON_SIZE[1];
    let buttons = (0..count_i)
        .map(|i| {
            Rect::new(
                rect.x + 16 + gap * (i + 1) + btn_w * i,
                btn_y,
                btn_w,
                BUTTON_SIZE[1],
            )
        })
        .collect();
    Layout {
        caller: message.caller,
        rect,
        icon: None,
        title: [rect.x + 16, rect.y + 16],
        lines,
        buttons,
        entry: None,
        secondary_entry: None,
    }
}

/// Fallback frame colors used only when [`BACKGROUND`] (`PopupInfoTiles.bmp`,
/// 3008) is absent from `Art` (such as synthetic unit tests).
const FILL: [u8; 3] = rgb565(0x000a);
const TOP: [u16; 7] = [0x6b75, 0x7395, 0x7bf6, 0x73b5, 0x39d0, 0x108c, 0x002a];
const LEFT: [u16; 7] = [0x086a, 0x084a, 0x086a, 0x108b, 0x18cc, 0x108b, 0x002a];
const RIGHT: [u16; 7] = [0x0005, 0x0005, 0x0005, 0x0006, 0x0006, 0x0008, 0x0009];
const BOTTOM: [u16; 7] = [0x0004, 0x0004, 0x0004, 0x0005, 0x0005, 0x0008, 0x0009];

/// Fallback frame color at `x`, `y` inside a `width` by `height` frame.
fn frame_color(x: i32, y: i32, width: i32, height: i32) -> [u8; 3] {
    let edges = [
        (y, &TOP),
        (x, &LEFT),
        (width - 1 - x, &RIGHT),
        (height - 1 - y, &BOTTOM),
    ];
    edges
        .iter()
        .filter(|(d, _)| *d < 7)
        .min_by_key(|(d, _)| *d)
        .map_or(FILL, |(d, edge)| rgb565(edge[*d as usize]))
}

/// Interactive visual and text-editing state of an open modal dialog
/// (`cTSWinMsgBox` + `cTSWinTextEdit2`, `0x5f2000..0x5f2388`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogState {
    /// Whether the single-line `cTSWinTextEdit2` field currently has keyboard
    /// focus (controls the 1 px border and blinking caret at `0x5f21d5..0x5f2366`).
    pub focused: bool,
    /// Caret position in characters (`0..=text.chars().count()`), or `None`
    /// to place the caret at the end of `message.entry`.
    pub caret: Option<usize>,
    /// Preferred visual wrapped line for the caret in multiline fields when
    /// `caret` sits on a soft-wrap boundary shared by two rows
    /// (`0x5f2475..0x5f2522`, `0x5f29f2..0x5f2c13`).
    pub caret_line: Option<usize>,
    /// Active text selection range `(start, end)` in characters (`start < end`),
    /// matching `cTSWinTextEdit2::SetText` initial all-selection for nonempty
    /// text (`0x523879..0x523887`, `0x5f462f..0x5f4685`, drawn at `0x5f2107..0x5f2125`).
    pub selection: Option<(usize, usize)>,
    /// Selection anchor character index while a mouse-drag selection is
    /// captured in `active_field` (`[esi + 0x157]` at `0x5f27a6..0x5f2887`).
    pub dragging_entry: Option<usize>,
    /// Pointer coordinate where the active `dragging_entry` press landed, so a
    /// same-position `CursorMoved` emitted by macOS/winit before mouse-up does
    /// not re-hit-test against a scroll offset shifted by the click.
    drag_origin: Option<[i32; 2]>,
    /// Whether the blinking caret is currently on its visible half-period
    /// (`[esi + 0x15c]`, toggled every [`CARET_BLINK_US`] = 500 ms at `0x5f203e`).
    pub caret_visible: bool,
    /// Timestamp (`clock_us`) when `caret_visible` last toggled.
    blink_since: Option<u64>,
    /// Which text field is currently active: `0` for `message.entry` (default,
    /// or Lot Name on a Save Card) and `1` for `message.secondary_entry` (Lot
    /// Description on an edit-mode Save Card, `0x50db31`).
    pub active_field: u8,
    /// Index of the button currently held down (draws `WinBtn.bmp` cell 1
    /// with cyan text `[0, 255, 255]` per `0x4ff509..0x4ff538`).
    pub pressed_button: Option<usize>,
    /// Bitmask of disabled buttons (bit `i` disables button `i`, drawing
    /// `WinBtn.bmp` cell 3 with dim text `[0x40, 0x5d, 0x5f]`).
    pub disabled_buttons: u8,
}

impl Default for DialogState {
    fn default() -> Self {
        Self {
            focused: true,
            caret: None,
            caret_line: None,
            selection: None,
            dragging_entry: None,
            drag_origin: None,
            caret_visible: true,
            blink_since: None,
            active_field: 0,
            pressed_button: None,
            disabled_buttons: 0,
        }
    }
}

impl DialogState {
    /// Creates initial interactive state for `message`: for a nonempty text
    /// entry (`0x523879..0x523887` -> `0x5f462f..0x5f4685`), selects the entire
    /// prefilled text `(0, len)` so the first character typed replaces it.
    pub fn for_message(message: &Message) -> Self {
        let mut state = Self::default();
        if let Some(text) = &message.entry {
            let len = text.chars().count();
            state.caret = Some(len);
            if len > 0 {
                state.selection = Some((0, len));
            }
        }
        state
    }

    /// Returns the normalized active selection `(start, end)` clamped to `len`,
    /// or `None` when empty or unset.
    pub fn selection_in(&self, len: usize) -> Option<(usize, usize)> {
        let (a, b) = self.selection?;
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let start = start.min(len);
        let end = end.min(len);
        (start < end).then_some((start, end))
    }

    /// Whether button `index` is disabled.
    pub fn is_disabled(&self, index: usize) -> bool {
        index < 8 && (self.disabled_buttons & (1 << index)) != 0
    }

    /// Marks button `index` enabled or disabled.
    pub fn set_disabled(&mut self, index: usize, disabled: bool) {
        if index < 8 {
            if disabled {
                self.disabled_buttons |= 1 << index;
            } else {
                self.disabled_buttons &= !(1 << index);
            }
        }
    }

    /// Advances the 500 ms `cTSWinTextEdit2` caret blink timer (`0x5f203e`),
    /// returning whether `caret_visible` flipped.
    pub fn sync_blink(&mut self, clock_us: u64) -> bool {
        if !self.focused {
            let was = self.caret_visible;
            self.caret_visible = false;
            self.blink_since = None;
            return was;
        }
        let Some(since) = self.blink_since else {
            self.blink_since = Some(clock_us);
            return false;
        };
        if clock_us.saturating_sub(since) >= CARET_BLINK_US {
            self.caret_visible = !self.caret_visible;
            self.blink_since = Some(clock_us);
            return true;
        }
        false
    }

    /// Resets the caret to visible on user typing or cursor movement.
    fn reset_blink(&mut self) {
        self.caret_visible = true;
        self.blink_since = None;
    }

    /// Computes wrapped visual `(char_start, char_end)` line ranges for a
    /// multiline entry field using `layout` + `art` when available, or explicit
    /// `'\n'` line boundaries as fallback.
    fn multiline_lines(
        &self,
        text: &str,
        layout: Option<&Layout>,
        art: Option<&Art>,
    ) -> Vec<(usize, usize)> {
        if let (Some(field), Some(font)) = (
            layout.and_then(|l| l.secondary_entry),
            art.and_then(|a| a.font(BODY_FONT)),
        ) {
            let wrap_w = (field.width - 8).max(16);
            return wrap_multiline_ranges(font, text, wrap_w);
        }
        let chars: Vec<char> = text.chars().collect();
        let mut ranges = Vec::new();
        let mut start = 0;
        while start <= chars.len() {
            let mut nl = start;
            while nl < chars.len() && chars[nl] != '\n' {
                nl += 1;
            }
            ranges.push((start, nl));
            if nl == chars.len() {
                break;
            }
            start = nl + 1;
        }
        if ranges.is_empty() {
            ranges.push((0, 0));
        }
        ranges
    }

    /// Current caret index clamped to `text`'s character count.
    pub fn caret_in(&self, text: &str) -> usize {
        let len = text.chars().count();
        self.caret.map_or(len, |c| c.min(len))
    }

    /// Returns the active editable text field on `message` and its character limit.
    fn active_text<'a>(&self, message: &'a Message) -> Option<(&'a str, usize)> {
        if self.active_field == 1
            && let Some(desc) = &message.secondary_entry
        {
            return Some((desc.as_str(), SAVE_CARD_DESC_MOST));
        }
        let max = if message.save_card {
            SAVE_CARD_NAME_MOST
        } else {
            ENTRY_MOST
        };
        message.entry.as_deref().map(|s| (s, max))
    }

    /// Returns mutable access to the active editable text field and its limit.
    fn active_text_mut<'a>(&self, message: &'a mut Message) -> Option<(&'a mut String, usize)> {
        if self.active_field == 1 && message.secondary_entry.is_some() {
            return message
                .secondary_entry
                .as_mut()
                .map(|s| (s, SAVE_CARD_DESC_MOST));
        }
        let max = if message.save_card {
            SAVE_CARD_NAME_MOST
        } else {
            ENTRY_MOST
        };
        message.entry.as_mut().map(|s| (s, max))
    }

    /// Switches keyboard focus between `entry` (Lot Name) and `secondary_entry`
    /// (Lot Description) on a multi-field Save Card (`Tab`).
    pub fn switch_field(&mut self, message: &Message) -> bool {
        if message.entry.is_none() || message.secondary_entry.is_none() {
            return false;
        }
        self.active_field = if self.active_field == 0 { 1 } else { 0 };
        self.focused = true;
        self.caret_line = None;
        self.dragging_entry = None;
        if let Some((text, _)) = self.active_text(message) {
            let len = text.chars().count();
            self.caret = Some(len);
            self.selection = (len > 0).then_some((0, len));
        }
        self.reset_blink();
        true
    }

    /// Whether the currently focused field is the multiline Lot Description
    /// box on an edit-mode Save Card (`0x50db98..0x50dba0`, `0x5f28b6..0x5f28f9`).
    pub fn is_multiline(&self, message: &Message) -> bool {
        self.active_field == 1 && message.save_card && message.secondary_entry.is_some()
    }

    /// Inserts `ch` at the caret position (replacing any active selection
    /// first, `0x5f462f..0x5f4685`) if an editable entry is present, within
    /// its character limit, and not a disallowed control character (`'\n'` is
    /// accepted in multiline Lot Description fields per `0x5f28b6..0x5f28f9`).
    pub fn insert(&mut self, message: &mut Message, ch: char) -> bool {
        let multiline = self.is_multiline(message);
        if !self.focused || (ch.is_control() && !(ch == '\n' && multiline)) {
            return false;
        }
        let Some((text, limit)) = self.active_text_mut(message) else {
            return false;
        };
        self.dragging_entry = None;
        let len = text.chars().count();
        if let Some((start, end)) = self.selection_in(len) {
            let b0 = char_byte_index(text, start);
            let b1 = char_byte_index(text, end);
            text.replace_range(b0..b1, "");
            text.insert(b0, ch);
            self.selection = None;
            self.caret = Some(start + 1);
            self.caret_line = None;
            self.reset_blink();
            return true;
        }
        if len >= limit {
            return false;
        }
        let pos = self.caret.map_or(len, |c| c.min(len));
        let byte_idx = char_byte_index(text, pos);
        text.insert(byte_idx, ch);
        self.caret = Some(pos + 1);
        self.caret_line = None;
        self.reset_blink();
        true
    }

    /// Deletes the active selection or the character before the caret (Backspace).
    pub fn backspace(&mut self, message: &mut Message) -> bool {
        if !self.focused {
            return false;
        }
        let Some((text, _)) = self.active_text_mut(message) else {
            return false;
        };
        self.dragging_entry = None;
        let len = text.chars().count();
        if let Some((start, end)) = self.selection_in(len) {
            let b0 = char_byte_index(text, start);
            let b1 = char_byte_index(text, end);
            text.replace_range(b0..b1, "");
            self.selection = None;
            self.caret = Some(start);
            self.caret_line = None;
            self.reset_blink();
            return true;
        }
        let pos = self.caret.map_or(len, |c| c.min(len));
        if pos == 0 {
            return false;
        }
        let start = char_byte_index(text, pos - 1);
        let end = char_byte_index(text, pos);
        text.replace_range(start..end, "");
        self.caret = Some(pos - 1);
        self.caret_line = None;
        self.reset_blink();
        true
    }

    /// Deletes the active selection or the character at the caret (Delete).
    pub fn delete(&mut self, message: &mut Message) -> bool {
        if !self.focused {
            return false;
        }
        let Some((text, _)) = self.active_text_mut(message) else {
            return false;
        };
        self.dragging_entry = None;
        let len = text.chars().count();
        if let Some((start, end)) = self.selection_in(len) {
            let b0 = char_byte_index(text, start);
            let b1 = char_byte_index(text, end);
            text.replace_range(b0..b1, "");
            self.selection = None;
            self.caret = Some(start);
            self.caret_line = None;
            self.reset_blink();
            return true;
        }
        let pos = self.caret.map_or(len, |c| c.min(len));
        if pos >= len {
            return false;
        }
        let start = char_byte_index(text, pos);
        let end = char_byte_index(text, pos + 1);
        text.replace_range(start..end, "");
        self.caret = Some(pos);
        self.caret_line = None;
        self.reset_blink();
        true
    }

    /// Moves the caret one character left, clearing any active selection.
    pub fn move_left(&mut self, message: &Message) -> bool {
        let Some((text, _)) = self.active_text(message) else {
            return false;
        };
        self.dragging_entry = None;
        let len = text.chars().count();
        if let Some((start, _)) = self.selection_in(len) {
            self.selection = None;
            self.caret = Some(start);
            self.caret_line = None;
            self.reset_blink();
            return true;
        }
        let pos = self.caret_in(text);
        let next = pos.saturating_sub(1);
        self.caret = Some(next);
        self.caret_line = None;
        self.reset_blink();
        next != pos
    }

    /// Moves the caret one character right, clearing any active selection.
    pub fn move_right(&mut self, message: &Message) -> bool {
        let Some((text, _)) = self.active_text(message) else {
            return false;
        };
        self.dragging_entry = None;
        let len = text.chars().count();
        if let Some((_, end)) = self.selection_in(len) {
            self.selection = None;
            self.caret = Some(end);
            self.caret_line = None;
            self.reset_blink();
            return true;
        }
        let pos = self.caret_in(text);
        let next = (pos + 1).min(len);
        self.caret = Some(next);
        self.caret_line = None;
        self.reset_blink();
        next != pos
    }

    /// Moves the caret one visual row up in a multiline field (`0x5f29f2..0x5f2a90`),
    /// preserving the caret's horizontal offset and clearing any selection.
    pub fn move_up(&mut self, message: &Message, layout: Option<&Layout>, art: &Art) -> bool {
        self.move_vertical(message, layout, art, -1)
    }

    /// Moves the caret one visual row down in a multiline field (`0x5f2a90..0x5f2b40`),
    /// preserving the caret's horizontal offset and clearing any selection.
    pub fn move_down(&mut self, message: &Message, layout: Option<&Layout>, art: &Art) -> bool {
        self.move_vertical(message, layout, art, 1)
    }

    fn move_vertical(
        &mut self,
        message: &Message,
        layout: Option<&Layout>,
        art: &Art,
        delta: isize,
    ) -> bool {
        if !self.is_multiline(message) {
            return false;
        }
        let Some((text, _)) = self.active_text(message) else {
            return false;
        };
        self.dragging_entry = None;
        let lines = self.multiline_lines(text, layout, Some(art));
        let cur = self.caret_in(text);
        let row = multiline_caret_line(&lines, cur, self.caret_line);
        let target_row = if delta < 0 {
            row.saturating_sub(1)
        } else {
            (row + 1).min(lines.len().saturating_sub(1))
        };
        let had_selection = self.selection.take().is_some();
        if target_row == row {
            self.caret_line = Some(row);
            self.reset_blink();
            return had_selection;
        }
        // `0x5f37f4..0x5f38cd`: carry the character column from the current
        // visual row to the destination row and clamp to its length.
        let (r0, _) = lines[row];
        let (t0, t1) = lines[target_row];
        let target_pos = (t0 + cur.saturating_sub(r0)).min(t1);
        let changed =
            self.caret != Some(target_pos) || self.caret_line != Some(target_row) || had_selection;
        self.caret = Some(target_pos);
        self.caret_line = Some(target_row);
        self.reset_blink();
        changed
    }

    /// Moves the caret to the start of the active line (`0x5f2c13`: start of
    /// current visual row in multiline fields, start of text in single-line
    /// fields), always clearing any active selection.
    pub fn home_in_layout(
        &mut self,
        message: &Message,
        layout: Option<&Layout>,
        art: Option<&Art>,
    ) -> bool {
        let Some((text, _)) = self.active_text(message) else {
            return false;
        };
        self.dragging_entry = None;
        let had_selection = self.selection.take().is_some();
        if self.is_multiline(message) {
            let lines = self.multiline_lines(text, layout, art);
            let cur = self.caret_in(text);
            let row = multiline_caret_line(&lines, cur, self.caret_line);
            let target = lines[row].0;
            let changed =
                self.caret != Some(target) || self.caret_line != Some(row) || had_selection;
            self.caret = Some(target);
            self.caret_line = Some(row);
            self.reset_blink();
            return changed;
        }
        let changed = self.caret != Some(0) || had_selection;
        self.caret = Some(0);
        self.caret_line = None;
        self.reset_blink();
        changed
    }

    /// Moves the caret to the start of the active entry field (or current line
    /// when multiline), always clearing any active selection (`0x5f2c13`).
    pub fn home(&mut self, message: &Message) -> bool {
        self.home_in_layout(message, None, None)
    }

    /// Moves the caret to the end of the active line (`0x5f2c13`: end of
    /// current visual row in multiline fields, end of text in single-line
    /// fields), always clearing any active selection.
    pub fn end_in_layout(
        &mut self,
        message: &Message,
        layout: Option<&Layout>,
        art: Option<&Art>,
    ) -> bool {
        let Some((text, _)) = self.active_text(message) else {
            return false;
        };
        self.dragging_entry = None;
        let had_selection = self.selection.take().is_some();
        if self.is_multiline(message) {
            let lines = self.multiline_lines(text, layout, art);
            let cur = self.caret_in(text);
            let row = multiline_caret_line(&lines, cur, self.caret_line);
            let target = lines[row].1;
            let changed =
                self.caret != Some(target) || self.caret_line != Some(row) || had_selection;
            self.caret = Some(target);
            self.caret_line = Some(row);
            self.reset_blink();
            return changed;
        }
        let len = text.chars().count();
        let changed = self.caret != Some(len) || had_selection;
        self.caret = Some(len);
        self.caret_line = None;
        self.reset_blink();
        changed
    }

    /// Moves the caret to the end of the active entry field (or current line
    /// when multiline), always clearing any active selection (`0x5f2c13`).
    pub fn end(&mut self, message: &Message) -> bool {
        self.end_in_layout(message, None, None)
    }

    /// Hit-tests `point` against `field` (`0` = primary entry, `1` = multiline
    /// Save Card description), returning `(char_index, preferred_line)`.
    #[allow(clippy::too_many_arguments)]
    fn hit_test_entry_pos(
        &self,
        field_idx: u8,
        field: Rect,
        text: &str,
        message: &Message,
        art: &Art,
        point: [i32; 2],
        switched: bool,
    ) -> (usize, Option<usize>) {
        let Some(font) = art.font(BODY_FONT) else {
            return (self.caret_in(text), None);
        };
        let inside_w = (field.width - 2).max(0);
        if field_idx == 1 && message.save_card {
            let wrap_w = (inside_w - 6).max(16);
            let lines = wrap_multiline_ranges(font, text, wrap_w);
            let line_h = font.line_height.max(1);
            let inside_h = (field.height - 2).max(line_h);
            let old_caret = if switched {
                text.chars().count()
            } else {
                self.caret_in(text)
            };
            let caret_line = multiline_caret_line(
                &lines,
                old_caret,
                if switched { None } else { self.caret_line },
            );
            let scroll_y = ((caret_line as i32 + 1) * line_h + 2 - inside_h).max(0);
            let rel_y = point[1] - (field.y + 1 + 2) + scroll_y;
            // `0x5f2825..0x5f2878`: dragging above the first row clamps to
            // index 0; dragging below the last row clamps to the end of text.
            if rel_y < 0 {
                return (0, Some(0));
            }
            let raw_row = (rel_y / line_h) as usize;
            if raw_row >= lines.len() {
                return (text.chars().count(), Some(lines.len().saturating_sub(1)));
            }
            let line_idx = raw_row;
            let (c0, c1) = lines.get(line_idx).copied().unwrap_or((0, 0));
            let rel_x = point[0] - (field.x + 1 + 3);
            let b0 = char_byte_index(text, c0);
            let mut best = c0;
            let mut best_dist = rel_x.abs();
            for c in (c0 + 1)..=c1 {
                let b = char_byte_index(text, c);
                let w = font.width(&text[b0..b]);
                let dist = (rel_x - w).abs();
                if dist < best_dist {
                    best_dist = dist;
                    best = c;
                }
            }
            (best, Some(line_idx))
        } else {
            let old_caret = if switched {
                text.chars().count()
            } else {
                self.caret_in(text)
            };
            let slide = entry_slide(font, text, old_caret, inside_w);
            let rel_x = point[0] - (field.x + 1 + 3 - slide);
            let mut best = 0;
            let mut best_dist = rel_x.abs();
            for (idx, _) in text.chars().enumerate() {
                let prefix_end = char_byte_index(text, idx + 1);
                let w = font.width(&text[..prefix_end]);
                let dist = (rel_x - w).abs();
                if dist < best_dist {
                    best_dist = dist;
                    best = idx + 1;
                }
            }
            (best, None)
        }
    }

    /// Handles a left press inside any entry field (`entry` or `secondary_entry`),
    /// focusing it, clearing any selection, placing the caret closest to `point`,
    /// and capturing a drag selection anchored at that caret (`0x5f27ae..0x5f2887`).
    pub fn click_entry(
        &mut self,
        layout: &Layout,
        message: &Message,
        art: &Art,
        point: [i32; 2],
    ) -> bool {
        let target = if let (Some(field), Some(text)) = (layout.entry, message.entry.as_deref())
            && field.contains(point[0], point[1])
        {
            Some((0u8, field, text))
        } else if let (Some(field), Some(text)) =
            (layout.secondary_entry, message.secondary_entry.as_deref())
            && field.contains(point[0], point[1])
        {
            Some((1u8, field, text))
        } else {
            None
        };
        let Some((field_idx, field, text)) = target else {
            return false;
        };
        let switched = self.active_field != field_idx;
        let (pos, line_idx) =
            self.hit_test_entry_pos(field_idx, field, text, message, art, point, switched);
        self.active_field = field_idx;
        self.focused = true;
        self.selection = None;
        self.caret = Some(pos);
        self.caret_line = line_idx;
        self.dragging_entry = Some(pos);
        self.drag_origin = Some(point);
        self.reset_blink();
        true
    }

    /// Updates a captured mouse-drag selection inside the active `cTSWinTextEdit2`
    /// field (`0x5f27ae..0x5f2887`), extending `selection` from the mouse-down
    /// anchor to the character closest to `point`.
    pub fn drag_entry(
        &mut self,
        layout: &Layout,
        message: &Message,
        art: &Art,
        point: [i32; 2],
    ) -> bool {
        let Some(anchor) = self.dragging_entry else {
            return false;
        };
        if self.drag_origin == Some(point) {
            return false;
        }
        self.drag_origin = None;
        let target = if self.active_field == 1 {
            layout
                .secondary_entry
                .zip(message.secondary_entry.as_deref())
        } else {
            layout.entry.zip(message.entry.as_deref())
        };
        let Some((field, text)) = target else {
            self.dragging_entry = None;
            return false;
        };
        let len = text.chars().count();
        let anchor = anchor.min(len);
        let (pos, line_idx) =
            self.hit_test_entry_pos(self.active_field, field, text, message, art, point, false);
        let new_sel = (anchor != pos).then_some((anchor.min(pos), anchor.max(pos)));
        let changed =
            self.caret != Some(pos) || self.caret_line != line_idx || self.selection != new_sel;
        self.caret = Some(pos);
        self.caret_line = line_idx;
        self.selection = new_sel;
        if changed {
            self.reset_blink();
        }
        changed
    }

    /// Releases an active `cTSWinTextEdit2` mouse-drag capture (`0x5f27a6`),
    /// preserving any resulting selection.
    pub fn release_entry(&mut self) -> bool {
        self.drag_origin = None;
        self.dragging_entry.take().is_some()
    }
}

fn char_byte_index(text: &str, char_pos: usize) -> usize {
    text.char_indices()
        .nth(char_pos)
        .map_or(text.len(), |(idx, _)| idx)
}

/// Draws a 3×3 tiled 9-slice bitmap (`PopupInfoTiles.bmp` 3008 or
/// `GenDlg.bmp` 22) across `rect`, matching `cTSWinMsgBox::OnDraw`
/// (`0x524603..0x524645`: tile width `bmp.width / 3`, tile height
/// `bmp.height / 3`, four fixed corners, tiled edges, tiled center).
pub fn draw_nine_slice(frame: &mut Frame, sheet: &Image, rect: Rect) {
    let tw = i32::try_from(sheet.width / 3).unwrap_or(0);
    let th = i32::try_from(sheet.height / 3).unwrap_or(0);
    if tw <= 0 || th <= 0 || rect.width <= 0 || rect.height <= 0 {
        return;
    }
    let frame_w = i64::from(frame.width());
    let frame_h = i64::from(frame.height());
    let left_w = tw.min(rect.width);
    let right_w = tw.min(rect.width - left_w);
    let top_h = th.min(rect.height);
    let bottom_h = th.min(rect.height - top_h);

    let x_mid_start = i64::from(rect.x) + i64::from(left_w);
    let x_mid_end = i64::from(rect.x) + i64::from(rect.width) - i64::from(right_w);
    let y_mid_start = i64::from(rect.y) + i64::from(top_h);
    let y_mid_end = i64::from(rect.y) + i64::from(rect.height) - i64::from(bottom_h);

    // 4 corners.
    frame.draw(sheet, Rect::new(0, 0, left_w, top_h), [rect.x, rect.y]);
    if right_w > 0 {
        frame.draw(
            sheet,
            Rect::new(tw * 2 + (tw - right_w), 0, right_w, top_h),
            [rect.x.saturating_add(rect.width - right_w), rect.y],
        );
    }
    if bottom_h > 0 {
        frame.draw(
            sheet,
            Rect::new(0, th * 2 + (th - bottom_h), left_w, bottom_h),
            [rect.x, rect.y.saturating_add(rect.height - bottom_h)],
        );
        if right_w > 0 {
            frame.draw(
                sheet,
                Rect::new(
                    tw * 2 + (tw - right_w),
                    th * 2 + (th - bottom_h),
                    right_w,
                    bottom_h,
                ),
                [
                    rect.x.saturating_add(rect.width - right_w),
                    rect.y.saturating_add(rect.height - bottom_h),
                ],
            );
        }
    }

    // Clamp tiled spans to the visible frame so huge/offscreen rects stay bounded.
    let vis_x0 = x_mid_start.max(0);
    let vis_x1 = x_mid_end.min(frame_w);
    let vis_y0 = y_mid_start.max(0);
    let vis_y1 = y_mid_end.min(frame_h);
    let tw64 = i64::from(tw);
    let th64 = i64::from(th);

    // Top and bottom edges.
    if vis_x0 < vis_x1 {
        let mut x = vis_x0;
        while x < vis_x1 {
            let phase = ((x - x_mid_start).rem_euclid(tw64)) as i32;
            let step = (tw - phase).min((vis_x1 - x) as i32);
            frame.draw(
                sheet,
                Rect::new(tw + phase, 0, step, top_h),
                [x as i32, rect.y],
            );
            if bottom_h > 0 {
                frame.draw(
                    sheet,
                    Rect::new(tw + phase, th * 2 + (th - bottom_h), step, bottom_h),
                    [x as i32, rect.y.saturating_add(rect.height - bottom_h)],
                );
            }
            x += i64::from(step);
        }
    }

    // Left and right edges.
    if vis_y0 < vis_y1 {
        let mut y = vis_y0;
        while y < vis_y1 {
            let phase = ((y - y_mid_start).rem_euclid(th64)) as i32;
            let step = (th - phase).min((vis_y1 - y) as i32);
            frame.draw(
                sheet,
                Rect::new(0, th + phase, left_w, step),
                [rect.x, y as i32],
            );
            if right_w > 0 {
                frame.draw(
                    sheet,
                    Rect::new(tw * 2 + (tw - right_w), th + phase, right_w, step),
                    [rect.x.saturating_add(rect.width - right_w), y as i32],
                );
            }
            y += i64::from(step);
        }
    }

    // Center interior tiles.
    if vis_x0 < vis_x1 && vis_y0 < vis_y1 {
        let mut y = vis_y0;
        while y < vis_y1 {
            let py = ((y - y_mid_start).rem_euclid(th64)) as i32;
            let sy = (th - py).min((vis_y1 - y) as i32);
            let mut x = vis_x0;
            while x < vis_x1 {
                let px = ((x - x_mid_start).rem_euclid(tw64)) as i32;
                let sx = (tw - px).min((vis_x1 - x) as i32);
                frame.draw(
                    sheet,
                    Rect::new(tw + px, th + py, sx, sy),
                    [x as i32, y as i32],
                );
                x += i64::from(sx);
            }
            y += i64::from(sy);
        }
    }
}

/// Draws the dialog. `pointer` is where the pointer is, for the buttons'
/// hover look.
pub fn draw(
    frame: &mut Frame,
    art: &Art,
    message: &Message,
    layout: &Layout,
    pointer: Option<[i32; 2]>,
) {
    draw_with_state(
        frame,
        art,
        message,
        layout,
        pointer,
        &DialogState::default(),
    );
}

/// Draws the dialog with full interactive chrome state (9-slice
/// [`BACKGROUND`] frame, `45×45` [`ICON_STUB`] portrait backing, 4-state
/// `WinBtn.bmp` buttons, and focused/blinking `cTSWinTextEdit2` input box).
pub fn draw_with_state(
    frame: &mut Frame,
    art: &Art,
    message: &Message,
    layout: &Layout,
    pointer: Option<[i32; 2]>,
    state: &DialogState,
) {
    let r = layout.rect;
    if let Some(sheet) = art.image(message.frame.resource_id()) {
        draw_nine_slice(frame, sheet, r);
    } else {
        // Fallback when `PopupInfoTiles.bmp` (3008) is not loaded.
        let visible = |start: i32, length: i32, size: u32| {
            let start64 = i64::from(start);
            let low = (-start64).max(0);
            let high = i64::from(length).min(i64::from(size) - start64);
            (low..high.max(low)).map(move |n| n as i32)
        };
        for y in visible(r.y, r.height, frame.height()) {
            for x in visible(r.x, r.width, frame.width()) {
                frame.blend(r.x + x, r.y + y, frame_color(x, y, r.width, r.height), 255);
            }
        }
    }
    if let (Some(icon), Some(at)) = (&message.icon, layout.icon) {
        // 0x524660..0x524782: a 45×45 portrait is centered over `SimStub.BMP` (4992).
        if icon.width == 45
            && icon.height == 45
            && let Some(stub) = art.image(ICON_STUB)
        {
            frame.draw_all(stub, at);
            let dx = (i32::try_from(stub.width).unwrap_or(45) - 45) / 2;
            let dy = (i32::try_from(stub.height).unwrap_or(45) - 45) / 2;
            frame.draw_all(icon, [at[0].saturating_add(dx), at[1].saturating_add(dy)]);
        } else {
            frame.draw_all(icon, at);
        }
    }
    let title_font_size = match message.frame {
        FrameStyle::Generic => BODY_FONT,
        FrameStyle::Popup => TITLE_FONT,
    };
    if !message.title.is_empty()
        && let Some(font) = art.font(title_font_size)
    {
        font.draw(frame, &message.title, layout.title, LABEL);
    }
    if let Some(font) = art.font(BODY_FONT) {
        for (text, at) in &layout.lines {
            font.draw(frame, text, *at, LABEL);
        }
    }
    if let Some(font) = art.font(BODY_FONT) {
        let unfocused = DialogState {
            focused: false,
            caret_visible: false,
            selection: None,
            ..DialogState::default()
        };
        if let (Some(field), Some(text)) = (layout.entry, &message.entry) {
            let field_state = if state.active_field == 0 {
                state
            } else {
                &unfocused
            };
            draw_entry(frame, font, field, text, field_state);
        }
        if let (Some(field), Some(text)) = (layout.secondary_entry, &message.secondary_entry) {
            let field_state = if state.active_field == 1 {
                state
            } else {
                &unfocused
            };
            if message.save_card {
                draw_multiline_entry(frame, font, field, text, field_state);
            } else {
                draw_entry(frame, font, field, text, field_state);
            }
        }
    }
    for (idx, (rect, label)) in layout.buttons.iter().zip(&message.buttons).enumerate() {
        let disabled = state.is_disabled(idx);
        let hover = !disabled && pointer.is_some_and(|[x, y]| rect.contains(x, y));
        // 0x5e44b4..0x5e45fc: a held button stays visually pressed only while
        // the pointer remains inside it (clearing when the pointer leaves the
        // window or drags outside the button).
        let pressed = !disabled
            && state.pressed_button == Some(idx)
            && pointer.is_some_and(|[x, y]| rect.contains(x, y));
        // 0x5e3b40..0x5e3c3e & 0x4ff509..0x4ff538:
        // cell 0 normal (LABEL), 1 pressed/selected (cyan), 2 hover (white), 3 disabled (dim).
        let (cell_idx, rgb) = if disabled {
            (3, BUTTON_DISABLED)
        } else if pressed {
            (1, BUTTON_SELECTED)
        } else if hover {
            (2, BUTTON_HOVER)
        } else {
            (0, LABEL)
        };
        if let Some(cell) = art.cell(BUTTON, 4, cell_idx) {
            let w = i32::try_from(cell.width).unwrap_or(0);
            let h = i32::try_from(cell.height).unwrap_or(0);
            // Measured: each half of the cell goes to its end of the
            // button as it is, and the column in the middle fills the gap.
            let end = (w / 2).min(rect.width / 2);
            let right = rect.x.saturating_add(rect.width - end);
            frame.draw(&cell, Rect::new(0, 0, end, h), [rect.x, rect.y]);
            // Only the middle columns inside the frame are visited.
            let first = rect.x.saturating_add(end).max(0);
            let last = right.min(i32::try_from(frame.width()).unwrap_or(i32::MAX));
            for x in first..last {
                frame.draw(&cell, Rect::new(w / 2, 0, 1, h), [x, rect.y]);
            }
            frame.draw(&cell, Rect::new(w - end, 0, end, h), [right, rect.y]);
        }
        if let Some(font) = art.font(BODY_FONT) {
            // Measured: "OK" starts 37 pixels into the 100-pixel button.
            let offset = (i64::from(rect.width) - 1 - i64::from(font.width(label))).div_euclid(2);
            let x = (i64::from(rect.x) + offset).clamp(i64::from(i32::MIN), i64::from(i32::MAX));
            font.draw(frame, label, [x as i32, rect.y.saturating_add(5)], rgb);
        }
    }
}

/// Horizontal scroll offset keeping the caret visible inside `inside_w`.
fn entry_slide(font: &Font, text: &str, caret_chars: usize, inside_w: i32) -> i32 {
    let prefix_end = char_byte_index(text, caret_chars);
    let caret = 3i32
        .saturating_add(font.width(&text[..prefix_end]))
        .saturating_add(1);
    caret.saturating_sub(inside_w - 3).max(0)
}

/// Draws the `cTSWinTextEdit2` single-line entry field (`0x523820..0x5238f3`,
/// `0x5f2000..0x5f2388`):
/// - fills the field with `[0x6f2aa4]` (`ENTRY_FILL` = `rgb565(0x000a)`,
///   `0x5f20b6..0x5f20c6`);
/// - draws the text in `LABEL` (`0x5f20df..0x5f21a0`), clipped to the field
///   and scrolled to keep the caret in view;
/// - draws the 1 px vertical caret in `LABEL` (`0x5f22e2..0x5f22f5`) while
///   focused and `state.caret_visible`;
/// - draws the 1 px focus border in `LABEL` (`0x5f2352..0x5f2363`) while
///   focused.
fn draw_entry(frame: &mut Frame, font: &Font, field: Rect, text: &str, state: &DialogState) {
    // 0x5f20b6..0x5f20c6: solid background fill (`[esi + 0x148] = [0x6f2aa4]`).
    frame.fill(field, ENTRY_FILL);

    let wide = i32::try_from(frame.width()).unwrap_or(i32::MAX);
    let high = i32::try_from(frame.height()).unwrap_or(i32::MAX);
    if state.focused {
        // 0x5f2352..0x5f2363: 1 px focus border (`[esi + 0x150] = font->GetColor()`).
        let [right, bottom] = [
            field.x.saturating_add(field.width - 1),
            field.y.saturating_add(field.height - 1),
        ];
        let mut dot = |x: i32, y: i32| frame.blend(x, y, LABEL, 255);
        for x in field.x.max(0)..=right.min(wide) {
            dot(x, field.y);
            dot(x, bottom);
        }
        for y in field.y.max(0)..=bottom.min(high) {
            dot(field.x, y);
            dot(right, y);
        }
    }

    let inside = Rect::new(
        field.x.saturating_add(1),
        field.y.saturating_add(1),
        (field.width - 2).max(0),
        (field.height - 2).max(0),
    );
    let left = inside.x.max(0);
    let top = inside.y.max(0);
    let end_x = inside.x.saturating_add(inside.width).min(wide);
    let end_y = inside.y.saturating_add(inside.height).min(high);
    let (Ok(w), Ok(h)) = (u32::try_from(end_x - left), u32::try_from(end_y - top)) else {
        return;
    };
    if w == 0 || h == 0 {
        return;
    }
    let mut under = Frame::new(w, h, ENTRY_FILL);
    let line = font.line_height.clamp(0, inside.height);
    let caret_chars = state.caret_in(text);
    let prefix_end = char_byte_index(text, caret_chars);
    let caret = 3i32
        .saturating_add(font.width(&text[..prefix_end]))
        .saturating_add(1);
    let slide = entry_slide(font, text, caret_chars, inside.width);
    let [ox, oy] = [inside.x.saturating_sub(left), inside.y.saturating_sub(top)];
    let at = [
        ox.saturating_add(3 - slide),
        oy.saturating_add((inside.height - line) / 2),
    ];
    let len = text.chars().count();
    if let Some((sel_start, sel_end)) = state.selection_in(len) {
        // 0x5f2107..0x5f21a0: draw prefix, then highlighted selection range
        // with LABEL background and ENTRY_FILL text, then suffix.
        let b0 = char_byte_index(text, sel_start);
        let b1 = char_byte_index(text, sel_end);
        let pre_w = font.width(&text[..b0]);
        let sel_w = font.width(&text[b0..b1]);
        font.draw(&mut under, &text[..b0], at, LABEL);
        under.fill(
            Rect::new(at[0].saturating_add(pre_w), at[1], sel_w.max(1), line),
            LABEL,
        );
        font.draw(
            &mut under,
            &text[b0..b1],
            [at[0].saturating_add(pre_w), at[1]],
            ENTRY_FILL,
        );
        font.draw(
            &mut under,
            &text[b1..],
            [at[0].saturating_add(pre_w).saturating_add(sel_w), at[1]],
            LABEL,
        );
    } else {
        font.draw(&mut under, text, at, LABEL);
    }
    if state.focused && state.caret_visible {
        let caret_x = ox.saturating_add(caret - slide);
        for y in at[1]..at[1].saturating_add(line) {
            under.blend(caret_x, y, LABEL, 255);
        }
    }
    for y in 0..end_y - top {
        for x in 0..end_x - left {
            if let Some(rgb) = under.pixel(x, y) {
                frame.blend(left + x, top + y, rgb, 255);
            }
        }
    }
}

/// Splits multiline `text` into wrapped `(char_start, char_end)` line ranges
/// (excluding explicit `'\n'` delimiters) that fit within `wrap_w` pixels.
fn wrap_multiline_ranges(font: &Font, text: &str, wrap_w: i32) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut ranges = Vec::new();
    let mut start = 0;
    let max_w = wrap_w.max(1);
    while start <= chars.len() {
        let mut nl = start;
        while nl < chars.len() && chars[nl] != '\n' {
            nl += 1;
        }
        if start == nl {
            ranges.push((start, start));
        } else {
            let mut line_start = start;
            while line_start < nl {
                let mut line_end = line_start;
                let mut last_space = None;
                let mut width = 0i32;
                while line_end < nl {
                    let ch = chars[line_end];
                    let cw = font.width(&ch.to_string());
                    if line_end > line_start && width.saturating_add(cw) > max_w {
                        break;
                    }
                    width = width.saturating_add(cw);
                    line_end += 1;
                    if ch == ' ' {
                        last_space = Some(line_end);
                    }
                }
                if line_end < nl
                    && let Some(sp) = last_space
                    && sp > line_start
                {
                    line_end = sp;
                }
                ranges.push((line_start, line_end));
                line_start = line_end;
            }
        }
        if nl == chars.len() {
            break;
        }
        start = nl + 1;
    }
    if ranges.is_empty() {
        ranges.push((0, 0));
    }
    ranges
}

/// Finds the wrapped visual line index owning character offset `caret`,
/// honoring `preferred` at soft-wrap boundaries shared by two adjacent rows
/// (`0x5f2475..0x5f2522`, `0x5f29f2..0x5f2c13`).
fn multiline_caret_line(lines: &[(usize, usize)], caret: usize, preferred: Option<usize>) -> usize {
    if let Some(pref) = preferred
        && let Some(&(c0, c1)) = lines.get(pref)
        && caret >= c0
        && caret <= c1
    {
        return pref;
    }
    for (idx, &(c0, c1)) in lines.iter().enumerate() {
        if caret >= c0 && caret <= c1 {
            if caret == c1 && lines.get(idx + 1).is_some_and(|&(nc0, _)| nc0 == c1) {
                return idx + 1;
            }
            return idx;
        }
    }
    lines.len().saturating_sub(1)
}

/// Draws a multiline `cTSWinTextEdit2` field (Lot Description on the edit-mode
/// Save Card, `0x50db98..0x50dc50`, `0x5f28b6..0x5f28f9`), wrapping lines
/// vertically inside `field` with vertical scrolling, selection highlighting,
/// and a blinking 1 px caret.
fn draw_multiline_entry(
    frame: &mut Frame,
    font: &Font,
    field: Rect,
    text: &str,
    state: &DialogState,
) {
    frame.fill(field, ENTRY_FILL);
    let wide = i32::try_from(frame.width()).unwrap_or(i32::MAX);
    let high = i32::try_from(frame.height()).unwrap_or(i32::MAX);
    if state.focused {
        let [right, bottom] = [
            field.x.saturating_add(field.width - 1),
            field.y.saturating_add(field.height - 1),
        ];
        let mut dot = |x: i32, y: i32| frame.blend(x, y, LABEL, 255);
        for x in field.x.max(0)..=right.min(wide) {
            dot(x, field.y);
            dot(x, bottom);
        }
        for y in field.y.max(0)..=bottom.min(high) {
            dot(field.x, y);
            dot(right, y);
        }
    }
    let inside = Rect::new(
        field.x.saturating_add(1),
        field.y.saturating_add(1),
        (field.width - 2).max(0),
        (field.height - 2).max(0),
    );
    let left = inside.x.max(0);
    let top = inside.y.max(0);
    let end_x = inside.x.saturating_add(inside.width).min(wide);
    let end_y = inside.y.saturating_add(inside.height).min(high);
    let (Ok(w), Ok(h)) = (u32::try_from(end_x - left), u32::try_from(end_y - top)) else {
        return;
    };
    if w == 0 || h == 0 {
        return;
    }
    let mut under = Frame::new(w, h, ENTRY_FILL);
    let line_h = font.line_height.max(1);
    let wrap_w = (inside.width - 6).max(16);
    let lines = wrap_multiline_ranges(font, text, wrap_w);
    let len = text.chars().count();
    let caret_chars = state.caret_in(text);
    let caret_line = multiline_caret_line(&lines, caret_chars, state.caret_line);
    let scroll_y = ((caret_line as i32 + 1) * line_h + 2 - inside.height).max(0);
    let [ox, oy] = [inside.x.saturating_sub(left), inside.y.saturating_sub(top)];
    let selection = state.selection_in(len);
    for (idx, &(c0, c1)) in lines.iter().enumerate() {
        let y = oy
            .saturating_add(2)
            .saturating_add((idx as i32) * line_h)
            .saturating_sub(scroll_y);
        let b0 = char_byte_index(text, c0);
        let b1 = char_byte_index(text, c1);
        let slice = &text[b0..b1];
        let x0 = ox.saturating_add(3);
        if let Some((sel_s, sel_e)) = selection
            && sel_e > c0
            && sel_s < c1
        {
            let ls = sel_s.max(c0);
            let le = sel_e.min(c1);
            let bs = char_byte_index(text, ls);
            let be = char_byte_index(text, le);
            let pre_w = font.width(&text[b0..bs]);
            let sel_w = font.width(&text[bs..be]);
            font.draw(&mut under, &text[b0..bs], [x0, y], LABEL);
            under.fill(
                Rect::new(x0.saturating_add(pre_w), y, sel_w.max(1), line_h),
                LABEL,
            );
            font.draw(
                &mut under,
                &text[bs..be],
                [x0.saturating_add(pre_w), y],
                ENTRY_FILL,
            );
            font.draw(
                &mut under,
                &text[be..b1],
                [x0.saturating_add(pre_w).saturating_add(sel_w), y],
                LABEL,
            );
        } else {
            font.draw(&mut under, slice, [x0, y], LABEL);
        }
        if state.focused && state.caret_visible && idx == caret_line {
            let bc = char_byte_index(text, caret_chars.clamp(c0, c1));
            let caret_x = x0
                .saturating_add(font.width(&text[b0..bc]))
                .saturating_add(1);
            for cy in y..y.saturating_add(line_h) {
                under.blend(caret_x, cy, LABEL, 255);
            }
        }
    }
    for y in 0..end_y - top {
        for x in 0..end_x - left {
            if let Some(rgb) = under.pixel(x, y) {
                frame.blend(left + x, top + y, rgb, 255);
            }
        }
    }
}

/// What a press at `point` asks for: the answer of the button there.
/// The dialog stops presses everywhere else while it is shown, as a modal
/// dialog does.
pub fn press(layout: &Layout, point: [i32; 2]) -> Option<Command> {
    press_with_state(layout, point, &DialogState::default())
}

/// Like [`press`], ignoring disabled buttons in `state`.
pub fn press_with_state(layout: &Layout, point: [i32; 2], state: &DialogState) -> Option<Command> {
    let button = layout
        .buttons
        .iter()
        .position(|r| r.contains(point[0], point[1]))?;
    if state.is_disabled(button) {
        return None;
    }
    Some(Command::AnswerDialog {
        caller: layout.caller,
        button: u8::try_from(button).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_is_mitred_and_filled() {
        assert_eq!(frame_color(0, 50, 100, 100), rgb565(LEFT[0]));
        assert_eq!(frame_color(50, 0, 100, 100), rgb565(TOP[0]));
        assert_eq!(frame_color(99, 50, 100, 100), rgb565(RIGHT[0]));
        assert_eq!(frame_color(50, 99, 100, 100), rgb565(BOTTOM[0]));
        assert_eq!(frame_color(50, 50, 100, 100), FILL);
        assert_eq!(frame_color(1, 3, 100, 100), rgb565(LEFT[1]));
    }

    fn art() -> Art {
        let mut art = Art::default();
        art.insert_font(TITLE_FONT, crate::font::tests::font());
        art.insert_font(BODY_FONT, crate::font::tests::font());
        art.insert(
            BUTTON,
            Image {
                width: 260,
                height: 33,
                rgba: vec![255; 260 * 33 * 4],
            },
        );
        art
    }

    fn message(text: &str, buttons: usize) -> Message {
        Message {
            caller: 7,
            title: "AB".into(),
            text: text.into(),
            buttons: vec!["AB".into(); buttons],
            icon: None,
            entry: None,
            secondary_entry: None,
            frame: FrameStyle::Popup,
            save_card: false,
        }
    }

    #[test]
    fn a_long_message_leaves_the_buttons_on_screen() {
        let art = art();
        for text in ["A\n".repeat(60), "A\n".repeat(10_000)] {
            for buttons in 1..=3 {
                for size in [[800, 600], [100, 50], [0, 0]] {
                    let layout = layout(size, &message(&text, buttons), &art).unwrap();
                    assert!(!layout.lines.is_empty());
                    if size[1] >= 200 {
                        assert!(layout.rect.width <= size[0], "{:?}", layout.rect);
                    }
                }
            }
        }
    }

    #[test]
    fn the_buttons_are_on_screen_in_any_window_wide_enough_for_them() {
        let art = art();
        let icon = |w: u32, h: u32| Image {
            width: w,
            height: h,
            rgba: vec![255; (w * h * 4) as usize],
        };
        let texts = ["B".to_owned(), "A\n".repeat(60), vec!["B"; 3000].join(" ")];
        let icons = [
            None,
            Some(icon(10, 7)),
            Some(icon(124, 256)),
            Some(icon(700, 900)),
        ];
        for text in &texts {
            for picture in &icons {
                for buttons in 1..=3 {
                    let wide = 100 * buttons as i32;
                    for size in [
                        [800, 600],
                        [800, 200],
                        [wide, 54],
                        [wide + 7, 600],
                        [640, 54],
                    ] {
                        if size[0] < wide {
                            continue;
                        }
                        let mut m = message(text, buttons);
                        m.icon = picture.clone();
                        let laid = layout(size, &m, &art).unwrap();
                        assert_eq!(laid.buttons.len(), buttons);
                        for b in &laid.buttons {
                            let on = b.x >= 0
                                && b.x + b.width <= size[0]
                                && b.y >= 0
                                && b.y + b.height <= size[1];
                            assert!(
                                on,
                                "{size:?} {buttons} {:?} {b:?}",
                                picture.as_ref().map(|i| i.height)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_dialog_that_asks_for_text_has_an_entry_under_it() {
        let art = art();
        let mut m = message("B", 1);
        let plain = layout([800, 600], &m, &art).unwrap();
        assert_eq!(plain.entry, None);
        m.entry = Some(ENTRY_START.to_owned());
        let laid = layout([800, 600], &m, &art).unwrap();
        // 9 under the text (21 + 2 + 18 + 2 high), as wide as it, 36
        // high; the button 18 + 36 + 18 under the text.
        let field = laid.entry.unwrap();
        let text_bottom = laid.rect.y + 21 + 2 + 18 + 2;
        assert_eq!(field.y, text_bottom + 9);
        assert_eq!(
            (field.x, field.width),
            (laid.rect.x + 21, laid.rect.width - 42)
        );
        assert_eq!(field.height, 36);
        assert_eq!(laid.buttons[0].y, text_bottom + 18 + 36 + 18);
        assert_eq!(laid.rect.height, plain.rect.height + 54);
        // Drawn: its outline, at any place, without a panic.
        let mut frame = Frame::new(800, 600, [0, 0, 0]);
        draw(&mut frame, &art, &m, &laid, None);
        assert_eq!(frame.pixel(field.x, field.y), Some(LABEL));
        let mut far = laid.clone();
        far.entry = Some(Rect::new(i32::MAX - 3, i32::MIN, i32::MAX, i32::MAX));
        draw(&mut frame, &art, &m, &far, None);
    }

    #[test]
    fn the_entry_takes_64_characters_and_no_control_characters() {
        let mut m = message("B", 1);
        assert!(!type_into(&mut m, 'a'));
        assert!(!erase(&mut m));
        m.entry = Some(String::new());
        for _ in 0..70 {
            type_into(&mut m, 'é');
        }
        assert_eq!(m.entry.as_ref().unwrap().chars().count(), ENTRY_MOST);
        assert!(!type_into(&mut m, 'x'));
        assert!(erase(&mut m));
        assert!(!type_into(&mut m, '\n'));
        assert!(type_into(&mut m, 'x'));
        assert!(m.entry.as_ref().unwrap().ends_with('x'));
        m.entry = Some(String::new());
        assert!(!erase(&mut m));
    }

    #[test]
    fn a_long_entry_stays_inside_its_field_with_the_caret_in_view() {
        let font = crate::font::tests::font();
        let field = Rect::new(10, 10, 30, 36);
        let state = DialogState::default();
        let mut frame = Frame::new(100, 60, [0, 0, 0]);
        draw_entry(&mut frame, &font, field, &"A".repeat(ENTRY_MOST), &state);
        for y in 0..60 {
            for x in 0..100 {
                if !field.contains(x, y) {
                    assert_eq!(frame.pixel(x, y), Some([0, 0, 0]), "{x}, {y}");
                }
            }
        }
        // The caret 3 short of the inside's right edge: x 11 + 28 - 3.
        assert!((11..45).any(|y| frame.pixel(36, y) == Some(LABEL)));
        assert!((11..45).all(|y| frame.pixel(37, y) == Some(ENTRY_FILL)));
        // A short entry starts 4 in, its caret right after it.
        let mut frame = Frame::new(100, 60, [0, 0, 0]);
        draw_entry(&mut frame, &font, field, "", &state);
        assert!((11..45).any(|y| frame.pixel(15, y) == Some(LABEL)));
    }

    #[test]
    fn nine_slice_draws_corners_edges_and_center() {
        // 9x9 tile sheet (3x3 tiles of 3x3), each tile filled with its index 1..=9.
        let mut rgba = vec![0u8; 9 * 9 * 4];
        for y in 0..9 {
            for x in 0..9 {
                let tile = ((y / 3) * 3 + (x / 3) + 1) as u8;
                let idx = (y * 9 + x) * 4;
                rgba[idx..idx + 4].copy_from_slice(&[tile * 20, tile * 10, tile, 255]);
            }
        }
        let sheet = Image {
            width: 9,
            height: 9,
            rgba,
        };
        let mut frame = Frame::new(30, 24, [0, 0, 0]);
        draw_nine_slice(&mut frame, &sheet, Rect::new(2, 3, 20, 14));
        // Corners (tiles 1, 3, 7, 9).
        assert_eq!(frame.pixel(2, 3).unwrap()[2], 1);
        assert_eq!(frame.pixel(21, 3).unwrap()[2], 3);
        assert_eq!(frame.pixel(2, 16).unwrap()[2], 7);
        assert_eq!(frame.pixel(21, 16).unwrap()[2], 9);
        // Edges (tiles 2, 4, 6, 8) and center (tile 5).
        assert_eq!(frame.pixel(10, 3).unwrap()[2], 2);
        assert_eq!(frame.pixel(2, 9).unwrap()[2], 4);
        assert_eq!(frame.pixel(21, 9).unwrap()[2], 6);
        assert_eq!(frame.pixel(10, 16).unwrap()[2], 8);
        assert_eq!(frame.pixel(10, 9).unwrap()[2], 5);
    }

    #[test]
    fn forty_five_square_icon_uses_sim_stub_size_and_centers_portrait() {
        let mut art = art();
        art.insert(
            ICON_STUB,
            Image {
                width: 133,
                height: 103,
                rgba: [10, 20, 30, 255].repeat(133 * 103),
            },
        );
        let portrait = Image {
            width: 45,
            height: 45,
            rgba: [200, 100, 50, 255].repeat(45 * 45),
        };
        let m = Message::notice("Sim", "Hello", "OK").with_icon(Some(portrait));
        let laid = layout([800, 600], &m, &art).unwrap();
        let at = laid.icon.unwrap();
        let mut frame = Frame::new(800, 600, [0, 0, 0]);
        draw(&mut frame, &art, &m, &laid, None);
        // Outer corner of 133x103 stub shows stub color; center (dx=44, dy=29) shows portrait.
        assert_eq!(frame.pixel(at[0], at[1]), Some([10, 20, 30]));
        assert_eq!(frame.pixel(at[0] + 44, at[1] + 29), Some([200, 100, 50]));
    }

    #[test]
    fn dialog_state_edits_caret_blinks_and_handles_disabled_buttons() {
        let mut m = Message::prompt("Name", "Enter name:", "Abc", vec!["OK".into(), "No".into()]);
        let mut state = DialogState::default();
        assert_eq!(state.caret_in(m.entry.as_deref().unwrap()), 3);
        assert!(state.move_left(&m));
        assert!(state.insert(&mut m, 'X'));
        assert_eq!(m.entry.as_deref(), Some("AbXc"));
        assert!(state.backspace(&mut m));
        assert_eq!(m.entry.as_deref(), Some("Abc"));
        assert!(state.home(&m));
        assert!(state.delete(&mut m));
        assert_eq!(m.entry.as_deref(), Some("bc"));
        assert!(state.end(&m));
        assert_eq!(state.caret, Some(2));

        // 500 ms caret blink timer (`0x5f203e`).
        assert!(!state.sync_blink(0));
        assert!(state.caret_visible);
        assert!(!state.sync_blink(CARET_BLINK_US - 1));
        assert!(state.sync_blink(CARET_BLINK_US));
        assert!(!state.caret_visible);

        // Disabled button refuses press (`0x5e3b40`).
        let laid = layout([800, 600], &m, &art()).unwrap();
        state.set_disabled(0, true);
        let b0 = laid.buttons[0];
        let b1 = laid.buttons[1];
        assert_eq!(press_with_state(&laid, [b0.x + 5, b0.y + 5], &state), None);
        assert_eq!(
            press_with_state(&laid, [b1.x + 5, b1.y + 5], &state),
            Some(Command::AnswerDialog {
                caller: 0,
                button: 1,
            })
        );
    }

    #[test]
    fn the_title_is_centered_on_the_last_passs_width() {
        // With no icon the title is centered on the dialog's width as the
        // search's last pass left it, not the width it ends at (0x5243b5).
        let laid = layout([800, 600], &message("B", 1), &art()).unwrap();
        assert_eq!(laid.title[0] - laid.rect.x, 106);
        assert_eq!(laid.title[1] - laid.rect.y, 21);
    }

    #[test]
    fn buttons_spread_evenly_across_the_text() {
        // The text is at least 130 a button wide: 390 for three, so the
        // dialog is 432, with gaps of (390 - 300) / 4 = 22 around them.
        let laid = layout([800, 600], &message("B", 3), &art()).unwrap();
        let r = laid.rect;
        assert_eq!(r.width, 432);
        let starts: Vec<i32> = laid.buttons.iter().map(|b| b.x - r.x).collect();
        assert_eq!(starts, [21 + 22, 21 + 22 + 100 + 22, 21 + 22 + 200 + 44]);
        // Two: (260 - 200) / 3 = 20.
        let laid = layout([800, 600], &message("B", 2), &art()).unwrap();
        let starts: Vec<i32> = laid.buttons.iter().map(|b| b.x - laid.rect.x).collect();
        assert_eq!(laid.rect.width, 302);
        assert_eq!(starts, [41, 161]);
        // 18 under the text, and 21 from the bottom.
        for b in &laid.buttons {
            assert_eq!(b.y + b.height, laid.rect.y + laid.rect.height - 21);
        }
    }

    #[test]
    fn lines_go_beside_the_icon_then_under_it() {
        let font = crate::font::tests::font();
        let chars = |t: &str| t.chars().collect::<Vec<char>>();
        // A 2-high line, a band of one line and 1 across: "B " beside
        // (2 wide), then "B B" under it at the full 3.
        let b = text_box(&font, &chars("B B B"), 3, [1, 2]);
        assert_eq!(
            (b.beside, b.below, b.height),
            (vec!["B ".to_owned()], vec!["B B".to_owned()], 4)
        );
        // One line left over that fits beside the icon stays there.
        let b = text_box(&font, &chars("B B"), 3, [1, 2]);
        assert_eq!(b.beside, ["B ", "B"]);
        assert!(b.below.is_empty());
        assert_eq!(b.height, 4);
        // All of it beside: the band's height.
        let b = text_box(&font, &chars("B"), 3, [1, 6]);
        assert_eq!((b.beside.len(), b.height), (1, 6));
        // No text, no icon: nothing.
        assert_eq!(text_box(&font, &[], 3, [0, 0]).height, 0);
    }

    #[test]
    fn the_icons_band_rounds_to_whole_lines() {
        // An icon 10 by 7: it reaches 7 + 18 past the text's top, which is
        // 21 + 2 + 18 down, so 5, which rounds down to 2 lines (4).
        let mut m = message(&vec!["B"; 2000].join(" "), 1);
        m.icon = Some(Image {
            width: 10,
            height: 7,
            rgba: vec![255; 10 * 7 * 4],
        });
        let laid = layout([800, 600], &m, &art()).unwrap();
        let at: Vec<[i32; 2]> = laid
            .lines
            .iter()
            .map(|(_, [x, y])| [x - laid.rect.x, y - laid.rect.y])
            .collect();
        assert_eq!(&at[..3], [[49, 41], [49, 43], [21, 45]]);
        assert_eq!(laid.icon, Some([laid.rect.x + 21, laid.rect.y + 21]));
    }

    #[test]
    fn the_width_is_searched_for_twice_the_height() {
        // A dialog of a 2-high line per `B` 1 wide: the search ends within
        // 20 of where the dialog is twice as wide as tall.
        let text = vec!["B"; 400].join(" ");
        let laid = layout([4000, 3000], &message(&text, 1), &art()).unwrap();
        let r = laid.rect;
        let ratio = f64::from(r.width) / f64::from(r.height);
        assert!((1.8..2.4).contains(&ratio), "{r:?}");
        // A short one grows to it: 21 + 2 + 18 + 2 + 18 + 33 + 21 = 115
        // high, from 172 wide (the 130 one button needs and 42).
        let laid = layout([800, 600], &message("B", 1), &art()).unwrap();
        assert_eq!(laid.rect.height, 115);
        assert!((210..=250).contains(&laid.rect.width), "{:?}", laid.rect);
    }

    #[test]
    fn buttons_at_the_edges_of_i32_do_not_overflow() {
        let art = art();
        let mut frame = Frame::new(40, 40, [0, 0, 0]);
        let layout = Layout {
            caller: 1,
            rect: Rect::new(i32::MAX - 5, i32::MIN, i32::MAX, 10),
            icon: None,
            title: [0, 0],
            lines: Vec::new(),
            entry: None,
            secondary_entry: None,
            buttons: vec![
                Rect::new(i32::MAX - 5, i32::MAX - 2, i32::MAX, 33),
                Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
                Rect::new(-50, 0, 100, 33),
            ],
        };
        draw(
            &mut frame,
            &art,
            &message("", 3),
            &layout,
            Some([i32::MAX, i32::MAX]),
        );
        assert_eq!(frame.pixel(10, 10), Some([255, 255, 255]));
    }

    #[test]
    fn labels_count_from_one() {
        let labels = Labels::new(vec!["OK".into(), "Cancel".into()]);
        assert_eq!(labels.get(1), "OK");
        assert_eq!(labels.get(0), MISSING);
        assert_eq!(labels.get(9), MISSING);
    }

    #[test]
    fn a_press_answers_with_the_button_under_it() {
        let layout = Layout {
            caller: 15,
            rect: Rect::new(0, 0, 300, 200),
            icon: None,
            title: [0, 0],
            lines: Vec::new(),
            entry: None,
            secondary_entry: None,
            buttons: vec![Rect::new(10, 150, 100, 33), Rect::new(128, 150, 100, 33)],
        };
        assert_eq!(
            press(&layout, [130, 160]),
            Some(Command::AnswerDialog {
                caller: 15,
                button: 1
            })
        );
        assert_eq!(press(&layout, [5, 5]), None);
    }

    #[test]
    fn initial_text_selection_replaces_on_first_key_and_draws_generic_frame() {
        let mut msg = Message::prompt("Name", "Enter name:", ENTRY_START, vec!["OK".into()])
            .with_frame(FrameStyle::Generic);
        let mut state = DialogState::for_message(&msg);
        assert_eq!(state.selection, Some((0, ENTRY_START.len())));

        let mut art = art();
        // `FrameStyle::Generic` (`cTSWinGenDlg`) requests slot 5 (`0x5e5441`),
        // which binds to `PopupInfoTiles.bmp` (`3008`, `0x4efe3b..0x4efe59`).
        art.insert(
            BACKGROUND,
            Image {
                width: 36,
                height: 36,
                rgba: [0x20, 0x40, 0x60, 255].repeat(36 * 36),
            },
        );
        let laid = layout([800, 600], &msg, &art).unwrap();
        let mut frame = Frame::new(800, 600, [0, 0, 0]);
        draw_with_state(&mut frame, &art, &msg, &laid, None, &state);
        assert_eq!(
            frame.pixel(laid.rect.x, laid.rect.y),
            Some([0x20, 0x40, 0x60])
        );

        // `Home` and `End` MUST clear the initial selection even when the caret
        // moves (`0x5f2c13`), so typing after `Home` prepends instead of replacing.
        let mut home_msg = msg.clone();
        let mut home_state = DialogState::for_message(&home_msg);
        assert!(home_state.home(&home_msg));
        assert_eq!(home_state.selection, None);
        assert_eq!(home_state.caret, Some(0));
        assert!(home_state.insert(&mut home_msg, 'X'));
        assert_eq!(home_msg.entry.as_deref(), Some("XHello world"));

        let mut end_msg = msg.clone();
        let mut end_state = DialogState::for_message(&end_msg);
        assert!(end_state.end(&end_msg));
        assert_eq!(end_state.selection, None);
        assert!(end_state.insert(&mut end_msg, '!'));
        assert_eq!(end_msg.entry.as_deref(), Some("Hello world!"));

        // First typed character replaces the entire initial selection (`0x5f462f`).
        assert!(state.insert(&mut msg, 'B'));
        assert_eq!(msg.entry.as_deref(), Some("B"));
        assert_eq!(state.selection, None);
        assert_eq!(state.caret, Some(1));

        // Backspace on an active selection clears the selected span.
        state.selection = Some((0, 1));
        assert!(state.backspace(&mut msg));
        assert_eq!(msg.entry.as_deref(), Some(""));
        assert_eq!(state.selection, None);
        assert_eq!(state.caret, Some(0));
    }

    #[test]
    fn held_dialog_button_only_draws_pressed_while_pointer_is_inside() {
        let msg = Message::notice("Title", "Body", "OK");
        let mut art = art();
        // 4 distinct 20x33 button cells (cells 0..4 colored [10, 20, 30, 40]).
        let mut rgba = Vec::with_capacity(80 * 33 * 4);
        for _y in 0..33 {
            for cell in 0..4u8 {
                let v = (cell + 1) * 10;
                for _x in 0..20 {
                    rgba.extend_from_slice(&[v, 0, 0, 255]);
                }
            }
        }
        art.insert(
            BUTTON,
            Image {
                width: 80,
                height: 33,
                rgba,
            },
        );
        let laid = layout([800, 600], &msg, &art).unwrap();
        let btn = laid.buttons[0];
        let state = DialogState {
            pressed_button: Some(0),
            ..DialogState::default()
        };

        // Pointer inside button -> cell 1 (pressed, r=20).
        let mut inside = Frame::new(800, 600, [0, 0, 0]);
        draw_with_state(
            &mut inside,
            &art,
            &msg,
            &laid,
            Some([btn.x + 2, btn.y + 2]),
            &state,
        );
        assert_eq!(inside.pixel(btn.x + 1, btn.y + 1), Some([20, 0, 0]));

        // Pointer outside button -> cell 0 (normal, r=10).
        let mut outside = Frame::new(800, 600, [0, 0, 0]);
        draw_with_state(&mut outside, &art, &msg, &laid, Some([0, 0]), &state);
        assert_eq!(outside.pixel(btn.x + 1, btn.y + 1), Some([10, 0, 0]));

        // Pointer left window (`None`) -> cell 0 (normal, r=10).
        let mut left = Frame::new(800, 600, [0, 0, 0]);
        draw_with_state(&mut left, &art, &msg, &laid, None, &state);
        assert_eq!(left.pixel(btn.x + 1, btn.y + 1), Some([10, 0, 0]));
    }
}
