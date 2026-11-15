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
/// A dialog's icon is `BMP_` 5000 plus operand 1 in the caller's file
/// (icon mode 3; legacy/crates/sims-vm/src/dialog.rs `private_bitmap_id`).
pub const ICONS: u16 = 5000;
/// The button bitmap: four cells of 65 by 33 (normal, selected, hover,
/// disabled).
pub const BUTTON: i32 = 17;
/// The art a dialog needs besides the fonts.
pub const RESOURCES: [i32; 1] = [BUTTON];

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
}

/// The default button labels.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Labels(Vec<String>);

impl Labels {
    /// Reads them from the install's `GameData/UIText.iff`.
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
        Ok(Self(labels))
    }

    pub fn new(labels: Vec<String>) -> Self {
        Self(labels)
    }

    /// Label `n`, counting from 1 as the table's users do.
    fn get(&self, n: usize) -> &str {
        n.checked_sub(1)
            .and_then(|i| self.0.get(i))
            .map_or(MISSING, String::as_str)
    }
}

/// The first dialog record that waits for the player: made and not yet
/// answered.
pub fn pending(world: &World) -> Option<&Dialog> {
    world.dialogs.records.iter().find(|r| r.status <= 1)
}

impl Message {
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
        let buttons = match operands[5] & 0x7f {
            0 | 4 => vec![text(operands[3], labels.get(1))],
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
    let icon = message.icon.as_ref().map(|i| {
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
    let top = INSET + PAD;
    let text_top = match title {
        Some(_) => top + title_line + GAP,
        None => top,
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
        let buttons_top = text_top.saturating_add(text.height).saturating_add(GAP);
        let bottom = if count > 0 {
            buttons_top.saturating_add(BUTTON_SIZE[1] + PAD)
        } else {
            buttons_top
        };
        let size = [text_width + 2 * top, bottom.saturating_add(INSET)];
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
        icon: icon.map(|_| [rect.x + top, rect.y + top]),
        title: [rect.x + title_x, rect.y + top],
        lines,
        buttons,
    })
}

/// The frame's edges, outermost pixel first, then the fill: 16-bit colors
/// measured on the original's screen. The original's corners blend the
/// edges and its top left corner has a highlight; here the corners are
/// mitred, so they differ. **The frame is an approximation**: no bitmap in
/// the install matches it, and how the original draws it is not known.
const FILL: [u8; 3] = rgb565(0x000a);
const TOP: [u16; 7] = [0x6b75, 0x7395, 0x7bf6, 0x73b5, 0x39d0, 0x108c, 0x002a];
const LEFT: [u16; 7] = [0x086a, 0x084a, 0x086a, 0x108b, 0x18cc, 0x108b, 0x002a];
const RIGHT: [u16; 7] = [0x0005, 0x0005, 0x0005, 0x0006, 0x0006, 0x0008, 0x0009];
const BOTTOM: [u16; 7] = [0x0004, 0x0004, 0x0004, 0x0005, 0x0005, 0x0008, 0x0009];

/// The frame's color at `x`, `y` inside a `width` by `height` frame.
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

/// Draws the dialog. `pointer` is where the pointer is, for the buttons'
/// hover look.
pub fn draw(
    frame: &mut Frame,
    art: &Art,
    message: &Message,
    layout: &Layout,
    pointer: Option<[i32; 2]>,
) {
    let r = layout.rect;
    // Only the part inside the frame is visited.
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
    if let (Some(icon), Some(at)) = (&message.icon, layout.icon) {
        frame.draw_all(icon, at);
    }
    if let Some(font) = art.font(TITLE_FONT) {
        font.draw(frame, &message.title, layout.title, LABEL);
    }
    if let Some(font) = art.font(BODY_FONT) {
        for (text, at) in &layout.lines {
            font.draw(frame, text, *at, LABEL);
        }
    }
    for (rect, label) in layout.buttons.iter().zip(&message.buttons) {
        let hover = pointer.is_some_and(|[x, y]| rect.contains(x, y));
        // Cell 0 is the button at rest, 2 under the pointer.
        if let Some(cell) = art.cell(BUTTON, 4, if hover { 2 } else { 0 }) {
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
            let rgb = if hover { [255; 3] } else { LABEL };
            font.draw(frame, label, [x as i32, rect.y.saturating_add(5)], rgb);
        }
    }
}

/// What a press at `point` asks for: the answer of the button there.
/// The dialog stops presses everywhere else while it is shown, as a modal
/// dialog does.
pub fn press(layout: &Layout, point: [i32; 2]) -> Option<Command> {
    let button = layout
        .buttons
        .iter()
        .position(|r| r.contains(point[0], point[1]))?;
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
}
