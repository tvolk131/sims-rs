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
use crate::household::{Careers, Faces, Fame, money};
use crate::hud::{LABEL, rgb565};
use crate::thumbnail::Thumbnails;
use crate::{Error, Result};
use sims_content::ui_graphics::US_ENGLISH;
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

/// Optional cached UI context passed into [`Message::read_with_context`] so
/// ordinary VM dialogs (`0..=4`) resolve `$Me`/`$Object`/`$Neighbor`/`$Job`/
/// `$FameTitleLocal` tokens and icon modes `0, 1, 2, 4` without re-reading
/// character or career archives from disk.
#[derive(Default)]
pub struct DialogContext<'a> {
    pub faces: Option<&'a Faces>,
    pub thumbnails: Option<&'a mut Thumbnails>,
    pub art: Option<&'a Art>,
    pub careers: Option<&'a Careers>,
    pub fame: Option<&'a Fame>,
}

/// OBJD byte offsets used when resolving object display names and portraits.
const OBJD_MASTER_OFFSET: usize = 0x14;
const OBJD_SUB_INDEX_OFFSET: usize = 0x16;
const OBJD_CTSS_OFFSET: usize = 0x52;
const PERSON_KIND: u16 = 2;
/// `BMP_` 2002 is the character portrait embedded in `Characters/*.iff` (`0x4e9338`).
const PORTRAIT_BMP_ID: u16 = 2002;
const FACE_SIZE: u32 = 45;

/// Reads a localized catalog display name (`CATS` -> `CTSS[0]` -> `OBJD` chunk
/// label, `0x457d3b` / `0x4061e3`) for `guid`.
fn catalog_display_name(
    guid: u32,
    world: &World,
    index: &Index,
    catalog: &Catalog,
    faces: Option<&Faces>,
) -> Option<String> {
    if let Some(name) = faces.and_then(|f| f.name(guid)).filter(|s| !s.is_empty()) {
        return Some(name.to_owned());
    }
    let resolved_guid = world
        .characters
        .made
        .get(&guid)
        .map_or(guid, |made| made.template);
    let obj = catalog.object(resolved_guid)?;
    let res = index.resources.get(obj.resource)?;
    let bytes = res.read().ok()?;
    let iff = Iff::parse(bytes).ok()?;
    let chunk = iff.get(b"OBJD", obj.chunk)?;
    let data = iff.data(chunk);
    if data.len() >= OBJD_CTSS_OFFSET + 2 {
        let ctss_id = u16::from_le_bytes([data[OBJD_CTSS_OFFSET], data[OBJD_CTSS_OFFSET + 1]]);
        if ctss_id != 0 {
            if let Some(cats) = iff.get(b"CATS", ctss_id) {
                let raw = iff.data(cats);
                let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                let name = sims_formats::strings::decode(&raw[..end], US_ENGLISH);
                if !name.is_empty() {
                    return Some(name);
                }
            }
            if let Some(ctss) = iff.get(b"CTSS", ctss_id)
                && let Ok(table) = StringTable::parse(iff.data(ctss))
                && let Some(name) = table
                    .get(0, US_ENGLISH)
                    .or_else(|| table.entries.first().map(|e| e.text.as_str()))
                && !name.is_empty()
            {
                return Some(name.to_owned());
            }
        }
    }
    (!chunk.label.is_empty()).then(|| chunk.label.clone())
}

/// Resolves a multi-tile object GUID to its group master GUID (`0x45728d`).
fn master_guid_for(guid: u32, index: &Index, catalog: &Catalog) -> u32 {
    let Some(obj) = catalog.object(guid) else {
        return guid;
    };
    let Some(res) = index.resources.get(obj.resource) else {
        return guid;
    };
    let Ok(bytes) = res.read() else {
        return guid;
    };
    let Ok(iff) = Iff::parse(bytes) else {
        return guid;
    };
    let Some(chunk) = iff.get(b"OBJD", obj.chunk) else {
        return guid;
    };
    let data = iff.data(chunk);
    if data.len() < OBJD_SUB_INDEX_OFFSET + 2 {
        return guid;
    }
    let master = i16::from_le_bytes([data[OBJD_MASTER_OFFSET], data[OBJD_MASTER_OFFSET + 1]]);
    let sub = i16::from_le_bytes([data[OBJD_SUB_INDEX_OFFSET], data[OBJD_SUB_INDEX_OFFSET + 1]]);
    if master == 0 || sub == -1 {
        return guid;
    }
    for (&cand_guid, &idx) in &catalog.by_guid {
        let cand = &catalog.objects[idx];
        if cand.resource != obj.resource {
            continue;
        }
        let Some(c) = iff.get(b"OBJD", cand.chunk) else {
            continue;
        };
        let d = iff.data(c);
        if d.len() >= OBJD_SUB_INDEX_OFFSET + 2 {
            let cm = i16::from_le_bytes([d[OBJD_MASTER_OFFSET], d[OBJD_MASTER_OFFSET + 1]]);
            let cs = i16::from_le_bytes([d[OBJD_SUB_INDEX_OFFSET], d[OBJD_SUB_INDEX_OFFSET + 1]]);
            if cm == master && cs == -1 {
                return cand_guid;
            }
        }
    }
    guid
}

/// Returns whether `guid` is a person object definition (`OBJD +0x12 == 2`).
fn is_person_guid(guid: u32, world: &World, catalog: &Catalog) -> bool {
    if world.characters.made.contains_key(&guid) {
        return true;
    }
    if world
        .objects
        .values()
        .any(|o| o.guid == guid && o.person.is_some())
    {
        return true;
    }
    if world
        .neighbors
        .slots
        .iter()
        .flatten()
        .any(|n| n.guid == guid)
    {
        return true;
    }
    catalog.object(guid).is_some_and(|o| o.kind == PERSON_KIND)
}

/// Normalizes a character portrait bitmap into a 45×45 face image so
/// `dialog::layout` backs it with `SimStub.BMP` (`4992`, `0x524a6c..0x524ae7`).
fn normalize_face_45(src: &Image) -> Image {
    if src.width == FACE_SIZE && src.height == FACE_SIZE {
        return src.clone();
    }
    // Many character `BMP_ 2002` strips pack multiple square cells side by side.
    let src_w =
        if src.height > 0 && src.width >= src.height * 2 && src.width.is_multiple_of(src.height) {
            src.height
        } else {
            src.width.max(1)
        };
    let src_h = src.height.max(1);
    let mut rgba = vec![0u8; (FACE_SIZE * FACE_SIZE * 4) as usize];
    for y in 0..FACE_SIZE {
        let sy = ((u64::from(y) * u64::from(src_h)) / u64::from(FACE_SIZE))
            .min(u64::from(src_h - 1)) as u32;
        for x in 0..FACE_SIZE {
            let sx = ((u64::from(x) * u64::from(src_w)) / u64::from(FACE_SIZE))
                .min(u64::from(src_w - 1)) as u32;
            if let Some(px) = src.pixel(sx, sy) {
                let off = ((y * FACE_SIZE + x) * 4) as usize;
                rgba[off..off + 4].copy_from_slice(&px);
            }
        }
    }
    Image {
        width: FACE_SIZE,
        height: FACE_SIZE,
        rgba,
    }
}

/// Resolves a 45×45 person portrait for `guid` (backed by `SimStub.BMP` `4992`).
fn resolve_person_portrait(
    guid: u32,
    world: &World,
    index: &Index,
    catalog: &Catalog,
    faces: Option<&Faces>,
) -> Option<Image> {
    if let Some(face) = faces.and_then(|f| f.face(guid)) {
        return Some(normalize_face_45(face));
    }
    let lookup_guid = world
        .characters
        .made
        .get(&guid)
        .map_or(guid, |made| made.template);
    if let Some(face) = faces.and_then(|f| f.face(lookup_guid)) {
        return Some(normalize_face_45(face));
    }
    let obj = catalog.object(lookup_guid)?;
    let res = index.resources.get(obj.resource)?;
    let bytes = res.read().ok()?;
    let iff = Iff::parse(bytes).ok()?;
    let chunk = iff
        .get(b"BMP_", PORTRAIT_BMP_ID)
        .or_else(|| iff.get(b"BMP_", 2001))
        .or_else(|| iff.get(b"BMP_", 2000))?;
    let mut img = bitmap::bmp(iff.data(chunk)).ok()?;
    img.key(KEY);
    Some(normalize_face_45(&img))
}

/// Resolves a catalog icon / thumbnail for non-person object `guid`.
fn resolve_object_icon(
    guid: u32,
    index: &Index,
    catalog: &Catalog,
    thumbnails: Option<&mut Thumbnails>,
    art: Option<&Art>,
) -> Option<Image> {
    let master = master_guid_for(guid, index, catalog);
    let extract = |thumbs: &mut Thumbnails| -> Option<Image> {
        let _ = thumbs.read(index, catalog, master);
        if let Some(art) = art
            && let Some(icon) = thumbs.icon(art, master)
        {
            return Some(icon);
        }
        let thumb = thumbs.item(master).and_then(|i| i.thumbnail.as_ref())?;
        let w = if thumb.width > crate::thumbnail::SIZE {
            thumb.width / 2
        } else {
            thumb.width
        };
        let h = thumb.height;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                if let Some(px) = thumb.pixel(x, y) {
                    let off = ((y * w + x) * 4) as usize;
                    rgba[off..off + 4].copy_from_slice(&px);
                }
            }
        }
        Some(Image {
            width: w,
            height: h,
            rgba,
        })
    };
    if let Some(thumbs) = thumbnails {
        extract(thumbs)
    } else {
        let mut temp = Thumbnails::default();
        extract(&mut temp)
    }
}

/// Resolves either a 45×45 person portrait or a catalog object icon for `guid`.
fn resolve_guid_icon(
    guid: u32,
    world: &World,
    index: &Index,
    catalog: &Catalog,
    faces: Option<&Faces>,
    thumbnails: Option<&mut Thumbnails>,
    art: Option<&Art>,
) -> Option<Image> {
    if is_person_guid(guid, world, catalog)
        && let Some(portrait) = resolve_person_portrait(guid, world, index, catalog, faces)
    {
        return Some(portrait);
    }
    resolve_object_icon(guid, index, catalog, thumbnails, art)
}

/// Parses optional `:N` and `:M` integer suffixes after a `$Local` / `$TimeLocal`
/// / `$NeighborLocal` / `$Job*` token (`0x44caf6`).
fn parse_token_numbers(bytes: &[u8], mut end: usize) -> (i32, i32, usize) {
    let mut numbers = [-1i32; 2];
    for slot in &mut numbers {
        if bytes.get(end) != Some(&b':') {
            break;
        }
        end += 1;
        let digits = bytes[end..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits > 0 {
            *slot = bytes[end..end + digits].iter().take(15).fold(0i32, |n, d| {
                n.wrapping_mul(10).wrapping_add(i32::from(d - b'0'))
            });
        }
        end += digits;
    }
    (numbers[0], numbers[1], end)
}

/// Replaces each occurrence of `token` in `text` with `replacement`, advancing
/// past the substituted bytes so names containing `$` cannot loop forever.
fn replace_token_safe(text: &str, token: &str, replacement: &str) -> String {
    if !text.contains(token) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(token) {
        out.push_str(&rest[..at]);
        out.push_str(replacement);
        rest = &rest[at + token.len()..];
    }
    out.push_str(rest);
    out
}

/// Formats an hour in `[0, 24)` in US English (`0x4a6ab1` -> `0x5fe51e`:
/// `"%2u:00 am/pm"`).
fn format_hour(hour: i16) -> String {
    let suffix = if hour < 12 { "am" } else { "pm" };
    let display = if hour == 0 {
        12
    } else if hour > 12 {
        hour - 12
    } else {
        hour
    };
    format!("{display:2}:00 {suffix}")
}

/// Formats one ordinary dialog string through the Complete Collection 1.91
/// `0x44bf1a` pass order without mutating `world`.
///
/// Updates `portrait_guid` whenever `$Object`, `$NeighborLocal`, or `$Neighbor`
/// resolves a valid entity (matching `[ebp+0x14]` in `0x44bf1a`).
#[allow(clippy::too_many_arguments)]
fn format_dialog_text(
    input: &str,
    caller: u16,
    world: &World,
    index: &Index,
    catalog: &Catalog,
    faces: Option<&Faces>,
    careers: Option<&Careers>,
    fame: Option<&Fame>,
    portrait_guid: &mut Option<u32>,
) -> String {
    if !input.contains('$') {
        return input.to_owned();
    }
    let mut text = input.to_owned();
    let caller_obj = world.objects.get(&caller);
    let top_frame = caller_obj.and_then(|o| o.stack.last());
    let locals: &[i16] = top_frame.map_or(&[], |f| f.locals.as_slice());
    let stack_object = top_frame.map_or(0, |f| f.stack_object);
    let stack_obj = u16::try_from(stack_object)
        .ok()
        .and_then(|id| world.objects.get(&id));
    let neighbor_rec = world.neighbors.get(stack_object);

    // Pass 1: `$FamilyAssets` and `$Family`.
    if text.contains("$FamilyAssets") {
        let assets_str = money(world.budget.funds);
        text = replace_token_safe(&text, "$FamilyAssets", &assets_str);
    }
    if text.contains("$Family") {
        // Fallback to active household's first resident name when family surname is absent.
        let family_name = caller_obj
            .and_then(|o| catalog_display_name(o.guid, world, index, catalog, faces))
            .unwrap_or_default();
        if !family_name.is_empty() {
            text = replace_token_safe(&text, "$Family", &family_name);
        }
    }

    // Pass 2: `$Me` (`0x44c0fc`).
    if text.contains("$Me")
        && let Some(obj) = caller_obj
        && let Some(name) = catalog_display_name(obj.guid, world, index, catalog, faces)
    {
        text = replace_token_safe(&text, "$Me", &name);
    }

    // Pass 3: `$Object` (`0x44c155`).
    if text.contains("$Object")
        && let Some(obj) = stack_obj
    {
        *portrait_guid = Some(obj.guid);
        if let Some(name) = catalog_display_name(obj.guid, world, index, catalog, faces) {
            text = replace_token_safe(&text, "$Object", &name);
        }
    }

    // Pass 4: `$Local:N` (`0x44c1f2`; `$$Local:N` naturally preserves the leading `$`).
    if text.contains("$Local") {
        let mut out = text.clone();
        let mut from = 0;
        while let Some(at) = out
            .get(from..)
            .and_then(|s| s.find("$Local"))
            .map(|i| i + from)
        {
            let (n, _, end) = parse_token_numbers(out.as_bytes(), at + "$Local".len());
            if let Ok(idx) = usize::try_from(n)
                && idx < locals.len()
            {
                let val = locals[idx].to_string();
                out.replace_range(at..end, &val);
                from = at + val.len();
            } else {
                from = at + "$Local".len();
            }
        }
        text = out;
    }

    // Pass 5: `$Attrib:N`, `$NameAttrib:N`, `$NameLocal:N`, `$TokenNameLocal:N`.
    if text.contains("$Attrib")
        && let Some(obj) = stack_obj
    {
        let mut out = text.clone();
        let mut from = 0;
        while let Some(at) = out
            .get(from..)
            .and_then(|s| s.find("$Attrib"))
            .map(|i| i + from)
        {
            let (n, _, end) = parse_token_numbers(out.as_bytes(), at + "$Attrib".len());
            if let Ok(idx) = usize::try_from(n)
                && let Some(&val) = obj.attributes.get(idx)
            {
                let s = val.to_string();
                out.replace_range(at..end, &s);
                from = at + s.len();
            } else {
                from = at + "$Attrib".len();
            }
        }
        text = out;
    }
    if text.contains("$NameAttrib")
        && let Some(obj) = stack_obj
    {
        let mut out = text.clone();
        let mut from = 0;
        while let Some(at) = out
            .get(from..)
            .and_then(|s| s.find("$NameAttrib"))
            .map(|i| i + from)
        {
            let (n, _, end) = parse_token_numbers(out.as_bytes(), at + "$NameAttrib".len());
            if let Ok(idx) = usize::try_from(n)
                && let Some(&attr_val) = obj.attributes.get(idx)
                && let Ok(oid) = u16::try_from(attr_val)
                && let Some(target) = world.objects.get(&oid)
                && let Some(name) = catalog_display_name(target.guid, world, index, catalog, faces)
            {
                out.replace_range(at..end, &name);
                from = at + name.len();
            } else {
                from = at + "$NameAttrib".len();
            }
        }
        text = out;
    }
    if text.contains("$NameLocal") {
        let mut out = text.clone();
        let mut from = 0;
        while let Some(at) = out
            .get(from..)
            .and_then(|s| s.find("$NameLocal"))
            .map(|i| i + from)
        {
            let (n, _, end) = parse_token_numbers(out.as_bytes(), at + "$NameLocal".len());
            if let Ok(idx) = usize::try_from(n)
                && idx < locals.len()
                && let Ok(oid) = u16::try_from(locals[idx])
                && let Some(target) = world.objects.get(&oid)
                && let Some(name) = catalog_display_name(target.guid, world, index, catalog, faces)
            {
                out.replace_range(at..end, &name);
                from = at + name.len();
            } else {
                from = at + "$NameLocal".len();
            }
        }
        text = out;
    }

    // Pass 6: `$NeighborLocal:N` (`0x44c590`).
    if text.contains("$NeighborLocal") {
        let mut out = text.clone();
        let mut from = 0;
        while let Some(at) = out
            .get(from..)
            .and_then(|s| s.find("$NeighborLocal"))
            .map(|i| i + from)
        {
            let (n, _, end) = parse_token_numbers(out.as_bytes(), at + "$NeighborLocal".len());
            if let Ok(idx) = usize::try_from(n)
                && idx < locals.len()
                && let Some(rec) = world.neighbors.get(locals[idx])
            {
                *portrait_guid = Some(rec.guid);
                if let Some(name) = catalog_display_name(rec.guid, world, index, catalog, faces) {
                    out.replace_range(at..end, &name);
                    from = at + name.len();
                    continue;
                }
            }
            from = at + "$NeighborLocal".len();
        }
        text = out;
    }

    // Pass 7: `$Neighbor` (`0x44c65f`).
    if text.contains("$Neighbor")
        && let Some(rec) = neighbor_rec
    {
        *portrait_guid = Some(rec.guid);
        if let Some(name) = catalog_display_name(rec.guid, world, index, catalog, faces) {
            text = replace_token_safe(&text, "$Neighbor", &name);
        }
    }

    // Pass 8: `$FameTitleLocal:N` and `$Grade`.
    if text.contains("$FameTitleLocal")
        && let Some(fame) = fame
    {
        let mut out = text.clone();
        let mut from = 0;
        while let Some(at) = out
            .get(from..)
            .and_then(|s| s.find("$FameTitleLocal"))
            .map(|i| i + from)
        {
            let (n, _, end) = parse_token_numbers(out.as_bytes(), at + "$FameTitleLocal".len());
            if let Ok(idx) = usize::try_from(n)
                && idx < locals.len()
            {
                let title = fame.level_title(locals[idx]);
                if !title.is_empty() {
                    out.replace_range(at..end, title);
                    from = at + title.len();
                    continue;
                }
            }
            from = at + "$FameTitleLocal".len();
        }
        text = out;
    }

    // Lazily load `work.iff` careers only if a `$Grade` or `$Job*` token is present
    // and the caller did not supply a cached `&Careers`.
    let needs_careers = text.contains("$Grade")
        || text.contains("$JobOffer")
        || text.contains("$JobDesc")
        || text.contains("$Job");
    let loaded_careers = if needs_careers && careers.is_none() {
        Careers::load(index).ok()
    } else {
        None
    };
    let careers_ref = careers.or(loaded_careers.as_ref());

    if text.contains("$Grade")
        && let Some(c) = careers_ref
        && let Some(person) = caller_obj.and_then(|o| o.person.as_ref())
    {
        let grade = c.grade(person.data.get(57).copied().unwrap_or(0));
        if !grade.is_empty() {
            text = replace_token_safe(&text, "$Grade", grade);
        }
    }

    // Pass 9: `$TimeLocal:N` (`0x44c8c4`).
    if text.contains("$TimeLocal") {
        let mut out = text.clone();
        let mut from = 0;
        while let Some(at) = out
            .get(from..)
            .and_then(|s| s.find("$TimeLocal"))
            .map(|i| i + from)
        {
            let (n, _, end) = parse_token_numbers(out.as_bytes(), at + "$TimeLocal".len());
            if let Ok(idx) = usize::try_from(n)
                && idx < locals.len()
                && (0..24).contains(&locals[idx])
            {
                let formatted = format_hour(locals[idx]);
                out.replace_range(at..end, &formatted);
                from = at + formatted.len();
            } else {
                from = at + "$TimeLocal".len();
            }
        }
        text = out;
    }

    // Pass 10: `$JobOffer`, `$JobDesc`, `$Job` (`0x44c966`).
    if let Some(c) = careers_ref
        && let Some(person) = caller_obj.and_then(|o| o.person.as_ref())
    {
        let default_career = person.data.get(56).copied().unwrap_or(0);
        let default_level = i32::from(person.data.get(57).copied().unwrap_or(0));
        let female = (person.data.get(65).copied().unwrap_or(0) & 1) != 0;
        for token in ["$JobOffer", "$JobDesc", "$Job"] {
            if !text.contains(token) {
                continue;
            }
            let mut out = text.clone();
            let mut from = 0;
            while let Some(at) = out
                .get(from..)
                .and_then(|s| s.find(token))
                .map(|i| i + from)
            {
                let (n1, n2, end) = parse_token_numbers(out.as_bytes(), at + token.len());
                let (career_id, level) = match (usize::try_from(n1), usize::try_from(n2)) {
                    (Ok(i1), Ok(i2)) if i1 < locals.len() && i2 < locals.len() => {
                        (locals[i1], i32::from(locals[i2]))
                    }
                    (Ok(i1), _) if i1 < locals.len() => (locals[i1], default_level),
                    _ => (default_career, default_level),
                };
                let replacement = match token {
                    "$JobOffer" => c
                        .offer(career_id)
                        .or_else(|| c.title(career_id, level, female)),
                    "$JobDesc" => c.description(career_id, level),
                    _ => c.title(career_id, level, female),
                };
                if let Some(rep) = replacement
                    && !rep.is_empty()
                {
                    out.replace_range(at..end, rep);
                    from = at + rep.len();
                } else {
                    from = at + token.len();
                }
            }
            text = out;
        }
    }

    text
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

    /// Reads what `dialog` shows, running `0x44bf1a` string formatting,
    /// caller display-name title fallback (`0x457d3b`), and icon modes
    /// `0, 1, 2, 3, 4` (`0x436279..0x4363b5`). Returns `None` when the caller,
    /// its code owner, or its file is gone, or the dialog is a special panel.
    pub fn read(
        world: &World,
        index: &Index,
        catalog: &Catalog,
        labels: &Labels,
        dialog: &Dialog,
    ) -> Result<Option<Self>> {
        Self::read_with_context(
            world,
            index,
            catalog,
            labels,
            dialog,
            &mut DialogContext::default(),
        )
    }

    /// Like [`Message::read`], using cached HUD faces, thumbnails, art,
    /// careers, and fame tables when available.
    pub fn read_with_context(
        world: &World,
        index: &Index,
        catalog: &Catalog,
        labels: &Labels,
        dialog: &Dialog,
        ctx: &mut DialogContext<'_>,
    ) -> Result<Option<Self>> {
        let Some(caller_obj) = world.objects.get(&dialog.caller) else {
            return Ok(None);
        };
        let Some(owner) = caller_obj
            .stack
            .last()
            .and_then(|f| u16::try_from(f.code_owner_type).ok())
            .and_then(|t| world.types.get(&t))
            .and_then(|&guid| catalog.object(guid))
            .and_then(|o| index.resources.get(o.resource))
        else {
            return Ok(None);
        };
        let iff = Iff::parse(owner.read().map_err(Error::Content)?).map_err(Error::Format)?;
        let Some(mut msg) = Self::from_file(&iff, labels, dialog.caller, &dialog.operands)? else {
            return Ok(None);
        };

        // 0x457d3b: when operand 6 is 0 (or yields an empty title), fall back
        // to the caller object's localized catalog display name (`CTSS[0]`).
        if msg.title.is_empty()
            && let Some(caller_name) =
                catalog_display_name(caller_obj.guid, world, index, catalog, ctx.faces)
        {
            msg.title = caller_name;
        }

        // 0x436219..0x436265: format buttons, title, and body through `0x44bf1a`,
        // tracking `[ebp+0x14]` portrait GUID written by `$Object` / `$Neighbor`.
        let mut portrait_guid = None;
        for btn in &mut msg.buttons {
            *btn = format_dialog_text(
                btn,
                dialog.caller,
                world,
                index,
                catalog,
                ctx.faces,
                ctx.careers,
                ctx.fame,
                &mut portrait_guid,
            );
        }
        msg.title = format_dialog_text(
            &msg.title,
            dialog.caller,
            world,
            index,
            catalog,
            ctx.faces,
            ctx.careers,
            ctx.fame,
            &mut portrait_guid,
        );
        msg.text = format_dialog_text(
            &msg.text,
            dialog.caller,
            world,
            index,
            catalog,
            ctx.faces,
            ctx.careers,
            ctx.fame,
            &mut portrait_guid,
        );

        // 0x436279..0x4363b5: resolve icon modes 0, 1, 2, 4 alongside mode 3.
        let icon_mode = (dialog.operands[7] >> 1) & 7;
        let stack_object = caller_obj.stack.last().map_or(0, |f| f.stack_object);
        let stack_obj_guid = u16::try_from(stack_object)
            .ok()
            .and_then(|id| world.objects.get(&id))
            .map(|o| o.guid);
        let stack_neighbor_guid = world.neighbors.get(stack_object).map(|n| n.guid);
        if msg.icon.is_none() {
            match icon_mode {
                // Mode 0 (auto): `$Object`/`$Neighbor` portrait GUID, else stack
                // object's master GUID, else caller's portrait/icon.
                0 => {
                    msg.icon = portrait_guid
                        .and_then(|g| {
                            resolve_guid_icon(
                                g,
                                world,
                                index,
                                catalog,
                                ctx.faces,
                                ctx.thumbnails.as_deref_mut(),
                                ctx.art,
                            )
                        })
                        .or_else(|| {
                            stack_obj_guid.and_then(|g| {
                                resolve_guid_icon(
                                    g,
                                    world,
                                    index,
                                    catalog,
                                    ctx.faces,
                                    ctx.thumbnails.as_deref_mut(),
                                    ctx.art,
                                )
                            })
                        })
                        .or_else(|| {
                            resolve_guid_icon(
                                caller_obj.guid,
                                world,
                                index,
                                catalog,
                                ctx.faces,
                                ctx.thumbnails.as_deref_mut(),
                                ctx.art,
                            )
                        });
                }
                // Mode 1 (neighbor / person 45×45 portrait over SimStub.BMP 4992).
                1 => {
                    let target = stack_neighbor_guid
                        .or(portrait_guid)
                        .or(stack_obj_guid)
                        .unwrap_or(caller_obj.guid);
                    msg.icon = resolve_person_portrait(target, world, index, catalog, ctx.faces)
                        .or_else(|| {
                            resolve_guid_icon(
                                target,
                                world,
                                index,
                                catalog,
                                ctx.faces,
                                ctx.thumbnails.as_deref_mut(),
                                ctx.art,
                            )
                        })
                        .or_else(|| {
                            resolve_person_portrait(
                                caller_obj.guid,
                                world,
                                index,
                                catalog,
                                ctx.faces,
                            )
                        });
                }
                // Mode 2 (stack object thumbnail/icon or stack neighbor portrait).
                2 => {
                    msg.icon = stack_obj_guid
                        .and_then(|g| {
                            resolve_guid_icon(
                                g,
                                world,
                                index,
                                catalog,
                                ctx.faces,
                                ctx.thumbnails.as_deref_mut(),
                                ctx.art,
                            )
                        })
                        .or_else(|| {
                            stack_neighbor_guid.and_then(|g| {
                                resolve_person_portrait(g, world, index, catalog, ctx.faces)
                            })
                        })
                        .or_else(|| {
                            resolve_guid_icon(
                                caller_obj.guid,
                                world,
                                index,
                                catalog,
                                ctx.faces,
                                ctx.thumbnails.as_deref_mut(),
                                ctx.art,
                            )
                        });
                }
                // Mode 4 (caller/person face portrait).
                4 => {
                    msg.icon =
                        resolve_person_portrait(caller_obj.guid, world, index, catalog, ctx.faces)
                            .or_else(|| {
                                resolve_guid_icon(
                                    caller_obj.guid,
                                    world,
                                    index,
                                    catalog,
                                    ctx.faces,
                                    ctx.thumbnails.as_deref_mut(),
                                    ctx.art,
                                )
                            });
                }
                _ => {}
            }
        }

        Ok(Some(msg))
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

    /// Exact microseconds until the focused `cTSWinTextEdit2` caret next flips
    /// visibility, or `None` when no text field is focused.
    pub fn next_blink_us(&self, clock_us: u64) -> Option<u64> {
        if !self.focused {
            return None;
        }
        match self.blink_since {
            None => Some(CARET_BLINK_US),
            Some(since) => {
                let elapsed = clock_us.saturating_sub(since);
                Some(CARET_BLINK_US.saturating_sub(elapsed).max(1_000))
            }
        }
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

/// Draws a 3×3 tiled 9-slice bitmap (`PopupInfoTiles.bmp` 3008, `GenDlg.bmp`
/// 22, or `HotspotPopupTiler.TGA` 5107) across `rect`, matching
/// `cTSWinMsgBox::OnDraw` (`0x524603..0x524645`) and `cTSWinGenDlg::OnDraw`
/// (`0x5e5a33..0x5e5a77`).
pub fn draw_nine_slice(frame: &mut Frame, sheet: &Image, rect: Rect) {
    draw_nine_slice_alpha(frame, sheet, rect, 255);
}

/// Like [`draw_nine_slice`], modulating every pixel's alpha (including all four
/// corners, edges, and tiled interior) by `alpha` (`0..=255`) through
/// [`Frame::blend`]'s native 5-bit blend table (`0x6e5eac`, matching
/// `cTSWinFade` at `0x5e8c93..0x5e8e10`).
pub fn draw_nine_slice_alpha(frame: &mut Frame, sheet: &Image, rect: Rect, alpha: u8) {
    if alpha <= 7 {
        return;
    }
    let tw = i32::try_from(sheet.width / 3).unwrap_or(0);
    let th = i32::try_from(sheet.height / 3).unwrap_or(0);
    if tw <= 0 || th <= 0 || rect.width <= 0 || rect.height <= 0 {
        return;
    }
    let draw_part = |frame: &mut Frame, src: Rect, at: [i32; 2]| {
        if alpha >= 248 {
            frame.draw(sheet, src, at);
            return;
        }
        let range = |start: i32, length: i32, size: u32, target: i32, limit: u32| {
            let (start, target) = (i64::from(start), i64::from(target));
            let low = 0.max(-start).max(-target);
            let high = i64::from(length)
                .min(i64::from(size) - start)
                .min(i64::from(limit) - target);
            low..high.max(low)
        };
        let xs = range(src.x, src.width, sheet.width, at[0], frame.width());
        for sy in range(src.y, src.height, sheet.height, at[1], frame.height()) {
            for sx in xs.clone() {
                let (ix, iy) = (i64::from(src.x) + sx, i64::from(src.y) + sy);
                if let Some([r, g, b, pa]) = sheet.pixel(ix as u32, iy as u32) {
                    let eff = ((u16::from(pa) * u16::from(alpha) + 127) / 255) as u8;
                    if eff > 7 {
                        let (dx, dy) = (i64::from(at[0]) + sx, i64::from(at[1]) + sy);
                        frame.blend(dx as i32, dy as i32, [r, g, b], eff);
                    }
                }
            }
        }
    };
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

    // 4 corners (preserving per-pixel corner alpha & global alpha modulation).
    draw_part(frame, Rect::new(0, 0, left_w, top_h), [rect.x, rect.y]);
    if right_w > 0 {
        draw_part(
            frame,
            Rect::new(tw * 2 + (tw - right_w), 0, right_w, top_h),
            [rect.x.saturating_add(rect.width - right_w), rect.y],
        );
    }
    if bottom_h > 0 {
        draw_part(
            frame,
            Rect::new(0, th * 2 + (th - bottom_h), left_w, bottom_h),
            [rect.x, rect.y.saturating_add(rect.height - bottom_h)],
        );
        if right_w > 0 {
            draw_part(
                frame,
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
            draw_part(
                frame,
                Rect::new(tw + phase, 0, step, top_h),
                [x as i32, rect.y],
            );
            if bottom_h > 0 {
                draw_part(
                    frame,
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
            draw_part(
                frame,
                Rect::new(0, th + phase, left_w, step),
                [rect.x, y as i32],
            );
            if right_w > 0 {
                draw_part(
                    frame,
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
                draw_part(
                    frame,
                    Rect::new(tw + px, th + py, sx, sy),
                    [x as i32, y as i32],
                );
                x += i64::from(sx);
            }
            y += i64::from(sy);
        }
    }
}

/// Pure-state translucent dialog / overlay fade helper matching `cTSWinFade`
/// (`0x5e8c93..0x5e8e10`, converting normalized opacity `0.0..=1.0` to `0..=255`
/// via `round(t * 255.0)` at `0x5e8ddb..0x5e8e0d`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DialogFadeState {
    start_value: f32,
    target_value: f32,
    start_ms: u32,
    end_ms: u32,
}

impl DialogFadeState {
    pub const fn hidden() -> Self {
        Self {
            start_value: 0.0,
            target_value: 0.0,
            start_ms: 0,
            end_ms: 0,
        }
    }

    pub const fn opaque() -> Self {
        Self {
            start_value: 1.0,
            target_value: 1.0,
            start_ms: 0,
            end_ms: 0,
        }
    }

    /// Starts a linear opacity ramp to `target` (`0.0..=1.0`) over `duration_ms`.
    pub fn fade_to(&mut self, target: f32, now_ms: u32, duration_ms: u32) {
        let cur = self.progress(now_ms);
        self.start_value = cur;
        self.target_value = if target.is_finite() {
            target.clamp(0.0, 1.0)
        } else if target.is_sign_positive() && target.is_infinite() {
            1.0
        } else {
            0.0
        };
        self.start_ms = now_ms;
        self.end_ms = now_ms.saturating_add(duration_ms);
    }

    pub fn progress(&self, now_ms: u32) -> f32 {
        if now_ms >= self.end_ms || self.end_ms <= self.start_ms {
            return self.target_value.clamp(0.0, 1.0);
        }
        if now_ms <= self.start_ms {
            return self.start_value.clamp(0.0, 1.0);
        }
        let dur = (self.end_ms - self.start_ms) as f64;
        let rem = (self.end_ms - now_ms) as f64;
        let rate = f64::from(self.target_value - self.start_value) / dur;
        (f64::from(self.target_value) - rem * rate).clamp(0.0, 1.0) as f32
    }

    /// Quantized 8-bit alpha `0..=255` matching `0x5e8ddb..0x5e8e0d`.
    pub fn alpha(&self, now_ms: u32) -> u8 {
        ((f64::from(self.progress(now_ms)) * 255.0 + 0.5).trunc() as i32).clamp(0, 255) as u8
    }

    pub fn is_animating(&self, now_ms: u32) -> bool {
        now_ms < self.end_ms && self.end_ms > self.start_ms
    }
}

/// Like [`draw_with_state`], compositing the entire dialog (including its
/// 9-slice corners, border, text, and buttons) at global opacity `alpha`
/// (`0..=255`) via the native 5-bit blend table (`0x6e5eac`).
pub fn draw_with_state_alpha(
    frame: &mut Frame,
    art: &Art,
    message: &Message,
    layout: &Layout,
    pointer: Option<[i32; 2]>,
    state: &DialogState,
    alpha: u8,
) {
    if alpha <= 7 || layout.rect.width <= 0 || layout.rect.height <= 0 {
        return;
    }
    if alpha >= 248 {
        draw_with_state(frame, art, message, layout, pointer, state);
        return;
    }
    let fw = i32::try_from(frame.width()).unwrap_or(i32::MAX);
    let fh = i32::try_from(frame.height()).unwrap_or(i32::MAX);
    let r = layout.rect;
    let x0 = r.x.clamp(0, fw);
    let y0 = r.y.clamp(0, fh);
    let x1 = r.x.saturating_add(r.width).clamp(x0, fw);
    let y1 = r.y.saturating_add(r.height).clamp(y0, fh);
    let w = (x1 - x0) as u32;
    let h = (y1 - y0) as u32;
    if w == 0 || h == 0 {
        return;
    }
    let mut scratch = Frame::new(w, h, [0, 0, 0]);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if let Some(bg) = frame.pixel(x0 + x, y0 + y) {
                scratch.blend(x, y, bg, 255);
            }
        }
    }
    let dx = -x0;
    let dy = -y0;
    let shifted = Layout {
        caller: layout.caller,
        rect: r.offset(dx, dy),
        icon: layout
            .icon
            .map(|[x, y]| [x.saturating_add(dx), y.saturating_add(dy)]),
        title: [
            layout.title[0].saturating_add(dx),
            layout.title[1].saturating_add(dy),
        ],
        lines: layout
            .lines
            .iter()
            .map(|(s, [x, y])| (s.clone(), [x.saturating_add(dx), y.saturating_add(dy)]))
            .collect(),
        entry: layout.entry.map(|e| e.offset(dx, dy)),
        secondary_entry: layout.secondary_entry.map(|e| e.offset(dx, dy)),
        buttons: layout.buttons.iter().map(|b| b.offset(dx, dy)).collect(),
    };
    let shifted_ptr = pointer.map(|[x, y]| [x.saturating_add(dx), y.saturating_add(dy)]);
    draw_with_state(&mut scratch, art, message, &shifted, shifted_ptr, state);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let fx = x0 + x;
            let fy = y0 + y;
            if let (Some(rgb), Some(bg)) = (scratch.pixel(x, y), frame.pixel(fx, fy))
                && rgb != bg
            {
                frame.blend(fx, fy, rgb, alpha);
            }
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

    #[test]
    fn draw_nine_slice_alpha_and_dialog_fade_state_blend_corners_cleanly() {
        let mut fade = DialogFadeState::hidden();
        assert_eq!(fade.alpha(0), 0);
        assert!(!fade.is_animating(0));
        fade.fade_to(1.0, 100, 200);
        assert!(fade.is_animating(100));
        assert_eq!(fade.alpha(100), 0);
        assert_eq!(fade.alpha(200), 128);
        assert_eq!(fade.alpha(300), 255);
        assert!(!fade.is_animating(300));
        fade.fade_to(0.0, 400, 200);
        assert_eq!(fade.alpha(500), 128);
        assert_eq!(fade.alpha(600), 0);
        assert!(!fade.is_animating(600));

        // 36x36 nine-slice with corner pixel [200, 100, 50, 128] and opaque fill [200, 100, 50, 255].
        let mut rgba = [200, 100, 50, 255].repeat(36 * 36);
        rgba[3] = 128; // top-left corner half-alpha
        let sheet = Image {
            width: 36,
            height: 36,
            rgba,
        };
        let mut frame = Frame::new(60, 60, [0, 0, 0]);
        draw_nine_slice_alpha(&mut frame, &sheet, Rect::new(10, 10, 40, 40), 128);
        let corner = frame.pixel(10, 10).unwrap();
        let center = frame.pixel(25, 25).unwrap();
        // Corner combines sheet alpha (128) with global alpha (128) -> ~64/255.
        assert!(corner[0] > 0 && corner[0] < center[0]);
        assert!(center[0] > 80 && center[0] < 120);

        // `draw_with_state_alpha` at 0 is a no-op and at 255 matches `draw_with_state`.
        let msg = Message::notice("Title", "Body", "OK");
        let art = art();
        let laid = layout([800, 600], &msg, &art).unwrap();
        let state = DialogState::default();
        let mut f0 = Frame::new(800, 600, [10, 20, 30]);
        draw_with_state_alpha(&mut f0, &art, &msg, &laid, None, &state, 0);
        assert_eq!(
            f0.pixel(laid.rect.x + 15, laid.rect.y + 15),
            Some([10, 20, 30])
        );
        let mut f255 = Frame::new(800, 600, [10, 20, 30]);
        let mut direct = Frame::new(800, 600, [10, 20, 30]);
        draw_with_state_alpha(&mut f255, &art, &msg, &laid, None, &state, 255);
        draw_with_state(&mut direct, &art, &msg, &laid, None, &state);
        assert_eq!(f255.rgb(), direct.rgb());
    }

    #[test]
    fn token_parsers_and_formatters_handle_edge_cases() {
        assert_eq!(parse_token_numbers(b"$Local:3:12x", 6), (3, 12, 11));
        assert_eq!(parse_token_numbers(b"$Local:0!", 6), (0, -1, 8));
        assert_eq!(parse_token_numbers(b"$Local ", 6), (-1, -1, 6));
        assert_eq!(format_hour(0), "12:00 am");
        assert_eq!(format_hour(9), " 9:00 am");
        assert_eq!(format_hour(12), "12:00 pm");
        assert_eq!(format_hour(13), " 1:00 pm");
        assert_eq!(format_hour(23), "11:00 pm");
        // Replacement containing the token itself must not loop.
        assert_eq!(
            replace_token_safe("Hi $Me and $Me!", "$Me", "$Me$"),
            "Hi $Me$ and $Me$!"
        );
    }
}
