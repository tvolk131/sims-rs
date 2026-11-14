//! Message dialogs: what the Dialog primitive asks the player, drawn the
//! way the original draws it.
//!
//! The simulation keeps only a dialog's record: its caller, the
//! instruction's operands and its status (`sims_sim::Dialog`). What the
//! dialog says is presentation, so it is read here: the strings are
//! entries of the caller's code owner's `STR#` 301, picked by the
//! operands, and the icon is one of its `BMP_` chunks.
//!
//! The layout was measured on two screenshots of the original at 800 by
//! 600 (the Magic Town and Studio Town notices on the Goth house). The
//! numbers are in `docs/decisions.md`; how the original sizes a dialog it
//! was not measured on is **unverified**.

use crate::art::{Art, KEY};
use crate::command::Command;
use crate::frame::{Frame, Rect};
use crate::hud::{LABEL, rgb565};
use crate::{Error, Result};
use sims_content::{Catalog, Index, Install};
use sims_formats::bitmap::{self, Image};
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use sims_sim::{Dialog, World};

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
/// Space between the frame and what is inside it.
const MARGIN: i32 = 21;
/// Space between the icon and the text, the title and the text, and the
/// text and the buttons.
const GAP: i32 = 18;
/// Where the title's centering starts when there is an icon: the title
/// is centered between this and the right margin.
const TITLE_LEFT: i32 = 141;
const BUTTON_SIZE: [i32; 2] = [100, 33];
/// Widths go up in steps of 10, all ending in 7, until the dialog is at
/// least twice as wide as tall: the rule that gives both measured widths
/// (557 and 567). **Unverified** beyond those two.
const WIDTH_STEP: i32 = 10;
const MIN_WIDTH: i32 = 207;

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

/// Wraps `text` with line `n` allowed `width(n)` pixels. Words longer than
/// a line get a line of their own.
fn wrap(font: &crate::font::Font, text: &str, width: impl Fn(usize) -> i32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if line.is_empty() || font.width(&candidate) <= width(lines.len()) {
                line = candidate;
            } else {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            }
        }
        lines.push(line);
    }
    lines
}

/// Lays `message` out centered in a window of `size`. None without the
/// fonts.
///
/// A message too long for the window is cut after the last line that
/// leaves the buttons on screen, so the dialog can always be answered.
/// How the original handles one is **unverified**; its messages are a
/// few lines.
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
    let body_top = MARGIN + title_font.line_height.clamp(0, 1024) + GAP;
    let line = body.line_height.clamp(1, 1024);
    // The most lines that leave the buttons inside the window.
    let room = size[1] - body_top - GAP - BUTTON_SIZE[1] - MARGIN;
    let most = usize::try_from(room / line).unwrap_or(0).max(1);
    // Lines that start above the icon's bottom go beside it.
    let beside = |n: usize| {
        icon.is_some_and(|[_, h]| {
            let top = i64::from(body_top) + i64::from(line) * n as i64;
            top < i64::from(MARGIN + h)
        })
    };
    let text_left = icon.map_or(MARGIN, |[w, _]| MARGIN + w + GAP);
    let lay = |width: i32| {
        let mut lines = wrap(body, &message.text, |n| {
            width - MARGIN - if beside(n) { text_left } else { MARGIN }
        });
        lines.truncate(most);
        // At most 32768 lines of at most 1024 pixels: no overflow.
        let text_bottom = body_top + line * i32::try_from(lines.len()).unwrap_or(1);
        let content = text_bottom.max(icon.map_or(0, |[_, h]| MARGIN + h));
        let height = content + GAP + BUTTON_SIZE[1] + MARGIN;
        (lines, height)
    };
    let count = i32::try_from(message.buttons.len()).unwrap_or(0).min(3);
    let row = count * BUTTON_SIZE[0] + (count - 1).max(0) * GAP;
    // Wide enough for the buttons, keeping to the steps.
    let narrowest = MIN_WIDTH
        + ((row + 2 * MARGIN - MIN_WIDTH).max(0) + WIDTH_STEP - 1) / WIDTH_STEP * WIDTH_STEP;
    let widest = (size[0] - 1).max(narrowest);
    let mut width = narrowest;
    let (mut lines, mut height) = lay(width);
    while width < widest && width < 2 * height {
        width = (width + WIDTH_STEP).min(widest);
        (lines, height) = lay(width);
    }
    let rect = Rect::new((size[0] - width) / 2, (size[1] - height) / 2, width, height);
    let title_width = title_font.width(&message.title);
    let title_x = match icon {
        Some(_) => (TITLE_LEFT + width - MARGIN - title_width).div_euclid(2),
        None => (width - title_width).div_euclid(2),
    };
    let lines = lines
        .into_iter()
        .enumerate()
        .map(|(n, text)| {
            let x = if beside(n) { text_left } else { MARGIN };
            let y = body_top + line * n as i32;
            (text, [rect.x + x, rect.y + y])
        })
        .collect();
    let buttons = (0..count)
        .map(|n| {
            Rect::new(
                rect.x + (width - row) / 2 + n * (BUTTON_SIZE[0] + GAP),
                rect.y + height - MARGIN - BUTTON_SIZE[1],
                BUTTON_SIZE[0],
                BUTTON_SIZE[1],
            )
        })
        .collect();
    Some(Layout {
        caller: message.caller,
        rect,
        icon: icon.map(|_| [rect.x + MARGIN, rect.y + MARGIN]),
        title: [rect.x + title_x, rect.y + MARGIN],
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
        let text = "A\n".repeat(100_000);
        for size in [[800, 600], [100, 50], [0, 0]] {
            let layout = layout(size, &message(&text, 1), &art).unwrap();
            let button = layout.buttons[0];
            if size[1] >= 200 {
                assert!(button.y + button.height <= size[1], "{size:?} {button:?}");
            }
            assert!(!layout.lines.is_empty());
        }
    }

    #[test]
    fn three_buttons_fit_inside_the_frame() {
        let layout = layout([800, 600], &message("B", 3), &art()).unwrap();
        let r = layout.rect;
        assert!(r.width >= 3 * 100 + 2 * 18 + 2 * MARGIN);
        assert_eq!((r.width - MIN_WIDTH) % WIDTH_STEP, 0);
        for b in &layout.buttons {
            assert!(b.x >= r.x + MARGIN && b.x + b.width <= r.x + r.width - MARGIN);
        }
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
