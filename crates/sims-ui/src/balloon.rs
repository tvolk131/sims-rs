//! The help balloons the live-mode panels open over the lot: a title and
//! a body of text on a bevelled tile background.
//!
//! The original keeps one balloon for the control panel (made by
//! 0x50f6f2, set up by 0x50f7cd) and gives it its text when a control
//! asks for help. Only the personality panel's sign of the zodiac is
//! ported ([`Help::Sign`]); the needs' balloons, which also show an
//! object's picture, are not.
//!
//! The texts are the game's own and are read from the install
//! ([`Texts::load`]); none is kept here.

use crate::art::Art;
use crate::frame::{Frame, Rect};
use crate::hud::LABEL;
use crate::{Error, Result};
use sims_content::Install;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// The balloon's background (UIGraphics 3008, `kCatalogPopupBackTiles`):
/// 36 by 36, cut in three each way into [`TILE`]-sized corners, edges and
/// a middle (0x50fe18).
pub const BACKGROUND: i32 = 3008;
/// How wide the balloon is (0x50f7cd).
pub const WIDTH: i32 = 559;
/// A third of the background's side.
const TILE: i32 = 12;
/// The title's font (0x6f2aec) and the body's (0x6f2ae8).
const TITLE_FONT: u8 = 10;
const BODY_FONT: u8 = 9;
/// The text's left and right edges, inside the balloon, with no picture
/// (0x50fc30): the right one is 530 in language 0x13, which is not
/// ported.
const TEXT_LEFT: i32 = 5;
const TEXT_RIGHT: i32 = 535;
/// How far the text is drawn right of its edges, and where the title and
/// the body start (0x50fea6..).
const TEXT_INDENT: i32 = 10;
const TITLE_TOP: i32 = 12;
/// What the balloon adds to its text's height, and its least height
/// (0x50fd24): text shorter than this less [`PADDING`] takes 103.
const PADDING: i32 = 24;
const MIN_HEIGHT: i32 = 127;

/// What a balloon explains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Help {
    /// The sign of the zodiac, 1 (Aries) to 12, as the personality panel
    /// read it when it was pressed (0x51bfb3).
    Sign(u8),
}

impl Help {
    /// The balloon object of the panel that shows it: the original's
    /// personality panel keeps one for its sign (+0x1c8), whatever the
    /// sign.
    pub fn balloon(self) -> u8 {
        match self {
            Self::Sign(_) => 0,
        }
    }
}

/// A balloon's text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Text {
    pub title: String,
    pub body: String,
}

/// The balloons' texts, read from the install.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Texts {
    /// The signs', Aries first.
    signs: Vec<Text>,
}

/// UIText.iff's tables the sign balloons read (0x519e68..0x51a00f): the
/// signs' titles and bodies, two entries a sign; the signs' names; and
/// the panel's labels, of which the compatibility lines' are entries 6
/// and 7.
const SIGN_TEXTS: u16 = 165;
const SIGN_NAMES: u16 = 164;
const PANEL_LABELS: u16 = 154;
const MOST_LABEL: usize = 6;
const LEAST_LABEL: usize = 7;

/// The signs each sign gets on best and worst with, 1 to 12 (the tables
/// at 0x6c5ce0 and 0x6c5d40, two a sign).
const MOST: [[u8; 2]; 12] = [
    [3, 2],
    [1, 7],
    [12, 6],
    [2, 8],
    [9, 4],
    [11, 9],
    [6, 4],
    [12, 5],
    [12, 10],
    [11, 2],
    [10, 9],
    [8, 3],
];
const LEAST: [[u8; 2]; 12] = [
    [4, 7],
    [6, 4],
    [10, 1],
    [3, 1],
    [10, 3],
    [5, 2],
    [12, 8],
    [7, 11],
    [7, 8],
    [5, 3],
    [8, 6],
    [5, 1],
];

impl Texts {
    /// Reads them from the install's `GameData/UIText.iff`, in English
    /// (language 1, as the dialogs' labels are read).
    pub fn load(install: &Install) -> Result<Self> {
        let path = install
            .file("GameData/UIText.iff")
            .map_err(Error::Content)?;
        let bytes = std::fs::read(&path).map_err(Error::Io)?;
        let iff = Iff::parse(bytes).map_err(Error::Format)?;
        let table = |id: u16| -> Result<Vec<String>> {
            let Some(chunk) = iff.get(b"STR#", id) else {
                return Ok(Vec::new());
            };
            let table = StringTable::parse(iff.data(chunk)).map_err(Error::Format)?;
            Ok((0..table.entries.len())
                .map_while(|i| table.get(i, 1).map(str::to_owned))
                .collect())
        };
        let (texts, names, labels) = (table(SIGN_TEXTS)?, table(SIGN_NAMES)?, table(PANEL_LABELS)?);
        Ok(Self::signs(&texts, &names, &labels))
    }

    /// The signs' balloons from the three tables' strings, as the
    /// personality panel puts them together (0x519e68..0x51a00f): each
    /// sign's title, and its body followed by a line of the signs it gets
    /// on best with and one of those it gets on worst with. A missing
    /// string is left empty.
    pub fn signs(texts: &[String], names: &[String], labels: &[String]) -> Self {
        let get = |list: &[String], n: usize| list.get(n).cloned().unwrap_or_default();
        let name = |sign: u8| get(names, usize::from(sign) - 1);
        let signs = (0..12)
            .map(|n| {
                let [a, b] = MOST[n];
                let [c, d] = LEAST[n];
                Text {
                    title: get(texts, 2 * n),
                    body: format!(
                        "{}\n{}{}, {}\n{}{}, {}",
                        get(texts, 2 * n + 1),
                        get(labels, MOST_LABEL),
                        name(a),
                        name(b),
                        get(labels, LEAST_LABEL),
                        name(c),
                        name(d),
                    ),
                }
            })
            .collect();
        Self { signs }
    }

    /// What `help` shows; None for a sign out of range.
    pub fn get(&self, help: Help) -> Option<&Text> {
        match help {
            Help::Sign(sign) => self.signs.get(usize::from(sign).checked_sub(1)?),
        }
    }
}

/// A balloon laid out: where it goes and its text broken into lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Balloon {
    pub rect: Rect,
    pub title: String,
    pub lines: Vec<String>,
}

/// Lays `text` out in a balloon over `owner`, the subpanel that asked
/// for it (0x50fc30). The body is broken to the text's width; the
/// balloon is as tall as the title's line and the body's lines, plus
/// [`PADDING`], and at least [`MIN_HEIGHT`]. Its right edge is the
/// owner's and its bottom the owner's top.
///
/// **Not ported:** the 50 pixels more an East Asian language takes, and
/// the owner 50 lower when it is 150 tall (no live-mode panel is).
pub fn lay_out(art: &Art, text: &Text, owner: Rect) -> Balloon {
    let title_line = art.font(TITLE_FONT).map_or(0, |f| f.line_height);
    let (body_line, lines) = art.font(BODY_FONT).map_or((0, Vec::new()), |f| {
        (
            f.line_height,
            f.break_lines(&text.body, TEXT_RIGHT - TEXT_LEFT),
        )
    });
    let count = i32::try_from(lines.len()).unwrap_or(i32::MAX);
    let mut inner = title_line.saturating_add(body_line.saturating_mul(count));
    if inner.saturating_add(PADDING) < MIN_HEIGHT {
        inner = MIN_HEIGHT - PADDING;
    }
    let height = inner.saturating_add(PADDING);
    let right = owner.x.saturating_add(owner.width);
    Balloon {
        rect: Rect::new(
            right.saturating_sub(WIDTH),
            owner.y.saturating_sub(height),
            WIDTH,
            height,
        ),
        title: text.title.clone(),
        lines,
    }
}

/// Draws `balloon`: the background's corners, its edges and middle
/// repeated from their top left to fill the rest (0x5df45d), then the
/// title and as many of the body's lines as start above the body's
/// bottom, in the labels' colour, left aligned.
pub fn draw(frame: &mut Frame, art: &Art, balloon: &Balloon) {
    let r = balloon.rect;
    if let Some(back) = art.image(BACKGROUND) {
        let (right, bottom) = (r.x.saturating_add(r.width), r.y.saturating_add(r.height));
        let columns = [
            (r.x, TILE, 0),
            (r.x + TILE, r.width - 2 * TILE, TILE),
            (right - TILE, TILE, 2 * TILE),
        ];
        let rows = [
            (r.y, TILE, 0),
            (r.y + TILE, r.height - 2 * TILE, TILE),
            (bottom - TILE, TILE, 2 * TILE),
        ];
        for &(y, height, sy) in &rows {
            for &(x, width, sx) in &columns {
                tile(
                    frame,
                    back,
                    Rect::new(sx, sy, TILE, TILE),
                    Rect::new(x, y, width, height),
                );
            }
        }
    }
    let left = r.x.saturating_add(TEXT_LEFT + TEXT_INDENT);
    if let Some(font) = art.font(TITLE_FONT) {
        font.draw(
            frame,
            &balloon.title,
            [left, r.y.saturating_add(TITLE_TOP)],
            LABEL,
        );
    }
    if let (Some(title), Some(font)) = (art.font(TITLE_FONT), art.font(BODY_FONT)) {
        let top = r.y.saturating_add(TITLE_TOP + title.line_height);
        // The body's box ends 12 above the balloon's bottom.
        let bottom = r.y.saturating_add(r.height - PADDING + TITLE_TOP);
        let mut y = top;
        for line in &balloon.lines {
            if y >= bottom {
                break;
            }
            font.draw(frame, line, [left, y], LABEL);
            y = y.saturating_add(font.line_height.max(1));
        }
    }
}

/// Fills `target` with copies of `source`, the first at its top left.
fn tile(frame: &mut Frame, image: &sims_formats::bitmap::Image, source: Rect, target: Rect) {
    if source.width <= 0 || source.height <= 0 {
        return;
    }
    let mut y = 0;
    while y < target.height {
        let mut x = 0;
        while x < target.width {
            let part = Rect::new(
                source.x,
                source.y,
                source.width.min(target.width - x),
                source.height.min(target.height - y),
            );
            frame.draw(image, part, [target.x + x, target.y + y]);
            x += source.width;
        }
        y += source.height;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sign_balloon_names_the_signs_it_gets_on_with() {
        let texts: Vec<String> = (0..24).map(|n| format!("t{n}")).collect();
        let names: Vec<String> = (1..=12).map(|n| format!("s{n}")).collect();
        let labels: Vec<String> = (0..8).map(|n| format!("l{n}: ")).collect();
        let t = Texts::signs(&texts, &names, &labels);
        // Taurus: Aries and Libra, then Virgo and Cancer.
        let taurus = t.get(Help::Sign(2)).unwrap();
        assert_eq!(taurus.title, "t2");
        assert_eq!(taurus.body, "t3\nl6: s1, s7\nl7: s6, s4");
        assert!(t.get(Help::Sign(0)).is_none());
        assert!(t.get(Help::Sign(13)).is_none());
        assert_eq!(t.get(Help::Sign(12)).unwrap().title, "t22");
    }

    #[test]
    fn missing_strings_leave_the_balloon_empty_where_they_go() {
        let t = Texts::signs(&[], &[], &[]);
        assert_eq!(t.get(Help::Sign(1)).unwrap().body, "\n, \n, ");
    }

    #[test]
    fn a_balloon_sits_on_its_owner_flush_right() {
        let art = Art::default();
        let b = lay_out(&art, &Text::default(), Rect::new(413, 500, 387, 100));
        assert_eq!(b.rect, Rect::new(241, 373, WIDTH, MIN_HEIGHT));
    }

    #[test]
    fn a_balloon_grows_with_its_body() {
        let mut art = Art::default();
        let font = crate::font::tests::font();
        let line = font.line_height;
        art.insert_font(TITLE_FONT, font.clone());
        art.insert_font(BODY_FONT, font);
        let text = |lines: usize| Text {
            title: "A".into(),
            body: vec!["A"; lines].join("\n"),
        };
        let owner = Rect::new(413, 500, 387, 100);
        // Short text takes the least height.
        assert_eq!(lay_out(&art, &text(1), owner).rect.height, MIN_HEIGHT);
        // Enough lines to pass it: the title's, the body's and 24.
        let n = usize::try_from((MIN_HEIGHT / line) + 1).unwrap();
        let b = lay_out(&art, &text(n), owner);
        assert_eq!(b.lines.len(), n);
        let height = line * (1 + i32::try_from(n).unwrap()) + PADDING;
        assert_eq!(b.rect, Rect::new(241, 500 - height, WIDTH, height));
    }
}
