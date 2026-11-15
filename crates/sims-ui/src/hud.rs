//! The live-mode control panel: the cluster of mode, camera and speed
//! controls at the bottom left, and the strip beside it with the
//! household's portraits, the mood gauge and the needs.
//!
//! The positions are the original's at 800 by 600. The old prototype
//! captured them from the running game's windows
//! (legacy/crates/sims-app/src/hud_cluster.rs and original_hud.rs); the
//! rest were measured on a screenshot of the original (see
//! `docs/decisions.md`). At another size the cluster keeps to the bottom
//! left, the strip runs from it to the right edge, the needs keep to the
//! right edge and the strip's background stretches its last column, as
//! the original does when its window is wider than 800.

use crate::art::Art;
use crate::command::{Camera, Command, Mode, Panel, Speed, Walls};
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::household::{Faces, Snapshot};
use sims_formats::bitmap::Image;

/// The cluster's size, the size of its background (kUniversalBkgTGA).
pub const CLUSTER: [i32; 2] = [220, 183];
/// The strip's height in live mode, and in buy mode (kTallSubpanel).
pub const STRIP: i32 = 100;
pub const TALL_STRIP: i32 = 150;
/// How far from the strip's right end the needs gadget starts: its 108
/// pixels and the needs panel after it. Measured on the original's screen
/// (the old prototype used 388).
pub const NEEDS_WIDTH: i32 = 387;

/// Resources the panel draws (`UIGraphics/Res_CPanel.h`).
pub mod id {
    pub const CLUSTER: i32 = 2037;
    pub const STRIP: i32 = 4910;
    pub const GAUGE: i32 = 4911;
    pub const LIVE: i32 = 2010;
    pub const BUY: i32 = 2008;
    pub const BUILD: i32 = 2003;
    pub const OPTIONS: i32 = 2009;
    pub const ROTATE_RIGHT: i32 = 2011;
    pub const ROTATE_LEFT: i32 = 2012;
    pub const ZOOM_IN: i32 = 2013;
    pub const ZOOM_OUT: i32 = 2014;
    pub const FLOOR_DOWN: i32 = 2023;
    pub const FLOOR_UP: i32 = 2024;
    pub const PAUSE: i32 = 2026;
    pub const WALLS_DOWN: i32 = 2007;
    pub const CUTAWAY: i32 = 2004;
    pub const WALLS_UP: i32 = 2006;
    pub const ROOF: i32 = 2025;
    pub const PHOTO: i32 = 2034;
    pub const NEEDS: i32 = 4500;
    pub const PERSONALITY: i32 = 4501;
    pub const HOUSE: i32 = 4502;
    pub const JOB: i32 = 4503;
    pub const RELATIONSHIPS: i32 = 4504;
    pub const INTERESTS: i32 = 4505;
    pub const GREEN_BARS: i32 = 4506;
    pub const GIFTS: i32 = 4507;
    pub const RED_BARS: i32 = 4510;
    /// A personality bar, full and empty (0x520f67).
    pub const TRAIT_FULL: i32 = 4900;
    pub const TRAIT_EMPTY: i32 = 4903;
    pub const TEMPLATE: i32 = crate::household::TEMPLATE;
    pub const NORMAL: i32 = 4705;
    pub const FAST: i32 = 4706;
    pub const ULTRA: i32 = 4707;
    pub const LIVE_TRIM: i32 = 4803;
    pub const BUY_TRIM: i32 = 4804;
    pub const BUILD_TRIM: i32 = 4805;
    pub const OPTIONS_TRIM: i32 = 4806;
}

/// Every resource the panel may draw, for [`Art::load`].
pub const RESOURCES: [i32; 39] = [
    id::WALLS_DOWN,
    id::CUTAWAY,
    id::WALLS_UP,
    id::ROOF,
    id::PHOTO,
    id::CLUSTER,
    id::STRIP,
    id::GAUGE,
    id::LIVE,
    id::BUY,
    id::BUILD,
    id::OPTIONS,
    id::ROTATE_RIGHT,
    id::ROTATE_LEFT,
    id::ZOOM_IN,
    id::ZOOM_OUT,
    id::FLOOR_DOWN,
    id::FLOOR_UP,
    id::PAUSE,
    id::NEEDS,
    id::PERSONALITY,
    id::HOUSE,
    id::JOB,
    id::RELATIONSHIPS,
    id::INTERESTS,
    id::GREEN_BARS,
    id::GIFTS,
    id::RED_BARS,
    id::TRAIT_FULL,
    id::TRAIT_EMPTY,
    id::TEMPLATE,
    id::NORMAL,
    id::FAST,
    id::ULTRA,
    id::LIVE_TRIM,
    id::BUY_TRIM,
    id::BUILD_TRIM,
    id::OPTIONS_TRIM,
    4918,
];

/// What the player has open: state of the presentation, not of the
/// simulation. The host keeps it and applies the presentation-only
/// commands to it ([`Controls::apply`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Controls {
    pub mode: Mode,
    /// Whether the strip beside the cluster is open. After a house loads
    /// the original shows it closed until Live is pressed.
    pub expanded: bool,
    pub panel: Panel,
    pub speed: Speed,
    pub camera: CameraState,
    /// The pointer's position, for the buttons' hover cells.
    pub pointer: Option<[i32; 2]>,
}

/// What the camera buttons show. The host's camera owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraState {
    /// The floor shown: 1 or 2.
    pub floor: u8,
    pub walls: Walls,
    pub roof: bool,
    /// Whether the camera can zoom in, or out, any further.
    pub zoom_in: bool,
    pub zoom_out: bool,
}

impl Default for CameraState {
    /// The original after loading the Goths' house: the first floor, walls
    /// cut away, zoomed fully out.
    fn default() -> Self {
        Self {
            floor: 1,
            walls: Walls::Cutaway,
            roof: false,
            zoom_in: true,
            zoom_out: false,
        }
    }
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            mode: Mode::Live,
            expanded: false,
            panel: Panel::Needs,
            speed: Speed::Normal,
            camera: CameraState::default(),
            pointer: None,
        }
    }
}

impl Controls {
    /// Applies a command that changes only what is shown. Commands for
    /// the simulation and the camera are left to the host.
    pub fn apply(&mut self, command: &Command) {
        match command {
            Command::TogglePanel => self.expanded = !self.expanded,
            Command::OpenPanel(panel) => self.panel = *panel,
            _ => {}
        }
    }
}

/// A button's cells: the original keeps four states side by side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cell {
    Normal = 0,
    Selected = 1,
    Hover = 2,
    Disabled = 3,
}

/// One thing the panel draws or the player can press.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub rect: Rect,
    pub kind: Kind,
    /// What pressing it asks for.
    pub command: Option<Command>,
}

/// What an [`Item`] is.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// A whole bitmap.
    Image(i32),
    /// The strip's background, its last column stretched.
    Stretched(i32),
    /// A cell of a button strip of `cells` cells. A disabled button has
    /// no command.
    Button { id: i32, cells: u32, selected: bool },
    /// The pause button: row 1 of its bitmap when paused.
    Pause { paused: bool },
    /// A resident's portrait.
    Portrait { guid: u32, selected: bool },
    /// Part of a mood bar bitmap.
    Bars { id: i32, source: Rect },
    /// Text in font `size`, aligned in `rect`.
    Text {
        text: String,
        size: u8,
        rgb: [u8; 3],
        align: Align,
    },
    /// A motive meter.
    Meter(f32),
    /// The personality panel's bars, in `rect`, the subpanel: the
    /// selected Sim's traits in the panel's order
    /// ([`crate::household::PERSONALITY`]). Laid out when drawn, from the
    /// labels' font.
    Personality([i16; 5]),
    /// The first name of the person of type `guid`, in a box centered
    /// over `rect`'s bottom edge, while the pointer is on their portrait.
    Name { guid: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// The clock's and the funds' color, and the needs panel's: 16-bit colors
/// measured on the original's screen (the second is also the color the
/// old prototype captured for tutorial text).
const GREEN: [u8; 3] = rgb565(0x2f2b);
pub(crate) const LABEL: [u8; 3] = rgb565(0xc679);

/// A 16-bit color widened to 8 bits a channel by repeating the high bits.
pub(crate) const fn rgb565(c: u16) -> [u8; 3] {
    let (r, g, b) = ((c >> 11) as u8, ((c >> 5) & 63) as u8, (c & 31) as u8);
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}

/// Lays the panel out in a window of `size`, in drawing order.
pub fn layout(size: [i32; 2], snapshot: &Snapshot, controls: &Controls) -> Vec<Item> {
    // Sizes outside what a window can be are clamped, so no arithmetic
    // below can overflow.
    let [width, height] = size.map(|v| v.clamp(0, 1 << 15));
    let mut items = Vec::new();
    let tall = controls.mode == Mode::Buy;
    let strip_height = if tall { TALL_STRIP } else { STRIP };
    let top = height - strip_height;
    let cluster = [0, height - CLUSTER[1]];
    let at = |x: i32, y: i32, w: i32, h: i32| Rect::new(cluster[0] + x, cluster[1] + y, w, h);
    let image = |rect: Rect, id: i32| Item {
        rect,
        kind: Kind::Image(id),
        command: None,
    };

    // The strip, behind the cluster.
    if controls.expanded {
        let strip = Rect::new(CLUSTER[0], top, width - CLUSTER[0], strip_height);
        items.push(Item {
            rect: strip,
            kind: Kind::Stretched(if tall { 4918 } else { id::STRIP }),
            command: None,
        });
        if controls.mode == Mode::Live {
            live_strip(&mut items, strip, snapshot, controls);
        }
    }

    items.push(image(at(0, 0, CLUSTER[0], CLUSTER[1]), id::CLUSTER));
    // 0x53a0b6 draws one trim over the cluster while the strip is open.
    if controls.expanded {
        let (trim, x, y, w, h) = match controls.mode {
            Mode::Build => (id::BUILD_TRIM, 113, 51, 107, 132),
            Mode::Buy => (id::BUY_TRIM, 64, 35, 156, 148),
            Mode::Live => (id::LIVE_TRIM, 7, 30, 213, 153),
            Mode::Options => (id::OPTIONS_TRIM, 183, 83, 37, 100),
        };
        items.push(image(at(x, y, w, h), trim));
    }
    let button = |id: i32, cells: u32, rect: Rect, selected: bool, command: Command| Item {
        rect,
        kind: Kind::Button {
            id,
            cells,
            selected,
        },
        command: Some(command),
    };
    let mode = |m: Mode| {
        if controls.mode == m {
            Command::TogglePanel
        } else {
            Command::SetMode(m)
        }
    };
    let camera = controls.camera;
    let view = |id: i32, rect: Rect, selected: bool, enabled: bool, c: Camera| Item {
        command: enabled.then_some(Command::Camera(c)),
        ..button(id, 4, rect, selected, Command::Camera(c))
    };
    items.extend([
        button(
            id::LIVE,
            4,
            at(10, 6, 50, 50),
            controls.mode == Mode::Live,
            mode(Mode::Live),
        ),
        button(
            id::BUY,
            4,
            at(69, 20, 45, 47),
            controls.mode == Mode::Buy,
            mode(Mode::Buy),
        ),
        button(
            id::BUILD,
            4,
            at(117, 45, 39, 39),
            controls.mode == Mode::Build,
            mode(Mode::Build),
        ),
        button(
            id::OPTIONS,
            4,
            at(193, 125, 23, 23),
            controls.mode == Mode::Options,
            mode(Mode::Options),
        ),
        // The old prototype had the two rotations the other way round;
        // the original's screen has rotate-right (kRotRight) on the left.
        view(
            id::ROTATE_RIGHT,
            at(9, 127, 27, 27),
            false,
            true,
            Camera::RotateRight,
        ),
        view(
            id::ROTATE_LEFT,
            at(43, 127, 27, 27),
            false,
            true,
            Camera::RotateLeft,
        ),
        view(
            id::ZOOM_IN,
            at(26, 105, 27, 27),
            false,
            camera.zoom_in,
            Camera::ZoomIn,
        ),
        view(
            id::ZOOM_OUT,
            at(26, 149, 27, 27),
            false,
            camera.zoom_out,
            Camera::ZoomOut,
        ),
        view(
            id::FLOOR_UP,
            at(5, 77, 22, 15),
            camera.floor == 2,
            true,
            Camera::FloorUp,
        ),
        view(
            id::FLOOR_DOWN,
            at(5, 92, 21, 11),
            camera.floor == 1,
            true,
            Camera::FloorDown,
        ),
        // Measured on the original's screen: the roof, the three wall
        // views and the snapshot camera.
        view(
            id::ROOF,
            at(32, 78, 22, 19),
            camera.roof,
            true,
            Camera::Roof,
        ),
        view(
            id::WALLS_UP,
            at(56, 80, 21, 21),
            camera.walls == Walls::Up,
            true,
            Camera::Walls(Walls::Up),
        ),
        view(
            id::CUTAWAY,
            at(80, 85, 21, 21),
            camera.walls == Walls::Cutaway,
            true,
            Camera::Walls(Walls::Cutaway),
        ),
        view(
            id::WALLS_DOWN,
            at(103, 95, 22, 15),
            camera.walls == Walls::Down,
            true,
            Camera::Walls(Walls::Down),
        ),
        view(id::PHOTO, at(175, 95, 29, 29), false, true, Camera::Photo),
    ]);
    items.push(Item {
        rect: at(85, 135, 15, 15),
        kind: Kind::Pause {
            paused: snapshot.paused,
        },
        command: Some(Command::TogglePause),
    });
    for (id, speed, x, w) in [
        (id::NORMAL, Speed::Normal, 105, 11),
        (id::FAST, Speed::Fast, 120, 20),
        (id::ULTRA, Speed::Ultra, 141, 32),
    ] {
        items.push(button(
            id,
            4,
            at(x, 135, w, 15),
            controls.speed == speed && !snapshot.paused,
            Command::SetSpeed(speed),
        ));
    }
    let text = |rect: Rect, text: String, size: u8, align: Align| Item {
        rect,
        kind: Kind::Text {
            text,
            size,
            rgb: GREEN,
            align,
        },
        command: None,
    };
    // Fonts and lines measured on the original's screen.
    items.push(text(
        at(75, 118, 100, 14),
        snapshot.clock(),
        8,
        Align::Center,
    ));
    items.push(text(
        at(70, 158, 108, 16),
        snapshot.money(),
        10,
        Align::Right,
    ));
    items
}

/// The live-mode strip: portraits at the left, the gadget and the needs
/// at the right.
fn live_strip(items: &mut Vec<Item>, strip: Rect, snapshot: &Snapshot, controls: &Controls) {
    let gadget = Rect::new(strip.x + strip.width - NEEDS_WIDTH, strip.y, 108, STRIP);
    // The portraits, 45 by 45 from (9, 5) in a strip (the old prototype's
    // padding), in as many columns as fit before the gadget and two rows:
    // four columns at 800 by 600, so a family of eight fits. The second
    // row is **unverified**; people past the two rows are left out.
    let columns = ((gadget.x - strip.x - 9) / 45).max(1);
    let mut named = None;
    for (n, resident) in snapshot.residents.iter().enumerate() {
        let n = i32::try_from(n).unwrap_or(i32::MAX);
        if n >= columns.saturating_mul(2) {
            break;
        }
        let x = strip.x + 9 + 45 * (n % columns);
        let y = strip.y + 5 + 45 * (n / columns);
        let selected = snapshot.selected == Some(resident.object);
        let rect = Rect::new(x, y, 45, 45);
        if controls
            .pointer
            .is_some_and(|[px, py]| rect.contains(px, py))
        {
            named = Some((rect, resident.guid));
        }
        items.push(Item {
            rect,
            kind: Kind::Portrait {
                guid: resident.guid,
                selected,
            },
            command: Some(Command::Select {
                sim: resident.object,
            }),
        });
    }
    items.push(Item {
        rect: gadget,
        kind: Kind::Image(id::GAUGE),
        command: None,
    });
    let selected = snapshot.selected.and_then(|s| snapshot.resident(s));
    if let Some(resident) = selected {
        mood_bars(items, gadget, resident.motives[sims_sim::motives::MOOD]);
    }
    // Measured on the original's screen; the old prototype had the needs
    // button at (4, 33) and the first two rows two and one pixels higher.
    let g = |x: i32, y: i32, w: i32, h: i32| gadget.offset(x, y).with_size(w, h);
    for (id, panel, x, y, w, h) in [
        (id::NEEDS, Panel::Needs, 9, 31, 32, 39),
        (id::INTERESTS, Panel::Interests, 44, 4, 27, 30),
        (id::RELATIONSHIPS, Panel::Relationships, 76, 4, 27, 30),
        (id::PERSONALITY, Panel::Personality, 44, 35, 27, 30),
        (id::JOB, Panel::Job, 76, 35, 27, 30),
        (id::GIFTS, Panel::Gifts, 44, 66, 27, 30),
        (id::HOUSE, Panel::House, 76, 66, 27, 30),
    ] {
        items.push(Item {
            rect: g(x, y, w, h),
            kind: Kind::Button {
                id,
                cells: 4,
                selected: controls.panel == panel,
            },
            command: Some(Command::OpenPanel(panel)),
        });
    }
    // The original shows no needs without a selected Sim.
    if let (Some(resident), Panel::Needs) = (selected, controls.panel) {
        let panel = Rect::new(gadget.x + 108, strip.y, NEEDS_WIDTH - 108, STRIP);
        // The title, labels and meters, measured on the original's
        // screen: meters 60 by 5 in two columns 100 apart, rows 19 apart,
        // each label centered 15 above its meter.
        items.push(Item {
            rect: panel.offset(6, 2).with_size(100, 20),
            kind: Kind::Text {
                text: "Needs".into(),
                size: 11,
                rgb: LABEL,
                align: Align::Left,
            },
            command: None,
        });
        for (n, (name, motive)) in NEEDS.iter().enumerate() {
            let (column, row) = ((n % 2) as i32, (n / 2) as i32);
            let x = panel.x + 19 + 100 * column;
            let y = panel.y + 33 + 19 * row;
            items.push(Item {
                rect: Rect::new(x, y - 15, 60, 14),
                kind: Kind::Text {
                    text: (*name).into(),
                    size: 8,
                    rgb: LABEL,
                    align: Align::Center,
                },
                command: None,
            });
            items.push(Item {
                rect: Rect::new(x, y, 60, 5),
                kind: Kind::Meter(resident.motives[*motive]),
                command: None,
            });
        }
    }
    if let (Some(resident), Panel::Personality) = (selected, controls.panel) {
        let panel = Rect::new(gadget.x + 108, strip.y, NEEDS_WIDTH - 108, STRIP);
        items.push(Item {
            rect: panel.offset(6, 2).with_size(100, 20),
            kind: Kind::Text {
                text: "Personality".into(),
                size: 11,
                rgb: LABEL,
                align: Align::Left,
            },
            command: None,
        });
        items.push(Item {
            rect: panel,
            kind: Kind::Personality(resident.personality),
            command: None,
        });
    }
    // The name of the person whose portrait the pointer is on, over the
    // strip above the portrait. Ours: where and how the original shows a
    // name there is **unverified**.
    if let Some((rect, guid)) = named {
        items.push(Item {
            rect: Rect::new(rect.x, strip.y - NAME_HEIGHT - 2, rect.width, NAME_HEIGHT),
            kind: Kind::Name { guid },
            command: None,
        });
    }
}

/// The name item's height, the font size, the room around the name in
/// its box (across, then down), and the box's colors: the needs panel's label color on
/// black. The box drawn is as tall as the font's line needs; its bottom
/// is the item's.
const NAME_HEIGHT: i32 = 16;
const NAME_SIZE: u8 = 8;
const NAME_PAD: [i32; 2] = [4, 2];
const NAME_BACK: [u8; 3] = [0, 0, 0];

/// The personality's labels in the panel's order (Live.iff STR# 131).
const TRAITS: [&str; 5] = ["Neat", "Outgoing", "Active", "Playful", "Nice"];
/// The labels' font (0x6f2ae4, font 8).
const TRAIT_FONT: u8 = 8;
/// Where the bars end and start in the subpanel, and how far apart they
/// are (0x519cc8, not the CJK languages').
const TRAITS_RIGHT: i32 = 202;
const TRAITS_TOP: i32 = 8;
const TRAITS_PITCH: i32 = 18;

/// Draws the personality bars (0x51df31, 0x521491, 0x5211bd) in the
/// subpanel `panel`. Each bar is a label and an empty bar with the full
/// one drawn over it, cut to 4 pixels a point. Every bar's label is as
/// wide as the widest, plus 3, and its bar starts 3 after that; the bars
/// end at [`TRAITS_RIGHT`]. The shorter of the label's line and the bar
/// is centred on the other. A trait is `value / 100` points, 0 to 10.
///
/// **Unverified:** the labels' color (the needs' is taken) and their
/// alignment (right, against the bar); which of the font's heights the
/// original compares (the line's is taken).
fn draw_personality(frame: &mut Frame, art: &Art, panel: Rect, values: &[i16; 5]) {
    let (Some(font), Some(empty), Some(full)) = (
        art.font(TRAIT_FONT),
        art.image(id::TRAIT_EMPTY),
        art.image(id::TRAIT_FULL),
    ) else {
        return;
    };
    let size = |image: &Image| [image.width, image.height].map(|v| i32::try_from(v).unwrap_or(0));
    let ([empty_w, empty_h], [full_w, full_h]) = (size(empty), size(full));
    let widest = TRAITS.iter().map(|t| font.width(t)).max().unwrap_or(0);
    let label = widest.saturating_add(3);
    let bar_x = label.saturating_add(3);
    let width = bar_x.saturating_add(empty_w).saturating_add(3);
    let line = font.line_height;
    let (text_y, bar_y) = if line < empty_h {
        ((empty_h - line) / 2, 0)
    } else {
        (0, (line - empty_h) / 2)
    };
    let left = panel.x.saturating_add(TRAITS_RIGHT.saturating_sub(width));
    for (n, (name, value)) in TRAITS.iter().zip(values).enumerate() {
        let top = panel.y.saturating_add(TRAITS_TOP + TRAITS_PITCH * n as i32);
        let x = left.saturating_add(label.saturating_sub(font.width(name)));
        font.draw(frame, name, [x, top.saturating_add(text_y)], LABEL);
        let at = [left.saturating_add(bar_x), top.saturating_add(bar_y)];
        frame.draw_all(empty, at);
        let points = i32::from(*value / 100).clamp(0, 10);
        if points > 0 {
            // The full bar goes a pixel above the empty one (0x5212b2).
            let source = Rect::new(0, 0, (points * 4).min(full_w), full_h);
            frame.draw(full, source, [at[0], at[1].saturating_sub(1)]);
        }
    }
}

/// The needs in the panel's order, left column first, with their motive
/// numbers (`crates/sims-sim/src/motives.rs`). The order is the
/// original's screen.
const NEEDS: [(&str, usize); 8] = [
    ("Hunger", 7),
    ("Energy", 5),
    ("Comfort", 6),
    ("Fun", 15),
    ("Hygiene", 8),
    ("Social", 14),
    ("Bladder", 9),
    ("Room", 13),
];

/// The mood gauge's bars (0x51b366): the mood, rounded, becomes -5 to 5
/// bars, green rising from the middle or red falling from it. The scale
/// is the original's float constant (0x3d6028c2, about 0.0547), read by
/// the old prototype (legacy/crates/sims-app/src/original_hud.rs).
fn mood_bars(items: &mut Vec<Item>, gadget: Rect, mood: f32) {
    let Some(n) = mood_segments(mood) else {
        return;
    };
    let bars = |id: i32, source: Rect, y: i32| Item {
        rect: Rect::new(gadget.x + 12, gadget.y + y, source.width, source.height),
        kind: Kind::Bars { id, source },
        command: None,
    };
    if n > 0 {
        let top = 25 - n * 5;
        items.push(bars(
            id::GREEN_BARS,
            Rect::new(0, top, 27, 25 - top),
            3 + top,
        ));
    } else if n < 0 {
        items.push(bars(id::RED_BARS, Rect::new(0, 0, 27, -n * 5), 73));
    } else {
        items.push(bars(id::GREEN_BARS, Rect::new(0, 23, 27, 2), 26));
        items.push(bars(id::RED_BARS, Rect::new(0, 0, 27, 2), 73));
    }
}

/// The number of mood bars, -5 to 5, or None for a mood that is not a
/// number.
pub fn mood_segments(mood: f32) -> Option<i32> {
    let mood = f64::from(mood).round();
    if !mood.is_finite() {
        return None;
    }
    let scale = f64::from(f32::from_bits(0x3d60_28c2));
    Some(((mood + 100.) * scale - 5.).round().clamp(-5., 5.) as i32)
}

/// A motive meter's look (0x5134bf and its palette 0x5139dc, as the old
/// prototype read them: legacy/crates/sims-app/src/original_motives.rs).
/// The filled part is `width` of 60 pixels; its color runs from red to
/// green with the value, and the rest from grey to red.
fn meter(value: f32) -> (i32, [u8; 3], [u8; 3]) {
    let value = if value.is_finite() {
        value.clamp(-100., 100.).trunc() as i32
    } else {
        0
    };
    let mix = |end: [u8; 3], t: f64| {
        let step = (t * 255. + 0.5).trunc() as u8;
        let factor = f64::from(step) * f64::from(1.0f32 / 255.);
        let start = [64., 93., 95.];
        std::array::from_fn(|i| {
            (start[i] + (f64::from(end[i]) - start[i]) * factor + 0.5).trunc() as u8
        })
    };
    let factor = f64::from(value + 50) * f64::from(0.01f32);
    let width = ((f64::from(value + 100) * f64::from(0.3f32) + 0.5).trunc() as i32).min(60);
    let t = f64::from((factor as f32).clamp(0., 1.));
    (
        width,
        mix([0, 202, 57], t),
        mix([209, 0, 0], 1. - factor.clamp(0., 1.)),
    )
}

trait WithSize {
    fn with_size(self, width: i32, height: i32) -> Self;
}

impl WithSize for Rect {
    fn with_size(self, width: i32, height: i32) -> Self {
        Rect::new(self.x, self.y, width, height)
    }
}

/// Draws the panel into `frame`. Pieces whose art is missing are left
/// out.
pub fn draw(frame: &mut Frame, art: &Art, faces: &Faces, items: &[Item], controls: &Controls) {
    for item in items {
        let r = item.rect;
        match &item.kind {
            Kind::Image(id) => {
                if let Some(image) = art.image(*id) {
                    frame.draw_all(image, [r.x, r.y]);
                }
            }
            Kind::Stretched(id) => {
                if let Some(image) = art.image(*id) {
                    let w = i32::try_from(image.width).unwrap_or(0);
                    let h = i32::try_from(image.height).unwrap_or(0);
                    frame.draw(image, Rect::new(0, 0, w.min(r.width), h), [r.x, r.y]);
                    // The last column repeated across the part of the
                    // item that is inside the frame.
                    let left = i64::from(r.x);
                    let start = i64::from(w).max(-left);
                    let end = i64::from(r.width).min(i64::from(frame.width()) - left);
                    for x in start..end {
                        frame.draw(image, Rect::new(w - 1, 0, 1, h), [(left + x) as i32, r.y]);
                    }
                }
            }
            Kind::Button {
                id,
                cells,
                selected,
            } => {
                let hover = controls.pointer.is_some_and(|[x, y]| r.contains(x, y));
                let cell = if *selected {
                    Cell::Selected
                } else if item.command.is_none() {
                    Cell::Disabled
                } else if hover {
                    Cell::Hover
                } else {
                    Cell::Normal
                };
                if let Some(image) = art.cell(*id, *cells, cell as u32) {
                    frame.draw_all(&image, [r.x, r.y]);
                }
            }
            Kind::Pause { paused } => {
                if let Some(image) = art.image(id::PAUSE) {
                    let row = if *paused { 15 } else { 0 };
                    frame.draw(image, Rect::new(0, row, 15, 15), [r.x, r.y]);
                }
            }
            Kind::Portrait { guid, selected } => {
                let cell = if *selected {
                    Cell::Selected
                } else {
                    Cell::Normal
                };
                if let Some(portrait) = faces.portrait(art, *guid, cell as u32) {
                    frame.draw_all(&portrait, [r.x, r.y]);
                }
            }
            Kind::Bars { id, source } => {
                if let Some(image) = art.image(*id) {
                    frame.draw(image, *source, [r.x, r.y]);
                }
            }
            Kind::Text {
                text,
                size,
                rgb,
                align,
            } => {
                if let Some(font) = art.font(*size) {
                    let (x, width, w) = (
                        i64::from(r.x),
                        i64::from(r.width),
                        i64::from(font.width(text)),
                    );
                    let x = match align {
                        Align::Left => x,
                        // Odd space is split with the extra pixel on the
                        // left, as the original's labels are.
                        Align::Center => x + (width - w + 1).div_euclid(2),
                        Align::Right => x + width - w,
                    };
                    let x = x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
                    font.draw(frame, text, [x, r.y], *rgb);
                }
            }
            Kind::Name { guid } => {
                if let (Some(name), Some(font)) = (faces.name(*guid), art.font(NAME_SIZE)) {
                    draw_name(frame, font, name, r);
                }
            }
            Kind::Personality(values) => draw_personality(frame, art, r, values),
            Kind::Meter(value) => {
                let (width, left, right) = meter(*value);
                frame.fill(r, right);
                frame.fill(r.with_size(width, r.height), left);
                // 0x51376e draws the divider only with two pixels left.
                if width > 0 && 60 - width > 1 {
                    let x = r.x.saturating_add(width - 1);
                    frame.fill(Rect::new(x, r.y, 1, r.height), [0, 0, 0]);
                }
            }
        }
    }
}

/// Draws `name` in a box centred over `r`, its bottom on `r`'s bottom,
/// as tall as the font's line with [`NAME_PAD`] around it, and kept
/// inside the frame; a name wider than the frame is cut short.
fn draw_name(frame: &mut Frame, font: &Font, name: &str, r: Rect) {
    let (right, bottom) = (i64::from(frame.width()), i64::from(frame.height()));
    let [pad_x, pad_y] = NAME_PAD.map(i64::from);
    let room = right - 2 * pad_x;
    if room <= 0 {
        return;
    }
    let mut text = name.to_owned();
    while i64::from(font.width(&text)) > room {
        text.pop();
    }
    let width = i64::from(font.width(&text)) + 2 * pad_x;
    let height = i64::from(font.line_height.max(0)) + 2 * pad_y;
    let x = i64::from(r.x) + (i64::from(r.width) - width).div_euclid(2);
    let y = i64::from(r.y) + i64::from(r.height) - height;
    let x = x.clamp(0, (right - width).max(0));
    let y = y.clamp(0, (bottom - height).max(0));
    // Inside the frame now, so each fits.
    let [x, y, width, height] = [x, y, width, height].map(|v| i32::try_from(v).unwrap_or(i32::MAX));
    frame.fill(Rect::new(x, y, width, height), LABEL);
    frame.fill(Rect::new(x + 1, y + 1, width - 2, height - 2), NAME_BACK);
    font.draw(frame, &text, [x + NAME_PAD[0], y + NAME_PAD[1]], LABEL);
}

impl Item {
    /// Whether the item stops a press from reaching what is under it.
    /// Text, mood bars and meters are drawn over the art and let presses
    /// through; art and buttons, disabled ones too, stop them.
    fn takes_presses(&self) -> bool {
        self.command.is_some()
            || !matches!(
                self.kind,
                Kind::Text { .. }
                    | Kind::Bars { .. }
                    | Kind::Meter(_)
                    | Kind::Name { .. }
                    | Kind::Personality(_)
            )
    }
}

/// The topmost item at `point` that takes presses.
fn topmost(items: &[Item], [x, y]: [i32; 2]) -> Option<usize> {
    items
        .iter()
        .rposition(|i| i.takes_presses() && i.rect.contains(x, y))
}

/// What a press at `point` asks for: the command of the topmost item
/// there, if it has one. An item drawn over a button hides it.
pub fn press(items: &[Item], point: [i32; 2]) -> Option<Command> {
    items.get(topmost(items, point)?)?.command.clone()
}

/// Whether the panel covers `point`, so a press there is the panel's and
/// not the lot's.
pub fn covers(items: &[Item], point: [i32; 2]) -> bool {
    // A name box floats over the lot and takes nothing from it.
    items
        .iter()
        .any(|i| !matches!(i.kind, Kind::Name { .. }) && i.rect.contains(point[0], point[1]))
}

/// The item under `point` that a press would act on, for hover changes.
pub fn hovered(items: &[Item], point: Option<[i32; 2]>) -> Option<usize> {
    let n = topmost(items, point?)?;
    items[n].command.is_some().then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::household::Resident;

    fn snapshot() -> Snapshot {
        Snapshot {
            hour: 6,
            minute: 30,
            funds: 2542,
            paused: false,
            residents: [21, 18, 16]
                .into_iter()
                .map(|object| Resident {
                    object,
                    guid: u32::from(object),
                    motives: [50.; 16],
                    personality: [0; 5],
                })
                .collect(),
            selected: Some(21),
        }
    }

    #[test]
    fn mood_segments_and_meters_follow_the_original() {
        assert_eq!(mood_segments(81.9), Some(5));
        assert_eq!(mood_segments(0.), Some(0));
        assert_eq!(mood_segments(-100.), Some(-5));
        assert_eq!(mood_segments(f32::NAN), None);
        // Full, empty and middle meters.
        assert_eq!(meter(100.).0, 60);
        assert_eq!(meter(-100.).0, 0);
        assert_eq!(meter(0.).0, 30);
        assert_eq!(meter(100.).1, [0, 202, 57]);
        assert_eq!(meter(f32::INFINITY).0, 30);
    }

    #[test]
    fn presses_give_commands() {
        let controls = Controls {
            expanded: true,
            ..Controls::default()
        };
        let items = layout([800, 600], &snapshot(), &controls);
        // Live is the mode shown, so it opens or closes the strip.
        assert_eq!(press(&items, [30, 440]), Some(Command::TogglePanel));
        assert_eq!(press(&items, [80, 440]), Some(Command::SetMode(Mode::Buy)));
        // The second portrait, from x 229 + 45.
        assert_eq!(press(&items, [280, 520]), Some(Command::Select { sim: 18 }));
        assert_eq!(press(&items, [90, 555]), Some(Command::TogglePause));
        assert_eq!(press(&items, [400, 300]), None);
        assert!(covers(&items, [5, 590]) && !covers(&items, [400, 300]));
        assert_eq!(hovered(&items, None), None);
        // At a widescreen size the needs keep to the right edge.
        let wide = layout([1280, 720], &snapshot(), &controls);
        assert_eq!(
            press(&wide, [1280 - 388 + 50, 720 - 100 + 15]),
            Some(Command::OpenPanel(Panel::Interests))
        );
        // In a narrow window the cluster covers the gadget, and a press
        // there does not reach the gadget's buttons under it.
        let narrow = layout([400, 600], &snapshot(), &controls);
        assert_eq!(press(&narrow, [60, 530]), None);
        assert_eq!(hovered(&narrow, Some([60, 530])), None);
    }

    #[test]
    fn the_pointer_on_a_portrait_shows_that_persons_name_over_it() {
        let on = |pointer| Controls {
            expanded: true,
            pointer: Some(pointer),
            ..Controls::default()
        };
        let names = |items: &[Item]| -> Vec<(Rect, u32)> {
            items
                .iter()
                .filter_map(|i| match i.kind {
                    Kind::Name { guid } => Some((i.rect, guid)),
                    _ => None,
                })
                .collect()
        };
        // The second portrait, from x 274: its name over the strip.
        let items = layout([800, 600], &snapshot(), &on([280, 520]));
        assert_eq!(names(&items), [(Rect::new(274, 482, 45, 16), 18)]);
        // The box takes nothing from the lot under it.
        assert!(!covers(&items, [290, 490]));
        assert_eq!(press(&items, [290, 490]), None);
        // Off the portraits, no name.
        for pointer in [[400, 300], [90, 555], [320, 560]] {
            assert!(names(&layout([800, 600], &snapshot(), &on(pointer))).is_empty());
        }
    }

    #[test]
    fn a_large_family_wraps_before_the_gadget() {
        let mut big = snapshot();
        big.residents = (1..=9)
            .map(|object| Resident {
                object,
                guid: u32::from(object),
                motives: [0.; 16],
                personality: [0; 5],
            })
            .collect();
        let controls = Controls {
            expanded: true,
            ..Controls::default()
        };
        let items = layout([800, 600], &big, &controls);
        let portraits: Vec<Rect> = items
            .iter()
            .filter(|i| matches!(i.kind, Kind::Portrait { .. }))
            .map(|i| i.rect)
            .collect();
        // Four columns and two rows; the ninth is left out.
        assert_eq!(portraits.len(), 8);
        assert_eq!(portraits[4], Rect::new(229, 550, 45, 45));
        assert!(portraits.iter().all(|r| r.x + r.width <= 800 - 387));
    }

    #[test]
    fn a_disabled_button_shows_its_disabled_cell_under_the_pointer() {
        let mut art = Art::default();
        // Four 1-pixel cells: normal, selected, hover, disabled.
        art.insert(
            id::ZOOM_OUT,
            Image {
                width: 4,
                height: 1,
                rgba: [
                    [1, 0, 0, 255],
                    [2, 0, 0, 255],
                    [3, 0, 0, 255],
                    [4, 0, 0, 255],
                ]
                .concat(),
            },
        );
        let item = Item {
            rect: Rect::new(0, 0, 1, 1),
            kind: Kind::Button {
                id: id::ZOOM_OUT,
                cells: 4,
                selected: false,
            },
            command: None,
        };
        let controls = Controls {
            pointer: Some([0, 0]),
            ..Controls::default()
        };
        let mut frame = Frame::new(1, 1, [0; 3]);
        draw(&mut frame, &art, &Faces::default(), &[item], &controls);
        assert_eq!(frame.pixel(0, 0), Some([4, 0, 0]));
    }

    #[test]
    fn items_at_the_edges_of_i32_do_not_overflow() {
        let mut art = Art::default();
        art.insert(
            id::STRIP,
            Image {
                width: 1,
                height: 1,
                rgba: vec![9, 9, 9, 255],
            },
        );
        let mut frame = Frame::new(2, 2, [0; 3]);
        let items: Vec<Item> = [i32::MIN, i32::MAX]
            .into_iter()
            .flat_map(|x| {
                [
                    Kind::Stretched(id::STRIP),
                    Kind::Meter(50.),
                    Kind::Text {
                        text: "x".into(),
                        size: 8,
                        rgb: GREEN,
                        align: Align::Right,
                    },
                ]
                .map(|kind| Item {
                    rect: Rect::new(x, 0, i32::MAX, 1),
                    kind,
                    command: None,
                })
            })
            .collect();
        draw(
            &mut frame,
            &art,
            &Faces::default(),
            &items,
            &Controls::default(),
        );
        assert_eq!(frame.pixel(1, 1), Some([0; 3]));
    }

    /// Art with an 8-point font of one glyph, "M": 3 by 4, solid.
    fn art_with_a_font() -> Art {
        let mut art = Art::default();
        art.insert_font(
            NAME_SIZE,
            Font::new(sims_formats::ffn::Font {
                flags: 0,
                glyphs: vec![sims_formats::ffn::Glyph {
                    code: u16::from(b'M'),
                    width: 3,
                    height: 4,
                    atlas_x: 0,
                    atlas_y: 0,
                    first_bearing: 0,
                    bearing_x: 0,
                    bearing_y: 0,
                }],
                width: 3,
                height: 4,
                coverage: vec![15; 12],
            }),
        );
        art
    }

    #[test]
    fn a_name_keeps_inside_its_box_and_the_frame() {
        let art = art_with_a_font();
        let mut faces = Faces::default();
        faces.insert_name(1, &"M".repeat(24));
        let name = |rect| Item {
            rect,
            kind: Kind::Name { guid: 1 },
            command: None,
        };
        // Above the top and wider than the frame: the box comes down to
        // the top and the name is cut to the frame's width.
        let mut frame = Frame::new(40, 30, [9; 3]);
        let item = name(Rect::new(30, -50, 45, NAME_HEIGHT));
        draw(&mut frame, &art, &faces, &[item], &Controls::default());
        // The box: the line's 4 rows and 2 above and below, and the ten
        // glyphs that fit (30 pixels) with 4 each side, from x 2.
        for (x, y) in [(2, 0), (39, 0), (2, 7), (39, 7)] {
            assert_eq!(frame.pixel(x, y), Some(LABEL), "({x}, {y})");
        }
        assert_eq!(frame.pixel(3, 1), Some(NAME_BACK));
        assert_eq!(frame.pixel(6, 2), Some(LABEL));
        assert_eq!(frame.pixel(6, 5), Some(LABEL));
        assert_eq!(frame.pixel(6, 6), Some(NAME_BACK));
        assert_eq!(frame.pixel(36, 2), Some(NAME_BACK));
        assert_eq!(frame.pixel(1, 0), Some([9; 3]));
        assert_eq!(frame.pixel(20, 8), Some([9; 3]));
        // At the edges of i32, nothing overflows.
        let mut frame = Frame::new(2, 2, [0; 3]);
        let items: Vec<Item> = [i32::MIN, i32::MAX]
            .into_iter()
            .flat_map(|v| {
                [
                    Rect::new(v, 0, i32::MAX, 1),
                    Rect::new(0, v, 1, i32::MAX),
                    Rect::new(v, v, i32::MAX, i32::MAX),
                ]
            })
            .map(name)
            .collect();
        draw(&mut frame, &art, &faces, &items, &Controls::default());
    }

    #[test]
    fn a_closed_strip_shows_no_people_and_odd_sizes_do_not_panic() {
        let items = layout([800, 600], &snapshot(), &Controls::default());
        assert!(
            !items
                .iter()
                .any(|i| matches!(i.kind, Kind::Portrait { .. }))
        );
        let mut frame = Frame::new(10, 10, [0; 3]);
        let open = Controls {
            expanded: true,
            ..Controls::default()
        };
        for size in [[0, 0], [10, 10], [-5, -5], [i32::MAX, i32::MAX]] {
            let items = layout(size, &snapshot(), &open);
            draw(
                &mut frame,
                &Art::default(),
                &Faces::default(),
                &items,
                &open,
            );
        }
    }

    /// An image of one color.
    fn solid(width: u32, height: u32, rgb: [u8; 3]) -> Image {
        Image {
            width,
            height,
            rgba: [rgb[0], rgb[1], rgb[2], 255].repeat((width * height) as usize),
        }
    }

    #[test]
    fn the_personality_panel_shows_the_selected_sims_traits() {
        let mut s = snapshot();
        s.residents[0].personality = [250, -5, 1500, 0, 999];
        let controls = Controls {
            expanded: true,
            panel: Panel::Personality,
            ..Controls::default()
        };
        let items = layout([800, 600], &s, &controls);
        let panels: Vec<&Item> = items
            .iter()
            .filter(|i| matches!(i.kind, Kind::Personality(_)))
            .collect();
        // The subpanel, as the needs' is.
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].rect, Rect::new(521, 500, 279, 100));
        assert_eq!(panels[0].kind, Kind::Personality([250, -5, 1500, 0, 999]));
        // Presses go through it; nobody selected, no panel.
        assert_eq!(press(&items, [700, 520]), None);
        s.selected = None;
        let items = layout([800, 600], &s, &controls);
        assert!(!items.iter().any(|i| matches!(i.kind, Kind::Personality(_))));

        // Drawn with a font whose labels are 0 wide: every bar's label is
        // 3 wide and its bar starts 3 after, so the 40-pixel bars run from
        // 159, and 3 more pixels end the bar's gadget at 202.
        let (empty, full) = ([1, 1, 1], [2, 2, 2]);
        let mut art = Art::default();
        art.insert_font(TRAIT_FONT, crate::font::tests::font());
        art.insert(id::TRAIT_EMPTY, solid(40, 14, empty));
        art.insert(id::TRAIT_FULL, solid(40, 16, full));
        let line = art.font(TRAIT_FONT).unwrap().line_height;
        assert!(line < 14);
        let mut frame = Frame::new(220, 110, [0; 3]);
        draw_personality(
            &mut frame,
            &art,
            Rect::new(0, 0, 220, 100),
            &[250, -5, 1500, 0, 999],
        );
        // 4 pixels a point: 2, none, 10 (all 40), none, 9.
        for (n, points) in [2, 0, 10, 0, 9].into_iter().enumerate() {
            let y = 8 + 18 * n as i32;
            let row: Vec<_> = (150..210).map(|x| frame.pixel(x, y).unwrap()).collect();
            let filled = row.iter().filter(|&&p| p == full).count();
            assert_eq!(filled, 4 * points, "bar {n}");
            assert_eq!(row[159 - 150], if points > 0 { full } else { empty });
            assert_eq!(row[158 - 150], [0; 3]);
            assert_eq!(row[198 - 150], if points == 10 { full } else { empty });
            assert_eq!(row[199 - 150], [0; 3]);
            // The empty bar is 14 high from the row's top; the full one,
            // 16 high, starts a pixel above it (0x5212b2).
            let top = 8 + 18 * n as i32;
            let filled = if points > 0 { full } else { [0; 3] };
            assert_eq!(frame.pixel(159, top - 1), Some(filled), "bar {n}");
            assert_eq!(frame.pixel(159, top + 14), Some(filled), "bar {n}");
            assert_eq!(frame.pixel(159, top + 15), Some([0; 3]), "bar {n}");
            assert_eq!(frame.pixel(198, top + 13), Some(row[198 - 150]));
            assert_eq!(
                frame.pixel(198, top - 1),
                Some(if points == 10 { full } else { [0; 3] })
            );
        }
        // Coordinates at the edge of i32 draw nothing rather than panic.
        let far = Rect::new(i32::MAX, i32::MAX, 220, 100);
        draw_personality(&mut frame, &art, far, &[1000; 5]);
        let near = Rect::new(i32::MIN, i32::MIN, 220, 100);
        draw_personality(&mut frame, &art, near, &[1000; 5]);
    }
}
