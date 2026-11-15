//! The pie menu: what the selected Sim can do with the object the player
//! clicked, laid out around the Sim's head as the original lays it out.
//!
//! Ported from the original's code, not measured on its screen:
//!
//! - **The menu window** (cTSPieMenu, vtable 0x674b54): layout 0x5ea8e1,
//!   drawing 0x5eb04c, a label's box 0x5eae63, the hit test 0x5eb7c9 and
//!   placement on the screen 0x5ebab2.
//! - **The people panel's two menus** (0x518665): resource 800
//!   (`PieFace1.bmp`, 201 by 201 and all colour key) in the middle, the
//!   label font, the colours, the radii 60 and 30, the dead zone 35 and
//!   the reach 150.
//! - **The object picker** (0x48d56c, 0x48d8a0, 0x48dac2): names with a
//!   `/` make a submenu, at most 12 entries are shown, and a submenu opens
//!   centered on the entry that opened it.
//!
//! The positions agree with two menus the input track read from the
//! running original (`tools/harness/input.py`, "Sit" and "Go Here" on the
//! Goth house), to the pixel. How the drawn menu looks has **not** been
//! compared with a screenshot.
//!
//! In the middle the original draws the selected Sim's head, rendered
//! live into the blank 201 by 201 face bitmap. The head is the host's
//! (`sims-people`); [`draw`] takes it as an image.

use crate::art::Art;
use crate::font::Font;
use crate::frame::{Frame, Rect};
use sims_formats::bitmap::Image;

/// The face bitmap the head is drawn into (kPieFaceBkg).
pub const FACE: i32 = 800;
/// Every resource the menu draws, for [`Art::load`].
pub const RESOURCES: [i32; 1] = [FACE];
/// The face's size when the art is missing: `PieFace1.bmp`'s.
pub const FACE_SIZE: [i32; 2] = [201, 201];
/// The label font, `variablesans_12` (the people panel asks the font
/// manager for font 1; its labels measure as this font).
pub const FONT: u8 = 12;
/// The most entries one menu shows (0x48d930).
pub const SHOWN: usize = 12;

/// The room left around the face (+0x16c, set to 8 by the constructor).
const PADDING: i32 = 8;
/// How far a label stands from the middle: above, below and to the sides
/// (+0x184), and along the diagonals (+0x188). The people panel sets 60
/// and 30 (0x518848); the constructor's 40 and 20 are never used.
const RADIUS: i32 = 60;
const DIAGONAL: i32 = 30;
/// Within this distance of the middle nothing is highlighted (+0x160, 35
/// from the people panel).
pub const DEAD_ZONE: i32 = 35;
/// Beyond this distance only a label's own box highlights it (+0x174).
pub const REACH: i32 = 150;
/// A label's box is its text with 3 pixels all round.
const INSET: i32 = 3;

/// The colours the people panel gives its menus (0x518712 to 0x518785),
/// as the 16-bit screen shows them.
const BOX: [u8; 3] = rgb(0, 40, 140);
const TEXT: [u8; 3] = rgb(165, 195, 214);
const HIGHLIGHT: [u8; 3] = rgb(255, 255, 255);
/// The highlighted label while a mouse button is held, and the title.
const PRESSED: [u8; 3] = rgb(0, 255, 255);

/// A colour through the original's 16-bit screen (5, 6 and 5 bits),
/// widened back by repeating the high bits.
const fn rgb(r: u8, g: u8, b: u8) -> [u8; 3] {
    let (r, g, b) = (r >> 3, g >> 2, b >> 3);
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}

/// The sounds the menu window itself plays (0x5eb421, 0x5eb593,
/// 0x5eba16). The people panel's menus do not turn them on (+0x17c stays
/// 0), so the game's pie menu plays none of them; they are listed for a
/// host that wants them.
pub const SOUNDS: [&str; 3] = [
    "UI_piemenu_appear",
    "UI_piemenu_highlight",
    "UI_piemenu_select",
];

/// One entry of a menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    /// The original keeps a disabled entry in the menu but neither draws
    /// it nor lets it be chosen. The game's menus enable every entry
    /// (0x5eb2be); hiding the first menu's entries while a submenu is
    /// open disables them (0x48dbe1).
    pub enabled: bool,
}

/// A label placed on the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub text: String,
    pub enabled: bool,
    pub rect: Rect,
}

/// A menu laid out on the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// The menu's window: everything it draws is inside.
    pub window: Rect,
    /// The middle, where the face is centered. It is where the menu was
    /// asked to open unless the window had to move to stay on the screen
    /// ([`Layout::moved`]); the original then moves the pointer to it.
    pub center: [i32; 2],
    pub moved: bool,
    /// Where the face (and the head) goes.
    pub face: Rect,
    /// The title's box. The original always makes room for it; it is
    /// drawn only when there is a title.
    pub title: Option<Label>,
    pub labels: Vec<Label>,
}

/// The size of `text`'s box: the text measured with the font's line
/// height, 3 pixels added all round.
fn measure(font: &Font, text: &str) -> [i32; 2] {
    [
        font.width(text).saturating_add(2 * INSET),
        font.line_height.saturating_add(2 * INSET),
    ]
}

/// How many places round the middle `n` entries are spread over
/// (0x5eaa54): 2, 4 or 8, and 1 for an empty menu.
fn places(n: usize) -> usize {
    match n {
        0 => 1,
        1..=2 => 2,
        3..=4 => 4,
        _ => 8,
    }
}

/// Lays out a menu of `entries` around `at` on a screen of `screen`,
/// with `title` in the middle (empty for none). None without the label
/// font. Entries past [`SHOWN`] are left out, as the picker leaves them.
///
/// The first entry goes above the middle and the rest follow clockwise:
/// two entries above and below, four at the sides too, up to eight at the
/// diagonals too. A ninth and later entry stacks above the top entry or
/// below the bottom one, in turn.
pub fn layout(
    screen: [i32; 2],
    at: [i32; 2],
    title: &str,
    entries: &[Entry],
    art: &Art,
) -> Option<Layout> {
    let font = art.font(FONT)?;
    let face = art.image(FACE).map_or(FACE_SIZE, |image| {
        [image.width, image.height].map(|n| i32::try_from(n).unwrap_or(0).min(4096))
    });
    // Positions round the middle first, as 0x5ea8e1 works them out.
    let face_at = face.map(|n| n / -2);
    let mut low = face_at.map(|n| n - PADDING);
    let mut high = [0, 1].map(|i| PADDING + face_at[i] + face[i]);
    let mut include = |rect: Rect| {
        low = [low[0].min(rect.x), low[1].min(rect.y)];
        high = [
            high[0].max(rect.x.saturating_add(rect.width)),
            high[1].max(rect.y.saturating_add(rect.height)),
        ];
    };
    let title_size = measure(font, title);
    let title_rect = Rect::new(
        title_size[0] / -2,
        title_size[1] / -2,
        title_size[0],
        title_size[1],
    );
    include(title_rect);
    let shown = &entries[..entries.len().min(SHOWN)];
    let count = places(shown.len());
    let step = 8 / count;
    let mut rects: Vec<Rect> = Vec::with_capacity(shown.len());
    for (i, entry) in shown.iter().enumerate() {
        let [w, h] = measure(font, &entry.label);
        let (x, y) = if i >= count {
            // Stacked above the top entry (even) or below the bottom one
            // (odd), the start direction being 0.
            let k = i - count;
            let under = if k < 2 { [0, 4][k] } else { i - 2 };
            let under = rects[under];
            if k % 2 == 1 {
                (w / -2, under.y + under.height)
            } else {
                (w / -2, under.y - h)
            }
        } else {
            match (step * i) & 7 {
                0 => (w / -2, -(RADIUS + h)),
                1 => (DIAGONAL, -(DIAGONAL + h)),
                2 => (RADIUS, h / -2),
                3 => (DIAGONAL, DIAGONAL),
                4 => (w / -2, RADIUS),
                5 => (-(DIAGONAL + w), DIAGONAL),
                6 => (-(RADIUS + w), h / -2),
                _ => (-(DIAGONAL + w), -(DIAGONAL + h)),
            }
        };
        let rect = Rect::new(x, y, w, h);
        include(rect);
        rects.push(rect);
    }
    // Four pixels more all round, and the window's origin at the top left.
    let low = low.map(|n| n - 4);
    let size = [0, 1].map(|i| high[i] - low[i] + 4);
    let middle = low.map(|n| -n);
    // Kept on the screen (0x5ebb10): each side is clamped on its own, to
    // 0 first and then to the far edge.
    let mut moved = false;
    let origin: [i32; 2] = std::array::from_fn(|i| {
        let start = at[i].saturating_sub(middle[i]);
        if start < 0 {
            moved = true;
            0
        } else if start.saturating_add(size[i]) >= screen[i] {
            moved = true;
            screen[i].saturating_sub(size[i])
        } else {
            start
        }
    });
    let center = [0, 1].map(|i| origin[i].saturating_add(middle[i]));
    let place = |r: Rect| r.offset(center[0], center[1]);
    Some(Layout {
        window: Rect::new(origin[0], origin[1], size[0], size[1]),
        center,
        moved,
        face: place(Rect::new(face_at[0], face_at[1], face[0], face[1])),
        title: (!title.is_empty()).then(|| Label {
            text: title.to_owned(),
            enabled: true,
            rect: place(title_rect),
        }),
        labels: shown
            .iter()
            .zip(rects)
            .map(|(entry, rect)| Label {
                text: entry.label.clone(),
                enabled: entry.enabled,
                rect: place(rect),
            })
            .collect(),
    })
}

/// The entry a pointer at `point` highlights (0x5eb7c9), or None.
///
/// A label's own box always counts. Elsewhere, the pointer's direction
/// from the middle picks the place it points at, between 35 and 150
/// pixels out; nearer the middle nothing is highlighted.
pub fn hit(layout: &Layout, point: [i32; 2]) -> Option<usize> {
    let n = layout.labels.len();
    if n == 0 {
        return None;
    }
    let dx = f64::from(point[0]) - f64::from(layout.center[0]);
    let dy = f64::from(point[1]) - f64::from(layout.center[1]);
    // The original keeps the distance as a 32-bit float.
    let distance = (dx * dx + dy * dy).sqrt() as f32;
    if distance < 1.0 {
        return None;
    }
    if let Some(i) = layout
        .labels
        .iter()
        .position(|l| l.enabled && l.rect.contains(point[0], point[1]))
    {
        return Some(i);
    }
    if distance <= DEAD_ZONE as f32 || distance > REACH as f32 {
        return None;
    }
    // Clockwise from straight up, in places; the first place is centered
    // straight up (0x5eb859 to 0x5eb942).
    let places = places(n) as f64;
    let pi = f64::from(std::f32::consts::PI);
    let mut place = libm::atan2(dx, -dy) * (places / (2.0 * pi)) + 0.5;
    while place >= places {
        place -= places;
    }
    while place < 0.0 {
        place += places;
    }
    let i = place as usize;
    layout.labels.get(i).filter(|l| l.enabled).map(|_| i)
}

/// Draws a label's box (0x5eae63): its inside less two pixels all round,
/// and two rings of one-pixel lines that round its corners.
fn draw_box(frame: &mut Frame, r: Rect, rgb: [u8; 3]) {
    let (left, top) = (r.x.saturating_add(2), r.y.saturating_add(2));
    let (right, bottom) = (
        r.x.saturating_add(r.width - 2),
        r.y.saturating_add(r.height - 2),
    );
    frame.fill(Rect::new(left, top, right - left, bottom - top), rgb);
    for (ring, inset) in [(1, 1), (2, 3)] {
        let span = |from: i32, to: i32| (from.saturating_add(inset), to.saturating_sub(inset));
        let (x0, x1) = span(left, right);
        let (y0, y1) = span(top, bottom);
        frame.fill(Rect::new(x0, top - ring, x1 - x0, 1), rgb);
        frame.fill(Rect::new(x0, bottom + ring - 1, x1 - x0, 1), rgb);
        frame.fill(Rect::new(left - ring, y0, 1, y1 - y0), rgb);
        frame.fill(Rect::new(right + ring - 1, y0, 1, y1 - y0), rgb);
    }
}

/// A label's box and its text, 3 pixels in (0x5eafda).
fn draw_label(frame: &mut Frame, font: &Font, label: &Label, rgb: [u8; 3]) {
    draw_box(frame, label.rect, BOX);
    font.draw(
        frame,
        &label.text,
        [
            label.rect.x.saturating_add(INSET),
            label.rect.y.saturating_add(INSET),
        ],
        rgb,
    );
}

/// Draws the menu (0x5eb04c): the head in the middle, the title, then
/// each enabled entry. The `highlighted` entry is white, or cyan while a
/// mouse button is `pressed`; the others are pale blue.
pub fn draw(
    frame: &mut Frame,
    art: &Art,
    layout: &Layout,
    head: Option<&Image>,
    highlighted: Option<usize>,
    pressed: bool,
) {
    // The face bitmap is all colour key in the game's art; the head is
    // rendered into it.
    for image in [art.image(FACE), head].into_iter().flatten() {
        frame.draw_all(image, [layout.face.x, layout.face.y]);
    }
    let Some(font) = art.font(FONT) else {
        return;
    };
    if let Some(title) = &layout.title {
        draw_label(frame, font, title, PRESSED);
    }
    for (i, label) in layout.labels.iter().enumerate() {
        if !label.enabled {
            continue;
        }
        let rgb = match (highlighted == Some(i), pressed) {
            (true, true) => PRESSED,
            (true, false) => HIGHLIGHT,
            (false, _) => TEXT,
        };
        draw_label(frame, font, label, rgb);
    }
}

/// An entry of the picker's menu: a name the player can choose, or a
/// submenu holding the names that start with its label and a `/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// Choosing it chooses `choice`, the index of its name in the list
    /// given to [`group`].
    Choice { label: String, choice: usize },
    Submenu {
        label: String,
        /// Each entry's label (the name after the first `/`) and choice.
        entries: Vec<(String, usize)>,
    },
}

impl Node {
    pub fn label(&self) -> &str {
        match self {
            Self::Choice { label, .. } | Self::Submenu { label, .. } => label,
        }
    }
}

/// Groups action names as the picker does (0x48d56c): a name with a `/`
/// goes into the submenu named by the text before its first `/`, made
/// where the first such name is; the rest of the name, further `/`s and
/// all, is its label there. Submenu names match exactly, case and all
/// (`strcmp`). Other names are entries of their own.
pub fn group<S: AsRef<str>>(names: &[S]) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
    for (choice, name) in names.iter().enumerate() {
        let name = name.as_ref();
        let Some((head, rest)) = name.split_once('/') else {
            nodes.push(Node::Choice {
                label: name.to_owned(),
                choice,
            });
            continue;
        };
        let existing = nodes.iter_mut().find_map(|n| match n {
            Node::Submenu { label, entries } if label == head => Some(entries),
            _ => None,
        });
        match existing {
            Some(entries) => entries.push((rest.to_owned(), choice)),
            None => nodes.push(Node::Submenu {
                label: head.to_owned(),
                entries: vec![(rest.to_owned(), choice)],
            }),
        }
    }
    nodes
}

/// What a press on an open [`Pie`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressed {
    /// The player chose this name (its index in the list given to
    /// [`Pie::new`]). The menu is closed.
    Chose(usize),
    /// A submenu opened, centered where the original moves the pointer.
    Opened { center: [i32; 2] },
    /// The press was on the submenu's head: back to the first menu, at
    /// the place it opened.
    Back,
    /// The press was elsewhere: the menu is closed.
    Closed,
}

/// The picker's menu while it is open: the first menu, and the submenu
/// the player opened from it, if any. Pure state: the host keeps it,
/// lays it out and draws it each frame, and hands it presses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pie {
    pub nodes: Vec<Node>,
    /// Where the first menu was asked to open.
    pub at: [i32; 2],
    /// The open submenu: its node and where it was asked to open.
    pub submenu: Option<(usize, [i32; 2])>,
}

impl Pie {
    /// A menu of `names` (each action's name, in the menu's order) at
    /// `at`, the pointer where the player clicked.
    pub fn new<S: AsRef<str>>(names: &[S], at: [i32; 2]) -> Self {
        Self {
            nodes: group(names),
            at,
            submenu: None,
        }
    }

    /// The menus on the screen, the first menu first. While a submenu is
    /// open, the first menu stays with its entries hidden (0x48dbe1) and
    /// the submenu, titled with its name, opens over it.
    pub fn layout(&self, screen: [i32; 2], art: &Art) -> Option<Vec<Layout>> {
        let open = self.submenu.is_some();
        let entries: Vec<Entry> = self
            .nodes
            .iter()
            .map(|n| Entry {
                label: n.label().to_owned(),
                enabled: !open,
            })
            .collect();
        let mut out = vec![layout(screen, self.at, "", &entries, art)?];
        if let Some((n, at)) = self.submenu
            && let Some(Node::Submenu { label, entries }) = self.nodes.get(n)
        {
            let entries: Vec<Entry> = entries
                .iter()
                .map(|(label, _)| Entry {
                    label: label.clone(),
                    enabled: true,
                })
                .collect();
            out.push(layout(screen, at, label, &entries, art)?);
        }
        Some(out)
    }

    /// The entry a pointer at `point` highlights in the top menu of
    /// `layouts` (from [`Pie::layout`]).
    pub fn hit(layouts: &[Layout], point: [i32; 2]) -> Option<usize> {
        hit(layouts.last()?, point)
    }

    /// The choice whose entry a pointer at `point` highlights in the top
    /// menu of `layouts`: the one a press there would choose. An entry
    /// that opens a submenu is no choice.
    pub fn choice_at(&self, layouts: &[Layout], point: [i32; 2]) -> Option<usize> {
        let i = Self::hit(layouts, point)?;
        match self.submenu {
            Some((n, _)) => match self.nodes.get(n) {
                Some(Node::Submenu { entries, .. }) => entries.get(i).map(|e| e.1),
                _ => None,
            },
            None => match self.nodes.get(i) {
                Some(Node::Choice { choice, .. }) => Some(*choice),
                _ => None,
            },
        }
    }

    /// A press at `point`, as the picker takes the menu's answer
    /// (0x48dac2 to 0x48dc5e).
    pub fn press(&mut self, layouts: &[Layout], point: [i32; 2]) -> Pressed {
        let Some(top) = layouts.last() else {
            return Pressed::Closed;
        };
        let hit = hit(top, point);
        if let Some((n, _)) = self.submenu {
            let choice = hit.and_then(|i| match self.nodes.get(n) {
                Some(Node::Submenu { entries, .. }) => entries.get(i).map(|e| e.1),
                _ => None,
            });
            if let Some(choice) = choice {
                return Pressed::Chose(choice);
            }
            // A press within the dead zone goes back (0x48db04).
            let dx = f64::from(point[0]) - f64::from(top.center[0]);
            let dy = f64::from(point[1]) - f64::from(top.center[1]);
            if (dx * dx + dy * dy).sqrt() as f32 <= DEAD_ZONE as f32 {
                self.submenu = None;
                return Pressed::Back;
            }
            return Pressed::Closed;
        }
        match hit.and_then(|i| Some((i, self.nodes.get(i)?))) {
            Some((_, Node::Choice { choice, .. })) => Pressed::Chose(*choice),
            Some((i, Node::Submenu { .. })) => {
                // Centered on the entry that opened it (0x48d9c0).
                let r = top.labels[i].rect;
                let center = [r.x + r.width / 2, r.y + r.height / 2];
                self.submenu = Some((i, center));
                Pressed::Opened { center }
            }
            None => Pressed::Closed,
        }
    }

    /// Draws the menus of `layouts` with the head in each middle, the
    /// pointer at `pointer` and a mouse button held when `pressed`.
    pub fn draw(
        frame: &mut Frame,
        art: &Art,
        layouts: &[Layout],
        head: Option<&Image>,
        pointer: Option<[i32; 2]>,
        pressed: bool,
    ) {
        for (n, layout) in layouts.iter().enumerate() {
            let highlighted = if n + 1 == layouts.len() {
                pointer.and_then(|p| hit(layout, p))
            } else {
                None
            };
            draw(frame, art, layout, head, highlighted, pressed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font whose every glyph is `w` wide and whose lines are 23 high,
    /// as `variablesans_12`'s are.
    fn art() -> Art {
        let glyphs = (32u16..127)
            .map(|code| sims_formats::ffn::Glyph {
                code,
                width: 4,
                height: 23,
                atlas_x: 0,
                atlas_y: 0,
                first_bearing: 0,
                bearing_x: 0,
                bearing_y: 0,
            })
            .collect();
        let mut art = Art::default();
        art.insert_font(
            FONT,
            Font::new(sims_formats::ffn::Font {
                flags: 9,
                glyphs,
                width: 4,
                height: 23,
                coverage: vec![15; 4 * 23],
            }),
        );
        art
    }

    fn entries(labels: &[&str]) -> Vec<Entry> {
        labels
            .iter()
            .map(|l| Entry {
                label: (*l).into(),
                enabled: true,
            })
            .collect()
    }

    #[test]
    fn places_go_clockwise_from_the_top() {
        let art = art();
        // Eight labels of 4 characters: boxes 22 by 29.
        let names = ["AAAA"; 8];
        let l = layout([2000, 2000], [1000, 1000], "", &entries(&names), &art).unwrap();
        let rel: Vec<[i32; 2]> = l
            .labels
            .iter()
            .map(|label| [label.rect.x - 1000, label.rect.y - 1000])
            .collect();
        assert_eq!(
            rel,
            [
                [-11, -89],
                [30, -59],
                [60, -14],
                [30, 30],
                [-11, 60],
                [-52, 30],
                [-82, -14],
                [-52, -59]
            ]
        );
        // The face sets the window: 100 + 8 + 4 left of the middle.
        assert_eq!(l.window, Rect::new(888, 888, 225, 225));
        assert_eq!(l.face, Rect::new(900, 900, 201, 201));
        // Two entries go above and below, four round the sides too.
        let two = layout([2000, 2000], [1000, 1000], "", &entries(&["A", "B"]), &art).unwrap();
        assert_eq!(two.labels[1].rect.y - 1000, RADIUS);
        let three = layout([2000, 2000], [1000, 1000], "", &entries(&["A"; 3]), &art).unwrap();
        assert_eq!(three.labels[1].rect.x - 1000, RADIUS);
        assert_eq!(three.labels[2].rect.y - 1000, RADIUS);
    }

    #[test]
    fn later_entries_stack_above_and_below() {
        let art = art();
        let l = layout([2000, 2000], [1000, 1000], "", &entries(&["A"; 12]), &art).unwrap();
        assert_eq!(l.labels.len(), 12);
        let top = l.labels[0].rect;
        let bottom = l.labels[4].rect;
        assert_eq!(l.labels[8].rect.y, top.y - 29);
        assert_eq!(l.labels[9].rect.y, bottom.y + 29);
        assert_eq!(l.labels[10].rect.y, top.y - 58);
        assert_eq!(l.labels[11].rect.y, bottom.y + 58);
        // A thirteenth is not shown.
        let more = layout([2000, 2000], [1000, 1000], "", &entries(&["A"; 13]), &art).unwrap();
        assert_eq!(more.labels.len(), SHOWN);
    }

    #[test]
    fn the_window_stays_on_the_screen() {
        let art = art();
        let l = layout([800, 600], [10, 590], "", &entries(&["A"]), &art).unwrap();
        assert!(l.moved);
        assert_eq!((l.window.x, l.window.y), (0, 600 - l.window.height));
        assert_eq!(l.center, [112, 600 - l.window.height + 112]);
        let fine = layout([800, 600], [400, 300], "", &entries(&["A"]), &art).unwrap();
        assert!(!fine.moved);
        assert_eq!(fine.center, [400, 300]);
    }

    #[test]
    fn the_pointer_highlights_by_box_then_by_direction() {
        let art = art();
        let l = layout([2000, 2000], [1000, 1000], "", &entries(&["A"; 4]), &art).unwrap();
        // On the right entry's box.
        let r = l.labels[1].rect;
        assert_eq!(hit(&l, [r.x + 1, r.y + 1]), Some(1));
        // Pointing left, between the dead zone and the reach.
        assert_eq!(hit(&l, [1000 - 100, 1000]), Some(3));
        assert_eq!(hit(&l, [1000, 1000 + 100]), Some(2));
        // Too near or too far.
        assert_eq!(hit(&l, [1000 + 30, 1000]), None);
        assert_eq!(hit(&l, [1000 + 151, 1000]), None);
        assert_eq!(hit(&l, [1000, 1000]), None);
        // A disabled entry is never highlighted.
        let mut off = entries(&["A"; 4]);
        off[3].enabled = false;
        let l = layout([2000, 2000], [1000, 1000], "", &off, &art).unwrap();
        assert_eq!(hit(&l, [1000 - 100, 1000]), None);
    }

    #[test]
    fn names_with_a_slash_make_submenus() {
        let nodes = group(&["Talk", "Ask/Dance", "Hug", "Ask/Date/Now", "ask/x"]);
        assert_eq!(
            nodes,
            [
                Node::Choice {
                    label: "Talk".into(),
                    choice: 0
                },
                Node::Submenu {
                    label: "Ask".into(),
                    entries: vec![("Dance".into(), 1), ("Date/Now".into(), 3)]
                },
                Node::Choice {
                    label: "Hug".into(),
                    choice: 2
                },
                Node::Submenu {
                    label: "ask".into(),
                    entries: vec![("x".into(), 4)]
                },
            ]
        );
    }

    #[test]
    fn a_submenu_opens_on_its_entry_and_its_head_goes_back() {
        let art = art();
        let mut pie = Pie::new(&["Talk", "Ask/Dance", "Ask/Out"], [1000, 1000]);
        let layouts = pie.layout([2000, 2000], &art).unwrap();
        let ask = layouts[0].labels[1].rect;
        let opened = pie.press(&layouts, [ask.x + 1, ask.y + 1]);
        let center = [ask.x + ask.width / 2, ask.y + ask.height / 2];
        assert_eq!(opened, Pressed::Opened { center });
        let layouts = pie.layout([2000, 2000], &art).unwrap();
        assert_eq!(layouts.len(), 2);
        assert!(layouts[0].labels.iter().all(|l| !l.enabled));
        assert_eq!(layouts[1].title.as_ref().unwrap().text, "Ask");
        let out = layouts[1].labels[1].rect;
        assert_eq!(
            pie.clone().press(&layouts, [out.x + 1, out.y + 1]),
            Pressed::Chose(2)
        );
        assert_eq!(pie.clone().press(&layouts, [0, 0]), Pressed::Closed);
        assert_eq!(pie.press(&layouts, layouts[1].center), Pressed::Back);
        assert_eq!(pie.submenu, None);
    }

    #[test]
    fn boxes_have_rounded_corners() {
        let mut frame = Frame::new(12, 12, [0; 3]);
        draw_box(&mut frame, Rect::new(0, 0, 12, 12), [9; 3]);
        // The corner pixels stay, the edges' middles are filled.
        assert_eq!(frame.pixel(0, 0), Some([0; 3]));
        assert_eq!(frame.pixel(4, 0), Some([0; 3]));
        assert_eq!(frame.pixel(5, 0), Some([9; 3]));
        assert_eq!(frame.pixel(1, 1), Some([0; 3]));
        assert_eq!(frame.pixel(1, 3), Some([9; 3]));
        assert_eq!(frame.pixel(11, 6), Some([9; 3]));
        assert_eq!(frame.pixel(6, 11), Some([9; 3]));
        assert_eq!(frame.pixel(11, 11), Some([0; 3]));
    }

    #[test]
    fn odd_inputs_do_not_panic() {
        let art = art();
        let mut frame = Frame::new(10, 10, [0; 3]);
        for at in [[i32::MIN, i32::MIN], [i32::MAX, i32::MAX], [0, 0]] {
            for screen in [[0, 0], [-5, -5], [i32::MAX, i32::MAX], [i32::MIN, 600]] {
                let l = layout(screen, at, "T", &entries(&["A"; 13]), &art).unwrap();
                draw(&mut frame, &art, &l, None, Some(0), true);
                let _ = hit(&l, at);
            }
        }
        assert!(layout([1, 1], [0, 0], "", &[], &Art::default()).is_none());
        let mut pie = Pie::new(&["Ask/Dance"], [400, 300]);
        let layouts = pie.layout([800, 600], &art).unwrap();
        let ask = layouts[0].labels[0].rect;
        pie.press(&layouts, [ask.x + 1, ask.y + 1]);
        let layouts = pie.layout([800, 600], &art).unwrap();
        for far in [[i32::MIN, i32::MIN], [i32::MAX, i32::MAX]] {
            assert_eq!(pie.clone().press(&layouts, far), Pressed::Closed);
        }
    }
}
