//! The live-mode control panel: the cluster of mode, camera and speed
//! controls at the bottom left, and the strip beside it with the
//! household's portraits, the mood gauge and the needs.
//!
//! The positions are the original's at 800 by 600. The old prototype
//! captured them from the running game's windows
//! (legacy/crates/sims-app/src/hud_cluster.rs and original_hud.rs); the
//! rest were measured on a screenshot of the original (see
//! `docs/decisions/`). At another size the cluster keeps to the bottom
//! left, the strip runs from it to the right edge, the needs keep to the
//! right edge and the strip's background stretches its last column, as
//! the original does when its window is wider than 800.

use crate::art::Art;
use crate::balloon::{self, Balloon, Help, Texts};
use crate::command::{Camera, Command, Mode, Panel, Relations, Speed, Walls};
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::house::{self, House};
use crate::household::{Faces, Relation, Snapshot, Work, money};
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
    /// A skill bar, full and empty, in a western language (0x52f356).
    pub const SKILL_FULL: i32 = 4901;
    pub const SKILL_EMPTY: i32 = 4920;
    /// The house panel's empty rating bar (0x52dd4e); its full one is
    /// [`TRAIT_FULL`].
    pub const RATING_EMPTY: i32 = 4919;
    /// The job panel's career icons, and the salary's, performance's and
    /// friends' icons (0x52fc27, 0x66dfdc).
    pub const CAREERS: i32 = 4902;
    pub const SALARY: i32 = 4981;
    pub const PERFORMANCE: i32 = 4982;
    pub const FRIENDS_NEEDED: i32 = 4983;
    /// The relationship panel's page arrows (0x5193d6) and list buttons,
    /// in the panel's order (0x51a5ba).
    pub const PAGE_BACK: i32 = 100;
    pub const PAGE_ON: i32 = 101;
    pub const FAMILY: i32 = 4108;
    pub const FRIENDS: i32 = 4109;
    pub const STARS: i32 = 4111;
    pub const EVERYONE: i32 = 4110;
    /// A relationship's icons: friends, and the two hearts (0x5281cb).
    pub const FRIENDS_ICON: i32 = 4105;
    pub const PINK_HEART: i32 = 4106;
    pub const RED_HEART: i32 = 4107;
    pub const TEMPLATE: i32 = crate::household::TEMPLATE;
    pub const NORMAL: i32 = 4705;
    pub const FAST: i32 = 4706;
    pub const ULTRA: i32 = 4707;
    pub const LIVE_TRIM: i32 = 4803;
    pub const BUY_TRIM: i32 = 4804;
    pub const BUILD_TRIM: i32 = 4805;
    pub const OPTIONS_TRIM: i32 = 4806;
}

/// Every resource the panel needs, for [`Art::load`]. The help
/// balloon's background, [`balloon::BACKGROUND`], is loaded apart with
/// [`Art::load_optional`]: without it the balloon shows its text alone.
pub const RESOURCES: [i32; 55] = [
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
    id::SKILL_FULL,
    id::SKILL_EMPTY,
    id::RATING_EMPTY,
    id::CAREERS,
    id::SALARY,
    id::PERFORMANCE,
    id::FRIENDS_NEEDED,
    id::PAGE_BACK,
    id::PAGE_ON,
    id::FAMILY,
    id::FRIENDS,
    id::STARS,
    id::EVERYONE,
    id::FRIENDS_ICON,
    id::PINK_HEART,
    id::RED_HEART,
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
    /// Whom the relationship panel lists, and its page, from 0. A page
    /// past the list's end shows its last page, as the original steps
    /// back to it (0x51db4b).
    pub relations: Relations,
    pub relations_page: u32,
    /// The help balloon open, if any. It shows while the panel that
    /// opened it does.
    pub help: Option<Help>,
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
            relations: Relations::default(),
            relations_page: 0,
            help: None,
        }
    }
}

impl Controls {
    /// Applies a command that changes only what is shown. Commands for
    /// the simulation and the camera are left to the host.
    ///
    /// A control's balloon closes when the control is pressed again, even
    /// if it would now show other text (0x51c025 compares the balloon,
    /// not the text), and when another panel opens (0x51d375); and here also when the strip closes
    /// (**unverified**). The original also closes it on a press outside
    /// it and on some focus changes (0x51f3e5, 0x51d312, 0x51d354), which
    /// are not ported.
    pub fn apply(&mut self, command: &Command) {
        match command {
            Command::TogglePanel => {
                self.expanded = !self.expanded;
                self.help = None;
            }
            Command::OpenPanel(panel) => {
                if *panel != self.panel {
                    self.help = None;
                }
                self.panel = *panel;
            }
            // A sign outside 1 to 12 has no balloon and does nothing.
            Command::Help(Help::Sign(sign)) if !(1..=12).contains(sign) => {}
            Command::Help(help) => {
                self.help = match self.help {
                    Some(open) if open.balloon() == help.balloon() => None,
                    _ => Some(*help),
                };
            }
            Command::ShowRelations(list) => self.relations = *list,
            Command::RelationsPage(page) => self.relations_page = *page,
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
    Personality {
        traits: [i16; 5],
        zodiac: Option<u8>,
    },
    /// The job panel's skills, in `rect`, the subpanel: the selected
    /// Sim's skills and what the next level of their career needs of
    /// each, in the panel's order ([`crate::household::SKILLS`]).
    Skills { skills: [i16; 6], needs: [i32; 6] },
    /// The job panel's left column, in `rect`, the subpanel: the selected
    /// Sim's career icon and title, salary, performance, and the family
    /// friends the next level needs where the family has fewer than
    /// `family_friends`.
    Work {
        work: Work,
        performance: i16,
        family_friends: i32,
    },
    /// A person in the relationship panel, `rect` the entry: their face
    /// for the lifetime relationship, the two relationships as numbers
    /// and bars, and the icons. `lit` while the pointer is on it.
    Relation { relation: Relation, lit: bool },
    /// The house panel's ratings and facts, in `rect`, the subpanel.
    House { house: House },
    /// The first name of the person of type `guid`, in a box centered
    /// over `rect`'s bottom edge, while the pointer is on their portrait.
    Name { guid: u32 },
    /// The personality panel's sign of the zodiac, which opens its help
    /// balloon when pressed. [`fit`] sizes `rect` to the name; until
    /// then it is empty. The name itself is drawn with
    /// [`Kind::Personality`].
    Sign { sign: u8, panel: Rect },
    /// The help balloon open over `owner`, the subpanel that opened it,
    /// before [`fit`] lays it out ([`Kind::Balloon`]). Draws nothing.
    Help { help: Help, owner: Rect },
    /// A help balloon, laid out.
    Balloon(Balloon),
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
    // The help balloon, over everything, while its panel shows.
    if let Some(help @ Help::Sign(_)) = controls.help {
        let owner = items.iter().find_map(|i| match i.kind {
            Kind::Personality { .. } => Some(i.rect),
            _ => None,
        });
        if let Some(owner) = owner {
            items.push(Item {
                rect: Rect::new(owner.x + owner.width, owner.y, 0, 0),
                kind: Kind::Help { help, owner },
                command: None,
            });
        }
    }
    items
}

/// Finishes what [`layout`] cannot without the fonts and the install's
/// texts: sizes the sign of the zodiac's press area to its name, as the
/// personality panel sizes its control (0x51cd7f..0x51cdfe), and lays out
/// the help balloon. Hosts call it after [`layout`]; without it the sign
/// cannot be pressed and no balloon shows.
pub fn fit(items: &mut [Item], art: &Art, texts: &Texts) {
    for item in items {
        match &item.kind {
            Kind::Sign { sign, panel } => {
                if let Some(font) = art.font(ZODIAC_FONT) {
                    let name = ZODIAC_NAMES[usize::from((*sign).clamp(1, 12) - 1)];
                    let line = font.line_height;
                    item.rect = Rect::new(
                        panel.x.saturating_add(5),
                        panel.y.saturating_add(panel.height - line - 5),
                        font.width(name),
                        line,
                    );
                }
            }
            Kind::Help { help, owner } => {
                if let Some(text) = texts.get(*help) {
                    let laid = balloon::lay_out(art, text, *owner);
                    item.rect = laid.rect;
                    item.kind = Kind::Balloon(laid);
                }
            }
            _ => {}
        }
    }
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
            kind: Kind::Personality {
                traits: resident.personality,
                zodiac: resident.zodiac,
            },
            command: None,
        });
        if let Some(sign) = resident.zodiac {
            items.push(Item {
                rect: Rect::new(panel.x + 5, panel.y + panel.height - 5, 0, 0),
                kind: Kind::Sign { sign, panel },
                command: Some(Command::Help(Help::Sign(sign))),
            });
        }
    }
    if let (Some(resident), Panel::Job) = (selected, controls.panel) {
        let panel = Rect::new(gadget.x + 108, strip.y, NEEDS_WIDTH - 108, STRIP);
        // The title where the job panel's draw puts it in a western
        // language (0x530340).
        items.push(Item {
            rect: panel.offset(JOB_TITLE[0], JOB_TITLE[1]).with_size(100, 20),
            kind: Kind::Text {
                text: "Job".into(),
                size: 11,
                rgb: LABEL,
                align: Align::Left,
            },
            command: None,
        });
        items.push(Item {
            rect: panel,
            kind: Kind::Skills {
                skills: resident.skills,
                needs: resident.needs,
            },
            command: None,
        });
        items.push(Item {
            rect: panel,
            kind: Kind::Work {
                work: resident.work.clone(),
                performance: resident.performance,
                family_friends: snapshot.family_friends,
            },
            command: None,
        });
    }
    if let (Some(_), Panel::Relationships) = (selected, controls.panel) {
        let panel = Rect::new(gadget.x + 108, strip.y, NEEDS_WIDTH - 108, STRIP);
        relationships(items, panel, snapshot, controls);
    }
    // The house panel needs no selected Sim (0x52e49e).
    if controls.panel == Panel::House {
        let panel = Rect::new(gadget.x + 108, strip.y, NEEDS_WIDTH - 108, STRIP);
        items.push(Item {
            rect: panel
                .offset(HOUSE_TITLE[0], HOUSE_TITLE[1])
                .with_size(100, 20),
            kind: Kind::Text {
                text: "House".into(),
                size: 11,
                rgb: LABEL,
                align: Align::Left,
            },
            command: None,
        });
        items.push(Item {
            rect: panel,
            kind: Kind::House {
                house: snapshot.house.clone(),
            },
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

/// The signs of the zodiac, Aries first (UIText.iff STR# 164, strings 1
/// to 12, read by 0x48410b), and their font (0x6f2af4).
const ZODIAC_NAMES: [&str; 12] = [
    "Aries",
    "Taurus",
    "Gemini",
    "Cancer",
    "Leo",
    "Virgo",
    "Libra",
    "Scorpio",
    "Sagittarius",
    "Capricorn",
    "Aquarius",
    "Pisces",
];
const ZODIAC_FONT: u8 = 12;

/// The personality panel's sign of the zodiac (0x51cd11..0x51cdfe): its
/// name, sized to the text, 5 from the panel's left and 5 above its
/// bottom less the line. **Unverified:** the color (the labels' is
/// taken).
fn draw_zodiac(frame: &mut Frame, art: &Art, panel: Rect, sign: u8) {
    let Some(font) = art.font(ZODIAC_FONT) else {
        return;
    };
    let name = ZODIAC_NAMES[usize::from(sign.clamp(1, 12) - 1)];
    let y = panel.y.saturating_add(panel.height - font.line_height - 5);
    font.draw(frame, name, [panel.x.saturating_add(5), y], LABEL);
}

/// The job panel's title, from the subpanel (0x530340).
const JOB_TITLE: [i32; 2] = [35, 3];
/// The job panel's skill labels, in a western language (Live.iff STR#
/// 136, strings 1 to 6).
const SKILL_NAMES: [&str; 6] = [
    "Cooking",
    "Mechanical",
    "Charisma",
    "Body",
    "Logic",
    "Creativity",
];
/// The skill labels' font (0x6f2ae0) and the boxes they are right
/// aligned in: 62 by 16, the first at (93, 6), 15 apart (0x6f4420).
const SKILL_FONT: u8 = 7;
const SKILL_LABEL: [i32; 4] = [93, 6, 62, 16];
/// The first skill bar, from the subpanel; the others are 15 below each
/// other (0x6f4300).
const SKILL_BAR: [i32; 2] = [157, 7];
const SKILL_PITCH: i32 = 15;

/// The job panel's skills as the original draws them (0x5301ca). Each
/// skill's label and empty bar, then the full bar over it cut to 4
/// pixels a point (at 10 points, all of it). Where the next level of
/// the career needs more of a skill, its label is underlined (0x5e4c01
/// sets the label's font flag 4) and a line marks the need: down the
/// bar at the need, and along the bar's bottom from 16 pixels left of
/// the bar to it, in the labels' color (0x6f2a94).
fn draw_skills(frame: &mut Frame, art: &Art, panel: Rect, skills: &[i16; 6], needs: &[i32; 6]) {
    let (Some(font), Some(empty), Some(full)) = (
        art.font(SKILL_FONT),
        art.image(id::SKILL_EMPTY),
        art.image(id::SKILL_FULL),
    ) else {
        return;
    };
    let full_w = i32::try_from(full.width).unwrap_or(0);
    let full_h = i32::try_from(full.height).unwrap_or(0);
    for (n, ((name, skill), need)) in SKILL_NAMES.iter().zip(skills).zip(needs).enumerate() {
        let row = SKILL_PITCH * n as i32;
        let bar = [
            panel.x.saturating_add(SKILL_BAR[0]),
            panel.y.saturating_add(SKILL_BAR[1] + row),
        ];
        frame.draw_all(empty, bar);
        let points = i32::from(*skill / 100).clamp(0, 10);
        let width = points * 4;
        if points > 0 {
            let cut = if width <= full_w && points != 10 {
                width
            } else {
                full_w
            };
            frame.draw(full, Rect::new(0, 0, cut, full_h), bar);
        }
        let [x, y, w, h] = SKILL_LABEL;
        let label = Rect::new(
            panel.x.saturating_add(x),
            panel.y.saturating_add(y + row),
            w,
            h,
        );
        let at = [
            label.x.saturating_add(label.width - font.width(name)),
            label
                .y
                .saturating_add((label.height - font.line_height) / 2),
        ];
        if *need <= 0 {
            font.draw(frame, name, at, LABEL);
            continue;
        }
        font.draw_underlined(frame, name, at, LABEL);
        let mark = bar[0]
            .saturating_add((*need / 100).saturating_mul(4))
            .saturating_sub(1);
        let bottom = bar[1].saturating_add(full_h - 1);
        frame.fill(Rect::new(mark, bar[1], 1, full_h), LABEL);
        frame.fill(
            Rect::new(
                bar[0].saturating_sub(16),
                bottom,
                mark.saturating_sub(bar[0].saturating_sub(16))
                    .saturating_add(1),
                1,
            ),
            LABEL,
        );
    }
}

/// The house panel's title, where its draw puts it in a western language
/// (0x52e7af: (5, 0), font 11).
const HOUSE_TITLE: [i32; 2] = [5, 0];
/// The house panel's labels' font (0x6f2ae4, font 8; Japanese takes
/// 0x6f2ae0 for the lot's).
const HOUSE_FONT: u8 = 8;
/// Where each rating's label starts down the subpanel, the label's right
/// end being 3 left of its bar (0x52e0bb), and where each bar is
/// (0x6f4170, 0x6f3f30).
const RATING_TOPS: [i32; 5] = [7, 25, 43, 61, 79];
const RATING_BARS: [[i32; 2]; 5] = [[157, 6], [157, 24], [157, 42], [157, 60], [157, 79]];
/// The facts' boxes, 80 by 16, at x 0 (0x6f4170, 0x6f4008), the text
/// right aligned in each (0x5e4bf4 sets alignment 2).
const FACT_TOPS: [i32; 4] = [18, 38, 58, 78];
const FACT_SIZE: [i32; 2] = [80, 16];

/// Draws the house panel (0x52e49e) in the subpanel `panel`.
///
/// - **Ratings:** each rating's label, in the labels' font, sized to its
///   text with its right end 3 left of its bar (0x52e0bb); the empty bar
///   ([`id::RATING_EMPTY`]); and the full bar ([`id::TRAIT_FULL`]) over
///   it, cut to 4 pixels a tenth: the rating kept to 0 to 100, over 10,
///   times 4, at most the bar's width. A rating not ported draws its
///   empty bar.
/// - **Facts:** "Sq. Ft.", "Bedrooms", "Bathrooms" and "Lot" with their
///   values (Live.iff `STR#` 133, strings 6 to 9; the lot's size from
///   `STR#` 138), right aligned in their boxes. The square feet are not
///   converted to square meters, as the original does where the
///   language measures in meters.
///
/// **Unverified:** the labels' color (the needs labels' is taken), and
/// how a label sits in its box down the panel (the rating labels at
/// their top; the facts centred by the font's line height, where the
/// original centres by the height of an "A").
fn draw_house(frame: &mut Frame, art: &Art, panel: Rect, house: &House) {
    let (Some(font), Some(empty), Some(full)) = (
        art.font(HOUSE_FONT),
        art.image(id::RATING_EMPTY),
        art.image(id::TRAIT_FULL),
    ) else {
        return;
    };
    let full_w = i32::try_from(full.width).unwrap_or(0);
    let full_h = i32::try_from(full.height).unwrap_or(0);
    let at = |[x, y]: [i32; 2]| [panel.x.saturating_add(x), panel.y.saturating_add(y)];
    for (n, name) in house::RATINGS.iter().enumerate() {
        let bar = at(RATING_BARS[n]);
        let label = [
            bar[0].saturating_sub(3).saturating_sub(font.width(name)),
            panel.y.saturating_add(RATING_TOPS[n]),
        ];
        font.draw(frame, name, label, LABEL);
        frame.draw_all(empty, bar);
        let tenths = house.ratings[n].map_or(0, |r| r.clamp(0, 100) / 10);
        if tenths > 0 {
            let width = (tenths * 4).min(full_w);
            frame.draw(full, Rect::new(0, 0, width, full_h), bar);
        }
    }
    let lot = house::LOT_SIZES.get(house.lot).copied().unwrap_or("");
    let facts = [
        format!("Sq. Ft.: {}", house.square_feet),
        format!("Bedrooms: {}", house.bedrooms),
        format!("Bathrooms: {}", house.bathrooms),
        format!("Lot: {lot}"),
    ];
    for (text, top) in facts.iter().zip(FACT_TOPS) {
        let [w, h] = FACT_SIZE;
        let [x, y] = at([0, top]);
        let place = [
            x.saturating_add(w - font.width(text)),
            y.saturating_add((h - font.line_height) / 2),
        ];
        font.draw(frame, text, place, LABEL);
    }
}

/// The job panel's left column (0x52f2ff): four gadgets, the first at
/// (3, 4) in the subpanel and each of the others under the one before
/// (0x52fe37), 95 wide; their fonts (0x6ccce4: the career's, the
/// salary's, the performance's and the friends'); and the strings the
/// panel shows in a western language (Live.iff STR# 136, strings 7 to
/// 10; work.iff STR# 4096, strings 1 to 5, 0x4034fd).
const WORK_LEFT: [i32; 2] = [3, 4];
const WORK_WIDTH: i32 = 95;
const WORK_FONTS: [u8; 4] = [7, 8, 7, 7];
const UNEMPLOYED: &str = "Unemployed";
const NOT_AVAILABLE: &str = "n/a";
const PERFORMANCE_NAMES: [&str; 5] = ["Dismal", "Poor", "Average", "Good", "Excellent"];
/// The career icons' grid: four states across and 22 careers down
/// (0x52fc27); the other icons have the four states alone (0x52fc64).
const WORK_ICON_STATES: u32 = 4;
const CAREER_ROWS: u32 = 22;
/// The bottom of the career title's box, from the gadget's top: the
/// gadget's first height (0x6f4210).
const TITLE_BOTTOM: i32 = 45;

/// The performance's name (0x4034fd): below -60, -30, 30 and 60, then
/// the best.
fn performance_name(performance: i16) -> &'static str {
    let n = [-60, -30, 30, 60]
        .iter()
        .position(|&bound| performance < bound)
        .unwrap_or(4);
    PERFORMANCE_NAMES[n]
}

/// Cell `cell` of the icons `id`, `rows` careers down: four states
/// across, and the gadgets show the first (0x5e4b0f).
fn work_icon(art: &Art, id: i32, rows: u32, row: u32) -> Option<(&Image, Rect)> {
    let image = art.image(id)?;
    let (w, h) = (image.width / WORK_ICON_STATES, image.height / rows.max(1));
    let (w, h) = (i32::try_from(w).ok()?, i32::try_from(h).ok()?);
    Some((
        image,
        Rect::new(0, h.checked_mul(i32::try_from(row).ok()?)?, w, h),
    ))
}

/// The job panel's left column as the original draws it (0x530501), top
/// to bottom:
/// - the career: its icon (the row of [`Work::icon`]; none for a row the
///   icons lack) and under it the level's title, wrapped to the
///   gadget's width, or "Unemployed" (0x5303b3, 0x530674);
/// - the salary in money, or "n/a", after its icon, centered down the
///   line (0x5305f7, 0x52fdbb);
/// - the performance's name, or "n/a", after its icon (0x530563);
/// - "Need" and the family friends the next level needs, where the
///   family has fewer, with the icon after it, 2 down (0x531050).
///
/// Text is in the labels' color. **Not drawn:** the performance's blink
/// below 0 (0x5306df, every 333 ms), and the tooltips. Which lines of a
/// title that wraps past its box show is **unverified**: here the lines
/// that fit.
fn draw_work(
    frame: &mut Frame,
    art: &Art,
    panel: Rect,
    work: &Work,
    performance: i16,
    family_friends: i32,
) {
    let [Some(career), Some(salary), Some(rating), Some(friends)] =
        WORK_FONTS.map(|size| art.font(size))
    else {
        return;
    };
    let x = panel.x.saturating_add(WORK_LEFT[0]);
    let mut y = panel.y.saturating_add(WORK_LEFT[1]);

    let row = u32::try_from(work.icon).ok().filter(|&r| r < CAREER_ROWS);
    let icon = work_icon(art, id::CAREERS, CAREER_ROWS, row.unwrap_or(0));
    let cell = icon.map_or(22, |(_, source)| source.height);
    if let (Some((image, source)), Some(_)) = (icon, row) {
        frame.draw(image, source, [x, y]);
    }
    let title = work.level.as_ref().map_or(UNEMPLOYED, |l| l.title.as_str());
    // A line shows when its top is above the box's bottom, as the
    // original's wrapped text does (0x60b510), however far it reaches.
    let mut top = cell;
    for line in career.break_lines(title, WORK_WIDTH) {
        if top >= TITLE_BOTTOM {
            break;
        }
        career.draw(frame, &line, [x, y.saturating_add(top)], LABEL);
        top = top.saturating_add(career.line_height.max(1));
    }
    y = y.saturating_add(cell.saturating_add(career.line_height.saturating_mul(2)));

    for (font, id, text) in [
        (
            salary,
            id::SALARY,
            work.level
                .as_ref()
                .map_or_else(|| NOT_AVAILABLE.to_owned(), |l| money(l.salary)),
        ),
        (
            rating,
            id::PERFORMANCE,
            if work.level.is_some() {
                performance_name(performance)
            } else {
                NOT_AVAILABLE
            }
            .to_owned(),
        ),
    ] {
        let mut text_x = x;
        if let Some((image, source)) = work_icon(art, id, 1, 0) {
            // The salary's icon is centered down its line; the
            // performance's is at the top (0x52fdfd).
            let down = if id == id::SALARY {
                (font.line_height - source.height) / 2
            } else {
                0
            };
            frame.draw(image, source, [x, y.saturating_add(down)]);
            text_x = x.saturating_add(source.width + 3);
        }
        font.draw(frame, &text, [text_x, y], LABEL);
        y = y.saturating_add(font.line_height);
    }

    if work.friends > family_friends {
        let text = format!("Need {}", work.friends);
        friends.draw(frame, &text, [x, y], LABEL);
        if let Some((image, source)) = work_icon(art, id::FRIENDS_NEEDED, 1, 0) {
            let at = x.saturating_add(friends.width(&text)).saturating_add(3);
            frame.draw(image, source, [at, y.saturating_add(2)]);
        }
    }
}

/// The relationship panel's list buttons, top to bottom, and whom each
/// lists (0x51d9aa); the Superstar one is disabled without that pack
/// (0x51a7ea, [`Snapshot::superstar`]).
const RELATION_BUTTONS: [(i32, Relations); 4] = [
    (id::FAMILY, Relations::Family),
    (id::FRIENDS, Relations::Friends),
    (id::STARS, Relations::Stars),
    (id::EVERYONE, Relations::Everyone),
];
/// The height of the font the panel lays out by (font 6, 0x6f2adc).
/// The install has no 6-point font, so the original gets an empty one:
/// taken as 0 high (**unverified**).
const RELATIONS_FONT_HEIGHT: i32 = 0;
/// People a page (0x51926a). The original lists 10 in a window exactly
/// 1024 wide; ours lists 5 at every size, as the subpanel's width there
/// is **unverified**.
const RELATIONS_PER_PAGE: usize = 5;
/// The list's entries (0x5281cb, 0x51db6e): 45 by 90, in rows from (6
/// right of the arrow and the first button, 18), to 2 short of the
/// panel's right edge.
const RELATION_SIZE: [i32; 2] = [45, 90];
const RELATIONS_TOP: i32 = 18;

/// Lays out the relationship panel in the subpanel `panel` (0x51d75b,
/// in a western language): the four list buttons down its left, the
/// page arrows when there is a page that way, and a page of entries.
fn relationships(items: &mut Vec<Item>, panel: Rect, snapshot: &Snapshot, controls: &Controls) {
    let fh = RELATIONS_FONT_HEIGHT;
    // 0x51dd1d: the first button 3 in, the others 2, each a pixel below
    // the one before.
    let mut y = fh + 1;
    let mut button_w = 0;
    for (n, (id, list)) in RELATION_BUTTONS.iter().enumerate() {
        let rect = panel
            .offset(if n == 0 { 3 } else { 2 }, y)
            .with_size(20, 20);
        if n == 0 {
            button_w = rect.width;
        }
        y += rect.height + 1;
        items.push(Item {
            rect,
            kind: Kind::Button {
                id: *id,
                cells: 4,
                selected: controls.relations == *list,
            },
            command: (*list != Relations::Stars || snapshot.superstar)
                .then_some(Command::ShowRelations(*list)),
        });
    }
    let shown: Vec<&Relation> = snapshot
        .relations
        .iter()
        .filter(|r| match controls.relations {
            Relations::Family => r.family,
            Relations::Friends => r.friends,
            Relations::Stars => r.star,
            Relations::Everyone => true,
        })
        .collect();
    // 0x51da2e: a page past the end steps back to the last one.
    let pages = shown.len().div_ceil(RELATIONS_PER_PAGE).max(1);
    let page = (controls.relations_page as usize).min(pages - 1);
    // 0x51ddec: the arrows centred on the panel below the font's line.
    let [arrow_w, arrow_h] = [9, 49];
    let arrow_y = (panel.height - arrow_h - fh) / 2 + fh;
    if page > 0 {
        items.push(Item {
            rect: panel
                .offset(button_w + 4, arrow_y)
                .with_size(arrow_w, arrow_h),
            kind: Kind::Button {
                id: id::PAGE_BACK,
                cells: 4,
                selected: false,
            },
            command: Some(Command::RelationsPage((page - 1) as u32)),
        });
    }
    if (page + 1) * RELATIONS_PER_PAGE < shown.len() {
        items.push(Item {
            rect: panel
                .offset(panel.width - arrow_w - 2, arrow_y)
                .with_size(arrow_w, arrow_h),
            kind: Kind::Button {
                id: id::PAGE_ON,
                cells: 4,
                selected: false,
            },
            command: Some(Command::RelationsPage((page + 1) as u32)),
        });
    }
    // 0x4ebdfa: left to right, a new row where the next would pass the
    // right edge.
    let [w, h] = RELATION_SIZE;
    let left = arrow_w + button_w + 6;
    let right = panel.width - 2;
    let (mut x, mut row) = (left, 0);
    for relation in shown
        .iter()
        .skip(page * RELATIONS_PER_PAGE)
        .take(RELATIONS_PER_PAGE)
    {
        let rect = panel.offset(x, RELATIONS_TOP + h * row).with_size(w, h);
        x += w;
        if x + w > right {
            row += 1;
            x = left;
        }
        let lit = controls
            .pointer
            .is_some_and(|[px, py]| rect.contains(px, py));
        items.push(Item {
            rect,
            kind: Kind::Relation {
                relation: (*relation).clone(),
                lit,
            },
            command: None,
        });
    }
}

/// The 16-bit colors the relationship entries are drawn in, made from
/// the original's 8-bit values as its display does (0x4ff3c8): the
/// numbers' background, the bars' base, the colors they lean to, green
/// for the part filled and red for the rest, and the brighter pair of a
/// lit entry (0x5285c7).
const NUMBER_BACK: u16 = to565([0, 0, 0x4a]);
const BAR_BASE: u16 = to565([0x40, 0x5d, 0x5f]);
const BAR_GREEN: u16 = to565([0, 0xca, 0x39]);
const BAR_RED: u16 = to565([0xd1, 0, 0]);
const LIT_GREEN: u16 = to565([0, 0xff, 0]);
const LIT_RED: u16 = to565([0xff, 0, 0]);
/// The friends smiley's width in the install (4105), where the heart's
/// place is measured from when the art has no smiley.
const FRIENDS_ICON_WIDTH: i32 = 9;
/// The entries' font (0x5281cb).
const RELATION_FONT: u8 = 7;

const fn to565([r, g, b]: [u8; 3]) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

/// One channel of the original's blend table (built by 0x49a2fb): 5-bit
/// `base` moved toward 5-bit `to` by `alpha` of 31, as the mean of the
/// 512 8-bit colors each value stands for, each rounded.
fn blend_channel(alpha: u16, to: u16, base: u16) -> u16 {
    let step = f64::from(f32::from_bits(0x3b80_8081));
    let mut sum = 0i64;
    for u in 0..8 {
        let a = f64::from(base * 8 + u);
        for v in 0..8 {
            let d = f64::from(to * 8 + v) - a;
            for w in 0..8 {
                let t = f64::from(alpha * 8 + w);
                sum += (t * step * d + a + 0.5) as i64;
            }
        }
    }
    ((sum as f64 * (1.0 / 512.0) + 0.5) as i64 >> 3) as u16
}

/// `base` moved toward `to` by `alpha` of 31, a channel at a time, in
/// the 16-bit display (0x49a60b); green keeps its top five bits.
fn blend565(base: u16, to: u16, alpha: u16) -> u16 {
    let alpha = alpha & 31;
    let r = blend_channel(alpha, (to >> 11) & 31, (base >> 11) & 31);
    let g = blend_channel(alpha, (to >> 6) & 31, (base >> 6) & 31);
    let b = blend_channel(alpha, to & 31, base & 31);
    (r << 11) | (g << 6) | b
}

/// The bars' two colors for a lifetime relationship (0x528565): the
/// filled part leans to green by the relationship, the rest to red by
/// what it lacks.
fn relation_colors(lifetime: i32, lit: bool) -> ([u8; 3], [u8; 3]) {
    let t = (f64::from(lifetime.saturating_add(100)) * f64::from(f32::from_bits(0x3fa3_3333)) + 0.5)
        as i32;
    let (green, red) = if lit {
        (LIT_GREEN, LIT_RED)
    } else {
        (BAR_GREEN, BAR_RED)
    };
    let shade = |v: i32| (v >> 3) as u16;
    (
        rgb565(blend565(BAR_BASE, green, shade(t))),
        rgb565(blend565(BAR_BASE, red, shade(255 - t))),
    )
}

/// Draws a relationship entry (0x52838f) in `r`. The face is the one of
/// five for the lifetime relationship, centred at the top; the daily
/// relationship's number goes in a box across the entry two pixels over
/// the face's bottom; under it the daily bar, 2 high, then 2 below that
/// the lifetime bar, 4 high, each 40 wide and centred, a fifth of a
/// pixel a point from -100; then the lifetime's number, and the icons
/// a pixel below its box less the 3 the original takes off: the smiley
/// for friends, then the red heart, or else the pink one, 3 pixels on.
///
/// **Unverified:** where the lifetime number's box starts (an
/// instruction the copy protection hides; under the lifetime bar is
/// taken) and so the icons' height; the numbers' color (the labels'
/// is taken) and that they are centred in their boxes; that `lit` is
/// the pointer on the entry (0x528598).
fn draw_relation(
    frame: &mut Frame,
    art: &Art,
    faces: &Faces,
    r: Rect,
    relation: &Relation,
    lit: bool,
) {
    let mut bottom = r.y;
    if let Some(moods) = faces.moods(relation.guid) {
        let w = i32::try_from(moods.width / 5).unwrap_or(0);
        let h = i32::try_from(moods.height).unwrap_or(0);
        if w > 0 {
            // The original divides by the strip's cell width, not by a
            // fifth of the 200 points (0x5283fa..0x528424): with the
            // install's 40-pixel cells, the same.
            let face = ((relation.lifetime + 100) / w).min(4);
            let source = Rect::new(face * w, 0, w, h);
            frame.draw(moods, source, [r.x.saturating_add((r.width - w) / 2), r.y]);
        }
        bottom = r.y.saturating_add(h);
    }
    let font = art.font(RELATION_FONT);
    let number = |frame: &mut Frame, value: i32, box_: Rect| {
        let Some(font) = font else { return };
        frame.fill(box_, rgb565(NUMBER_BACK));
        let text = value.to_string();
        let x = box_
            .x
            .saturating_add((box_.width - font.width(&text) + 1).div_euclid(2));
        font.draw(frame, &text, [x, box_.y], LABEL);
    };
    let fh = font.map_or(0, |f| f.line_height);
    if font.is_some() {
        let top = bottom.saturating_sub(2);
        number(frame, relation.daily, Rect::new(r.x, top, r.width, fh));
        bottom = top.saturating_add(fh);
    }
    let (green, red) = relation_colors(relation.lifetime, lit);
    let x0 = r.x.saturating_add((r.width - 40) / 2);
    let x1 = r.x.saturating_add((r.width + 40) / 2);
    let bar = |frame: &mut Frame, value: i32, top: i32, height: i32| {
        let end = x0.saturating_add(
            (f64::from(value + 100) * f64::from(f32::from_bits(0x3e4c_cccd))) as i32,
        );
        frame.fill(Rect::new(x0, top, end.saturating_sub(x0), height), green);
        frame.fill(Rect::new(end, top, x1.saturating_sub(end), height), red);
        // A dark line ends the filled part where two pixels or more are
        // left (0x5287e9). Unlike the needs' meters (0x51376e) the
        // original does not ask for a filled part: with none it draws the
        // line a pixel left of the bar.
        if x1.saturating_sub(end) > 1 {
            frame.fill(Rect::new(end.saturating_sub(1), top, 1, height), [0, 0, 0]);
        }
    };
    bar(frame, relation.daily, bottom, 2);
    bar(frame, relation.lifetime, bottom.saturating_add(4), 4);
    let top = bottom.saturating_add(8);
    if font.is_some() {
        number(frame, relation.lifetime, Rect::new(x0, top, 40, fh));
    }
    let y = top.saturating_add(fh - 2);
    let friends = art.image(id::FRIENDS_ICON);
    if relation.friends
        && let Some(icon) = friends
    {
        frame.draw_all(icon, [x0, y]);
    }
    let heart = if relation.red_heart {
        art.image(id::RED_HEART)
    } else if relation.pink_heart {
        art.image(id::PINK_HEART)
    } else {
        None
    };
    // The heart goes 3 past the smiley's width whether the smiley shows
    // or not; art without a smiley takes the install's 9.
    if let Some(heart) = heart {
        let w = friends.map_or(FRIENDS_ICON_WIDTH, |f| {
            i32::try_from(f.width).unwrap_or(FRIENDS_ICON_WIDTH)
        });
        frame.draw_all(heart, [x0.saturating_add(w.saturating_add(3)), y]);
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
            Kind::Personality { traits, zodiac } => {
                draw_personality(frame, art, r, traits);
                if let Some(sign) = zodiac {
                    draw_zodiac(frame, art, r, *sign);
                }
            }
            Kind::Skills { skills, needs } => draw_skills(frame, art, r, skills, needs),
            Kind::Work {
                work,
                performance,
                family_friends,
            } => draw_work(frame, art, r, work, *performance, *family_friends),
            Kind::Relation { relation, lit } => draw_relation(frame, art, faces, r, relation, *lit),
            Kind::House { house } => draw_house(frame, art, r, house),
            Kind::Sign { .. } | Kind::Help { .. } => {}
            Kind::Balloon(b) => balloon::draw(frame, art, b),
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
                    | Kind::Personality { .. }
                    | Kind::Skills { .. }
                    | Kind::Work { .. }
                    | Kind::Relation { .. }
                    | Kind::Help { .. }
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

/// The relationship entry under `point`, which is drawn lit while the
/// pointer is on it, so that moving onto, off or between entries changes
/// the frame although the entries take no presses.
pub fn lit_relation(items: &[Item], point: Option<[i32; 2]>) -> Option<usize> {
    let [x, y] = point?;
    items
        .iter()
        .rposition(|i| matches!(i.kind, Kind::Relation { .. }) && i.rect.contains(x, y))
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
                    zodiac: None,
                    skills: [0; 6],
                    job: [0; 2],
                    needs: [0; 6],
                    child: false,
                    female: false,
                    performance: 0,
                    work: Work::default(),
                })
                .collect(),
            selected: Some(21),
            family_friends: 0,
            relations: Vec::new(),
            superstar: false,
            house: crate::house::House::default(),
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
                zodiac: None,
                skills: [0; 6],
                job: [0; 2],
                needs: [0; 6],
                child: false,
                female: false,
                performance: 0,
                work: Work::default(),
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
        s.residents[0].zodiac = Some(9);
        let controls = Controls {
            expanded: true,
            panel: Panel::Personality,
            ..Controls::default()
        };
        let items = layout([800, 600], &s, &controls);
        let panels: Vec<&Item> = items
            .iter()
            .filter(|i| matches!(i.kind, Kind::Personality { .. }))
            .collect();
        // The subpanel, as the needs' is.
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].rect, Rect::new(521, 500, 279, 100));
        assert_eq!(
            panels[0].kind,
            Kind::Personality {
                traits: [250, -5, 1500, 0, 999],
                zodiac: Some(9)
            }
        );
        // Presses go through it; nobody selected, no panel.
        assert_eq!(press(&items, [700, 520]), None);
        s.selected = None;
        let items = layout([800, 600], &s, &controls);
        assert!(
            !items
                .iter()
                .any(|i| matches!(i.kind, Kind::Personality { .. }))
        );

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

    fn relation(id: i16, daily: i32, family: bool, friends: bool) -> Relation {
        Relation {
            guid: 1000 + id as u32,
            id,
            daily,
            lifetime: -daily,
            friends,
            pink_heart: false,
            red_heart: false,
            star: id == 4,
            family,
        }
    }

    #[test]
    fn the_relationship_panel_lists_a_page_of_people() {
        let mut s = snapshot();
        s.relations = (1..=7)
            .map(|n| relation(n, 10 * i32::from(n), n <= 2, n % 2 == 1))
            .collect();
        let mut controls = Controls {
            expanded: true,
            panel: Panel::Relationships,
            ..Controls::default()
        };
        let panel = Rect::new(521, 500, 279, 100);
        let entries = |items: &[Item]| -> Vec<(Rect, i16)> {
            items
                .iter()
                .filter_map(|i| match &i.kind {
                    Kind::Relation { relation, .. } => Some((i.rect, relation.id)),
                    _ => None,
                })
                .collect()
        };
        let items = layout([800, 600], &s, &controls);
        // The buttons down the left: everyone chosen, the Superstar one
        // disabled.
        let buttons: Vec<_> = items
            .iter()
            .filter_map(|i| match i.kind {
                Kind::Button { id, selected, .. } if (4108..=4111).contains(&id) => {
                    Some((id, i.rect, selected, i.command.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            buttons,
            [
                (
                    id::FAMILY,
                    panel.offset(3, 1).with_size(20, 20),
                    false,
                    Some(Command::ShowRelations(Relations::Family))
                ),
                (
                    id::FRIENDS,
                    panel.offset(2, 22).with_size(20, 20),
                    false,
                    Some(Command::ShowRelations(Relations::Friends))
                ),
                (
                    id::STARS,
                    panel.offset(2, 43).with_size(20, 20),
                    false,
                    None
                ),
                (
                    id::EVERYONE,
                    panel.offset(2, 64).with_size(20, 20),
                    true,
                    Some(Command::ShowRelations(Relations::Everyone))
                ),
            ]
        );
        // Five a page, 45 apart from (35, 18); the arrow on.
        let row = |ids: &[i16]| -> Vec<(Rect, i16)> {
            ids.iter()
                .enumerate()
                .map(|(n, id)| (panel.offset(35 + 45 * n as i32, 18).with_size(45, 90), *id))
                .collect()
        };
        assert_eq!(entries(&items), row(&[1, 2, 3, 4, 5]));
        let arrows = |items: &[Item]| -> Vec<(i32, Rect, Option<Command>)> {
            items
                .iter()
                .filter_map(|i| match i.kind {
                    Kind::Button { id, .. } if id == id::PAGE_BACK || id == id::PAGE_ON => {
                        Some((id, i.rect, i.command.clone()))
                    }
                    _ => None,
                })
                .collect()
        };
        let on = (
            id::PAGE_ON,
            panel.offset(268, 25).with_size(9, 49),
            Some(Command::RelationsPage(1)),
        );
        let back = (
            id::PAGE_BACK,
            panel.offset(24, 25).with_size(9, 49),
            Some(Command::RelationsPage(0)),
        );
        assert_eq!(arrows(&items), std::slice::from_ref(&on));
        assert_eq!(press(&items, [521 + 270, 530]), on.2.clone());
        controls.apply(&Command::RelationsPage(1));
        let items = layout([800, 600], &s, &controls);
        assert_eq!(entries(&items), row(&[6, 7]));
        assert_eq!(arrows(&items), std::slice::from_ref(&back));
        // A page past the end shows the last.
        controls.relations_page = 9;
        assert_eq!(entries(&layout([800, 600], &s, &controls)), row(&[6, 7]));
        // The family's list, and the friends'.
        controls.apply(&Command::ShowRelations(Relations::Family));
        let items = layout([800, 600], &s, &controls);
        assert_eq!(entries(&items), row(&[1, 2]));
        assert!(arrows(&items).is_empty());
        controls.relations = Relations::Friends;
        let items = layout([800, 600], &s, &controls);
        assert_eq!(entries(&items), row(&[1, 3, 5, 7]));
        // With the Superstar pack its button can be pressed, and lists
        // those with fame.
        s.superstar = true;
        let items = layout([800, 600], &s, &controls);
        let stars = items
            .iter()
            .find(|i| matches!(i.kind, Kind::Button { id, .. } if id == id::STARS));
        assert_eq!(
            stars.and_then(|i| i.command.clone()),
            Some(Command::ShowRelations(Relations::Stars))
        );
        controls.apply(&Command::ShowRelations(Relations::Stars));
        assert_eq!(entries(&layout([800, 600], &s, &controls)), row(&[4]));
        s.superstar = false;
        controls.relations = Relations::Friends;
        // The pointer on an entry lights it.
        controls.pointer = Some([521 + 35 + 45 + 10, 540]);
        let lit: Vec<bool> = layout([800, 600], &s, &controls)
            .iter()
            .filter_map(|i| match i.kind {
                Kind::Relation { lit, .. } => Some(lit),
                _ => None,
            })
            .collect();
        assert_eq!(lit, [false, true, false, false]);
        // Moving between entries, or off them, changes what is lit,
        // though no entry takes a press.
        let items = layout([800, 600], &s, &controls);
        let second = lit_relation(&items, controls.pointer).expect("an entry under the pointer");
        assert!(matches!(
            items[second].kind,
            Kind::Relation { lit: true, .. }
        ));
        assert_eq!(hovered(&items, controls.pointer), None);
        let first = lit_relation(&items, Some([521 + 35 + 10, 540]));
        assert!(first.is_some_and(|n| n != second));
        assert_eq!(lit_relation(&items, Some([0, 0])), None);
        // Nobody selected, no panel.
        s.selected = None;
        let items = layout([800, 600], &s, &controls);
        assert!(entries(&items).is_empty());
        assert!(arrows(&items).is_empty());
    }

    #[test]
    fn relationship_bars_lean_from_the_panels_color_to_green_and_red() {
        // From the original's blend table: none of the lean is the base
        // (green losing its low bit), all of it the color.
        assert_eq!(blend565(BAR_BASE, BAR_GREEN, 0), 0x42cb);
        assert_eq!(blend565(BAR_BASE, BAR_GREEN, 31), BAR_GREEN);
        assert_eq!(relation_colors(-100, false), ([66, 89, 90], [214, 0, 0]));
        assert_eq!(relation_colors(0, false), ([33, 146, 74], [140, 40, 41]));
        assert_eq!(relation_colors(0, true), ([33, 170, 49], [165, 40, 41]));
        assert_eq!(relation_colors(100, false), ([0, 203, 57], [66, 89, 90]));
    }

    #[test]
    fn a_relationship_entry_follows_the_originals_edges() {
        let r = Rect::new(10, 10, 45, 90);
        let relation = |daily: i32, lifetime: i32| Relation {
            guid: 7,
            id: 1,
            daily,
            lifetime,
            friends: false,
            pink_heart: false,
            red_heart: true,
            star: false,
            family: false,
        };
        // The face: lifetime plus 100 over the strip's cell width, as the
        // original divides. With 40-pixel cells (the install's) 0 is the
        // middle face; with 4-pixel ones, the last.
        let strip = |cell: u32| {
            let mut moods = solid(cell * 5, 2, [0; 3]);
            for face in 0..5 {
                for x in 0..cell {
                    let i = ((face * cell + x) * 4) as usize;
                    moods.rgba[i..i + 3].copy_from_slice(&[10 * (face as u8 + 1); 3]);
                }
            }
            moods
        };
        for (cell, lifetime, face) in [(40, 0, 2), (40, -61, 0), (40, 60, 4), (4, 0, 4)] {
            let mut faces = Faces::default();
            faces.insert_moods(7, strip(cell));
            let mut frame = Frame::new(80, 120, [0; 3]);
            draw_relation(
                &mut frame,
                &Art::default(),
                &faces,
                r,
                &relation(0, lifetime),
                false,
            );
            let x = 10 + (45 - cell as i32) / 2;
            assert_eq!(
                frame.pixel(x, 10),
                Some([10 * (face + 1); 3]),
                "{cell} {lifetime}"
            );
        }
        // An empty bar still gets its dark line, a pixel left of the bar
        // (0x5287e9 asks only for room on the right); a full one none.
        let back = [5; 3];
        let mut frame = Frame::new(80, 120, back);
        let faces = Faces::default();
        draw_relation(
            &mut frame,
            &Art::default(),
            &faces,
            r,
            &relation(-97, 100),
            false,
        );
        let (green, red) = relation_colors(100, false);
        assert_eq!(frame.pixel(11, 10), Some([0; 3]));
        assert_eq!(frame.pixel(12, 10), Some(red));
        assert_eq!(frame.pixel(11, 14), Some(back));
        assert_eq!(frame.pixel(51, 14), Some(green));
        assert_eq!(frame.pixel(52, 14), Some(back));
        // A heart without the smiley's art goes where the install's
        // 9-wide smiley would put it.
        let mut art = Art::default();
        art.insert(id::RED_HEART, solid(9, 9, [2, 2, 2]));
        let mut frame = Frame::new(80, 120, [0; 3]);
        draw_relation(&mut frame, &art, &faces, r, &relation(0, 0), false);
        assert_eq!(frame.pixel(23, 18), Some([0; 3]));
        assert_eq!(frame.pixel(24, 18), Some([2; 3]));
    }

    #[test]
    fn a_relationship_entry_shows_the_face_numbers_bars_and_icons() {
        // Five 4 by 6 faces, each its own color.
        let mut moods = solid(20, 6, [0; 3]);
        for face in 0..5u32 {
            for y in 0..6 {
                for x in 0..4 {
                    let i = ((y * 20 + face * 4 + x) * 4) as usize;
                    moods.rgba[i..i + 3].copy_from_slice(&[10 * (face as u8 + 1); 3]);
                }
            }
        }
        let mut faces = Faces::default();
        faces.insert_moods(7, moods);
        let mut art = Art::default();
        art.insert(id::FRIENDS_ICON, solid(9, 9, [1, 1, 1]));
        art.insert(id::RED_HEART, solid(9, 9, [2, 2, 2]));
        art.insert(id::PINK_HEART, solid(9, 9, [3, 3, 3]));
        let r = Rect::new(10, 10, 45, 90);
        let draw = |relation: &Relation, art: &Art| {
            let mut frame = Frame::new(80, 120, [0; 3]);
            draw_relation(&mut frame, art, &faces, r, relation, false);
            frame
        };
        let mut relation = Relation {
            guid: 7,
            id: 1,
            daily: 50,
            lifetime: 100,
            friends: true,
            pink_heart: true,
            red_heart: true,
            star: false,
            family: false,
        };
        // No font: no numbers, and the bars right under the face.
        let frame = draw(&relation, &art);
        // The face for 100 is the last, centred: x 30 to 33.
        assert_eq!(frame.pixel(30, 10), Some([50; 3]));
        assert_eq!(frame.pixel(29, 10), Some([0; 3]));
        let (green, red) = relation_colors(100, false);
        // The daily bar, 2 high at y 16 from x 12: 30 green, then red,
        // with the dark line on the last green pixel.
        assert_eq!(frame.pixel(12, 16), Some(green));
        assert_eq!(frame.pixel(40, 17), Some(green));
        assert_eq!(frame.pixel(41, 16), Some([0; 3]));
        assert_eq!(frame.pixel(42, 16), Some(red));
        assert_eq!(frame.pixel(51, 17), Some(red));
        assert_eq!(frame.pixel(52, 16), Some([0; 3]));
        assert_eq!(frame.pixel(12, 18), Some([0; 3]));
        // The lifetime bar, 4 high from y 20, all of it green.
        assert_eq!(frame.pixel(12, 20), Some(green));
        assert_eq!(frame.pixel(51, 23), Some(green));
        assert_eq!(frame.pixel(51, 24), Some([0; 3]));
        // The icons at y 16 + 8 - 2: the smiley, and the red heart 3 on.
        assert_eq!(frame.pixel(12, 22), Some([1; 3]));
        assert_eq!(frame.pixel(24, 22), Some([2; 3]));
        relation.red_heart = false;
        assert_eq!(draw(&relation, &art).pixel(24, 22), Some([3; 3]));
        // A font: the number over the face's last two rows, the bars
        // under it.
        let font = crate::font::tests::font();
        let fh = font.line_height;
        art.insert_font(RELATION_FONT, font);
        let frame = draw(&relation, &art);
        let back = rgb565(NUMBER_BACK);
        assert_eq!(frame.pixel(10, 14), Some(back));
        assert_eq!(frame.pixel(54, 14 + fh - 1), Some(back));
        assert_eq!(frame.pixel(30, 13), Some([50; 3]));
        assert_eq!(frame.pixel(12, 14 + fh), Some(green));
        assert_eq!(frame.pixel(40, 14 + fh + 8), Some(back));
        assert_eq!(frame.pixel(11, 14 + fh + 8), Some([0; 3]));
        // Far coordinates do not overflow.
        let mut frame = Frame::new(4, 4, [0; 3]);
        for at in [i32::MAX - 10, i32::MIN + 10] {
            draw_relation(
                &mut frame,
                &art,
                &faces,
                Rect::new(at, at, 45, 90),
                &relation,
                true,
            );
        }
    }

    #[test]
    fn the_sign_of_the_zodiac_goes_at_the_panels_bottom_left() {
        let mut art = Art::default();
        art.insert_font(ZODIAC_FONT, crate::font::tests::font());
        let line = art.font(ZODIAC_FONT).unwrap().line_height;
        // "Aries": only its `A` is in the test font, drawn a pixel in
        // (its bearing) from 5 in, at 100 - line - 5 down.
        let mut frame = Frame::new(40, 110, [0; 3]);
        draw_zodiac(&mut frame, &art, Rect::new(0, 0, 279, 100), 1);
        let top = 100 - line - 5;
        assert_eq!(frame.pixel(6, top), Some(LABEL));
        assert_eq!(frame.pixel(5, top), Some([0; 3]));
        assert_eq!(frame.pixel(6, top - 1), Some([0; 3]));
        // Without the font nothing is drawn, and far places do not
        // overflow.
        let mut frame = Frame::new(4, 4, [0; 3]);
        draw_zodiac(&mut frame, &Art::default(), Rect::new(0, 0, 279, 100), 1);
        assert!(frame.rgb().iter().all(|p| *p == [0; 3]));
        for at in [i32::MAX - 2, i32::MIN + 2] {
            draw_zodiac(&mut frame, &art, Rect::new(at, at, 279, 100), 12);
        }
    }

    #[test]
    fn pressing_the_sign_opens_its_balloon_over_the_panel() {
        let mut s = snapshot();
        s.residents[0].zodiac = Some(1);
        let mut controls = Controls {
            expanded: true,
            panel: Panel::Personality,
            ..Controls::default()
        };
        let mut art = Art::default();
        art.insert_font(ZODIAC_FONT, crate::font::tests::font());
        let line = art.font(ZODIAC_FONT).unwrap().line_height;
        let texts = Texts::signs(&[], &[], &[]);
        // Before fit the sign cannot be pressed; after, its name can.
        let mut items = layout([800, 600], &s, &controls);
        let bottom_left = [526, 600 - 5 - line];
        assert_eq!(press(&items, bottom_left), None);
        fit(&mut items, &art, &texts);
        let sign = items
            .iter()
            .find(|i| matches!(i.kind, Kind::Sign { .. }))
            .unwrap();
        // Only the test font's `A` has width.
        let width = art.font(ZODIAC_FONT).unwrap().width("Aries");
        assert_eq!(sign.rect, Rect::new(526, 600 - 5 - line, width, line));
        let command = press(&items, bottom_left).unwrap();
        assert_eq!(command, Command::Help(Help::Sign(1)));
        assert!(!command.changes_simulation());

        // Open, the balloon is the last item, flush right on the strip.
        controls.apply(&command);
        assert_eq!(controls.help, Some(Help::Sign(1)));
        let mut items = layout([800, 600], &s, &controls);
        fit(&mut items, &art, &texts);
        let last = items.last().unwrap();
        assert!(matches!(last.kind, Kind::Balloon(_)));
        assert_eq!(last.rect, Rect::new(241, 373, 559, 127));
        // It takes presses over the lot, and asks for nothing.
        assert!(covers(&items, [300, 400]));
        assert_eq!(press(&items, [300, 400]), None);
        draw(
            &mut Frame::new(800, 600, [0; 3]),
            &art,
            &Faces::default(),
            &items,
            &controls,
        );

        // Another panel closes it; so does pressing the sign again.
        let mut other = controls;
        other.apply(&Command::OpenPanel(Panel::Needs));
        assert_eq!(other.help, None);
        let mut same = controls;
        same.apply(&Command::OpenPanel(Panel::Personality));
        assert_eq!(same.help, Some(Help::Sign(1)));
        let mut again = controls;
        again.apply(&command);
        assert_eq!(again.help, None);
        // Even another Sim's sign: the panel has one balloon (0x51c025).
        let mut again = controls;
        again.apply(&Command::Help(Help::Sign(4)));
        assert_eq!(again.help, None);
        again.apply(&Command::Help(Help::Sign(4)));
        assert_eq!(again.help, Some(Help::Sign(4)));
        let mut closed = controls;
        closed.apply(&Command::TogglePanel);
        assert_eq!(closed.help, None);
        // A sign outside 1 to 12 does nothing, open or not.
        for sign in [0, 13] {
            let mut odd = controls;
            odd.apply(&Command::Help(Help::Sign(sign)));
            assert_eq!(odd.help, Some(Help::Sign(1)));
            odd.help = None;
            odd.apply(&Command::Help(Help::Sign(sign)));
            assert_eq!(odd.help, None);
        }
        // With another panel showing, no balloon is laid out.
        let mut elsewhere = controls;
        elsewhere.panel = Panel::Needs;
        let items = layout([800, 600], &s, &elsewhere);
        assert!(!items.iter().any(|i| matches!(i.kind, Kind::Help { .. })));
    }

    #[test]
    fn the_job_panels_left_column_follows_the_original() {
        use crate::household::WorkLevel;
        let mut s = snapshot();
        s.residents[0].performance = -31;
        s.residents[0].work = Work {
            icon: 9,
            level: Some(WorkLevel {
                title: "Test Subject".into(),
                salary: 155,
            }),
            friends: 1,
        };
        s.family_friends = 4;
        let controls = Controls {
            expanded: true,
            panel: Panel::Job,
            ..Controls::default()
        };
        let items = layout([800, 600], &s, &controls);
        let work: Vec<&Item> = items
            .iter()
            .filter(|i| matches!(i.kind, Kind::Work { .. }))
            .collect();
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].rect, Rect::new(521, 500, 279, 100));
        assert_eq!(
            work[0].kind,
            Kind::Work {
                work: s.residents[0].work.clone(),
                performance: -31,
                family_friends: 4,
            }
        );
        assert_eq!(press(&items, [530, 510]), None);

        // The names of performance, at the original's bounds.
        let names: Vec<_> = [i16::MIN, -61, -60, -31, -30, 29, 30, 59, 60, i16::MAX]
            .map(performance_name)
            .into();
        assert_eq!(
            names,
            [
                "Dismal",
                "Dismal",
                "Poor",
                "Poor",
                "Average",
                "Average",
                "Good",
                "Good",
                "Excellent",
                "Excellent"
            ]
        );

        // The icons: the career's grid colored by (state, row), the
        // others plain. The fonts have only an `M`, 3 by 4, so a line is
        // 4 high.
        let mut careers = Image {
            width: 112,
            height: 484,
            rgba: Vec::new(),
        };
        for y in 0..484u32 {
            for x in 0..112u32 {
                careers
                    .rgba
                    .extend_from_slice(&[(x / 28) as u8 + 1, (y / 22) as u8 + 1, 9, 255]);
            }
        }
        let (salary, rating, friends) = ([3, 3, 3], [4, 4, 4], [5, 5, 5]);
        let mut art = art_with_a_font();
        let font = art.font(NAME_SIZE).unwrap().clone();
        art.insert_font(7, font.clone());
        art.insert_font(8, font.clone());
        art.insert(id::CAREERS, careers);
        art.insert(id::SALARY, solid(52, 11, salary));
        art.insert(id::PERFORMANCE, solid(52, 12, rating));
        art.insert(id::FRIENDS_NEEDED, solid(40, 11, friends));
        let draw = |work: &Work, family_friends: i32| {
            let mut frame = Frame::new(120, 60, [0; 3]);
            draw_work(
                &mut frame,
                &art,
                Rect::new(0, 0, 120, 60),
                work,
                0,
                family_friends,
            );
            frame
        };
        let mut work = Work {
            icon: 9,
            level: Some(WorkLevel {
                title: "M".into(),
                salary: 155,
            }),
            friends: 5,
        };
        let frame = draw(&work, 4);
        // Row 9, state 0, 28 by 22 at (3, 4), and the title under it.
        assert_eq!(frame.pixel(3, 4), Some([1, 10, 9]));
        assert_eq!(frame.pixel(30, 25), Some([1, 10, 9]));
        assert_eq!(frame.pixel(31, 4), Some([0; 3]));
        assert_eq!(frame.pixel(3, 26), Some(LABEL));
        assert_eq!(frame.pixel(5, 29), Some(LABEL));
        // The gadget is 22 and two lines high: the salary's line from 34,
        // its icon (13 by 11) centered on it, from 31; the font has none
        // of the text's characters.
        assert_eq!(frame.pixel(3, 30), Some([0; 3]));
        assert_eq!(frame.pixel(3, 31), Some(salary));
        assert_eq!(frame.pixel(15, 37), Some(salary));
        assert_eq!(frame.pixel(16, 37), Some([0; 3]));
        // The performance's line from 38, its icon (13 by 12) at its top.
        assert_eq!(frame.pixel(3, 38), Some(rating));
        assert_eq!(frame.pixel(3, 49), Some(rating));
        assert_eq!(frame.pixel(3, 50), Some([0; 3]));
        // "Need 5" from 42; its icon (10 by 11) 3 past the text, 2 down.
        let at = 3 + font.width("Need 5") + 3;
        assert_eq!(frame.pixel(at, 43), Some(rating));
        assert_eq!(frame.pixel(at, 44), Some(friends));
        assert_eq!(frame.pixel(at + 9, 54), Some(friends));
        // A family with enough friends: none.
        assert_eq!(draw(&work, 5).pixel(at + 9, 54), Some([0; 3]));
        assert_eq!(draw(&work, 5).pixel(at, 44), Some(rating));
        // A title wraps at 95: seven lines of 16 `M`s, of which the six
        // that start above the box's bottom (45) show.
        let word = "M".repeat(16);
        let mut title = |title: String| {
            work.level = Some(WorkLevel { title, salary: 0 });
            draw(&work, 5)
        };
        let frame = title([word.as_str(); 7].join(" "));
        for n in 0..6 {
            assert_eq!(frame.pixel(40, 26 + 4 * n), Some(LABEL), "line {n}");
        }
        assert_eq!(frame.pixel(40, 50), Some([0; 3]));
        // After a hyphen, and mid-word when a word is wider than the box.
        let frame = title(format!("{word}-{word}"));
        assert_eq!(frame.pixel(40, 30), Some(LABEL));
        let frame = title("M".repeat(40));
        assert_eq!(frame.pixel(3, 30), Some(LABEL));
        assert_eq!(frame.pixel(98, 26), Some([0; 3]));
        // Rows the icons lack, school's -1 and past the 22, draw none.
        for icon in [-1, 22] {
            work.icon = icon;
            let frame = draw(&work, 5);
            assert_eq!(frame.pixel(3, 4), Some([0; 3]), "icon {icon}");
            assert_eq!(frame.pixel(3, 26), Some(LABEL));
        }
        // Without a level, "Unemployed" and "n/a": the font has none of
        // their letters, so only the icons show.
        work.level = None;
        work.icon = 0;
        let frame = draw(&work, 5);
        assert_eq!(frame.pixel(3, 4), Some([1, 1, 9]));
        assert_eq!(frame.pixel(3, 26), Some([0; 3]));
        assert_eq!(frame.pixel(3, 31), Some(salary));
        assert_eq!(frame.pixel(3, 38), Some(rating));
    }

    #[test]
    fn the_house_panel_draws_each_rating_in_tenths() {
        let mut s = snapshot();
        s.selected = None;
        s.house = House {
            ratings: [Some(75), None, Some(150), Some(-3), Some(9)],
            square_feet: 1836,
            bedrooms: 2,
            bathrooms: 1,
            lot: 2,
        };
        let controls = Controls {
            expanded: true,
            panel: Panel::House,
            ..Controls::default()
        };
        // With nobody selected too.
        let items = layout([800, 600], &s, &controls);
        let panel = items
            .iter()
            .find(|i| matches!(i.kind, Kind::House { .. }))
            .unwrap();
        assert_eq!(panel.rect, Rect::new(521, 500, 279, 100));
        let title = items
            .iter()
            .find(|i| matches!(&i.kind, Kind::Text { text, .. } if text == "House"))
            .unwrap();
        assert_eq!((title.rect.x, title.rect.y), (521 + 5, 500));
        assert_eq!(press(&items, [700, 520]), None);

        // Empty bars 40 by 14 and full ones 40 by 16, from (157, y).
        let (empty, full) = ([1, 1, 1], [2, 2, 2]);
        let mut art = Art::default();
        art.insert_font(HOUSE_FONT, crate::font::tests::hanging_font());
        art.insert(id::RATING_EMPTY, solid(40, 14, empty));
        art.insert(id::TRAIT_FULL, solid(40, 16, full));
        let mut frame = Frame::new(220, 110, [0; 3]);
        draw_house(&mut frame, &art, Rect::new(0, 0, 220, 100), &s.house);
        // 7 tenths, none, all 10, none (kept to 0), none (9 is 0 tenths).
        for (n, tenths) in [7, 0, 10, 0, 0].into_iter().enumerate() {
            let [x, y] = RATING_BARS[n];
            let y = y + 5;
            assert_eq!(frame.pixel(x - 1, y), Some([0; 3]), "bar {n}");
            let first = if tenths > 0 { full } else { empty };
            assert_eq!(frame.pixel(x, y), Some(first), "bar {n}");
            if tenths > 0 {
                assert_eq!(frame.pixel(x + 4 * tenths - 1, y), Some(full));
            }
            if tenths < 10 {
                assert_eq!(frame.pixel(x + 4 * tenths, y), Some(empty));
            }
            assert_eq!(frame.pixel(x + 40, y), Some([0; 3]));
            // The full bar is drawn from the empty one's top, and runs
            // two rows below it.
            let below = if tenths > 0 { full } else { [0; 3] };
            assert_eq!(frame.pixel(x, y - 5 + 15), Some(below), "bar {n}");
        }
    }

    #[test]
    fn the_job_panel_shows_the_skills_and_what_the_next_level_needs() {
        let mut s = snapshot();
        s.residents[0].skills = [250, -5, 1000, 0, 999, 380];
        s.residents[0].needs = [0, 300, 0, 0, 0, 700];
        let controls = Controls {
            expanded: true,
            panel: Panel::Job,
            ..Controls::default()
        };
        let items = layout([800, 600], &s, &controls);
        let panels: Vec<&Item> = items
            .iter()
            .filter(|i| matches!(i.kind, Kind::Skills { .. }))
            .collect();
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].rect, Rect::new(521, 500, 279, 100));
        assert_eq!(
            panels[0].kind,
            Kind::Skills {
                skills: [250, -5, 1000, 0, 999, 380],
                needs: [0, 300, 0, 0, 0, 700],
            }
        );
        let title = items
            .iter()
            .find(|i| matches!(&i.kind, Kind::Text { text, .. } if text == "Job"))
            .unwrap();
        assert_eq!((title.rect.x, title.rect.y), (521 + 35, 500 + 3));
        assert_eq!(press(&items, [700, 520]), None);
        s.selected = None;
        let items = layout([800, 600], &s, &controls);
        assert!(!items.iter().any(|i| matches!(i.kind, Kind::Skills { .. })));

        // Bars 40 by 11 from (157, 7), 15 apart. The labels' font has
        // only a `g`, 1 wide and 2 high, hanging so that a line under
        // an `A` falls inside the line: "Cooking" and "Logic" draw it.
        let (empty, full) = ([1, 1, 1], [2, 2, 2]);
        let mut art = Art::default();
        art.insert_font(SKILL_FONT, crate::font::tests::hanging_font());
        art.insert(id::SKILL_EMPTY, solid(40, 11, empty));
        art.insert(id::SKILL_FULL, solid(40, 11, full));
        let mut frame = Frame::new(220, 110, [0; 3]);
        draw_skills(
            &mut frame,
            &art,
            Rect::new(0, 0, 220, 100),
            &[250, -5, 1000, 0, 450, 380],
            &[0, 300, 0, 0, 700, 700],
        );
        // 4 pixels a point: 2, none, 10 (all 40), none, 4, 3.
        for (n, points) in [2, 0, 10, 0, 4, 3].into_iter().enumerate() {
            let y = 7 + 15 * n as i32 + 5;
            let row: Vec<_> = (140..200).map(|x| frame.pixel(x, y).unwrap()).collect();
            let at = |x: i32| row[(x - 140) as usize];
            assert_eq!(at(156), [0; 3], "bar {n}");
            assert_eq!(at(157), if points > 0 { full } else { empty });
            assert_eq!(
                at(157 + 4 * points - 1),
                if points > 0 { full } else { [0; 3] }
            );
            // Logic's and Creativity's need lines are at 7 points.
            if points < 10 && n < 4 {
                assert_eq!(at(157 + 4 * points), empty, "bar {n}");
            }
            assert_eq!(at(196), if points == 10 { full } else { empty });
            assert_eq!(at(197), [0; 3]);
        }
        // Mechanical needs 3 points: a line down the bar's last pixel of
        // the third point (168), and along its bottom row (y 32) from 141.
        for y in 22..=32 {
            assert_eq!(frame.pixel(168, y), Some(LABEL), "y {y}");
        }
        assert_eq!(frame.pixel(168, 21), Some([0; 3]));
        for x in 141..=168 {
            assert_eq!(frame.pixel(x, 32), Some(LABEL), "x {x}");
        }
        assert_eq!(frame.pixel(140, 32), Some([0; 3]));
        assert_eq!(frame.pixel(169, 32), Some(empty));
        // Creativity needs 7, past its 3: the line is on the empty part.
        assert_eq!(frame.pixel(157 + 28 - 1, 7 + 75), Some(LABEL));
        assert_eq!(frame.pixel(157 + 28 - 1, 7 + 75 + 10), Some(LABEL));
        // No need, no line: Cooking's bottom row left of its bar is clear.
        assert_eq!(frame.pixel(150, 7 + 10), Some([0; 3]));
        // Every label is drawn, right-aligned in its 62 by 16 box from
        // (93, 6): the `g` at x 154, rows 2 and 3 of the 4-high line
        // centred in the box. Logic's, with a need, is underlined (on the
        // row under an `A`, 2 wide for the `g`'s advance); Cooking's is
        // not.
        let g = |row: i32| [154, 6 + 15 * row + 6 + 2];
        for row in [0, 4] {
            let [x, y] = g(row);
            assert_eq!(frame.pixel(x, y), Some(LABEL), "row {row}");
            assert_eq!(frame.pixel(x, y + 1), Some(LABEL), "row {row}");
            assert_eq!(frame.pixel(x - 1, y), Some([0; 3]), "row {row}");
        }
        let [x, y] = g(0);
        assert_eq!(frame.pixel(x + 1, y), Some([0; 3]));
        let [x, y] = g(4);
        assert_eq!(frame.pixel(x + 1, y), Some(LABEL));
        assert_eq!(frame.pixel(x + 2, y), Some([0; 3]));
        // A need past the panel's 16-bit range is kept: 400 points mark
        // 1,600 pixels on.
        let mut wide = Frame::new(1800, 110, [0; 3]);
        draw_skills(
            &mut wide,
            &art,
            Rect::new(0, 0, 220, 100),
            &[0; 6],
            &[0, 0, 0, 0, 40_000, 0],
        );
        assert_eq!(wide.pixel(157 + 1600 - 1, 67), Some(LABEL));
        assert_eq!(wide.pixel(157 + 1600, 67), Some([0; 3]));
        // Far coordinates do not overflow.
        let mut frame = Frame::new(4, 4, [0; 3]);
        for at in [i32::MAX - 10, i32::MIN + 200] {
            draw_skills(
                &mut frame,
                &art,
                Rect::new(at, at, 220, 100),
                &[1000; 6],
                &[i32::MAX; 6],
            );
        }
    }
}
