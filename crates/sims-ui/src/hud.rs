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
use crate::command::{
    Camera, Command, InventoryTab, Mode, Panel, Relations, Speed, VolumeChannel, Walls,
};
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::house::{self, House};
use crate::household::{Faces, InventoryEntry, JobPanel, PetKind, Relation, Snapshot, Work, money};
use crate::needs_history;
use crate::options::{self, OptionsState, OptionsTexts};
use crate::thumbnail::Thumbnails;
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
/// The live subpanel's width at 800 by 600 (0x517d9e), and so how far
/// past the gadget's left edge it starts: at 800 by 600 that is the
/// strip's 300 (0x517e3d), one pixel under the gadget's right edge.
const SUBPANEL_WIDTH: i32 = 280;
const SUBPANEL_FROM_GADGET: i32 = NEEDS_WIDTH - SUBPANEL_WIDTH;

/// Resources the panel draws (`UIGraphics/Res_CPanel.h`).
pub mod id {
    /// Background patch (`kBackPatch`, `CPanel/Backgrounds/BackPatch.bmp`, `52×100`)
    /// drawn at `(168, height - 100)` behind `CLUSTER` whenever the bottom strip
    /// is expanded (`0x50a19b`, `0x50a5e0..0x50a619`, `0x6f3630 = (168, 50)`).
    pub const BACK_PATCH: i32 = 2036;
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
    /// The cluster's `?` help button (`kHelpBtn`, `0x538f41`).
    pub const HELP: i32 = 4993;
    /// Needs & Pet Needs Social help balloon icon (`kSocialPopupIcon`, `0xea6`,
    /// `0x51974c`, `0x51995d`).
    pub const SOCIAL_POPUP: i32 = 3750;
    /// Career & School help balloon icon strip (`kJobPopupComposite`, `0x1329`,
    /// 22 rows of 84x63; row 11 is School for Report Card at `0x530a7b(0xb)`).
    pub const JOB_POPUP: i32 = 4905;
    /// Superstar / Fame panel art (`0x52bbeb..0x52bc7c`, `0x66daac`):
    /// full and empty 5-star bars (`80x16`), friend starpower smiley button
    /// (`88x20`, 4 cells), and fame-level help balloon bitmaps (`0x11..=0x13`).
    pub const FAME_RUNWAY: i32 = 278;
    pub const FAME_MOVIE_SET: i32 = 279;
    pub const FAME_MUSIC_VIDEO: i32 = 280;
    pub const FAME_BAR_FULL: i32 = 4822;
    pub const FAME_BAR_EMPTY: i32 = 4823;
    pub const FAME_FRIENDS: i32 = 4984;
    /// Interests panel (`cWinInterests`, `0x50f3c0..0x50f6f0`): the 19
    /// `Interest*.bmp` icons (`1600..=1618`, `75×25/26`) and the full/empty
    /// 10-segment bar art (`40×16`, 4 px per point).
    pub const INTEREST_ICONS: [i32; 19] = [
        1600, 1601, 1602, 1603, 1604, 1605, 1606, 1607, 1608, 1609, 1610, 1611, 1612, 1613, 1614,
        1615, 1616, 1617, 1618,
    ];
    pub const INTEREST_BARS: i32 = 1619;
    pub const INTEREST_BARS_BG: i32 = 1620;
    /// Gifts / Inventory panel sort buttons (`0x51aa96..0x51ac9e`, 4 cells of
    /// `20×20` each): Gift (`4112`), Magic (`4113`), and Other (`4114`).
    pub const INVENTORY_GIFT_SORT: i32 = 4112;
    pub const INVENTORY_MAGIC_SORT: i32 = 4113;
    pub const INVENTORY_OTHER_SORT: i32 = 4114;
}

/// Optional / expansion-gated panel and help-balloon artwork loaded with
/// [`Art::load_optional`] so installs without later expansions still open cleanly.
pub const OPTIONAL_RESOURCES: [i32; 33] = [
    balloon::BACKGROUND,
    id::SOCIAL_POPUP,
    id::JOB_POPUP,
    id::FAME_RUNWAY,
    id::FAME_MOVIE_SET,
    id::FAME_MUSIC_VIDEO,
    id::FAME_BAR_FULL,
    id::FAME_BAR_EMPTY,
    id::FAME_FRIENDS,
    1600,
    1601,
    1602,
    1603,
    1604,
    1605,
    1606,
    1607,
    1608,
    1609,
    1610,
    1611,
    1612,
    1613,
    1614,
    1615,
    1616,
    1617,
    1618,
    id::INTEREST_BARS,
    id::INTEREST_BARS_BG,
    id::INVENTORY_GIFT_SORT,
    id::INVENTORY_MAGIC_SORT,
    id::INVENTORY_OTHER_SORT,
];

/// Every resource the panel needs, for [`Art::load`]. The help
/// balloon's background, [`balloon::BACKGROUND`], is loaded apart with
/// [`Art::load_optional`]: without it the balloon shows its text alone.
pub const RESOURCES: [i32; 57] = [
    id::WALLS_DOWN,
    id::CUTAWAY,
    id::WALLS_UP,
    id::ROOF,
    id::PHOTO,
    id::BACK_PATCH,
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
    id::HELP,
    4918,
];

/// Half-period of the control-panel clock blink timer in microseconds:
/// 500 ms (`0x1f4` ms, `0x53b528`, `0x53b6f7`, `0x53b71e`), shared by both
/// pause blinking (state 2) and ultra-speed idle blinking (state 3).
pub const PAUSE_BLINK_US: u64 = 500_000;

/// The job panel performance blink's half period: `0x530530..0x5306c6`
/// starts it with `0x5306df(333)`, and the timer `0x52c659` re-arms with
/// the same period each time it flips the color.
pub const PERFORMANCE_BLINK_US: u64 = 333_000;

/// The performance text's color on the lit half of its blink: `[0x6f2ab0]`,
/// RGB (0, 202, 57) packed to 16 bits at `0x4ff43a`.
pub const PERFORMANCE_LIT: [u8; 3] = rgb565(0x0647);

/// Whether the job panel item `kind` blinks its performance: an adult's
/// panel with a job (`0x4ea722`) and performance below 0 (`0x4ea88c`).
pub fn performance_blinks(kind: &Kind) -> bool {
    matches!(kind, Kind::Work { work, performance, .. } if work.level.is_some() && *performance < 0)
}

/// Active 500 ms blink state of the clock controls (`[esi + 0x1e4]`,
/// `0x53b4b1..0x53b7e7`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClockBlinkMode {
    /// States 0 and 1: no 500 ms timer running.
    #[default]
    Off,
    /// State 2 (`0x53b500..0x53b528`): paused (in Live or non-Live mode);
    /// toggles `id::PAUSE` row 0 (cyan) <-> row 1 (red) every 500 ms.
    Pause,
    /// State 3 (`0x53b4c9..0x53b4fe`): unpaused in Live mode while idle
    /// speed is active (`[esi + 0x1ec] != 0`) and the saved return speed is
    /// not Ultra (`0x4b4513 != 0x46d26a`); toggles `id::ULTRA` selected
    /// state (`[ultra + 0xe0]`) every 500 ms.
    Ultra,
}

/// Which 500 ms blink mode (`0x53b536..0x53b7e7`) applies for the given
/// pause, mode, idle-speed flag (`world.return_speed() >= 0 && world.speed()
/// == sims_sim::speed::IDLE`), and highlighted speed (`speed_for(world)`).
pub fn clock_blink_mode(
    paused: bool,
    mode: Mode,
    idle_speed: bool,
    speed: Option<Speed>,
) -> ClockBlinkMode {
    if paused {
        ClockBlinkMode::Pause
    } else if mode == Mode::Live && idle_speed && speed != Some(Speed::Ultra) {
        ClockBlinkMode::Ultra
    } else {
        ClockBlinkMode::Off
    }
}

/// Who is selected on the lot, for [`Controls::follow`] and
/// [`check_press`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selected {
    /// Nobody is selected on the lot (`0x4e986d == -1`).
    None,
    /// Someone is selected on the lot (`0x4e986d != -1`), but is not one
    /// of the household's members (`+0xe8 == -1`, `0x51c5e7..0x51c619`).
    NonHousehold,
    /// A human household Sim (`person.data[65] <= 1`).
    Human,
    /// A pet household Sim (`person.data[65] > 1`, `0x51c61f`, `0x51ce13`).
    Pet,
}

impl Selected {
    /// Classifies the selection in `snapshot`.
    pub fn of(snapshot: &Snapshot) -> Self {
        match snapshot.selected_resident() {
            Some(r) if r.pet => Self::Pet,
            Some(_) => Self::Human,
            None if snapshot.selected.is_some() => Self::NonHousehold,
            None => Self::None,
        }
    }
}

/// What the player has open: state of the presentation, not of the
/// simulation. The host keeps it and applies the presentation-only
/// commands to it ([`Controls::apply`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Controls {
    pub mode: Mode,
    /// Whether the strip beside the cluster is open. After a house loads
    /// the original shows it closed until Live is pressed.
    pub expanded: bool,
    /// The live-mode panel open, if any. The original opens the needs
    /// when it builds the panel (0x51a0bc); with nobody selected none is
    /// open ([`Controls::follow`]).
    pub panel: Option<Panel>,
    /// Which speed button is selected (`0x53b0d8..0x53b16b`), or `None` when
    /// the simulation holds an unrecognized speed.
    pub speed: Option<Speed>,
    /// Whether the Live-mode paused blink is currently on its lit red row
    /// (row 1 of `id::PAUSE`, toggled every [`PAUSE_BLINK_US`] at
    /// `0x53b500..0x53b528`; starts `false` on entry at `0x53b707..0x53b74d`).
    pub pause_lit: bool,
    /// Whether the idle-speed ultra blink (state 3, `0x53b4c9..0x53b4fe`)
    /// currently highlights `id::ULTRA` (toggled every [`PAUSE_BLINK_US`];
    /// starts `false` on entry at `0x53b6e0..0x53b705`).
    pub ultra_lit: bool,
    /// Whether the job panel's performance is on the green half of its
    /// blink (`[gadget + 0x1a4]`, flipped every [`PERFORMANCE_BLINK_US`] by
    /// `0x52c659` while the Sim has a job and performance is below 0,
    /// `0x5306df`; off when that stops, `0x53070d`).
    pub performance_lit: bool,
    /// Whether the Interests (`[esi + 0x170]`) and Gifts (`[esi + 0x174]`)
    /// gadget buttons are enabled (`byte [btn + 0x168]`, `0x51c5a6..0x51c9e5`).
    /// Enabled for human household Sims, disabled for pets and non-household
    /// selections, and preserved when selection clears (`0x51c5e1 -> 0x51c775`).
    pub interests_gifts_enabled: bool,
    pub camera: CameraState,
    /// The pointer's position, for the buttons' hover cells.
    pub pointer: Option<[i32; 2]>,
    /// Whom the relationship panel lists, and its page, from 0. A page
    /// past the list's end shows its last page, as the original steps
    /// back to it (0x51db4b).
    pub relations: Relations,
    pub relations_page: u32,
    /// Current page of the Interests panel (`Panel::Interests`, from 0).
    pub interests_page: u32,
    /// Active category tab, page, and selected token GUID in the Gifts /
    /// Inventory panel (`Panel::Gifts`).
    pub inventory_tab: InventoryTab,
    pub inventory_page: u32,
    pub inventory_selected: Option<u32>,
    /// The help balloon open, if any. It shows while the panel that
    /// opened it does.
    pub help: Option<Help>,
    /// The Options panel's tab, graphics, sound, and play state (`cWinOptions`).
    pub options: OptionsState,
    /// Options action button currently held down (`0x5e44dd..0x5e44e1`), drawn
    /// in its pressed cell while the pointer remains over its rectangle.
    pub pressed_options_action: Option<crate::command::OptionsAction>,
    /// What each need bar's trend arrows show this frame, in the panel's
    /// order ([`NEEDS`]), from the host's
    /// [`crate::needs_history::NeedsTrend::draw`] (0x50c2fa); none while a
    /// bar does not draw.
    pub trends: [Option<needs_history::Arrows>; 8],
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
            panel: Some(Panel::Needs),
            speed: Some(Speed::Normal),
            pause_lit: false,
            ultra_lit: false,
            performance_lit: false,
            interests_gifts_enabled: true,
            camera: CameraState::default(),
            pointer: None,
            relations: Relations::default(),
            relations_page: 0,
            interests_page: 0,
            inventory_tab: InventoryTab::default(),
            inventory_page: 0,
            inventory_selected: None,
            help: None,
            options: OptionsState::default(),
            pressed_options_action: None,
            trends: [None; 8],
        }
    }
}

impl Controls {
    /// Applies a command that changes only what is shown. Commands for
    /// the simulation and the camera are left to the host.
    ///
    /// A control's balloon closes when the control is pressed again, even
    /// if it would now show other text (`0x51c025` compares the balloon,
    /// not the text), when another panel opens (`0x51d375`), when the strip
    /// closes, or when a control outside the live panel takes a press
    /// (`0x51d312`, `0x51d354`: portraits, cluster, speed, camera, mode,
    /// help dialog, or action queue). Pointer motion/exits outside the
    /// subpanel (including onto the balloon itself, `0x51a0a2..0x51a0b5`,
    /// `0x51d312`) and presses outside both the subpanel and balloon are
    /// dismissed by the host via [`in_subpanel`] and [`on_balloon`].
    pub fn apply(&mut self, command: &Command) {
        match command {
            Command::TogglePanel => {
                self.expanded = !self.expanded;
                self.help = None;
                self.options.help = None;
            }
            Command::OpenPanel(panel) => self.open(Some(*panel)),
            // Out-of-range help indices have no balloon and do nothing.
            Command::Help(help) if !help.is_valid() => {}
            Command::Help(help) => {
                self.help = match self.help {
                    Some(open) if open.balloon() == help.balloon() => None,
                    _ => Some(*help),
                };
            }
            Command::ShowRelations(list) => self.relations = *list,
            Command::RelationsPage(page) => self.relations_page = *page,
            Command::InterestsPage(page) => self.interests_page = *page,
            Command::ShowInventory(tab) => {
                if self.inventory_tab != *tab {
                    self.inventory_tab = *tab;
                    self.inventory_page = 0;
                    self.inventory_selected = None;
                }
            }
            Command::InventoryPage(page) => self.inventory_page = *page,
            Command::SelectInventoryItem(guid) => {
                self.inventory_selected = if self.inventory_selected == *guid {
                    None
                } else {
                    *guid
                };
            }
            Command::SetMode(mode) => {
                // 0x5179eb..0x5179f6: cWinOptions::Show resets its open subpanel
                // tab to 0 (`None`) whenever Options becomes visible.
                if *mode == Mode::Options && (self.mode != Mode::Options || !self.expanded) {
                    self.options.tab = None;
                }
                self.help = None;
                self.options.help = None;
            }
            Command::Select { .. }
            | Command::SetSpeed(_)
            | Command::TogglePause
            | Command::Camera(_)
            | Command::OpenHelp
            | Command::CancelAction { .. } => {
                self.help = None;
                self.options.help = None;
            }
            Command::OpenOptionsTab(_)
            | Command::ToggleOption(_)
            | Command::SetDetail { .. }
            | Command::SetVolume { .. }
            | Command::OptionsHelp(_) => {
                self.options.apply(command);
            }
            _ => {}
        }
    }

    /// Follows the selection, as the panel's household notification and
    /// selection refresh do (`0x51ce79`, `0x51c5a6`):
    /// - with nobody selected on the lot (`Selected::None`, `0x4cf9a4 == -1`),
    ///   no panel is open (`0x51d06e`) and the Interests/Gifts enabled state
    ///   is preserved (`0x51c5e1 -> 0x51c775`);
    /// - with a non-household Sim selected (`Selected::NonHousehold`,
    ///   `0x4cf9a4 != -1` but `+0xe8 == -1`), the open panel stays (opening
    ///   Needs if none, `0x51d082`) while Interests and Gifts are disabled
    ///   (`0x51c609..0x51c619 -> 0x51c8c2`);
    /// - with a human household Sim selected (`Selected::Human`), Interests
    ///   and Gifts are enabled (`0x51c6f1..0x51c8b3`) and Needs opens if no
    ///   panel is open;
    /// - with a pet household Sim selected (`Selected::Pet`,
    ///   `person.data[65] > 1`), Interests and Gifts are disabled
    ///   (`0x51c61f -> 0x51c8c2`) and an open Interests or Gifts panel
    ///   switches to Needs (`0x51ce13..0x51ce36 -> 0x51d375(1)`).
    ///
    /// True if that closed the help balloon, which the original plays
    /// `UI_whoosh` for (`0x50fb7f`).
    ///
    /// The original follows the notification. The host calls this before
    /// each layout; that a selection change always sends the
    /// notification is **unverified**.
    pub fn follow(&mut self, selected: Selected) -> bool {
        let help = self.help.is_some();
        match selected {
            Selected::None => self.open(None),
            Selected::NonHousehold => {
                self.interests_gifts_enabled = false;
                if self.panel.is_none() {
                    self.open(Some(Panel::Needs));
                }
            }
            Selected::Human => {
                self.interests_gifts_enabled = true;
                if self.panel.is_none() {
                    self.open(Some(Panel::Needs));
                }
            }
            Selected::Pet => {
                self.interests_gifts_enabled = false;
                if self.panel.is_none()
                    || matches!(self.panel, Some(Panel::Interests | Panel::Gifts))
                {
                    self.open(Some(Panel::Needs));
                }
            }
        }
        help && self.help.is_none()
    }

    /// Opens `panel` (0x51d375): another panel closes the help balloon
    /// (0x51d383).
    fn open(&mut self, panel: Option<Panel>) {
        if panel != self.panel {
            self.help = None;
        }
        self.panel = panel;
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
    /// A 6-cell checkbox or radio toggle (`optcheckbox.bmp` 4953 or
    /// `optradio.bmp` 4954, `0x5e3be6..0x5e3c3e`).
    Toggle { id: i32, checked: bool },
    /// A horizontal 11-step (`0..=10`) Sound volume slider (`cTSWinSlider`,
    /// `0x5f6091..0x5f6232`).
    Slider { channel: VolumeChannel, value: u8 },
    /// A fixed cell of a button strip of `cells` cells.
    Cell { id: i32, cells: u32, cell: u32 },
    /// The cluster's family-friends count (`[esi + 0x1d0]`, `0x539219`,
    /// `0x4cfcdb`), right-aligned immediately to the left of the
    /// `$9,999,999`-wide funds box ending at `rect.x + rect.width`.
    Friends(i32),
    /// The pause button (`id::PAUSE`, 4 columns by 2 rows, `0x538cd8`,
    /// `0x5e4b0f`): column 0 normal, 1 selected (`paused`), 2 hover,
    /// 3 disabled (`0x5e3b40`); row 1 when `lit` (toggled every 500 ms
    /// while paused at `0x53b500..0x53b528`, or steady row 1 while
    /// unpaused outside Live mode at `0x53b55c..0x53b690 -> 0x53b736`).
    Pause { paused: bool, lit: bool },
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
    /// A need bar's trend arrows, in `rect`, the need control (0x50c2fa).
    Trend(needs_history::Arrows),
    /// Marker for the open Needs (`[esi + 0x100]`) or Pet Needs (`[esi + 0x104]`)
    /// subpanel rect, used as the balloon owner (`0x519603`, `0x5197f3`).
    /// Draws nothing and takes no presses.
    Needs { pet: bool },
    /// The personality panel's bars, in `rect`, the subpanel: the
    /// selected Sim's traits in the panel's order
    /// ([`crate::household::PERSONALITY`]). Laid out when drawn, from the
    /// labels' font. When `pet` is true (`person.data[65] > 1`,
    /// `0x52157e..0x5215c0`), draws the pet labels from `Live.iff STR# 170`
    /// and hides the zodiac sign (`0x51d650..0x51d663`).
    Personality {
        traits: [i16; 5],
        zodiac: Option<u8>,
        pet: bool,
    },
    /// The job panel's skills, in `rect`, the subpanel: the selected
    /// Sim's skills and what the next level of their career needs of
    /// each, in the panel's order ([`crate::household::SKILLS`]).
    Skills { skills: [i16; 6], needs: [i32; 6] },
    /// The pet skills panel (`[esi + 0x114]` dog at `0x531469`, `[esi + 0x118]`
    /// cat at `0x531e43`), shown in `rect` on the Job tab when a pet is
    /// selected (`0x51d5ca..0x51d5f0`): three skill bars reading
    /// `person.data` words `[10, 12, 11]` (`0x66e360`).
    PetSkills { kind: PetKind, skills: [i16; 3] },
    /// The Child Report Card subpanel (`[esi + 0x124]`, `0x532967`), shown
    /// on the Job tab for school-age children (`age != 0 && age < 18`,
    /// `0x51d5af..0x51d5bf`).
    ReportCard { grade: String },
    /// Clickable grade button inside [`Kind::ReportCard`] (`[esi + 0xf8]`,
    /// `0x532c20..0x532c4a`); [`fit`] sizes its rect to font 18's line height
    /// centered vertically in `panel`. Draws nothing.
    ReportCardButton { panel: Rect },
    /// The Superstar / Fame subpanel (`[esi + 0x11c]`, `0x52b8cc`), shown
    /// on the Job tab when `0x4cfe2c` holds (Superstar installed, fame score
    /// > 0, no regular career).
    Fame {
        fame_level: i16,
        fame_title: String,
        skills: [i16; 6],
        needs: [i32; 6],
        friends_text: String,
    },
    /// The job panel's left column, in `rect`, the subpanel: the selected
    /// Sim's career icon and title, salary, performance, and the family
    /// friends the next level needs where the family has fewer than
    /// `family_friends`.
    Work {
        work: Work,
        performance: i16,
        family_friends: i32,
    },
    /// A transparent clickable hit region for a subpanel control (Need
    /// meter, Pet Skills label, or Superstar button). Draws nothing.
    Area,
    /// A person in the relationship panel, `rect` the entry: their face
    /// for the lifetime relationship, the two relationships as numbers
    /// and bars, and the icons. `lit` while the pointer is on it.
    Relation { relation: Relation, lit: bool },
    /// One interest row (`Interest*.bmp` + 10-point bar from `1619`/`1620`,
    /// `0x50f475..0x50f580`).
    Interest {
        icon_id: i32,
        points: u8,
        label: &'static str,
    },
    /// One inventory token cell in `Panel::Gifts` (`45×45` catalog template
    /// frame + object thumbnail + count badge + label).
    InventoryItem {
        entry: InventoryEntry,
        selected: bool,
    },
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
    /// An Options-panel setting help balloon (`cWinOptions`, `0x516719..0x51679a`).
    OptionsBalloon(options::OptionsBalloon),
    /// A Quick Tips hover tooltip over an Options button (`0x5164ef..0x5164ff`).
    OptionsTooltip(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    /// Centered with signed truncation toward zero (`(width - w) / 2`), as
    /// the cluster clock draws (`0x539f3e..0x539f43`).
    CenterTrunc,
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

/// Which speed button the control panel highlights for `world`
/// (`0x53b0d8..0x53b16b`): reads the idle loop's saved return speed
/// (`0x46d7cc` / `0x46d7d6`, global 18) when non-negative, otherwise the
/// active speed (`0x46d78b`, global 16), and matches against Normal
/// (`0x2f8`), Fast (`0x39c`), or Ultra (`0x3e2`). Does not inspect the
/// pause flag (`0x46d866`, global 17), so a paused lot keeps its speed
/// button highlighted; an unrecognized speed highlights none.
pub fn speed_for(world: &sims_sim::World) -> Option<Speed> {
    let raw = if world.return_speed() >= 0 {
        world.return_speed()
    } else {
        world.speed()
    };
    match raw {
        sims_sim::speed::NORMAL => Some(Speed::Normal),
        sims_sim::speed::FAST => Some(Speed::Fast),
        sims_sim::speed::ULTRA => Some(Speed::Ultra),
        _ => None,
    }
}

/// Lays the panel out in a window of `size`, in drawing order.
pub fn layout(size: [i32; 2], snapshot: &Snapshot, controls: &Controls) -> Vec<Item> {
    layout_with_options(size, snapshot, controls, &OptionsTexts::default(), None)
}

/// Lays the panel out in a window of `size`, in drawing order, using loaded
/// [`OptionsTexts`] and optional [`Art`] for font-measured Options subpanel
/// geometry (`0x51409b..0x515ef6`).
pub fn layout_with_options(
    size: [i32; 2],
    snapshot: &Snapshot,
    controls: &Controls,
    options_texts: &OptionsTexts,
    art: Option<&Art>,
) -> Vec<Item> {
    layout_with_options_lot(
        size,
        snapshot,
        controls,
        options_texts,
        options::LotVisibility::from_residential(snapshot.residential),
        art,
    )
}

/// Like [`layout_with_options`], using exact per-lot Options control
/// visibility ([`options::LotVisibility`], `0x4f2490..0x4f24e7`,
/// `0x5177aa..0x517833`).
pub fn layout_with_options_lot(
    size: [i32; 2],
    snapshot: &Snapshot,
    controls: &Controls,
    options_texts: &OptionsTexts,
    visibility: options::LotVisibility,
    art: Option<&Art>,
) -> Vec<Item> {
    // Sizes outside what a window can be are clamped, so no arithmetic
    // below can overflow.
    let [width, height] = size.map(|v| v.clamp(0, 1 << 15));
    let mut items = Vec::new();
    // Native `Sims.exe` `0x509af5`, `0x509d41`, `0x509e6f..0x509e9d`, `0x509f3c`:
    // Live (residential), Buy (`cWinCatalog`), Build (`cWinArch`), and Options
    // (`cWinOptions`) all use `kUniversalCPBkg` (`4910`, `PanelBack.bmp`, 100 px).
    // `kTallSubpanel` (`4918`, `TallSubPanel.TGA`, 150 px) is only used by
    // expansion community-lot Live Mode (`0x4cfde1` -> `0x509dc1`), never Buy Mode.
    let strip_height = STRIP;
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
        // Native `0x50a5e0..0x50a619`: `kBackPatch` (`BackPatch.bmp`, 52x100)
        // sits behind `UniversalBack.TGA` at `(168, height - STRIP)` so the
        // curved cluster edge next to the Camera/Options buttons joins
        // `PanelBack.bmp` seamlessly instead of exposing the unrendered band
        // below the shortened house viewport (`height - 100`).
        items.push(image(at(168, 83, 52, STRIP), id::BACK_PATCH));
        let strip = Rect::new(CLUSTER[0], top, width - CLUSTER[0], strip_height);
        items.push(Item {
            rect: strip,
            kind: Kind::Stretched(id::STRIP),
            command: None,
        });
        match controls.mode {
            Mode::Live => live_strip(&mut items, strip, snapshot, controls),
            Mode::Options => options::layout_strip_for_lot(
                &mut items,
                strip,
                visibility,
                &controls.options,
                options_texts,
                art,
                controls.pointer,
            ),
            Mode::Buy | Mode::Build => {}
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
            // 0x53a2c3 -> 0x4cf3af: pressing Buy while already in Buy Mode
            // toggles the catalog sort (ByFunction <-> ByRoom) rather than
            // collapsing the strip.
            if m == Mode::Buy {
                Command::BuyToggleSort
            } else {
                Command::TogglePanel
            }
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
        // `0x4f2482..0x4f248b` -> `0x53b233..0x53b254`: edit mode
        // (`global[32] == 1`, `!snapshot.show_money`) disables the Live button.
        Item {
            command: snapshot.show_money.then(|| mode(Mode::Live)),
            ..button(
                id::LIVE,
                4,
                at(10, 6, 50, 50),
                controls.mode == Mode::Live,
                mode(Mode::Live),
            )
        },
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
        // 0x53a51b..0x53a53e -> 0x4cea68..0x4cea73: pressing Options always
        // passes toggle=0 (`SetMode(Options)`), expanding when collapsed and
        // keeping an expanded Options strip open.
        button(
            id::OPTIONS,
            4,
            at(193, 125, 23, 23),
            controls.mode == Mode::Options,
            Command::SetMode(Mode::Options),
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
        // Native `Sims.exe` `0x53ad4f..0x53adb5` 4-way mutually exclusive radio
        // group (`[ui + 0x48]` = 8 Roof | 1 WallsUp | 4 Cutaway | 2 WallsDown):
        // exactly one of the four curved view buttons is lit at any time.
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
            !camera.roof && camera.walls == Walls::Up,
            true,
            Camera::Walls(Walls::Up),
        ),
        view(
            id::CUTAWAY,
            at(80, 85, 21, 21),
            !camera.roof && camera.walls == Walls::Cutaway,
            true,
            Camera::Walls(Walls::Cutaway),
        ),
        view(
            id::WALLS_DOWN,
            at(103, 95, 22, 15),
            !camera.roof && camera.walls == Walls::Down,
            true,
            Camera::Walls(Walls::Down),
        ),
        view(id::PHOTO, at(175, 95, 29, 29), false, true, Camera::Photo),
    ]);
    // 0x53b536..0x53b74d:
    // - While paused (global 17 != 0), state 2 holds Pause selected and
    //   blinks row 0 (cyan) <-> row 1 (red) every 500 ms via
    //   `controls.pause_lit` (including outside Live mode,
    //   0x53b58a -> 0x53b5d5 -> 0x53b5e8).
    // - While unpaused outside Live mode, state 1 holds Pause steady
    //   selected on lit row 1 (red) without blinking (0x53b55c -> 0x53b690).
    // - While unpaused in Live mode, state 0 draws unselected row 0.
    let steady_red = !snapshot.paused && controls.mode != Mode::Live;
    items.push(Item {
        rect: at(85, 135, 15, 15),
        kind: Kind::Pause {
            paused: snapshot.paused || steady_red,
            lit: steady_red || (snapshot.paused && controls.pause_lit),
        },
        command: Some(Command::TogglePause),
    });
    let ultra_blinking = clock_blink_mode(
        snapshot.paused,
        controls.mode,
        snapshot.idle_speed,
        controls.speed,
    ) == ClockBlinkMode::Ultra;
    for (id, speed, x, w) in [
        (id::NORMAL, Speed::Normal, 105, 11),
        (id::FAST, Speed::Fast, 120, 20),
        (id::ULTRA, Speed::Ultra, 141, 32),
    ] {
        // 0x53b0d8..0x53b16b: highlights the button matching the active speed
        // (0x46d78b) or, while idle-speed is active (0x46d7cc), the saved
        // return speed (0x46d7d6). The pause flag (0x46d866) is not checked
        // here, so a paused lot keeps its speed button selected. While in
        // state 3 (`0x53b4c9..0x53b4fe`), `id::ULTRA` additionally blinks
        // selected every 500 ms via `controls.ultra_lit`.
        let selected = controls.speed == Some(speed)
            || (speed == Speed::Ultra && ultra_blinking && controls.ultra_lit);
        items.push(button(
            id,
            4,
            at(x, 135, w, 15),
            selected,
            Command::SetSpeed(speed),
        ));
    }
    // 0x538f41..0x538fe6, 0x6f4530: kHelpBtn (4993) at (5, 164), 40x15
    // four-cell strip (10x15 per cell), enabled; hover draws cell 2 and
    // click opens the Help System dialog (0x53aaad -> 0x50e8d3).
    items.push(button(
        id::HELP,
        4,
        at(5, 164, 10, 15),
        false,
        Command::OpenHelp,
    ));
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
    // Clock box: 0x539ede..0x539f43, 0x6f4578..0x6f4584 = (95, 118)..(151, 125),
    // font [0x6f2ae4] (8 pt), centered with `(56 - w) / 2`.
    items.push(text(
        at(95, 118, 56, 14),
        snapshot.clock(),
        8,
        Align::CenterTrunc,
    ));
    // 0x4f24fa -> 0x4cfe0a/0x4cfe18 -> 0x53b2a4..0x53b2e1: when global 32
    // (budget mode) is 1, both the funds box ([esi + 0x108]) and the
    // family-friends counter ([esi + 0x1d0]) are hidden.
    if snapshot.show_money {
        // Money box ([esi + 0x108], 0x539071..0x53908f): font [0x6f2aec] (10 pt),
        // right-aligned to 0x6f4590 = 178 at y = 0x6f459c = 158.
        items.push(text(
            at(70, 158, 108, 16),
            snapshot.money(),
            10,
            Align::Right,
        ));
        // Family friends count ([esi + 0x1d0], 0x539219..0x539346, 0x4cfcdb):
        // font [0x6f2aec] (10 pt), right-aligned immediately left of the
        // `$9,999,999`-wide money box ending at x = 178, y = 158.
        items.push(Item {
            rect: at(0, 158, 178, 16),
            kind: Kind::Friends(snapshot.cluster_friends),
            command: None,
        });
    }
    // The help balloon, over everything, while its owning subpanel shows.
    if let Some(help) = controls.help {
        let owner = items.iter().find_map(|i| match (help, &i.kind) {
            (Help::Sign(_), Kind::Personality { pet: false, .. }) => Some(i.rect),
            (Help::Need { pet, .. }, Kind::Needs { pet: shown }) if *shown == pet => Some(i.rect),
            (Help::PetSkill { cat, .. }, Kind::PetSkills { kind, .. })
                if (*kind == PetKind::Cat) == cat =>
            {
                Some(i.rect)
            }
            (Help::ReportCard, Kind::ReportCard { .. }) => Some(i.rect),
            (Help::JobSkill(_), Kind::Skills { .. }) => Some(i.rect),
            (Help::FameSkill(_) | Help::FameLevel(_) | Help::FameFriends, Kind::Fame { .. }) => {
                Some(i.rect)
            }
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
    // `0x514008..0x51401b`: Options help balloons and Quick Tips attach to the
    // root HUD above the cluster so narrow windows never draw the cluster over
    // them.
    items.sort_by_key(|i| matches!(i.kind, Kind::OptionsBalloon(_) | Kind::OptionsTooltip(_)));
    items
}

/// Whether `help` belongs to the subpanel currently active for `snapshot`
/// and `panel`. Used to guard stale presses before redraw and to close an
/// open balloon when selection switches away from its owning subpanel.
pub fn help_matches_snapshot(help: Help, snapshot: &Snapshot, panel: Option<Panel>) -> bool {
    if !help.is_valid() {
        return false;
    }
    let Some(resident) = snapshot.selected_resident() else {
        return false;
    };
    match help {
        Help::Sign(_) => {
            panel == Some(Panel::Personality) && !resident.pet && resident.zodiac.is_some()
        }
        Help::Need { pet, .. } => panel == Some(Panel::Needs) && resident.pet == pet,
        Help::PetSkill { cat, .. } => {
            panel == Some(Panel::Job)
                && resident.job_panel(snapshot.superstar)
                    == JobPanel::PetSkills(if cat { PetKind::Cat } else { PetKind::Dog })
        }
        Help::ReportCard => {
            panel == Some(Panel::Job)
                && resident.job_panel(snapshot.superstar) == JobPanel::ReportCard
        }
        Help::FameSkill(_) | Help::FameLevel(_) | Help::FameFriends => {
            panel == Some(Panel::Job) && resident.job_panel(snapshot.superstar) == JobPanel::Fame
        }
        Help::JobSkill(_) => {
            panel == Some(Panel::Job) && resident.job_panel(snapshot.superstar) == JobPanel::Adult
        }
    }
}

/// Finishes what [`layout`] cannot without the fonts and the install's
/// texts: sizes the sign of the zodiac's press area to its name, as the
/// personality panel sizes its control (0x51cd7f..0x51cdfe), sizes the
/// Report Card grade button (`0x532c20..0x532c4a`), and lays out the help
/// balloon. Hosts call it after [`layout`]; without it the sign and grade
/// button cannot be pressed and no balloon shows.
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
            Kind::ReportCardButton { panel } => {
                if let Some(font) = art.font(REPORT_CARD_GRADE_FONT) {
                    let line = font.line_height;
                    item.rect = Rect::new(
                        panel.x,
                        panel.y.saturating_add((panel.height - line) / 2),
                        panel.width,
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

/// Object GUIDs referenced by any laid-out help balloon in `items`, so
/// hosts can pre-load their catalog thumbnails via [`Thumbnails::read`].
pub fn balloon_objects(items: &[Item]) -> impl Iterator<Item = u32> + '_ {
    items
        .iter()
        .flat_map(|item| match &item.kind {
            Kind::Balloon(balloon) => Some(balloon.objects()),
            _ => None,
        })
        .flatten()
}

/// The live-mode strip: portraits at the left, the gadget and the subpanel
/// to its right. At width 800 the gadget sits at `x = 413` with a 280 px
/// subpanel from 520; at width 1024 (`0x518f40..0x518f48`, `0x51f258..0x51f260`,
/// `0x51f527..0x51f52f`) the gadget stays at `x = 413` and the subpanel
/// widens by 224 px (`0xe0`). Any wider viewport keeps the gadget anchored
/// beside the 4-column portrait block at `x = 413` and gives the extra width
/// to the active subpanel so the gadget never teleports away from the portraits.
fn live_strip(items: &mut Vec<Item>, strip: Rect, snapshot: &Snapshot, controls: &Controls) {
    let extra = strip
        .x
        .saturating_add(strip.width)
        .saturating_sub(800)
        .max(0);
    let gadget = Rect::new(
        strip.x + strip.width - (NEEDS_WIDTH + extra),
        strip.y,
        108,
        STRIP,
    );
    // The subpanel (0x517d9e, 0x517e3d): (0, 50, 280, 150) in a holder at
    // (300, 0) of the 150-high live panel at the strip's left, so 280 by
    // 100 from 300 into the strip, a pixel under the gadget's right edge,
    // and 224 wider at 1024 (0x51f258..0x51f260). Held to the gadget, as
    // the hit test is, so a window narrower than 800 keeps them together.
    let subpanel = Rect::new(
        gadget.x.saturating_add(SUBPANEL_FROM_GADGET),
        strip.y,
        SUBPANEL_WIDTH + extra,
        STRIP,
    );
    // The portraits, 45 by 45 from (9, 5) in a strip (the old prototype's
    // padding), in as many columns as fit before the gadget and two rows:
    // four columns at 800 by 600, so a family of eight fits. The second
    // row is **unverified**; people past the two rows are left out.
    let columns = ((gadget.x - strip.x - 9) / 45).max(1);
    let mut named = None;
    let selected = snapshot.selected_resident();
    for (n, resident) in snapshot.residents.iter().enumerate() {
        let n = i32::try_from(n).unwrap_or(i32::MAX);
        if n >= columns.saturating_mul(2) {
            break;
        }
        let x = strip.x + 9 + 45 * (n % columns);
        let y = strip.y + 5 + 45 * (n / columns);
        // 0x51ca97: highlights the roster slot matching `+0xe8` (matched by
        // neighbor/GUID at 0x51c5ef -> 0x4e97dc).
        let is_selected = selected.is_some_and(|s| s.object == resident.object);
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
                selected: is_selected,
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
    if let Some(resident) = selected {
        mood_bars(items, gadget, resident.motives[sims_sim::motives::MOOD]);
    }
    // Measured on the original's screen; the old prototype had the needs
    // button at (4, 33) and the first two rows two and one pixels higher.
    let g = |x: i32, y: i32, w: i32, h: i32| gadget.offset(x, y).with_size(w, h);
    let ig_disabled = !controls.interests_gifts_enabled
        || matches!(
            Selected::of(snapshot),
            Selected::Pet | Selected::NonHousehold
        );
    let open_panel = controls
        .panel
        .filter(|&panel| panel != Panel::House || snapshot.residential);
    for (id, panel, x, y, w, h) in [
        (id::NEEDS, Panel::Needs, 9, 31, 32, 39),
        (id::INTERESTS, Panel::Interests, 44, 4, 27, 30),
        (id::RELATIONSHIPS, Panel::Relationships, 76, 4, 27, 30),
        (id::PERSONALITY, Panel::Personality, 44, 35, 27, 30),
        (id::JOB, Panel::Job, 76, 35, 27, 30),
        (id::GIFTS, Panel::Gifts, 44, 66, 27, 30),
        (id::HOUSE, Panel::House, 76, 66, 27, 30),
    ] {
        // 0x51c5a6..0x51c9e5: selecting a pet (`person.data[65] > 1`) or
        // non-household Sim disables Interests ([esi + 0x170]) and Gifts
        // ([esi + 0x174]), and clearing selection (`0x51c5e1 -> 0x51c775`)
        // preserves their previous enabled state via
        // `controls.interests_gifts_enabled`. Whether another panel opens is
        // for the host to check at the press ([`check_press`]).
        let disabled = ig_disabled && matches!(panel, Panel::Interests | Panel::Gifts);
        items.push(Item {
            rect: g(x, y, w, h),
            kind: Kind::Button {
                id,
                cells: 4,
                selected: !disabled && open_panel == Some(panel),
            },
            command: (!disabled).then_some(Command::OpenPanel(panel)),
        });
    }
    // The original shows no needs without a selected Sim.
    if let (Some(resident), Some(Panel::Needs)) = (selected, open_panel) {
        let panel = subpanel;
        items.push(Item {
            rect: panel,
            kind: Kind::Needs { pet: resident.pet },
            command: None,
        });
        // The title, labels and meters, measured on the original's
        // screen: meters 60 by 5 in two columns 100 apart, rows 19 apart,
        // each label centered 15 above its meter.
        items.push(Item {
            rect: panel.offset(7, 2).with_size(100, 20),
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
            let x = panel.x + 20 + 100 * column;
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
            // The bar draws its arrows after its meter (0x50c2fa), at its
            // anchors in the need control.
            if let Some(arrows) = controls.trends[n] {
                items.push(Item {
                    rect: Rect::new(panel.x + 100 * column, panel.y + 21 + 19 * row, 100, 20),
                    kind: Kind::Trend(arrows),
                    command: None,
                });
            }
            // Clickable NeedMeter control (`0x513330`, `0x519a34..0x519a9e`:
            // 100x20 controls placed at 19px vertical stride from (0, 21)).
            items.push(Item {
                rect: Rect::new(panel.x + 100 * column, panel.y + 21 + 19 * row, 100, 20),
                kind: Kind::Area,
                command: Some(Command::Help(Help::Need {
                    pet: resident.pet,
                    index: n as u8,
                })),
            });
        }
    }
    if let (Some(resident), Some(Panel::Personality)) = (selected, open_panel) {
        let panel = subpanel;
        // 0x51d650..0x51d663, 0x51d1cc..0x51d1ee: pets (`person.data[65] > 1`)
        // hide the zodiac sign control `[esi + 0x1c4]`.
        let zodiac = (!resident.pet).then_some(resident.zodiac).flatten();
        items.push(Item {
            rect: panel.offset(7, 2).with_size(100, 20),
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
                zodiac,
                pet: resident.pet,
            },
            command: None,
        });
        if let Some(sign) = zodiac {
            items.push(Item {
                rect: Rect::new(panel.x + 5, panel.y + panel.height - 5, 0, 0),
                kind: Kind::Sign { sign, panel },
                command: Some(Command::Help(Help::Sign(sign))),
            });
        }
    }
    if let (Some(resident), Some(Panel::Job)) = (selected, open_panel) {
        let panel = subpanel;
        // 0x51d598..0x51d624, 0x51cbf8..0x51ccf4: Job-tab subpanel priority:
        // 1) school-age child -> Report Card ([esi + 0x124]),
        // 2) dog/cat pet -> Pet Skills ([esi + 0x114]/[esi + 0x118]),
        // 3) Superstar fame (0x4cfe2c) -> Fame ([esi + 0x11c]),
        // 4) otherwise -> Adult Job ([esi + 0x110]).
        match resident.job_panel(snapshot.superstar) {
            JobPanel::ReportCard => {
                items.push(Item {
                    rect: panel,
                    kind: Kind::ReportCard {
                        grade: resident.school_grade.clone(),
                    },
                    command: None,
                });
                items.push(Item {
                    rect: Rect::new(panel.x, panel.y + panel.height / 2, 0, 0),
                    kind: Kind::ReportCardButton { panel },
                    command: Some(Command::Help(Help::ReportCard)),
                });
            }
            JobPanel::PetSkills(kind) => {
                items.push(Item {
                    rect: panel
                        .offset(PET_SKILLS_TITLE[0], PET_SKILLS_TITLE[1])
                        .with_size(100, 20),
                    kind: Kind::Text {
                        text: "Pet Skills".into(),
                        size: 11,
                        rgb: LABEL,
                        align: Align::Left,
                    },
                    command: None,
                });
                items.push(Item {
                    rect: panel,
                    kind: Kind::PetSkills {
                        kind,
                        skills: [resident.skills[0], resident.skills[1], resident.skills[2]],
                    },
                    command: None,
                });
                let [lx, ly, lw, lh] = PET_SKILL_LABEL;
                for n in 0..3i32 {
                    items.push(Item {
                        rect: Rect::new(panel.x + lx, panel.y + ly + SKILL_PITCH * n, lw, lh),
                        kind: Kind::Area,
                        command: Some(Command::Help(Help::PetSkill {
                            cat: kind == PetKind::Cat,
                            index: n as u8,
                        })),
                    });
                }
            }
            JobPanel::Fame => {
                items.push(Item {
                    rect: panel,
                    kind: Kind::Fame {
                        fame_level: resident.fame_level,
                        fame_title: resident.fame_title.clone(),
                        skills: resident.skills,
                        needs: resident.fame_needs,
                        friends_text: resident.fame_friends_text.clone(),
                    },
                    command: None,
                });
                // Six skill label buttons (0x6f3eb0[0..6], 62x16).
                let [lx, ly, lw, lh] = FAME_SKILL_LABEL;
                for n in 0..6i32 {
                    items.push(Item {
                        rect: Rect::new(panel.x + lx, panel.y + ly + SKILL_PITCH * n, lw, lh),
                        kind: Kind::Area,
                        command: Some(Command::Help(Help::FameSkill(n as u8))),
                    });
                }
                // Fame level title button (0x6f3eb0[6] = (3, 45), 102x16;
                // 0x52c2ba / 0x52d462 keep level 1 if fame_level is outside 0..=10).
                let [tx, ty, tw, th] = FAME_LEVEL_BUTTON;
                let fame_balloon_level = if (0..=10).contains(&resident.fame_level) {
                    resident.fame_level as u8
                } else {
                    1
                };
                items.push(Item {
                    rect: Rect::new(panel.x + tx, panel.y + ty, tw, th),
                    kind: Kind::Area,
                    command: Some(Command::Help(Help::FameLevel(fame_balloon_level))),
                });
                // Friends total starpower button (0x6f3c58 = (3, 75), 120x20).
                let [fx, fy, fw, fh] = FAME_FRIENDS_BUTTON;
                items.push(Item {
                    rect: Rect::new(panel.x + fx, panel.y + fy, fw, fh),
                    kind: Kind::Area,
                    command: Some(Command::Help(Help::FameFriends)),
                });
            }
            JobPanel::Adult => {
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
                // The six skill labels open their help balloons (`0x53074c`
                // code 1), as the Superstar panel's do.
                let [lx, ly, lw, lh] = JOB_SKILL_BUTTON;
                for n in 0..6i32 {
                    items.push(Item {
                        rect: Rect::new(panel.x + lx, panel.y + ly + SKILL_PITCH * n, lw, lh),
                        kind: Kind::Area,
                        command: Some(Command::Help(Help::JobSkill(n as u8))),
                    });
                }
            }
        }
    }
    if let (Some(resident), Some(Panel::Interests)) = (selected, open_panel) {
        let panel = subpanel;
        interests(items, panel, resident, controls);
    }
    if let (Some(_), Some(Panel::Relationships)) = (selected, open_panel) {
        let panel = subpanel;
        relationships(items, panel, snapshot, controls);
    }
    if let (Some(_), Some(Panel::Gifts)) = (selected, open_panel) {
        let panel = subpanel;
        gifts(items, panel, snapshot, controls);
    }
    // The house panel's draw needs no selected Sim (0x52e49e), but with
    // nobody selected no panel is open ([`Controls::follow`]), and
    // `0x51d3f9..0x51d40f` closes panel 4 on non-residential lots.
    if open_panel == Some(Panel::House) {
        let panel = subpanel;
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
/// Pet personality labels in the panel's order (Live.iff STR# 170,
/// swapped in by `0x52157e..0x5215c0` when `person.data[65] > 1`).
const PET_TRAITS: [&str; 5] = ["Quiet", "Friendly", "Playful", "Smart", "Loyal"];
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
/// wide as the widest human label (`0x51e062..0x51e0c9`, `0x52144a`;
/// `0x52157e` swaps pet text without resizing), plus 3, and its bar
/// starts 3 after that; the bars end at [`TRAITS_RIGHT`]. The shorter of
/// the label's line and the bar is centred on the other. A trait is
/// `value / 100` points, 0 to 10.
///
/// The height compared is the row under an "A" (0x60b927).
///
/// **Unverified:** the labels' color (the needs' is taken) and their
/// alignment (right, against the bar).
fn draw_personality(frame: &mut Frame, art: &Art, panel: Rect, values: &[i16; 5], pet: bool) {
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
    // 0x5214e1, 0x52152f: the font's 0x70 method (0x60b927), the row
    // under an "A", not the line's height.
    let line = font
        .char_box('A')
        .map_or(font.line_height, |[_, bottom]| bottom);
    let (text_y, bar_y) = if line < empty_h {
        ((empty_h - line) / 2, 0)
    } else {
        (0, (line - empty_h) / 2)
    };
    let left = panel.x.saturating_add(TRAITS_RIGHT.saturating_sub(width));
    let names = if pet { &PET_TRAITS } else { &TRAITS };
    for (n, (name, value)) in names.iter().zip(values).enumerate() {
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

/// The personality panel's sign of the zodiac (0x51cd11..0x51cdfe): a
/// text box sized to the name and the font's line, 5 from the panel's
/// left and 5 above its bottom, the name centred in it as a text box's
/// is ([`text_box_y`]). **Unverified:** the color (the labels' is
/// taken).
fn draw_zodiac(frame: &mut Frame, art: &Art, panel: Rect, sign: u8) {
    let Some(font) = art.font(ZODIAC_FONT) else {
        return;
    };
    let name = ZODIAC_NAMES[usize::from(sign.clamp(1, 12) - 1)];
    let line = font.line_height;
    let top = panel.y.saturating_add(panel.height - line - 5);
    font.draw(
        frame,
        name,
        [panel.x.saturating_add(5), text_box_y(font, top, line)],
        LABEL,
    );
}

/// Where the original's text box (0x5e3f3c..0x5e3f78) draws its line in
/// a box `height` high from `top`: centred by the box the font gives an
/// "A" ([`Font::char_box`]), not by the line, the odd pixel truncated
/// toward zero.
fn text_box_y(font: &Font, top: i32, height: i32) -> i32 {
    let [a_top, a_bottom] = font.char_box('A').unwrap_or([0, font.line_height]);
    top.saturating_add((height - (a_bottom - a_top)) / 2 - a_top)
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
/// aligned in: 62 wide, the first at x 93, 15 apart (0x6f4420, which
/// starts them at y 6). On the original's screen the text is a pixel
/// higher than a 16-high box at 6 centres it: the box is taken at y 5
/// (measured; what sizes the boxes is not found).
const SKILL_FONT: u8 = 7;
const SKILL_LABEL: [i32; 4] = [93, 5, 62, 16];
/// The skill labels as buttons, where `0x6f4420` puts them: 62 by 16 from
/// (93, 6), 15 apart.
const JOB_SKILL_BUTTON: [i32; 4] = [93, 6, 62, 16];
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
            label.x.saturating_add(label.width - font.box_width(name)),
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

/// The pet skills panel's title position (`0x531b7a..0x531bbf`,
/// `0x5326a9..0x5326d4`), labels (`Live.iff STR# 171`, `0x532262..0x532285`),
/// label boxes (`0x6f4518`, `0x66 x 0x10` right-aligned in font 7), and bar
/// origins (`0x6f4488`, pitch 15).
const PET_SKILLS_TITLE: [i32; 2] = [13, 8];
const DOG_SKILL_NAMES: [&str; 3] = ["House Breaking", "Tricks", "Obedience"];
const CAT_SKILL_NAMES: [&str; 3] = ["Hunting", "Tricks", "Obedience"];
const PET_SKILL_LABEL: [i32; 4] = [103, 25, 102, 16];
const PET_SKILL_BAR: [i32; 2] = [207, 27];

/// Draws the Dog (`0x5319f8..0x531b5c`) or Cat (`0x532512..0x532676`)
/// Pet Skills panel in `panel`: three right-aligned labels and skill bars
/// using `id::SKILL_EMPTY` (4920) and `id::SKILL_FULL` (4901) for
/// `person.data` words `[10, 12, 11]` (`0x66e360`).
fn draw_pet_skills(frame: &mut Frame, art: &Art, panel: Rect, kind: PetKind, skills: &[i16; 3]) {
    let (Some(font), Some(empty), Some(full)) = (
        art.font(SKILL_FONT),
        art.image(id::SKILL_EMPTY),
        art.image(id::SKILL_FULL),
    ) else {
        return;
    };
    let full_w = i32::try_from(full.width).unwrap_or(0);
    let full_h = i32::try_from(full.height).unwrap_or(0);
    let names = match kind {
        PetKind::Dog => &DOG_SKILL_NAMES,
        PetKind::Cat => &CAT_SKILL_NAMES,
    };
    for (n, (name, skill)) in names.iter().zip(skills).enumerate() {
        let row = SKILL_PITCH * n as i32;
        let bar = [
            panel.x.saturating_add(PET_SKILL_BAR[0]),
            panel.y.saturating_add(PET_SKILL_BAR[1] + row),
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
        let [x, y, w, h] = PET_SKILL_LABEL;
        let label = Rect::new(
            panel.x.saturating_add(x),
            panel.y.saturating_add(y + row),
            w,
            h,
        );
        let at = [
            label.x.saturating_add(label.width - font.box_width(name)),
            // 0x53180d..0x531826, 0x5321e7..0x532200, 0x5e3f0d..0x5e4053:
            // text rect local top is 0 and the renderer draws directly at it.
            label.y,
        ];
        font.draw(frame, name, at, LABEL);
    }
}

/// The Child Report Card subpanel's title font (`0x6f2aec` = font 10) and
/// centered grade button font (`0x6f2b0c` = font 18, `0x532bb1`).
const REPORT_CARD_TITLE_FONT: u8 = 10;
const REPORT_CARD_GRADE_FONT: u8 = 18;
const REPORT_CARD_TITLE: &str = "Report Card";

/// Draws the Child Report Card subpanel (`[esi + 0x124]`, `0x532d47..0x532e63`):
/// title `"Report Card"` in font 10 at `(panel.x, panel.y)` (western), and
/// the child's grade (`work.iff STR# 4097` or `"No cJob Object!"`) centered
/// in font 18 vertically centered in `panel`.
///
/// **Not drawn:** the 333 ms blink for grades `>= 10` (`0x532de9 -> 0x5306df`,
/// same blink button as the Adult Job panel's performance).
fn draw_report_card(frame: &mut Frame, art: &Art, panel: Rect, grade: &str) {
    if let Some(font) = art.font(REPORT_CARD_TITLE_FONT) {
        font.draw(frame, REPORT_CARD_TITLE, [panel.x, panel.y], LABEL);
    }
    if let Some(font) = art.font(REPORT_CARD_GRADE_FONT) {
        let w = font.width(grade);
        let x = panel.x.saturating_add((panel.width - w) / 2);
        let y = panel
            .y
            .saturating_add((panel.height - font.line_height) / 2);
        font.draw(frame, grade, [x, y], LABEL);
    }
}

/// Superstar / Fame subpanel (`[esi + 0x11c]`, `0x52b8cc..0x52d503`) layout
/// constants for western languages (`0x6f3c38`, `0x6f3c48`, `0x6f3c58`,
/// `0x6f3d28..0x6f3d58`, `0x6f3eb0`).
const FAME_TITLE_POS: [i32; 2] = [5, 0];
const FAME_TITLE_FONT: u8 = 11;
const FAME_TITLE: &str = "Fame";
const FAME_BAR_POS: [i32; 2] = [3, 30];
const FAME_LEVEL_BUTTON: [i32; 4] = [3, 45, 102, 16];
const FAME_FRIENDS_BUTTON: [i32; 4] = [3, 75, 120, 15];
const FAME_SKILL_LABEL: [i32; 4] = [145, 6, 62, 16];
const FAME_SKILL_BAR: [i32; 2] = [210, 7];

/// Draws the Superstar / Fame subpanel (`0x52c8cc`, `0x52ca13`, `0x52cb7f`,
/// `0x52cc11`, `0x52d1af`, `0x52d3e6`) in `panel`:
/// - title `"Fame"` in font 11 at `(5, 0)`;
/// - 5-star fame bar (`id::FAME_BAR_EMPTY` / `id::FAME_BAR_FULL`, 80x16) at
///   `(3, 30)`, filled 8 px per half-star (`fame_level` in `0..=10`);
/// - fame rank title in font 7 left-aligned in `(3, 45, 102, 16)`;
/// - friends starpower icon (`id::FAME_FRIENDS`, 4 cells) and text in font 7
///   at `(3, 75)`;
/// - six skill labels `(145, 6 + 15*n, 62, 16)` and bars `(210, 7 + 15*n)`
///   with underline + L-shaped target mark when `needs[n] > 0` (`needs` in
///   whole skill points `1..=10`).
#[allow(clippy::too_many_arguments)]
fn draw_fame(
    frame: &mut Frame,
    art: &Art,
    panel: Rect,
    pointer: Option<[i32; 2]>,
    fame_level: i16,
    fame_title: &str,
    skills: &[i16; 6],
    needs: &[i32; 6],
    friends_text: &str,
) {
    if let Some(title_font) = art.font(FAME_TITLE_FONT) {
        title_font.draw(
            frame,
            FAME_TITLE,
            [
                panel.x.saturating_add(FAME_TITLE_POS[0]),
                panel.y.saturating_add(FAME_TITLE_POS[1]),
            ],
            LABEL,
        );
    }
    // 0x52c8cc..0x52ca02: empty 5-star bar, then full bar cut to 8 px per
    // half-star (full width at level 10 or when level * 8 > full_w).
    let bar_at = [
        panel.x.saturating_add(FAME_BAR_POS[0]),
        panel.y.saturating_add(FAME_BAR_POS[1]),
    ];
    if let Some(empty) = art.image(id::FAME_BAR_EMPTY) {
        frame.draw_all(empty, bar_at);
    }
    if fame_level > 0
        && let Some(full) = art.image(id::FAME_BAR_FULL)
    {
        let full_w = i32::try_from(full.width).unwrap_or(0);
        let full_h = i32::try_from(full.height).unwrap_or(0);
        let level = i32::from(fame_level);
        let width = level.saturating_mul(8);
        let cut = if width <= full_w && level != 10 {
            width
        } else {
            full_w
        };
        if cut > 0 {
            frame.draw(full, Rect::new(0, 0, cut, full_h), bar_at);
        }
    }
    if let Some(font) = art.font(SKILL_FONT) {
        // Fame level title top-aligned, left-aligned at (3, 45) (`0x52c026..0x52c05c`,
        // `0x5e3f04..0x5e3f35`).
        let [tx, ty, _, _] = FAME_LEVEL_BUTTON;
        font.draw(
            frame,
            fame_title,
            [panel.x.saturating_add(tx), panel.y.saturating_add(ty)],
            LABEL,
        );
        // Friends total starpower 4-cell button icon + text at (3, 75)
        // (`0x52c369..0x52c4a1`, hover cell 2 via `0x5e3be6..0x5e3c11`).
        let [fx, fy, fw, fh] = FAME_FRIENDS_BUTTON;
        let button = Rect::new(
            panel.x.saturating_add(fx),
            panel.y.saturating_add(fy),
            fw,
            fh,
        );
        let mut text_x = button.x;
        let cell = if pointer.is_some_and(|[px, py]| button.contains(px, py)) {
            Cell::Hover
        } else {
            Cell::Normal
        };
        if let Some(icon) = art.cell(id::FAME_FRIENDS, 4, cell as u32) {
            frame.draw_all(&icon, [button.x, button.y]);
            text_x = button
                .x
                .saturating_add(i32::try_from(icon.width).unwrap_or(0) + 3);
        }
        font.draw(frame, friends_text, [text_x, button.y], LABEL);
    }
    // Six skills shifted to (145, 6) labels and (210, 7) bars (0x52ca13..0x52ccdf).
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
            panel.x.saturating_add(FAME_SKILL_BAR[0]),
            panel.y.saturating_add(FAME_SKILL_BAR[1] + row),
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
        let [x, y, w, h] = FAME_SKILL_LABEL;
        let label = Rect::new(
            panel.x.saturating_add(x),
            panel.y.saturating_add(y + row),
            w,
            h,
        );
        // 0x52c026..0x52c0c8, 0x5e3f04..0x5e3f35: top-aligned, right-aligned.
        let at = [
            label.x.saturating_add(label.width - font.box_width(name)),
            label.y,
        ];
        if *need <= 0 {
            font.draw(frame, name, at, LABEL);
            continue;
        }
        font.draw_underlined(frame, name, at, LABEL);
        let mark = bar[0]
            .saturating_add(need.saturating_mul(4))
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
/// - **Ratings:** each rating's label, in the labels' font, in a text box
///   the font's measure wide (`+0x68`) and its line high (`+0x58`), its
///   right end 3 left of its bar (0x52e0bb..0x52e10c); the empty bar
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
/// The labels and facts are centred in their boxes by the rows an "A"
/// inks, as the original's text boxes are (0x5e3f42..0x5e3f78), and
/// placed by the font's measure ([`Font::box_width`]).
///
/// **Unverified:** the labels' color (the needs labels' is taken).
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
        // A text box as high as the line (0x52e0f3), its text centred
        // as a text box's is.
        let top = panel.y.saturating_add(RATING_TOPS[n]);
        let label = [
            bar[0]
                .saturating_sub(3)
                .saturating_sub(font.box_width(name)),
            text_box_y(font, top, font.line_height),
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
            x.saturating_add(w - font.box_width(text)),
            text_box_y(font, y, h),
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
/// Text is in the labels' color, but the performance's is
/// [`PERFORMANCE_LIT`] on the lit half of its blink (`lit`, while
/// [`performance_blinks`]; `0x52c659` swaps the gadget's text color, not
/// its icon). **Not drawn:** the tooltips. Which lines of a title that
/// wraps past its box show is **unverified**: here the lines that fit.
fn draw_work(
    frame: &mut Frame,
    art: &Art,
    panel: Rect,
    work: &Work,
    performance: i16,
    family_friends: i32,
    lit: bool,
) {
    let lit = lit && work.level.is_some() && performance < 0;
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
        let rgb = if lit && id == id::PERFORMANCE {
            PERFORMANCE_LIT
        } else {
            LABEL
        };
        font.draw(frame, &text, [text_x, y], rgb);
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

/// Native `cWinInterests` labels (`Live.iff STR# 140[1..=19]`, indexed `0..19`
/// matching `id::INTEREST_ICONS` `1600..=1618` and `household::INTEREST_WORDS`).
const INTEREST_LABELS: [&str; 19] = [
    "Travel",
    "Money",
    "Politics",
    "The 60's",
    "Weather",
    "Sports",
    "Music",
    "Outdoors",
    "Toys",
    "Aliens",
    "Pets",
    "School",
    "Exercise",
    "Food",
    "Parties",
    "Style",
    "Hollywood",
    "Technology",
    "Romance",
];
/// Adult interest indices in `cWinInterests` order (`0x51e26a..0x51e293`: 15 items).
const ADULT_INTERESTS: [usize; 15] = [0, 1, 2, 3, 4, 5, 6, 7, 12, 13, 14, 15, 16, 17, 18];
/// Child interest indices in `cWinInterests` order (`0x51e26a..0x51e293`: 8 items).
const CHILD_INTERESTS: [usize; 8] = [8, 9, 10, 11, 4, 5, 6, 7];

/// Lays out the Interests subpanel (`Panel::Interests`, `4505`, `0x50f3c0..0x50f6f0`,
/// `0x51e260..0x51e342`) with native `Interest*.bmp` icons, `InterestBars.BMP`
/// meters, and left/right paging arrows.
fn interests(
    items: &mut Vec<Item>,
    panel: Rect,
    resident: &crate::household::Resident,
    controls: &Controls,
) {
    items.push(Item {
        rect: panel.offset(6, 2).with_size(100, 16),
        kind: Kind::Text {
            text: "Interests".into(),
            size: 11,
            rgb: LABEL,
            align: Align::Left,
        },
        command: None,
    });
    let list: &[usize] = if resident.school_age {
        &CHILD_INTERESTS
    } else {
        &ADULT_INTERESTS
    };
    let [arrow_w, arrow_h] = [9, 49];
    let cell_w = 120;
    let cell_h = 25;
    let col_stride = cell_w + 3;
    let row_stride = cell_h + 2;
    let rows = 3usize;
    let left = arrow_w + 6;
    let right = panel.width - arrow_w - 4;
    // 0x519251 / 0x519274: 2 columns (6/page) at 800 px (`panel.width == 280`),
    // 3 columns (9/page) at 1024 px (`panel.width == 504`), never exceeding
    // the columns that fit strictly inside `left..right` at intermediate widths
    // such as 880 px.
    let max_fit = ((right - left + 3) / col_stride).max(1) as usize;
    let cols = if panel.width <= 520 {
        max_fit.min(3)
    } else {
        max_fit
    };
    let per_page = (cols * rows).max(1);
    let pages = list.len().div_ceil(per_page).max(1);
    let page = (controls.interests_page as usize).min(pages - 1);
    let arrow_y = (panel.height - arrow_h) / 2;
    if page > 0 {
        items.push(Item {
            rect: panel.offset(3, arrow_y).with_size(arrow_w, arrow_h),
            kind: Kind::Button {
                id: id::PAGE_BACK,
                cells: 4,
                selected: false,
            },
            command: Some(Command::InterestsPage((page - 1) as u32)),
        });
    }
    if (page + 1) * per_page < list.len() {
        items.push(Item {
            rect: panel
                .offset(panel.width - arrow_w - 2, arrow_y)
                .with_size(arrow_w, arrow_h),
            kind: Kind::Button {
                id: id::PAGE_ON,
                cells: 4,
                selected: false,
            },
            command: Some(Command::InterestsPage((page + 1) as u32)),
        });
    }
    for (slot, &idx) in list.iter().skip(page * per_page).take(per_page).enumerate() {
        let col = (slot / rows) as i32;
        let row = (slot % rows) as i32;
        let x = left + col * col_stride;
        let y = 18 + row * row_stride;
        let points = (resident.interests[idx] / 100).clamp(0, 10) as u8;
        items.push(Item {
            rect: panel.offset(x, y).with_size(cell_w, cell_h),
            kind: Kind::Interest {
                icon_id: id::INTEREST_ICONS[idx],
                points,
                label: INTEREST_LABELS[idx],
            },
            command: None,
        });
    }
}

/// Native inventory sort tab buttons down the left edge of `Panel::Gifts`
/// (`0x51aa96..0x51ac9e`, 4-cell `20×20` buttons).
const INVENTORY_TABS: [(i32, InventoryTab); 3] = [
    (id::INVENTORY_GIFT_SORT, InventoryTab::Gifts),
    (id::INVENTORY_MAGIC_SORT, InventoryTab::Magic),
    (id::INVENTORY_OTHER_SORT, InventoryTab::Other),
];

/// Whether an inventory token kind belongs to `tab` (`0x51e3b4..0x51e440`:
/// kinds `0, 4, 5` -> Gifts, kind `7` -> Magic, kind `8` -> Other; internal
/// bookkeeping tokens such as kinds `1` and `3` are excluded).
pub fn inventory_kind_matches_tab(kind: i32, tab: InventoryTab) -> bool {
    match tab {
        InventoryTab::Gifts => matches!(kind, 0 | 4 | 5),
        InventoryTab::Magic => kind == 7,
        InventoryTab::Other => kind == 8,
    }
}

/// Lays out the Gifts / Inventory subpanel (`Panel::Gifts`, `4507`,
/// `0x51aa96..0x51ac9e`, `0x51e343..0x51e440`) with the three native sort tabs,
/// paging arrows, and item token slots.
fn gifts(items: &mut Vec<Item>, panel: Rect, snapshot: &Snapshot, controls: &Controls) {
    let mut y = 6;
    let button_w = 20;
    for &(bmp_id, tab) in &INVENTORY_TABS {
        let rect = panel.offset(3, y).with_size(button_w, 20);
        y += 24;
        items.push(Item {
            rect,
            kind: Kind::Button {
                id: bmp_id,
                cells: 4,
                selected: controls.inventory_tab == tab,
            },
            command: Some(Command::ShowInventory(tab)),
        });
    }
    let shown: Vec<&InventoryEntry> = snapshot
        .inventory
        .iter()
        .filter(|entry| inventory_kind_matches_tab(entry.kind, controls.inventory_tab))
        .collect();
    let [arrow_w, arrow_h] = [9, 49];
    let cell_w = 45;
    let cell_h = 45;
    // 0x519262 / 0x51927e / 0x51e355: native `[esi + 0x450] - 1` yields 5 items
    // per page at 800 px (`panel.width == 280`) and 9 items per page at 1024 px
    // (`panel.width == 504`).
    let pitch_x = if panel.width <= 300 { 46 } else { 48 };
    let left = button_w + arrow_w + 6;
    let right = panel.width - arrow_w - 2;
    let fit = ((right - left + (pitch_x - cell_w)) / pitch_x).max(1) as usize;
    let per_page = if panel.width <= 520 && panel.width > 300 {
        fit.min(9)
    } else {
        fit
    };
    let pages = shown.len().div_ceil(per_page).max(1);
    let page = (controls.inventory_page as usize).min(pages - 1);
    let arrow_y = (panel.height - arrow_h) / 2;

    let selected_entry = controls
        .inventory_selected
        .and_then(|guid| shown.iter().copied().find(|e| e.guid == guid));
    let header = match selected_entry {
        Some(entry) => format!("{} (x{})", entry.name, entry.count),
        None => match controls.inventory_tab {
            InventoryTab::Gifts => "Inventory - Gifts".into(),
            InventoryTab::Magic => "Inventory - Magic".into(),
            InventoryTab::Other => "Inventory - Other".into(),
        },
    };
    items.push(Item {
        rect: panel.offset(left, 4).with_size((right - left).max(40), 16),
        kind: Kind::Text {
            text: header,
            size: 10,
            rgb: LABEL,
            align: Align::Left,
        },
        command: None,
    });

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
            command: Some(Command::InventoryPage((page - 1) as u32)),
        });
    }
    if (page + 1) * per_page < shown.len() {
        items.push(Item {
            rect: panel
                .offset(panel.width - arrow_w - 2, arrow_y)
                .with_size(arrow_w, arrow_h),
            kind: Kind::Button {
                id: id::PAGE_ON,
                cells: 4,
                selected: false,
            },
            command: Some(Command::InventoryPage((page + 1) as u32)),
        });
    }

    if shown.is_empty() {
        items.push(Item {
            rect: panel.offset(left, 40).with_size((right - left).max(40), 16),
            kind: Kind::Text {
                text: "No items".into(),
                size: 9,
                rgb: rgb565(0x8430),
                align: Align::Left,
            },
            command: None,
        });
        return;
    }

    for (idx, entry) in shown
        .iter()
        .skip(page * per_page)
        .take(per_page)
        .enumerate()
    {
        let x = left + pitch_x * idx as i32;
        let rect = panel.offset(x, 24).with_size(cell_w, cell_h);
        let is_selected = controls.inventory_selected == Some(entry.guid);
        items.push(Item {
            rect,
            kind: Kind::InventoryItem {
                entry: (*entry).clone(),
                selected: is_selected,
            },
            command: Some(Command::SelectInventoryItem(Some(entry.guid))),
        });
    }
}

/// Draws one native interest icon + 10-point bar (`0x50f475..0x50f580`).
fn draw_interest(frame: &mut Frame, art: &Art, rect: Rect, icon_id: i32, points: u8, label: &str) {
    let icon_w = if let Some(icon) = art.image(icon_id) {
        frame.draw_all(icon, [rect.x, rect.y]);
        i32::try_from(icon.width).unwrap_or(75)
    } else {
        if let Some(font) = art.font(8) {
            font.draw(frame, label, [rect.x + 2, rect.y + 5], LABEL);
        }
        75
    };
    let bar_x = rect.x + icon_w + 5;
    let bar_y = rect.y + 5;
    let fill_w = i32::from(points.min(10)) * 4;
    if let Some(bg) = art.image(id::INTEREST_BARS_BG) {
        frame.draw_all(bg, [bar_x, bar_y]);
    } else {
        frame.fill(Rect::new(bar_x, bar_y, 40, 12), [24, 38, 76]);
    }
    if fill_w > 0 {
        if let Some(fg) = art.image(id::INTEREST_BARS) {
            let h = i32::try_from(fg.height).unwrap_or(16);
            let w = fill_w.min(i32::try_from(fg.width).unwrap_or(40));
            frame.draw(fg, Rect::new(0, 0, w, h), [bar_x, bar_y]);
        } else {
            frame.fill(Rect::new(bar_x, bar_y, fill_w, 12), GREEN);
        }
    }
}

/// Draws one inventory token slot inside `Panel::Gifts`: 45×45 catalog template
/// cell + object thumbnail + count badge (`xN`) + short name below.
fn draw_inventory_item(
    frame: &mut Frame,
    art: &Art,
    thumbnails: Option<&Thumbnails>,
    rect: Rect,
    entry: &InventoryEntry,
    selected: bool,
    hovered: bool,
) {
    let cell = if selected {
        1
    } else if hovered {
        2
    } else {
        0
    };
    if let Some(bg) = art.cell(crate::thumbnail::TEMPLATE, 4, cell) {
        frame.draw_all(&bg, [rect.x, rect.y]);
    } else {
        let fill = if selected {
            [44, 96, 176]
        } else if hovered {
            [32, 68, 140]
        } else {
            [20, 42, 102]
        };
        frame.fill(rect, fill);
    }
    if let Some(thumbs) = thumbnails {
        if let Some(thumb) = thumbs.item(entry.guid).and_then(|i| i.thumbnail.as_ref()) {
            let cell_sz = crate::thumbnail::SIZE;
            let half = if thumb.width > cell_sz {
                thumb.width / 2
            } else {
                thumb.width
            };
            let src_x = if (selected || hovered) && thumb.width > cell_sz {
                half
            } else {
                0
            };
            let dx = (cell_sz as i32 - half as i32) / 2;
            let dy = (cell_sz as i32 - thumb.height as i32) / 2;
            frame.draw(
                thumb,
                Rect::new(src_x as i32, 0, half as i32, thumb.height as i32),
                [rect.x + dx, rect.y + dy],
            );
        } else if let Some(icon) = thumbs.icon(art, entry.guid) {
            let draw_w = (icon.width as i32).min(rect.width);
            let draw_h = (icon.height as i32).min(rect.height);
            frame.draw(&icon, Rect::new(0, 0, draw_w, draw_h), [rect.x, rect.y]);
        }
    }
    if let Some(font) = art.font(8).or_else(|| art.font(9)) {
        if entry.count > 1 {
            let badge = format!("x{}", entry.count);
            let bw = font.width(&badge) + 4;
            let bh = font.line_height.max(10);
            let bx = rect.x + rect.width - bw - 1;
            let by = rect.y + rect.height - bh - 1;
            frame.fill(Rect::new(bx, by, bw, bh), [8, 16, 44]);
            font.draw(frame, &badge, [bx + 2, by], [255, 232, 128]);
        }
        // Abbreviated item label centered directly beneath the 45×45 slot.
        let mut short = entry.name.clone();
        while !short.is_empty() && font.width(&short) > rect.width + 2 {
            short.pop();
        }
        if !short.is_empty() {
            let tw = font.width(&short);
            let tx = rect.x + (rect.width - tw) / 2;
            font.draw(frame, &short, [tx, rect.y + rect.height + 2], LABEL);
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
/// The install has no 6-point font: the font table (0x4ff546..0x4ff62b)
/// loads sizes 0 to 23, drops those missing, and fills each empty slot
/// below the smallest font with it, so slot 6 is the 7-point font, 13
/// high.
const RELATIONS_FONT_HEIGHT: i32 = 13;
/// The panel's title, in that font (a label in slot 6's font), measured
/// on the original's screen.
const RELATIONS_TITLE: [i32; 2] = [7, 2];
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
    items.push(Item {
        rect: panel
            .offset(RELATIONS_TITLE[0], RELATIONS_TITLE[1])
            .with_size(100, fh),
        kind: Kind::Text {
            text: "Relationships".into(),
            size: RELATION_FONT,
            rgb: LABEL,
            align: Align::Left,
        },
        command: None,
    });
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
    // 0x519247..0x51926a: 5 people per page at 800px (`panel.width = 280`),
    // 10 at 1024px (`panel.width = 504`), scaling cleanly with wider panels
    // while keeping a gutter for the right page arrow (`arrow_w + 2`).
    let [arrow_w, arrow_h] = [9, 49];
    let [w, h] = RELATION_SIZE;
    let left = arrow_w + button_w + 6;
    let right = panel.width - arrow_w - 2;
    let per_page = ((right - left) / w).max(1) as usize;
    // 0x51da2e: a page past the end steps back to the last one.
    let pages = shown.len().div_ceil(per_page).max(1);
    let page = (controls.relations_page as usize).min(pages - 1);
    // 0x51ddec: the arrows centred on the panel below the font's line.
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
    if (page + 1) * per_page < shown.len() {
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
    // 0x4ebdfa: single row of `per_page` entries from `left` across the panel.
    for (idx, relation) in shown
        .iter()
        .skip(page * per_page)
        .take(per_page)
        .enumerate()
    {
        let x = left.saturating_add(w.saturating_mul(i32::try_from(idx).unwrap_or(0)));
        let rect = panel.offset(x, RELATIONS_TOP).with_size(w, h);
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
/// is taken); that `lit` is
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
        // The font's centred draw (0x60b574..0x60b58d): half the box,
        // less half the text, each halved toward zero.
        let x = box_
            .x
            .saturating_add(box_.width / 2 - font.box_width(&text) / 2);
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
pub const NEEDS: [(&str, usize); 8] = [
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

/// Draws the panel into `frame` without object catalog thumbnails. Pieces
/// whose art is missing are left out.
pub fn draw(frame: &mut Frame, art: &Art, faces: &Faces, items: &[Item], controls: &Controls) {
    draw_with_thumbnails(frame, art, faces, None, items, controls);
}

/// Draws the panel into `frame`, using `thumbnails` (when provided) for
/// object picture slots inside help balloons. Pieces whose art is missing
/// are left out.
pub fn draw_with_thumbnails(
    frame: &mut Frame,
    art: &Art,
    faces: &Faces,
    thumbnails: Option<&Thumbnails>,
    items: &[Item],
    controls: &Controls,
) {
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
                    if *id == id::STRIP && r.width > w && w > 16 {
                        // `PanelBack.bmp` (`804×100`) ends with an 8 px dark
                        // right bezel (`796..804`). On viewports wider than
                        // 1024 px (`r.width > 804`), repeat an interior navy
                        // column (`w - 16`) across the extension and place the
                        // right bezel at the viewport's right edge so no dark
                        // vertical band appears at `x = 1024`. Widths <= 1024
                        // take the `else` branch and stay 100% bit-exact.
                        const RIGHT_BEZEL: i32 = 8;
                        let body_w = w - RIGHT_BEZEL;
                        frame.draw(image, Rect::new(0, 0, body_w, h), [r.x, r.y]);
                        let left = i64::from(r.x);
                        let fill_col = w - 16;
                        let start = i64::from(body_w).max(-left);
                        let end =
                            i64::from(r.width - RIGHT_BEZEL).min(i64::from(frame.width()) - left);
                        for x in start..end {
                            frame.draw(
                                image,
                                Rect::new(fill_col, 0, 1, h),
                                [(left + x) as i32, r.y],
                            );
                        }
                        frame.draw(
                            image,
                            Rect::new(body_w, 0, RIGHT_BEZEL, h),
                            [r.x.saturating_add(r.width - RIGHT_BEZEL), r.y],
                        );
                    } else {
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
            }
            Kind::Button {
                id,
                cells,
                selected,
            } => {
                let hover = controls.pointer.is_some_and(|[x, y]| r.contains(x, y));
                let pressed = hover
                    && controls
                        .pressed_options_action
                        .is_some_and(|action| item.command == Some(Command::OptionsAction(action)));
                let cell = if *selected || pressed {
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
            Kind::Toggle { id, checked } => {
                let hover = controls.pointer.is_some_and(|[x, y]| r.contains(x, y));
                options::draw_toggle(frame, art, r, *id, *checked, item.command.is_some(), hover);
            }
            Kind::Slider { value, .. } => {
                options::draw_slider(frame, art, r, *value);
            }
            Kind::Cell { id, cells, cell } => {
                if let Some(image) = art.cell(*id, *cells, *cell) {
                    frame.draw_all(&image, [r.x, r.y]);
                }
            }
            Kind::Friends(count) => {
                if let Some(font) = art.font(10) {
                    let money_width = font.width("$9,999,999");
                    let right = r.x.saturating_add(r.width).saturating_sub(money_width);
                    let text = count.to_string();
                    let x = right.saturating_sub(font.width(&text));
                    font.draw(frame, &text, [x, r.y], GREEN);
                }
            }
            Kind::Pause { paused, lit } => {
                if let Some(image) = art.image(id::PAUSE) {
                    // 0x5e3b40..0x5e3c3e (4-column cGZWinBtn): disabled -> 3,
                    // selected -> 1, hovered -> 2, normal -> 0.
                    // 0x5e4b0f draws src_x = col * 15, src_y = row * 15.
                    let hover = controls.pointer.is_some_and(|[x, y]| r.contains(x, y));
                    let cell = if item.command.is_none() {
                        Cell::Disabled
                    } else if *paused {
                        Cell::Selected
                    } else if hover {
                        Cell::Hover
                    } else {
                        Cell::Normal
                    };
                    let col = (cell as i32) * 15;
                    let row = if *lit { 15 } else { 0 };
                    frame.draw(image, Rect::new(col, row, 15, 15), [r.x, r.y]);
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
                        // Signed truncation toward zero, as the cluster
                        // clock centers (`0x539f3e..0x539f43`).
                        Align::CenterTrunc => x + (width - w) / 2,
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
            Kind::Personality {
                traits,
                zodiac,
                pet,
            } => {
                draw_personality(frame, art, r, traits, *pet);
                if let Some(sign) = zodiac {
                    draw_zodiac(frame, art, r, *sign);
                }
            }
            Kind::Skills { skills, needs } => draw_skills(frame, art, r, skills, needs),
            Kind::PetSkills { kind, skills } => draw_pet_skills(frame, art, r, *kind, skills),
            Kind::ReportCard { grade } => draw_report_card(frame, art, r, grade),
            Kind::Fame {
                fame_level,
                fame_title,
                skills,
                needs,
                friends_text,
            } => draw_fame(
                frame,
                art,
                r,
                controls.pointer,
                *fame_level,
                fame_title,
                skills,
                needs,
                friends_text,
            ),
            Kind::Work {
                work,
                performance,
                family_friends,
            } => draw_work(
                frame,
                art,
                r,
                work,
                *performance,
                *family_friends,
                controls.performance_lit,
            ),
            Kind::Relation { relation, lit } => draw_relation(frame, art, faces, r, relation, *lit),
            Kind::Interest {
                icon_id,
                points,
                label,
            } => draw_interest(frame, art, r, *icon_id, *points, label),
            Kind::InventoryItem { entry, selected } => {
                let hovered = controls.pointer.is_some_and(|[x, y]| r.contains(x, y));
                draw_inventory_item(frame, art, thumbnails, r, entry, *selected, hovered);
            }
            Kind::House { house } => draw_house(frame, art, r, house),
            Kind::Needs { .. }
            | Kind::ReportCardButton { .. }
            | Kind::Area
            | Kind::Sign { .. }
            | Kind::Help { .. } => {}
            Kind::Balloon(b) => balloon::draw_with_thumbnails(frame, art, thumbnails, b),
            Kind::OptionsBalloon(b) => options::draw_balloon(frame, art, b),
            Kind::OptionsTooltip(tip) => options::draw_tooltip(frame, art, r, tip),
            Kind::Trend(arrows) => {
                for ([x, y], rgb) in arrows.pixels([r.x, r.y]) {
                    let [_, red, green, blue] = rgb.to_be_bytes();
                    frame.fill(Rect::new(x, y, 1, 1), [red, green, blue]);
                }
            }
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
                    | Kind::Friends(_)
                    | Kind::Bars { .. }
                    | Kind::Meter(_)
                    | Kind::Trend(_)
                    | Kind::Needs { .. }
                    | Kind::Name { .. }
                    | Kind::Personality { .. }
                    | Kind::Skills { .. }
                    | Kind::PetSkills { .. }
                    | Kind::ReportCard { .. }
                    | Kind::ReportCardButton { .. }
                    | Kind::Fame { .. }
                    | Kind::Area
                    | Kind::Work { .. }
                    | Kind::Relation { .. }
                    | Kind::Help { .. }
                    | Kind::OptionsTooltip(_)
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
/// there, if it has one. An item drawn over a button hides it. Pressing
/// a [`Kind::Slider`] maps `point[0]` to its volume step `0..=10`
/// (`0x5f633d..0x5f640b`).
pub fn press(items: &[Item], point: [i32; 2]) -> Option<Command> {
    let item = items.get(topmost(items, point)?)?;
    if let Kind::Slider { channel, .. } = item.kind {
        let step = options::slider_value_at(item.rect, point[0]);
        return Some(Command::SetVolume { channel, step });
    }
    item.command.clone()
}

/// If the topmost press-taking item at `point` is an Options volume slider,
/// returns its channel and bounding rectangle (for click-and-drag capture,
/// `0x5f6245..0x5f6336`).
pub fn slider_at(items: &[Item], point: [i32; 2]) -> Option<(VolumeChannel, Rect)> {
    let item = items.get(topmost(items, point)?)?;
    match item.kind {
        Kind::Slider { channel, .. } => Some((channel, item.rect)),
        _ => None,
    }
}

/// Returns the bounding rectangle of `channel`'s slider in `items`, if laid out.
pub fn slider_rect(items: &[Item], channel: VolumeChannel) -> Option<Rect> {
    items.iter().find_map(|item| match item.kind {
        Kind::Slider { channel: ch, .. } if ch == channel => Some(item.rect),
        _ => None,
    })
}

/// If the topmost press-taking item at `point` is an Interests or Gifts
/// gadget button that was laid out disabled (`command: None`), returns the
/// panel it opens. Hosts check this against the live selection so a press
/// after switching from a pet to a human before redraw still opens the panel
/// (`0x51c5a6` re-enables the button synchronously on selection).
pub fn disabled_panel(items: &[Item], point: [i32; 2]) -> Option<Panel> {
    let item = items.get(topmost(items, point)?)?;
    if item.command.is_some() {
        return None;
    }
    match item.kind {
        Kind::Button {
            id: id::INTERESTS, ..
        } => Some(Panel::Interests),
        Kind::Button { id: id::GIFTS, .. } => Some(Panel::Gifts),
        _ => None,
    }
}

/// What a press that asks for `command` does, by the state at the press,
/// as the original's press handler checks it then (`0x51bbe6..0x51be75`,
/// `0x51c61f..0x51c9e5`). Each gadget button needs a selected Sim
/// (`selected`: one of the household's, the panel's `+0xe8`), the House
/// button needs a residential lot too (`residential`, `0x40745f`), and
/// Interests/Gifts are disabled while a pet is selected (`0x51c982`,
/// `0x51c9da`). A refused House press sounds (`0x51be1a`); another refused
/// press does nothing (`0x51be6e`). Other commands pass through.
pub fn check_press(command: Command, selected: Selected, residential: bool) -> Command {
    match command {
        Command::OpenPanel(Panel::House)
            if matches!(selected, Selected::None | Selected::NonHousehold) || !residential =>
        {
            Command::Refused { error: true }
        }
        Command::OpenPanel(_) if matches!(selected, Selected::None | Selected::NonHousehold) => {
            Command::Refused { error: false }
        }
        Command::OpenPanel(Panel::Interests | Panel::Gifts) if selected == Selected::Pet => {
            Command::Refused { error: false }
        }
        other => other,
    }
}

/// Whether the panel covers `point`, so a press there is the panel's and
/// not the lot's.
pub fn covers(items: &[Item], point: [i32; 2]) -> bool {
    // Floating name/tooltip boxes let presses pass through to the lot.
    // `4918` (`kTallSubpanel`, 150 px) extends 50 px transparently above the
    // 100 px Buy strip; only its bottom `STRIP` pixels belong to the panel.
    items.iter().any(|i| match i.kind {
        Kind::Name { .. } | Kind::OptionsTooltip(_) => false,
        Kind::Stretched(4918) => {
            let strip = Rect::new(
                i.rect.x,
                i.rect.y + i.rect.height - STRIP,
                i.rect.width,
                STRIP,
            );
            strip.contains(point[0], point[1])
        }
        _ => i.rect.contains(point[0], point[1]),
    })
}

/// Whether `point` lies inside the live-mode subpanel (`[gadget.x + 107..,
/// gadget.y..]`, the subpanel's own rect, `0x51f22b..0x51f3e5`). Moving the pointer outside this area
/// (including onto an open help balloon, which attaches outside `LivePanel` at
/// `0x51a0a2..0x51a0b5`, onto the lot, cluster, portraits, gadget buttons, or
/// out of the window) dismisses an open help balloon with `UI_whoosh`
/// (`0x51f1f2..0x51f3e5`, `0x51d2ee..0x51d359`).
pub fn in_subpanel(items: &[Item], [x, y]: [i32; 2]) -> bool {
    let find = |kind: Kind| items.iter().find(|i| i.kind == kind).map(|i| i.rect);
    let (Some(gadget), Some(strip)) = (
        find(Kind::Image(id::GAUGE)),
        find(Kind::Stretched(id::STRIP)),
    ) else {
        return false;
    };
    // The subpanel as `live_strip` lays it out: from the gadget to the
    // strip's right end, as high as the gadget.
    let left = gadget.x.saturating_add(SUBPANEL_FROM_GADGET);
    let right = strip.x.saturating_add(strip.width);
    Rect::new(left, gadget.y, right.saturating_sub(left), gadget.height).contains(x, y)
}

/// Whether `point` lies inside the expanded Options panel (`cWinOptions`,
/// `0x517957..0x5179cd`). Moving outside the Options panel (including onto the
/// help balloon itself, which attaches outside `cWinOptions`) dismisses an open
/// Options help balloon with `UI_whoosh`.
pub fn in_options_panel(items: &[Item], [x, y]: [i32; 2]) -> bool {
    items
        .iter()
        .any(|i| matches!(i.kind, Kind::Stretched(id::STRIP | 4918)) && i.rect.contains(x, y))
}

/// Whether `point` lies inside an open help balloon (`Kind::Balloon` or
/// `Kind::OptionsBalloon`). A direct left press on the balloon
/// (`0x66bd68 -> 0x5dd85f`) is absorbed without dismissing it.
pub fn on_balloon(items: &[Item], [x, y]: [i32; 2]) -> bool {
    items.iter().any(|i| {
        matches!(i.kind, Kind::Balloon(_) | Kind::OptionsBalloon(_)) && i.rect.contains(x, y)
    })
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
            show_money: true,
            paused: false,
            idle_speed: false,
            residential: true,
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
                    has_career: false,
                    needs: [0; 6],
                    child: false,
                    school_age: false,
                    female: false,
                    pet: false,
                    pet_kind: None,
                    performance: 0,
                    school_grade: String::new(),
                    fame_score: 0,
                    fame_level: 0,
                    fame_title: String::new(),
                    fame_needs: [0; 6],
                    fame_friends_text: String::new(),
                    work: Work::default(),
                    interests: [0; 19],
                })
                .collect(),
            selected: Some(21),
            selected_guid: None,
            family_friends: 0,
            cluster_friends: 0,
            relations: Vec::new(),
            superstar: false,
            house: crate::house::House::default(),
            inventory: Vec::new(),
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
        // At a widescreen size the 7-button mood/tab gadget stays anchored beside
        // the 4-column portrait block at `x = 413` while the active subpanel
        // expands cleanly into the remaining width.
        let wide = layout([1280, 720], &snapshot(), &controls);
        assert_eq!(
            press(&wide, [413 + 50, 720 - 100 + 15]),
            Some(Command::OpenPanel(Panel::Interests))
        );
        // In a narrow window the cluster covers the gadget, and a press
        // there does not reach the gadget's buttons under it.
        let narrow = layout([400, 600], &snapshot(), &controls);
        assert_eq!(press(&narrow, [60, 530]), None);
        assert_eq!(hovered(&narrow, Some([60, 530])), None);
    }

    #[test]
    fn live_strip_anchors_gadget_at_413_across_all_widescreen_widths() {
        let controls = Controls {
            expanded: true,
            panel: Some(Panel::Needs),
            ..Controls::default()
        };
        for [w, h] in [
            [800, 600],
            [1024, 768],
            [1280, 720],
            [1600, 900],
            [1920, 1080],
        ] {
            let items = layout([w, h], &snapshot(), &controls);
            let gauge = items
                .iter()
                .find(|i| matches!(i.kind, Kind::Image(id::GAUGE)))
                .expect("gauge present");
            assert_eq!(
                gauge.rect,
                Rect::new(413, h - STRIP, 108, STRIP),
                "gadget must stay anchored at x=413 at {w}x{h}"
            );
            assert_eq!(
                press(&items, [413 + 50, h - STRIP + 15]),
                Some(Command::OpenPanel(Panel::Interests)),
                "Interests tab hit-target at {w}x{h}"
            );
            // Subpanel fills everything to the right of the gadget (`x = 521..w`).
            let strip = items
                .iter()
                .find(|i| matches!(i.kind, Kind::Stretched(id::STRIP)))
                .expect("strip present");
            assert_eq!(
                strip.rect,
                Rect::new(CLUSTER[0], h - STRIP, w - CLUSTER[0], STRIP)
            );
        }
    }

    #[test]
    fn back_patch_fills_camera_wedge_only_when_expanded() {
        let snap = snapshot();
        let collapsed = Controls::default();
        let closed_items = layout([800, 600], &snap, &collapsed);
        assert!(
            !closed_items
                .iter()
                .any(|i| matches!(i.kind, Kind::Image(id::BACK_PATCH))),
            "BackPatch must not be drawn while strip is collapsed"
        );

        let art = Art::clean_room_procedural();
        let faces = Faces::default();
        let bg = [115u8, 113, 115];
        for mode in [Mode::Live, Mode::Buy, Mode::Build, Mode::Options] {
            let controls = Controls {
                expanded: true,
                mode,
                ..Controls::default()
            };
            let items = layout([1280, 720], &snap, &controls);
            let patch_idx = items
                .iter()
                .position(|i| matches!(i.kind, Kind::Image(id::BACK_PATCH)))
                .expect("BackPatch emitted when expanded");
            let cluster_idx = items
                .iter()
                .position(|i| matches!(i.kind, Kind::Image(id::CLUSTER)))
                .expect("Cluster emitted");
            assert!(
                patch_idx < cluster_idx,
                "BackPatch must draw behind UniversalBack cluster ({mode:?})"
            );
            assert_eq!(
                items[patch_idx].rect,
                Rect::new(168, 720 - STRIP, 52, STRIP),
                "BackPatch native rect (168, h-100, 52, 100)"
            );
            let mut frame = Frame::new(1280, 720, bg);
            draw(&mut frame, &art, &faces, &items, &controls);
            for y in (720 - STRIP)..720 {
                for x in 168..220 {
                    assert_ne!(
                        frame.pixel(x, y),
                        Some(bg),
                        "gray background leaked at ({x}, {y}) in {mode:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn relationships_scales_per_page_with_subpanel_width() {
        let mut snap = snapshot();
        snap.relations = (0..25)
            .map(|i| Relation {
                id: (100 + i) as i16,
                guid: 100 + i,
                daily: 50,
                lifetime: 50,
                family: false,
                friends: true,
                star: false,
                pink_heart: false,
                red_heart: false,
            })
            .collect();
        let controls = Controls {
            expanded: true,
            panel: Some(Panel::Relationships),
            relations: Relations::Everyone,
            ..Controls::default()
        };
        for ([w, h], expected_per_page) in [
            ([800, 600], 5usize),
            ([1024, 768], 10usize),
            ([1280, 720], 15usize),
            ([1600, 900], 22usize),
        ] {
            let items = layout([w, h], &snap, &controls);
            let rels: Vec<Rect> = items
                .iter()
                .filter_map(|i| matches!(i.kind, Kind::Relation { .. }).then_some(i.rect))
                .collect();
            assert_eq!(
                rels.len(),
                expected_per_page,
                "relationships per page at {w}x{h}"
            );
            let page_on = items
                .iter()
                .find(|i| {
                    matches!(
                        i.kind,
                        Kind::Button {
                            id: id::PAGE_ON,
                            ..
                        }
                    )
                })
                .expect("next-page arrow shown for 25 relations");
            for r in &rels {
                assert!(
                    r.x + r.width <= page_on.rect.x,
                    "relation rect {:?} must not overlap PAGE_ON {:?} at {w}x{h}",
                    r,
                    page_on.rect
                );
            }
        }
    }

    #[test]
    fn panel_back_stretch_preserves_right_bezel_above_1024() {
        let mut art = Art::default();
        let w = 804u32;
        let h = STRIP as u32;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let rgb = if x >= w - 8 { [0, 0, 49] } else { [0, 0, 82] };
                let off = ((y * w + x) * 4) as usize;
                rgba[off..off + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        art.insert(
            id::STRIP,
            Image {
                width: w,
                height: h,
                rgba,
            },
        );
        let faces = Faces::default();
        let controls = Controls {
            expanded: true,
            ..Controls::default()
        };
        let mut frame = Frame::new(1280, 720, [0, 0, 0]);
        let item = Item {
            rect: Rect::new(CLUSTER[0], 620, 1280 - CLUSTER[0], STRIP),
            kind: Kind::Stretched(id::STRIP),
            command: None,
        };
        draw(&mut frame, &art, &faces, &[item], &controls);
        // No dark vertical seam at x = 1024 (old `220 + 804`), and bezel sits at right edge.
        assert_eq!(frame.pixel(1020, 670), Some([0, 0, 82]));
        assert_eq!(frame.pixel(1024, 670), Some([0, 0, 82]));
        assert_eq!(frame.pixel(1271, 670), Some([0, 0, 82]));
        assert_eq!(frame.pixel(1272, 670), Some([0, 0, 49]));
        assert_eq!(frame.pixel(1279, 670), Some([0, 0, 49]));
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
                has_career: false,
                needs: [0; 6],
                child: false,
                school_age: false,
                female: false,
                pet: false,
                pet_kind: None,
                performance: 0,
                school_grade: String::new(),
                fame_score: 0,
                fame_level: 0,
                fame_title: String::new(),
                fame_needs: [0; 6],
                fame_friends_text: String::new(),
                work: Work::default(),
                interests: [0; 19],
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
            panel: Some(Panel::Personality),
            ..Controls::default()
        };
        let items = layout([800, 600], &s, &controls);
        let panels: Vec<&Item> = items
            .iter()
            .filter(|i| matches!(i.kind, Kind::Personality { .. }))
            .collect();
        // The subpanel, as the needs' is.
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].rect, Rect::new(520, 500, 280, 100));
        assert_eq!(
            panels[0].kind,
            Kind::Personality {
                traits: [250, -5, 1500, 0, 999],
                zodiac: Some(9),
                pet: false,
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
            false,
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
        draw_personality(&mut frame, &art, far, &[1000; 5], false);
        let near = Rect::new(i32::MIN, i32::MIN, 220, 100);
        draw_personality(&mut frame, &art, near, &[1000; 5], false);
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
    fn a_text_box_centres_its_line_by_the_rows_an_a_inks() {
        // `A` inks rows 0 and 1 of the hanging font's 4-high line: a box
        // 13 high from 10 puts its top at 10 + (13 - 2) / 2.
        let font = crate::font::tests::hanging_font();
        assert_eq!(text_box_y(&font, 10, 13), 15);
        assert_eq!(text_box_y(&font, 10, 12), 15);
        // The odd pixel is cut toward zero, not floored: (1 - 2) / 2 is 0.
        assert_eq!(text_box_y(&font, 10, 1), 10);
        // Without an `A` glyph its slot is empty, zero high at the line's
        // top, as the original's table keeps it: 10 + 13 / 2.
        let without_a = crate::font::tests::without_a();
        assert_eq!(text_box_y(&without_a, 10, 13), 16);
    }

    #[test]
    fn the_relationship_panel_lists_a_page_of_people() {
        let mut s = snapshot();
        s.relations = (1..=7)
            .map(|n| relation(n, 10 * i32::from(n), n <= 2, n % 2 == 1))
            .collect();
        let mut controls = Controls {
            expanded: true,
            panel: Some(Panel::Relationships),
            ..Controls::default()
        };
        let panel = Rect::new(520, 500, 280, 100);
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
        // The title, in the 7-point font at the measured place.
        assert!(items.iter().any(|i| matches!(
            &i.kind,
            Kind::Text { text, size: RELATION_FONT, .. } if text == "Relationships"
        ) && (i.rect.x, i.rect.y) == (520 + 7, 500 + 2)));
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
                    // Under the title, a font-slot-6 line (13) plus one (`0x51dd1d`).
                    panel.offset(3, 14).with_size(20, 20),
                    false,
                    Some(Command::ShowRelations(Relations::Family))
                ),
                (
                    id::FRIENDS,
                    panel.offset(2, 35).with_size(20, 20),
                    false,
                    Some(Command::ShowRelations(Relations::Friends))
                ),
                (
                    id::STARS,
                    panel.offset(2, 56).with_size(20, 20),
                    false,
                    None
                ),
                (
                    id::EVERYONE,
                    panel.offset(2, 77).with_size(20, 20),
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
        // The arrows centred below the 13-high line (`0x51ddec`), the
        // right one 2 in from the panel's edge.
        let on = (
            id::PAGE_ON,
            panel.offset(269, 32).with_size(9, 49),
            Some(Command::RelationsPage(1)),
        );
        let back = (
            id::PAGE_BACK,
            panel.offset(24, 32).with_size(9, 49),
            Some(Command::RelationsPage(0)),
        );
        assert_eq!(arrows(&items), std::slice::from_ref(&on));
        assert_eq!(press(&items, [520 + 270, 540]), on.2.clone());
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
        controls.pointer = Some([520 + 35 + 45 + 10, 540]);
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
        let first = lit_relation(&items, Some([520 + 35 + 10, 540]));
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

    /// The pixels in `frame` left of `right` that are not black.
    fn inked(frame: &Frame, right: i32) -> Vec<[i32; 2]> {
        let height = i32::try_from(frame.height()).unwrap_or(0);
        (0..right)
            .flat_map(|x| (0..height).map(move |y| [x, y]))
            .filter(|&[x, y]| frame.pixel(x, y).is_some_and(|p| p != [0; 3]))
            .collect()
    }

    #[test]
    fn a_personality_label_is_centred_on_its_bar_by_the_row_under_an_a() {
        // The hanging font's line is 4 high but its `A` ends at row 2:
        // on an 8-high bar the label goes (8 - 2) / 2 = 3 down, so the `g`
        // of "Outgoing" (rows 2 and 3) first inks row 8 + 18 + 3 + 2.
        let mut art = Art::default();
        art.insert_font(TRAIT_FONT, crate::font::tests::hanging_font());
        let bar = |rgb: [u8; 3]| Image {
            width: 40,
            height: 8,
            rgba: [rgb[0], rgb[1], rgb[2], 255].repeat(40 * 8),
        };
        art.insert(id::TRAIT_EMPTY, bar([1, 1, 1]));
        art.insert(id::TRAIT_FULL, bar([2, 2, 2]));
        let mut frame = Frame::new(220, 110, [0; 3]);
        draw_personality(&mut frame, &art, Rect::new(0, 0, 220, 100), &[0; 5], false);
        let top = (0..110)
            .find(|&y| (0..220).any(|x| frame.pixel(x, y) == Some(LABEL)))
            .expect("the `g` is drawn");
        assert_eq!(top, 8 + 18 + 3 + 2);
    }

    #[test]
    fn right_aligned_and_centred_text_moves_by_the_fonts_measure() {
        use crate::font::tests::{digit_font, hanging_font_plus};
        let solid = |w: u32, h: u32| Image {
            width: w,
            height: h,
            rgba: [1, 1, 1, 255].repeat((w * h) as usize),
        };
        // Two fonts with the same advances, one with a `width_extra` of
        // 1: what is right aligned by the measure (`+0x68`) moves a pixel
        // left; what is drawn from the left does not.
        let shifted = |narrow: &[[i32; 2]], by: i32| -> Vec<[i32; 2]> {
            narrow.iter().map(|&[x, y]| [x - by, y]).collect()
        };
        let house = |font| {
            let mut art = Art::default();
            art.insert_font(HOUSE_FONT, font);
            art.insert(id::RATING_EMPTY, solid(40, 14));
            art.insert(id::TRAIT_FULL, solid(40, 16));
            let mut frame = Frame::new(220, 110, [0; 3]);
            let mut s = snapshot();
            s.house.bathrooms = 1;
            draw_house(&mut frame, &art, Rect::new(0, 0, 220, 100), &s.house);
            inked(&frame, 156)
        };
        // The facts ("Bathrooms", "Bedrooms": a `B`) and a rating label
        // ("Furnishings": a `g`).
        let narrow = house(hanging_font_plus(0));
        assert!(narrow.iter().any(|&[x, _]| x < 80), "a fact is drawn");
        assert!(narrow.iter().any(|&[x, _]| x > 80), "a label is drawn");
        assert_eq!(house(hanging_font_plus(3)), shifted(&narrow, 1));
        let skills = |font| {
            let mut art = Art::default();
            art.insert_font(SKILL_FONT, font);
            art.insert(id::SKILL_EMPTY, solid(40, 11));
            art.insert(id::SKILL_FULL, solid(40, 11));
            let mut frame = Frame::new(220, 110, [0; 3]);
            let panel = Rect::new(0, 0, 220, 100);
            draw_skills(&mut frame, &art, panel, &[0; 6], &[0; 6]);
            inked(&frame, 156)
        };
        let narrow = skills(hanging_font_plus(0));
        assert!(!narrow.is_empty(), "\"Cooking\" and \"Logic\" draw a `g`");
        assert_eq!(skills(hanging_font_plus(3)), shifted(&narrow, 1));
        // A relationship's number, "7", centred in the 45-wide box: at
        // 22 - 1 / 2 = 22 measured 1 wide, at 22 - 2 / 2 = 21 measured 2.
        let number = |font| {
            let mut art = Art::default();
            art.insert_font(RELATION_FONT, font);
            let relation = Relation {
                guid: 7,
                id: 1,
                daily: 7,
                lifetime: 0,
                friends: false,
                pink_heart: false,
                red_heart: false,
                star: false,
                family: false,
            };
            let mut frame = Frame::new(60, 120, [0; 3]);
            let r = Rect::new(0, 10, 45, 90);
            draw_relation(&mut frame, &art, &Faces::default(), r, &relation, false);
            (0..45)
                .find(|&x| frame.pixel(x, 8) == Some(LABEL))
                .expect("the number is drawn")
        };
        assert_eq!(number(digit_font(0)), 22);
        assert_eq!(number(digit_font(6)), 21);
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
        draw_zodiac(&mut frame, &art, Rect::new(0, 0, 280, 100), 1);
        let top = 100 - line - 5;
        assert_eq!(frame.pixel(6, top), Some(LABEL));
        assert_eq!(frame.pixel(5, top), Some([0; 3]));
        assert_eq!(frame.pixel(6, top - 1), Some([0; 3]));
        // Without the font nothing is drawn, and far places do not
        // overflow.
        let mut frame = Frame::new(4, 4, [0; 3]);
        draw_zodiac(&mut frame, &Art::default(), Rect::new(0, 0, 280, 100), 1);
        assert!(frame.rgb().iter().all(|p| *p == [0; 3]));
        for at in [i32::MAX - 2, i32::MIN + 2] {
            draw_zodiac(&mut frame, &art, Rect::new(at, at, 280, 100), 12);
        }
    }

    #[test]
    fn pressing_the_sign_opens_its_balloon_over_the_panel() {
        let mut s = snapshot();
        s.residents[0].zodiac = Some(1);
        let mut controls = Controls {
            expanded: true,
            panel: Some(Panel::Personality),
            ..Controls::default()
        };
        let mut art = Art::default();
        art.insert_font(ZODIAC_FONT, crate::font::tests::font());
        let line = art.font(ZODIAC_FONT).unwrap().line_height;
        let texts = Texts::signs(&[], &[], &[]);
        // Before fit the sign cannot be pressed; after, its name can.
        let mut items = layout([800, 600], &s, &controls);
        let bottom_left = [525, 600 - 5 - line];
        assert_eq!(press(&items, bottom_left), None);
        fit(&mut items, &art, &texts);
        let sign = items
            .iter()
            .find(|i| matches!(i.kind, Kind::Sign { .. }))
            .unwrap();
        // Only the test font's `A` has width.
        let width = art.font(ZODIAC_FONT).unwrap().width("Aries");
        assert_eq!(sign.rect, Rect::new(525, 600 - 5 - line, width, line));
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
        elsewhere.panel = Some(Panel::Needs);
        let items = layout([800, 600], &s, &elsewhere);
        assert!(!items.iter().any(|i| matches!(i.kind, Kind::Help { .. })));
    }

    /// The gadget buttons' commands, by button.
    fn gadget_commands(items: &[Item]) -> Vec<(i32, Command)> {
        items
            .iter()
            .filter_map(|i| match (&i.kind, &i.command) {
                (Kind::Button { id, .. }, Some(c)) if (id::NEEDS..=id::GIFTS).contains(id) => {
                    Some((*id, c.clone()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_gadget_buttons_need_a_selected_sim_and_house_a_residential_lot() {
        let controls = Controls {
            expanded: true,
            ..Controls::default()
        };
        let opens = |id: i32| match id {
            id::NEEDS => Panel::Needs,
            id::INTERESTS => Panel::Interests,
            id::RELATIONSHIPS => Panel::Relationships,
            id::PERSONALITY => Panel::Personality,
            id::JOB => Panel::Job,
            id::GIFTS => Panel::Gifts,
            id::HOUSE => Panel::House,
            _ => unreachable!(),
        };
        // Each button asks for its panel, selected or not: the check is
        // the press's. The buttons take the press either way, so they are
        // not drawn disabled.
        let mut s = snapshot();
        for selected in [Some(21), None] {
            s.selected = selected;
            let commands = gadget_commands(&layout([800, 600], &s, &controls));
            assert_eq!(commands.len(), 7);
            for (id, command) in &commands {
                assert_eq!(*command, Command::OpenPanel(opens(*id)));
            }
        }
        // The House button, at (76, 66) in the gadget.
        let house = press(&layout([800, 600], &s, &controls), [413 + 80, 500 + 70]);
        assert_eq!(house, Some(Command::OpenPanel(Panel::House)));
        let check = |panel: Panel, selected: Selected, residential: bool| {
            check_press(Command::OpenPanel(panel), selected, residential)
        };
        for panel in [
            Panel::Needs,
            Panel::Interests,
            Panel::Relationships,
            Panel::Personality,
            Panel::Job,
            Panel::Gifts,
        ] {
            // Human selected: it opens, on any lot.
            assert_eq!(
                check(panel, Selected::Human, true),
                Command::OpenPanel(panel)
            );
            assert_eq!(
                check(panel, Selected::Human, false),
                Command::OpenPanel(panel)
            );
            // Pet selected: Interests and Gifts are refused silently; the rest open.
            let pet_expected = if matches!(panel, Panel::Interests | Panel::Gifts) {
                Command::Refused { error: false }
            } else {
                Command::OpenPanel(panel)
            };
            assert_eq!(check(panel, Selected::Pet, true), pet_expected);
            assert_eq!(check(panel, Selected::Pet, false), pet_expected);
            // Nobody or non-household Sim selected (+0xe8 == -1): refused silently (0x51be6e).
            for missing in [Selected::None, Selected::NonHousehold] {
                for residential in [true, false] {
                    assert_eq!(
                        check(panel, missing, residential),
                        Command::Refused { error: false }
                    );
                }
            }
        }
        // House needs a household Sim selected and a residential lot, and its refusal sounds (0x51be1a).
        assert_eq!(
            check(Panel::House, Selected::Human, true),
            Command::OpenPanel(Panel::House)
        );
        assert_eq!(
            check(Panel::House, Selected::Pet, true),
            Command::OpenPanel(Panel::House)
        );
        for (selected, residential) in [
            (Selected::Human, false),
            (Selected::Pet, false),
            (Selected::None, true),
            (Selected::None, false),
            (Selected::NonHousehold, true),
            (Selected::NonHousehold, false),
        ] {
            assert_eq!(
                check(Panel::House, selected, residential),
                Command::Refused { error: true }
            );
        }
        // Other commands pass through.
        for command in [Command::TogglePanel, Command::Select { sim: 21 }] {
            assert_eq!(check_press(command.clone(), Selected::None, false), command);
        }
        // A selected person who is not one of the household's counts as
        // nobody.
        s.selected = Some(21);
        assert_eq!(s.selected_resident().map(|r| r.object), Some(21));
        s.selected = Some(999);
        assert!(s.selected_resident().is_none());
        // A refusal changes nothing that shows.
        let mut refused = Controls {
            panel: Some(Panel::Personality),
            help: Some(Help::Sign(3)),
            ..controls
        };
        refused.apply(&Command::Refused { error: true });
        assert_eq!(refused.panel, Some(Panel::Personality));
        assert_eq!(refused.help, Some(Help::Sign(3)));
    }

    #[test]
    fn the_panel_follows_the_selection() {
        let mut controls = Controls {
            expanded: true,
            panel: Some(Panel::Personality),
            help: Some(Help::Sign(3)),
            ..Controls::default()
        };
        // Someone selected: the panel stays, and so does its balloon.
        assert!(!controls.follow(Selected::Human));
        assert_eq!(controls.panel, Some(Panel::Personality));
        assert_eq!(controls.help, Some(Help::Sign(3)));
        // Nobody: no panel (0x51d06e), and the balloon closes with it.
        assert!(controls.follow(Selected::None));
        assert_eq!(controls.panel, None);
        assert_eq!(controls.help, None);
        assert!(!controls.follow(Selected::None));
        // With no panel open nothing is laid out beside the gadget.
        let mut s = snapshot();
        s.house = House {
            square_feet: 1836,
            ..House::default()
        };
        let items = layout([800, 600], &s, &controls);
        assert!(!items.iter().any(|i| i.rect.x >= 520 && i.rect.y >= 500));
        // Someone selected again: the needs open (0x51d082), whichever
        // panel was open before.
        assert!(!controls.follow(Selected::Human));
        assert_eq!(controls.panel, Some(Panel::Needs));
        // The house panel closes too.
        controls.panel = Some(Panel::House);
        controls.follow(Selected::None);
        assert_eq!(controls.panel, None);
        // Pet selected (0x51ce13..0x51ce36): Interests and Gifts switch to
        // Needs (closing any open balloon with a whoosh); other panels stay.
        for disallowed in [Panel::Interests, Panel::Gifts] {
            controls.panel = Some(disallowed);
            controls.help = Some(Help::Sign(3));
            assert!(controls.follow(Selected::Pet));
            assert_eq!(controls.panel, Some(Panel::Needs));
            assert_eq!(controls.help, None);
        }
        controls.panel = Some(Panel::Personality);
        assert!(!controls.follow(Selected::Pet));
        assert_eq!(controls.panel, Some(Panel::Personality));
        // Selecting a pet disables Interests and Gifts (0x51c61f..0x51c9e5),
        // and clearing selection (0x51c5e1 -> 0x51c775) preserves that disabled
        // state until a human household Sim re-enables them.
        assert!(!controls.interests_gifts_enabled);
        assert!(!controls.follow(Selected::None));
        assert_eq!(controls.panel, None);
        assert!(!controls.interests_gifts_enabled);
        let mut nobody_snap = snapshot();
        nobody_snap.selected = None;
        let cleared_items = layout([800, 600], &nobody_snap, &controls);
        for id in [id::INTERESTS, id::GIFTS] {
            let btn = cleared_items
                .iter()
                .find(|i| matches!(i.kind, Kind::Button { id: bid, .. } if bid == id))
                .expect("gadget button");
            assert_eq!(btn.command, None);
        }
        // Selecting a human re-enables Interests and Gifts (0x51c6f1..0x51c8b3).
        assert!(!controls.follow(Selected::Human));
        assert!(controls.interests_gifts_enabled);
        assert_eq!(controls.panel, Some(Panel::Needs));
        // Non-household selection (0x4cf9a4 != -1, +0xe8 == -1): keeps the open
        // panel (0x51d06e) while disabling Interests and Gifts (0x51c609..0x51c619).
        controls.panel = Some(Panel::Personality);
        assert!(!controls.follow(Selected::NonHousehold));
        assert_eq!(controls.panel, Some(Panel::Personality));
        assert!(!controls.interests_gifts_enabled);
        let mut visitor_snap = snapshot();
        visitor_snap.selected = Some(999);
        assert_eq!(Selected::of(&visitor_snap), Selected::NonHousehold);
        let visitor_items = layout([800, 600], &visitor_snap, &controls);
        for (id, want_disabled) in [
            (id::INTERESTS, true),
            (id::GIFTS, true),
            (id::NEEDS, false),
            (id::PERSONALITY, false),
        ] {
            let btn = visitor_items
                .iter()
                .find(|i| matches!(i.kind, Kind::Button { id: bid, .. } if bid == id))
                .expect("gadget button");
            assert_eq!(btn.command.is_none(), want_disabled);
        }
        // Selecting a pet directly in the snapshot also disables Interests/Gifts.
        controls.follow(Selected::Human);
        let mut pet_snap = snapshot();
        pet_snap.residents[0].pet = true;
        let pet_items = layout([800, 600], &pet_snap, &controls);
        for (id, want_disabled) in [
            (id::INTERESTS, true),
            (id::GIFTS, true),
            (id::NEEDS, false),
            (id::PERSONALITY, false),
        ] {
            let btn = pet_items
                .iter()
                .find(|i| matches!(i.kind, Kind::Button { id: bid, .. } if bid == id))
                .expect("gadget button");
            assert_eq!(btn.command.is_none(), want_disabled);
        }
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
            panel: Some(Panel::Job),
            ..Controls::default()
        };
        let items = layout([800, 600], &s, &controls);
        let work: Vec<&Item> = items
            .iter()
            .filter(|i| matches!(i.kind, Kind::Work { .. }))
            .collect();
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].rect, Rect::new(520, 500, 280, 100));
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
                false,
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
            ..House::default()
        };
        let controls = Controls {
            expanded: true,
            panel: Some(Panel::House),
            ..Controls::default()
        };
        // With nobody selected too.
        let items = layout([800, 600], &s, &controls);
        let panel = items
            .iter()
            .find(|i| matches!(i.kind, Kind::House { .. }))
            .unwrap();
        assert_eq!(panel.rect, Rect::new(520, 500, 280, 100));
        let title = items
            .iter()
            .find(|i| matches!(&i.kind, Kind::Text { text, .. } if text == "House"))
            .unwrap();
        assert_eq!((title.rect.x, title.rect.y), (520 + 5, 500));
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
            panel: Some(Panel::Job),
            ..Controls::default()
        };
        let items = layout([800, 600], &s, &controls);
        let panels: Vec<&Item> = items
            .iter()
            .filter(|i| matches!(i.kind, Kind::Skills { .. }))
            .collect();
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].rect, Rect::new(520, 500, 280, 100));
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
        assert_eq!((title.rect.x, title.rect.y), (520 + 35, 500 + 3));
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
        // (93, 5): the `g` at x 154, rows 2 and 3 of the 4-high line
        // centred in the box. Logic's, with a need, is underlined (on the
        // row under an `A`, 2 wide for the `g`'s advance); Cooking's is
        // not.
        let g = |row: i32| [154, 5 + 15 * row + 6 + 2];
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

    #[test]
    fn the_cluster_clock_friends_help_and_paused_speed_follow_the_original() {
        let mut s = snapshot();
        s.hour = 8;
        s.minute = 16;
        s.paused = true;
        s.family_friends = 3;
        s.cluster_friends = 12;
        let controls = Controls {
            speed: Some(Speed::Fast),
            ..Controls::default()
        };
        let items = layout([800, 600], &s, &controls);
        let cy = 600 - CLUSTER[1];

        // Clock box at (95, 118, 56, 14) with signed-truncation centering.
        let clock = items
            .iter()
            .find(|i| matches!(&i.kind, Kind::Text { text, .. } if text == "AM  8:16"))
            .expect("the clock item");
        assert_eq!(clock.rect, Rect::new(95, cy + 118, 56, 14));
        assert!(matches!(
            clock.kind,
            Kind::Text {
                size: 8,
                align: Align::CenterTrunc,
                ..
            }
        ));

        // Help button at (5, 164, 10, 15) is an enabled 4-cell button that
        // hovers (cell 2) and emits Command::OpenHelp (0x538f41, 0x53aaad).
        let help = items
            .iter()
            .find(|i| matches!(i.kind, Kind::Button { id: id::HELP, .. }))
            .expect("the help button");
        assert_eq!(help.rect, Rect::new(5, cy + 164, 10, 15));
        assert_eq!(
            help.kind,
            Kind::Button {
                id: id::HELP,
                cells: 4,
                selected: false,
            }
        );
        assert_eq!(help.command, Some(Command::OpenHelp));
        assert_eq!(press(&items, [8, cy + 170]), Some(Command::OpenHelp));
        assert!(hovered(&items, Some([8, cy + 170])).is_some());

        // Speed button stays selected while paused (0x53b0d8..0x53b16b).
        let fast = items
            .iter()
            .find(|i| matches!(i.kind, Kind::Button { id: id::FAST, .. }))
            .expect("the fast speed button");
        assert!(matches!(fast.kind, Kind::Button { selected: true, .. }));
        let pause = items
            .iter()
            .find(|i| matches!(i.kind, Kind::Pause { .. }))
            .expect("the pause button");
        assert_eq!(
            pause.kind,
            Kind::Pause {
                paused: true,
                lit: false
            }
        );
        // When pause_lit is true in Live mode (or paused in non-Live mode),
        // Pause draws lit row 1 (0x53b500..0x53b528, 0x53b736..0x53b74d).
        let lit_controls = Controls {
            pause_lit: true,
            ..controls
        };
        let lit_pause = layout([800, 600], &s, &lit_controls)
            .into_iter()
            .find(|i| matches!(i.kind, Kind::Pause { .. }))
            .expect("lit pause");
        assert_eq!(
            lit_pause.kind,
            Kind::Pause {
                paused: true,
                lit: true
            }
        );
        // Outside Live mode: unpaused enters state 1 (steady selected & lit red,
        // 0x53b55c -> 0x53b690); pausing enters state 2 and blinks via pause_lit
        // (0x53b58a -> 0x53b5d5 -> 0x53b5e8).
        let mut unpaused_snap = s.clone();
        unpaused_snap.paused = false;
        let buy_unpaused = layout(
            [800, 600],
            &unpaused_snap,
            &Controls {
                mode: Mode::Buy,
                pause_lit: false,
                ..controls
            },
        )
        .into_iter()
        .find(|i| matches!(i.kind, Kind::Pause { .. }))
        .expect("buy unpaused pause");
        assert_eq!(
            buy_unpaused.kind,
            Kind::Pause {
                paused: true,
                lit: true
            }
        );
        for pause_lit in [false, true] {
            let buy_paused = layout(
                [800, 600],
                &s,
                &Controls {
                    mode: Mode::Buy,
                    pause_lit,
                    ..controls
                },
            )
            .into_iter()
            .find(|i| matches!(i.kind, Kind::Pause { .. }))
            .expect("buy paused pause");
            assert_eq!(
                buy_paused.kind,
                Kind::Pause {
                    paused: true,
                    lit: pause_lit
                }
            );
        }
        // Cluster uses cluster_friends (12, 0x4cfcdb), while Job panel keeps
        // family_friends (3, 0x4ea8ed, 0x531095).
        let job_controls = Controls {
            expanded: true,
            panel: Some(Panel::Job),
            ..controls
        };
        let job_items = layout([800, 600], &s, &job_controls);
        assert!(job_items.iter().any(|i| matches!(
            i.kind,
            Kind::Work {
                family_friends: 3,
                ..
            }
        )));

        // Buy mode uses the standard 100 px `id::STRIP` (`PanelBack.bmp`, 4910;
        // `Sims.exe` `0x509e6f..0x509e9d`) with zero 50 px overhang into the lot.
        let buy_expanded = layout(
            [800, 600],
            &s,
            &Controls {
                mode: Mode::Buy,
                expanded: true,
                ..controls
            },
        );
        let buy_strip = buy_expanded
            .iter()
            .find(|i| matches!(i.kind, Kind::Stretched(_)))
            .expect("expanded buy strip");
        assert_eq!(buy_strip.kind, Kind::Stretched(id::STRIP));
        assert_eq!(
            buy_strip.rect,
            Rect::new(CLUSTER[0], 600 - STRIP, 800 - CLUSTER[0], STRIP)
        );
        assert!(!covers(&buy_expanded, [300, 470]));
        assert!(covers(&buy_expanded, [300, 520]));
        assert!(job_items.iter().any(|i| i.kind == Kind::Friends(12)));

        // Roof / Walls Up / Cutaway / Walls Down form a strict 4-way radio group
        // (`Sims.exe` `0x53ad4f..0x53adb5`): exactly one button is selected for
        // every `(roof, walls)` combination.
        for roof in [false, true] {
            for walls in [Walls::Down, Walls::Cutaway, Walls::Up] {
                let c = Controls {
                    camera: CameraState {
                        roof,
                        walls,
                        ..CameraState::default()
                    },
                    ..controls
                };
                let radio_items = layout([800, 600], &s, &c);
                let selected_ids: Vec<i32> = radio_items
                    .iter()
                    .filter_map(|i| match i.kind {
                        Kind::Button {
                            id, selected: true, ..
                        } if matches!(
                            id,
                            id::ROOF | id::WALLS_UP | id::CUTAWAY | id::WALLS_DOWN
                        ) =>
                        {
                            Some(id)
                        }
                        _ => None,
                    })
                    .collect();
                let expected_id = if roof {
                    id::ROOF
                } else {
                    match walls {
                        Walls::Up => id::WALLS_UP,
                        Walls::Cutaway => id::CUTAWAY,
                        Walls::Down => id::WALLS_DOWN,
                    }
                };
                assert_eq!(selected_ids, vec![expected_id]);
            }
        }

        // When show_money is false (global 32 == 1, 0x4f24fa -> 0x53b2a4),
        // both the money box and the family-friends counter are omitted.
        let mut hidden_money = s.clone();
        hidden_money.show_money = false;
        let hidden_items = layout([800, 600], &hidden_money, &controls);
        assert!(
            !hidden_items
                .iter()
                .any(|i| matches!(i.kind, Kind::Friends(_)))
        );
        assert!(!hidden_items.iter().any(|i| matches!(
            &i.kind,
            Kind::Text { text, .. } if text == "$2,542"
        )));

        // speed_for reads idle return speed (global 18 >= 0) first, else active
        // speed (global 16), ignoring pause (global 17); unknown speed -> None.
        let mut world = sims_sim::World::empty();
        world.globals.resize(19, 0);
        world.globals[16] = sims_sim::speed::FAST;
        world.globals[17] = 1;
        world.globals[18] = -1;
        assert_eq!(speed_for(&world), Some(Speed::Fast));
        world.globals[16] = sims_sim::speed::IDLE;
        world.globals[18] = sims_sim::speed::ULTRA;
        assert_eq!(speed_for(&world), Some(Speed::Ultra));
        world.globals[16] = sims_sim::speed::IDLE;
        world.globals[18] = -1;
        assert_eq!(speed_for(&world), None);
        let no_speed = Controls {
            speed: None,
            ..Controls::default()
        };
        let none_items = layout([800, 600], &s, &no_speed);
        assert!(
            !none_items.iter().any(|i| matches!(
                i.kind,
                Kind::Button {
                    id: id::NORMAL | id::FAST | id::ULTRA,
                    selected: true,
                    ..
                }
            )),
            "unrecognized speed should select no speed button"
        );

        // Family friends count is right-aligned immediately left of `$9,999,999`
        // ending at x = 178, y = 158, and lets presses pass through to buttons
        // underneath (such as Zoom Out at (26, 149, 27, 27)).
        let friends = items
            .iter()
            .find(|i| matches!(i.kind, Kind::Friends(_)))
            .expect("the friends counter");
        assert_eq!(friends.rect, Rect::new(0, cy + 158, 178, 16));
        assert_eq!(friends.kind, Kind::Friends(12));
        let mut zoom_controls = controls;
        zoom_controls.camera.zoom_out = true;
        let zoom_items = layout([800, 600], &s, &zoom_controls);
        assert_eq!(
            press(&zoom_items, [35, cy + 165]),
            Some(Command::Camera(Camera::ZoomOut))
        );

        // Drawing checks: CenterTrunc puts odd slack on the right (unlike Center),
        // Friends draws right-aligned to `178 - width("$9,999,999")`, and Help draws cell 0.
        let mut art = Art::default();
        let font = crate::font::tests::font(); // A=3 (2x2 at bearing_x=1), B=1 (1x1)
        art.insert_font(8, font.clone());
        art.insert_font(10, font.clone());
        let mut help_strip = Image {
            width: 40,
            height: 15,
            rgba: Vec::new(),
        };
        for _y in 0..15u32 {
            for x in 0..40u32 {
                let cell = (x / 10) as u8;
                help_strip.rgba.extend_from_slice(&[cell + 1, 9, 9, 255]);
            }
        }
        art.insert(id::HELP, help_strip);

        // Help button draws cell 0 unhovered and cell 2 hovered; Pause draws
        // (col, row) from its 4x2 sheet: unpaused normal (0,0), unpaused hover
        // (2,0), paused unlit (1,0), paused lit (1,1), disabled (3,0).
        let mut pause_sheet = Image {
            width: 60,
            height: 30,
            rgba: Vec::new(),
        };
        for y in 0..30u32 {
            for x in 0..60u32 {
                let col = (x / 15) as u8;
                let row = (y / 15) as u8;
                pause_sheet
                    .rgba
                    .extend_from_slice(&[col + 1, row + 1, 7, 255]);
            }
        }
        art.insert(id::PAUSE, pause_sheet);

        let mut frame = Frame::new(220, 183, [0; 3]);
        // CenterTrunc on width 56 with width-1 "B": (56 - 1) / 2 = 27 (vs 28 for Center).
        let synthetic = [
            Item {
                rect: Rect::new(95, 118, 56, 14),
                kind: Kind::Text {
                    text: "B".into(),
                    size: 8,
                    rgb: GREEN,
                    align: Align::CenterTrunc,
                },
                command: None,
            },
            Item {
                rect: Rect::new(5, 164, 10, 15),
                kind: Kind::Button {
                    id: id::HELP,
                    cells: 4,
                    selected: false,
                },
                command: Some(Command::OpenHelp),
            },
            Item {
                rect: Rect::new(85, 135, 15, 15),
                kind: Kind::Pause {
                    paused: false,
                    lit: false,
                },
                command: Some(Command::TogglePause),
            },
        ];
        draw(&mut frame, &art, &Faces::default(), &synthetic, &controls);
        // `B` lands at x = 95 + 27 = 122, y = 118 + 1 (shifted down by A's -1 bearing).
        assert_eq!(frame.pixel(122, 119), Some(GREEN));
        assert_eq!(frame.pixel(123, 119), Some([0; 3]));
        // Help cell 0 is [1, 9, 9]; unpaused Pause is col 0 row 0 -> [1, 1, 7].
        assert_eq!(frame.pixel(5, 164), Some([1, 9, 9]));
        assert_eq!(frame.pixel(14, 178), Some([1, 9, 9]));
        assert_eq!(frame.pixel(85, 135), Some([1, 1, 7]));

        // Hovering Help draws cell 2 [3, 9, 9]; hovering unpaused Pause draws col 2 row 0 [3, 1, 7].
        let hover_help = Controls {
            pointer: Some([8, 170]),
            ..controls
        };
        draw(&mut frame, &art, &Faces::default(), &synthetic, &hover_help);
        assert_eq!(frame.pixel(5, 164), Some([3, 9, 9]));
        let hover_pause = Controls {
            pointer: Some([90, 140]),
            ..controls
        };
        draw(
            &mut frame,
            &art,
            &Faces::default(),
            &synthetic,
            &hover_pause,
        );
        assert_eq!(frame.pixel(85, 135), Some([3, 1, 7]));

        // Paused unlit -> col 1 row 0 [2, 1, 7] (even when hovered, 0x5e3bea);
        // paused lit -> col 1 row 1 [2, 2, 7]; disabled -> col 3 row 0 [4, 1, 7].
        for (paused, lit, command, want) in [
            (true, false, Some(Command::TogglePause), [2, 1, 7]),
            (true, true, Some(Command::TogglePause), [2, 2, 7]),
            (true, false, None, [4, 1, 7]),
        ] {
            let item = [Item {
                rect: Rect::new(85, 135, 15, 15),
                kind: Kind::Pause { paused, lit },
                command,
            }];
            draw(&mut frame, &art, &Faces::default(), &item, &hover_pause);
            assert_eq!(frame.pixel(85, 135), Some(want));
        }

        // Friends with a font that has digits and `$` / `,` (each 5px wide):
        // "$9,999,999" is 10 * 5 = 50px, so right edge is 178 - 50 = 128;
        // "12" is 10px wide, so it starts at x = 118.
        let mut glyphs = Vec::new();
        for (i, ch) in b"0123456789$,".iter().enumerate() {
            glyphs.push(sims_formats::ffn::Glyph {
                code: u16::from(*ch),
                width: 5,
                height: 2,
                atlas_x: (i as u16) * 5,
                atlas_y: 0,
                first_bearing: 0,
                bearing_x: 0,
                bearing_y: 0,
            });
        }
        let digit_font = Font::new(sims_formats::ffn::Font {
            flags: 9,
            glyphs,
            width: 60,
            height: 2,
            coverage: vec![15; 120],
        });
        art.insert_font(10, digit_font);
        let mut frame = Frame::new(220, 183, [0; 3]);
        draw(
            &mut frame,
            &art,
            &Faces::default(),
            &[Item {
                rect: Rect::new(0, 158, 178, 16),
                kind: Kind::Friends(12),
                command: None,
            }],
            &controls,
        );
        assert_eq!(frame.pixel(117, 158), Some([0; 3]));
        assert_eq!(frame.pixel(118, 158), Some(GREEN));
        assert_eq!(frame.pixel(127, 158), Some(GREEN));
        assert_eq!(frame.pixel(128, 158), Some([0; 3]));
    }

    #[test]
    fn pet_personality_and_pet_skills_panels_match_disassembly() {
        // Pet Personality (`0x52157e..0x5215c0`, `0x51d650..0x51d663`):
        // draws pet trait labels (`STR# 170`) with fixed human widest-label
        // alignment and suppresses zodiac + `Kind::Sign` + Sign balloon.
        let mut s = snapshot();
        s.residents[0].pet = true;
        s.residents[0].pet_kind = Some(PetKind::Dog);
        s.residents[0].zodiac = Some(4); // suppressed even if non-None
        s.residents[0].personality = [300, 700, 500, 900, 100];
        s.residents[0].skills = [400, 1000, 200, 0, 0, 0];

        let mut controls = Controls {
            expanded: true,
            panel: Some(Panel::Personality),
            help: Some(Help::Sign(4)),
            ..Controls::default()
        };
        let items = layout([800, 600], &s, &controls);
        assert!(items.iter().any(|i| matches!(
            i.kind,
            Kind::Personality {
                traits: [300, 700, 500, 900, 100],
                zodiac: None,
                pet: true,
            }
        )));
        assert!(!items.iter().any(|i| matches!(i.kind, Kind::Sign { .. })));
        assert!(!items.iter().any(|i| matches!(i.kind, Kind::Help { .. })));

        // Build a synthetic font where each character has a distinct width so
        // pet vs human labels render visibly different widths/pixels.
        let mut glyphs = Vec::new();
        let mut atlas_x = 0u16;
        for (idx, ch) in (b' '..=b'z').enumerate() {
            let w = 2 + (idx as u8 % 3);
            glyphs.push(sims_formats::ffn::Glyph {
                code: u16::from(ch),
                width: w,
                height: 2,
                atlas_x,
                atlas_y: 0,
                first_bearing: 0,
                bearing_x: 0,
                bearing_y: 0,
            });
            atlas_x += u16::from(w);
        }
        let ffn = sims_formats::ffn::Font {
            flags: 9,
            glyphs,
            width: atlas_x,
            height: 2,
            coverage: vec![15; usize::from(atlas_x) * 2],
        };
        let mut art = Art::default();
        art.insert_font(TRAIT_FONT, Font::new(ffn.clone()));
        art.insert_font(SKILL_FONT, Font::new(ffn));
        art.insert(id::TRAIT_EMPTY, solid(40, 14, [1, 1, 1]));
        art.insert(id::TRAIT_FULL, solid(40, 16, [2, 2, 2]));
        art.insert(id::SKILL_EMPTY, solid(40, 10, [10, 10, 10]));
        art.insert(id::SKILL_FULL, solid(40, 10, [20, 20, 20]));

        let panel = Rect::new(0, 0, 280, 100);
        let mut human_frame = Frame::new(280, 100, [0; 3]);
        draw_personality(
            &mut human_frame,
            &art,
            panel,
            &[300, 700, 500, 900, 100],
            false,
        );
        let mut pet_frame = Frame::new(280, 100, [0; 3]);
        draw_personality(
            &mut pet_frame,
            &art,
            panel,
            &[300, 700, 500, 900, 100],
            true,
        );
        assert_ne!(human_frame.rgb(), pet_frame.rgb());

        // Pet Skills on Job tab (`0x51d5ca..0x51d5f0`, `0x531469`, `0x531e43`):
        // Dog shows `"Pet Skills"` + `Kind::PetSkills { kind: Dog, .. }` and no
        // human `Kind::Skills` or `Kind::Work`.
        controls.panel = Some(Panel::Job);
        let dog_items = layout([800, 600], &s, &controls);
        assert!(dog_items.iter().any(|i| matches!(
            &i.kind,
            Kind::Text { text, size: 11, .. } if text == "Pet Skills"
                && i.rect.x == 520 + PET_SKILLS_TITLE[0]
                && i.rect.y == 500 + PET_SKILLS_TITLE[1]
        )));
        assert!(dog_items.iter().any(|i| matches!(
            i.kind,
            Kind::PetSkills {
                kind: PetKind::Dog,
                skills: [400, 1000, 200],
            }
        )));
        assert!(
            !dog_items
                .iter()
                .any(|i| matches!(i.kind, Kind::Skills { .. } | Kind::Work { .. }))
        );

        // Cat (`PetKind::Cat`) selects Cat skills panel; school-age (`age in 1..18`)
        // takes precedence (`0x51d5af..0x51d5bf`) and falls back to Job.
        s.residents[0].pet_kind = Some(PetKind::Cat);
        let cat_items = layout([800, 600], &s, &controls);
        assert!(cat_items.iter().any(|i| matches!(
            i.kind,
            Kind::PetSkills {
                kind: PetKind::Cat,
                skills: [400, 1000, 200],
            }
        )));

        let mut dog_frame = Frame::new(280, 100, [0; 3]);
        draw_pet_skills(&mut dog_frame, &art, panel, PetKind::Dog, &[400, 1000, 200]);
        let mut cat_frame = Frame::new(280, 100, [0; 3]);
        draw_pet_skills(&mut cat_frame, &art, panel, PetKind::Cat, &[400, 1000, 200]);
        assert_ne!(dog_frame.rgb(), cat_frame.rgb());
        // Labels are top-aligned (`local top = 0`, `0x53180d..0x531826`) in
        // `(103, 25 + 15 * row, 102, 16)`: row 0 draws at y = 25 (not centered).
        assert_eq!(dog_frame.pixel(204, 25), Some(LABEL));
        assert_eq!(dog_frame.pixel(204, 24), Some([0, 0, 0]));
        assert_eq!(dog_frame.pixel(204, 27), Some([0, 0, 0]));
        // Row 0 bar (400 -> 4 pts -> 16 px full, rest empty) at (207, 27).
        assert_eq!(dog_frame.pixel(207, 27), Some([20, 20, 20]));
        assert_eq!(dog_frame.pixel(207 + 15, 27), Some([20, 20, 20]));
        assert_eq!(dog_frame.pixel(207 + 16, 27), Some([10, 10, 10]));
        // Row 1 bar (1000 -> 10 pts -> full 40 px) at (207, 42).
        assert_eq!(dog_frame.pixel(207 + 39, 42), Some([20, 20, 20]));
        // Row 2 bar (200 -> 2 pts -> 8 px full, rest empty) at (207, 57).
        assert_eq!(dog_frame.pixel(207 + 7, 57), Some([20, 20, 20]));
        assert_eq!(dog_frame.pixel(207 + 8, 57), Some([10, 10, 10]));

        // School-age takes priority over pet bits (`0x51d5af..0x51d5bf`),
        // routing to the Child Report Card subpanel (`[esi + 0x124]`).
        s.residents[0].school_age = true;
        let school_items = layout([800, 600], &s, &controls);
        assert!(
            school_items
                .iter()
                .any(|i| matches!(i.kind, Kind::ReportCard { .. }))
        );
        assert!(
            !school_items
                .iter()
                .any(|i| matches!(i.kind, Kind::PetSkills { .. }))
        );
    }

    #[test]
    fn the_subpanel_is_where_its_hit_test_starts_at_every_width() {
        let s = snapshot();
        let controls = Controls {
            expanded: true,
            panel: Some(Panel::Needs),
            ..Controls::default()
        };
        for (size, x) in [([640, 480], 360), ([800, 600], 520), ([1024, 768], 520)] {
            let items = layout(size, &s, &controls);
            let panel = items
                .iter()
                .find(|i| matches!(i.kind, Kind::Needs { pet: false }))
                .expect("the needs panel")
                .rect;
            assert_eq!(panel.x, x, "{size:?}");
            let [right, bottom] = [panel.x + panel.width, panel.y + panel.height];
            assert_eq!(right, size[0], "{size:?}");
            assert_eq!(bottom, size[1], "{size:?}");
            assert!(in_subpanel(&items, [panel.x, panel.y]), "{size:?}");
            assert!(in_subpanel(&items, [right - 1, bottom - 1]), "{size:?}");
            assert!(!in_subpanel(&items, [panel.x - 1, panel.y]), "{size:?}");
            assert!(!in_subpanel(&items, [panel.x, panel.y - 1]), "{size:?}");
            assert!(!in_subpanel(&items, [right, panel.y]), "{size:?}");
            assert!(!in_subpanel(&items, [panel.x, bottom]), "{size:?}");
        }
    }

    #[test]
    fn the_performance_blinks_only_for_a_job_below_zero() {
        let kind = |level: bool, performance: i16| Kind::Work {
            work: Work {
                icon: 1,
                level: level.then(|| crate::household::WorkLevel {
                    title: "M".into(),
                    salary: 1,
                }),
                friends: 0,
            },
            performance,
            family_friends: 0,
        };
        assert!(performance_blinks(&kind(true, -1)));
        assert!(!performance_blinks(&kind(true, 0)));
        assert!(!performance_blinks(&kind(false, -50)));
        assert!(!performance_blinks(&Kind::Area));
        // `[0x6f2ab0]` is RGB (0, 202, 57) in 16 bits.
        assert_eq!(PERFORMANCE_LIT, [0, 203, 57]);
    }

    #[test]
    fn an_adults_job_skill_labels_open_the_skill_balloons() {
        let mut s = snapshot();
        s.residents[0].school_age = false;
        s.residents[0].child = false;
        let mut controls = Controls {
            expanded: true,
            panel: Some(Panel::Job),
            ..Controls::default()
        };
        let mut texts = Texts::default();
        let skill_popups: Vec<String> = (0..12).map(|n| format!("K{n}")).collect();
        texts.set_live(&[], &[], &[], &[], &skill_popups);
        let items = layout([800, 600], &s, &controls);
        let panel = items
            .iter()
            .find(|i| matches!(i.kind, Kind::Work { .. }))
            .expect("the adult job panel")
            .rect;
        // `0x6f4420`: the labels are 62 by 16 from (93, 6), 15 apart.
        assert_eq!(
            press(&items, [panel.x + 93, panel.y + 6 + 30]),
            Some(Command::Help(Help::JobSkill(2)))
        );
        assert_eq!(
            press(&items, [panel.x + 154, panel.y + 6 + 75 + 15]),
            Some(Command::Help(Help::JobSkill(5)))
        );
        assert_eq!(press(&items, [panel.x + 92, panel.y + 6]), None);
        assert_eq!(press(&items, [panel.x + 93, panel.y + 5]), None);
        // The text and pictures are the Superstar skill's.
        assert_eq!(texts.get(Help::JobSkill(2)), texts.get(Help::FameSkill(2)));
        assert_eq!(texts.get(Help::JobSkill(2)).unwrap().title, "K4");
        // Another label replaces the balloon; the same one closes it.
        controls.apply(&Command::Help(Help::JobSkill(2)));
        assert_eq!(controls.help, Some(Help::JobSkill(2)));
        controls.apply(&Command::Help(Help::JobSkill(4)));
        assert_eq!(controls.help, Some(Help::JobSkill(4)));
        let items = layout([800, 600], &s, &controls);
        assert!(items.iter().any(|i| matches!(
            i.kind,
            Kind::Help { help: Help::JobSkill(4), owner } if owner == panel
        )));
        assert!(help_matches_snapshot(
            Help::JobSkill(4),
            &s,
            Some(Panel::Job)
        ));
        assert!(!help_matches_snapshot(
            Help::JobSkill(4),
            &s,
            Some(Panel::Needs)
        ));
        controls.apply(&Command::Help(Help::JobSkill(4)));
        assert_eq!(controls.help, None);
    }

    #[test]
    fn report_card_superstar_and_subpanel_help_balloons_match_disassembly() {
        use crate::balloon::{Picture, Text};
        let mut s = snapshot();
        let mut controls = Controls {
            expanded: true,
            panel: Some(Panel::Job),
            ..Controls::default()
        };
        let mut art = Art::default();
        let font = crate::font::tests::font();
        for size in [7, 8, 9, 10, 11, 18] {
            art.insert_font(size, font.clone());
        }
        let solid = |w: u32, h: u32, rgb: [u8; 3]| Image {
            width: w,
            height: h,
            rgba: [rgb[0], rgb[1], rgb[2], 255].repeat((w * h) as usize),
        };
        art.insert(id::SKILL_EMPTY, solid(40, 10, [10, 10, 10]));
        art.insert(id::SKILL_FULL, solid(40, 10, [20, 20, 20]));
        art.insert(id::FAME_BAR_EMPTY, solid(80, 16, [30, 30, 30]));
        art.insert(id::FAME_BAR_FULL, solid(80, 16, [250, 200, 0]));
        art.insert(id::FAME_FRIENDS, solid(88, 20, [0, 180, 250]));
        art.insert(id::SOCIAL_POPUP, solid(40, 40, [90, 140, 210]));
        art.insert(id::JOB_POPUP, solid(84, 22 * 63, [120, 90, 180]));

        let mut texts = Texts::default();
        let needs: Vec<String> = (0..17).map(|n| format!("A{n}")).collect();
        let pet_needs: Vec<String> = (0..17).map(|n| format!("P{n}")).collect();
        let pet_skills: Vec<String> = (0..8).map(|n| format!("S{n}")).collect();
        let report: Vec<String> = vec!["Report Card".into(), "Grades".into(), "A".into()];
        let skill_popups: Vec<String> = (0..12).map(|n| format!("K{n}")).collect();
        let fame_strs: Vec<String> = (0..39).map(|n| format!("F{n}")).collect();
        texts.set_live(&needs, &pet_needs, &pet_skills, &report, &skill_popups);
        texts.set_fame(&fame_strs);

        // 1. Child Report Card: layout -> fit -> press grade button -> balloon with row 11 icon.
        s.residents[0].school_age = true;
        s.residents[0].child = true;
        s.residents[0].school_grade = "A+".into();
        let mut items = layout([800, 600], &s, &controls);
        fit(&mut items, &art, &texts);
        let btn = items
            .iter()
            .find(|i| matches!(i.kind, Kind::ReportCardButton { .. }))
            .expect("grade button");
        assert_eq!(btn.command, Some(Command::Help(Help::ReportCard)));
        assert_eq!(
            press(&items, [btn.rect.x + 10, btn.rect.y + 1]),
            Some(Command::Help(Help::ReportCard))
        );
        controls.apply(&Command::Help(Help::ReportCard));
        let mut items = layout([800, 600], &s, &controls);
        fit(&mut items, &art, &texts);
        let balloon = items
            .iter()
            .find_map(|i| match &i.kind {
                Kind::Balloon(b) => Some(b),
                _ => None,
            })
            .expect("Report Card balloon");
        assert_eq!(balloon.title, "Grades");
        assert_eq!(balloon.text_left, 168);
        assert_eq!(
            balloon.pictures[0],
            Some(Picture::Row {
                id: id::JOB_POPUP,
                rows: 22,
                row: 11,
            })
        );
        // Pressing again closes it.
        controls.apply(&Command::Help(Help::ReportCard));
        assert_eq!(controls.help, None);

        // 2. Superstar / Fame routing (`0x4cfe2c`): requires superstar pack +
        //    fame_score > 0 + !has_career, and loses to school_age and pet_kind.
        s.residents[0].school_age = false;
        s.residents[0].child = false;
        s.residents[0].fame_score = 40;
        s.residents[0].fame_level = 3; // 1.5 stars -> 24 px of 80 px bar
        s.residents[0].fame_title = "Name Dropper".into();
        s.residents[0].fame_friends_text = "1.5 (Need 2)".into();
        s.residents[0].skills = [0, 0, 100, 0, 0, 200];
        s.residents[0].fame_needs = [0, 0, 3, 2, 0, 3];
        s.superstar = false;
        assert!(
            layout([800, 600], &s, &controls)
                .iter()
                .any(|i| matches!(i.kind, Kind::Work { .. })),
            "without Superstar pack, stays on Adult Job"
        );
        s.superstar = true;
        s.residents[0].has_career = true;
        assert!(
            layout([800, 600], &s, &controls)
                .iter()
                .any(|i| matches!(i.kind, Kind::Work { .. })),
            "with an active regular career, stays on Adult Job"
        );
        s.residents[0].has_career = false;
        let fame_items = layout([800, 600], &s, &controls);
        assert!(
            fame_items
                .iter()
                .any(|i| matches!(i.kind, Kind::Fame { .. }))
        );
        let panel = fame_items
            .iter()
            .find(|i| matches!(i.kind, Kind::Fame { .. }))
            .unwrap()
            .rect;
        // Clicking skill 2 (Charisma), fame level title, and friends starpower.
        assert_eq!(
            press(&fame_items, [panel.x + 150, panel.y + 6 + 30 + 4]),
            Some(Command::Help(Help::FameSkill(2)))
        );
        assert_eq!(
            press(&fame_items, [panel.x + 10, panel.y + 50]),
            Some(Command::Help(Help::FameLevel(3)))
        );
        assert_eq!(
            press(&fame_items, [panel.x + 10, panel.y + 80]),
            Some(Command::Help(Help::FameFriends))
        );
        // Switching directly between FameSkill(2) and FameFriends replaces the
        // balloon; clicking FameFriends again closes it.
        controls.apply(&Command::Help(Help::FameSkill(2)));
        assert_eq!(controls.help, Some(Help::FameSkill(2)));
        controls.apply(&Command::Help(Help::FameFriends));
        assert_eq!(controls.help, Some(Help::FameFriends));
        controls.apply(&Command::Help(Help::FameFriends));
        assert_eq!(controls.help, None);

        // Draw Superstar panel: verify 24px fame star fill at (3, 30), skill
        // target mark for Charisma (row 2, need = 3 -> mark at 210 + 12 - 1 = 221),
        // and 4-cell Fame friends smiley hover cell 2 (`0x52c369`, `0x5e3be6`).
        let mut art_with_smiley = art.clone();
        let mut smiley_rgba = Vec::with_capacity(88 * 20 * 4);
        for _y in 0..20u32 {
            for x in 0..88u32 {
                let px: [u8; 4] = match x / 22 {
                    0 => [10, 20, 30, 255],
                    1 => [40, 50, 60, 255],
                    2 => [200, 100, 50, 255],
                    _ => [80, 80, 80, 255],
                };
                smiley_rgba.extend_from_slice(&px);
            }
        }
        art_with_smiley.insert(
            id::FAME_FRIENDS,
            Image {
                width: 88,
                height: 20,
                rgba: smiley_rgba,
            },
        );
        let mut fame_frame = Frame::new(280, 100, [0; 3]);
        draw_fame(
            &mut fame_frame,
            &art_with_smiley,
            Rect::new(0, 0, 280, 100),
            None,
            3,
            "A",
            &[0, 0, 100, 0, 0, 200],
            &[0, 0, 3, 2, 0, 3],
            "A",
        );
        assert_eq!(fame_frame.pixel(3 + 23, 30), Some([250, 200, 0]));
        assert_eq!(fame_frame.pixel(3 + 24, 30), Some([30, 30, 30]));
        assert_eq!(fame_frame.pixel(221, 7 + 30), Some(LABEL));
        assert_eq!(fame_frame.pixel(210 - 16, 7 + 30 + 9), Some(LABEL));
        assert_eq!(fame_frame.pixel(5, 77), Some([10, 20, 30]));
        // Hovering the friends button selects cell 2.
        let mut fame_hover = Frame::new(280, 100, [0; 3]);
        draw_fame(
            &mut fame_hover,
            &art_with_smiley,
            Rect::new(0, 0, 280, 100),
            Some([10, 80]),
            3,
            "A",
            &[0, 0, 100, 0, 0, 200],
            &[0, 0, 3, 2, 0, 3],
            "A",
        );
        assert_eq!(fame_hover.pixel(5, 77), Some([200, 100, 50]));

        // 3. Needs & Pet Needs click hitboxes (`100x20` at 19px stride) and
        //    balloon owner matching + outside-live-panel dismissal.
        controls.panel = Some(Panel::Needs);
        let need_items = layout([800, 600], &s, &controls);
        let need_panel = need_items
            .iter()
            .find(|i| matches!(i.kind, Kind::Needs { pet: false }))
            .unwrap()
            .rect;
        // Index 0 (Hunger, col 0 row 0), index 5 (Social, col 1 row 2), and
        // bottom-row scanline y=97 (`21 + 19*3 + 19 = 97`, `0x513330`).
        assert_eq!(
            press(&need_items, [need_panel.x + 25, need_panel.y + 30]),
            Some(Command::Help(Help::Need {
                pet: false,
                index: 0,
            }))
        );
        assert_eq!(
            press(
                &need_items,
                [need_panel.x + 125, need_panel.y + 21 + 38 + 5]
            ),
            Some(Command::Help(Help::Need {
                pet: false,
                index: 5,
            }))
        );
        assert_eq!(
            press(&need_items, [need_panel.x + 125, need_panel.y + 97]),
            Some(Command::Help(Help::Need {
                pet: false,
                index: 7,
            }))
        );
        assert!(in_subpanel(
            &need_items,
            [need_panel.x + 10, need_panel.y + 10]
        ));
        assert!(!in_subpanel(&need_items, [10, 550]));
        assert!(!in_subpanel(
            &need_items,
            [need_panel.x + 10, need_panel.y - 1]
        ));
        assert!(!in_subpanel(
            &need_items,
            [need_panel.x - 10, need_panel.y + 10]
        ));
        // At 1024x768 (`0x518f40..0x518f48`, `0x51f258..0x51f260`), the western
        // subpanel stays anchored at x=520 (strip + 300, `0x517e3d`) and extends its right edge by 224px.
        let need_1024 = layout([1024, 768], &s, &controls);
        let panel_1024 = need_1024
            .iter()
            .find(|i| matches!(i.kind, Kind::Needs { pet: false }))
            .unwrap()
            .rect;
        assert_eq!(panel_1024, Rect::new(520, 668, 504, 100));
        assert!(in_subpanel(&need_1024, [650, 700]));
        // Report Card grade button spans the full 504px subpanel at 1024px (`0x532c08`).
        let mut rc_snap = s.clone();
        rc_snap.residents[0].school_age = true;
        rc_snap.residents[0].child = true;
        let rc_controls = Controls {
            expanded: true,
            panel: Some(Panel::Job),
            ..Controls::default()
        };
        let mut rc_1024 = layout([1024, 768], &rc_snap, &rc_controls);
        fit(&mut rc_1024, &art, &texts);
        let rc_btn_1024 = rc_1024
            .iter()
            .find(|i| matches!(i.kind, Kind::ReportCardButton { .. }))
            .unwrap()
            .rect;
        assert_eq!((rc_btn_1024.x, rc_btn_1024.width), (520, 504));
        controls.help = Some(Help::Need {
            pet: false,
            index: 0,
        });
        controls.apply(&Command::SetSpeed(Speed::Fast));
        assert_eq!(controls.help, None);
        assert!(help_matches_snapshot(
            Help::Need {
                pet: false,
                index: 5,
            },
            &s,
            Some(Panel::Needs)
        ));
        assert!(!help_matches_snapshot(
            Help::Need {
                pet: true,
                index: 5,
            },
            &s,
            Some(Panel::Needs)
        ));

        // Unused import check prevention for Text.
        let _ = Text::default();
    }

    #[test]
    fn ultra_speed_blink_duplicate_selection_and_non_residential_house() {
        // State-3 ultra-speed blink (`0x53b4c9..0x53b7e7`):
        assert_eq!(
            clock_blink_mode(false, Mode::Live, true, Some(Speed::Normal)),
            ClockBlinkMode::Ultra
        );
        assert_eq!(
            clock_blink_mode(false, Mode::Live, true, Some(Speed::Ultra)),
            ClockBlinkMode::Off
        );
        assert_eq!(
            clock_blink_mode(true, Mode::Live, true, Some(Speed::Normal)),
            ClockBlinkMode::Pause
        );
        assert_eq!(
            clock_blink_mode(false, Mode::Buy, true, Some(Speed::Normal)),
            ClockBlinkMode::Off
        );

        let mut s = snapshot();
        s.idle_speed = true;
        let mut controls = Controls {
            expanded: true,
            speed: Some(Speed::Normal),
            ultra_lit: false,
            ..Controls::default()
        };
        let speed_selected = |items: &[Item], target: i32| {
            items
                .iter()
                .find_map(|i| match i.kind {
                    Kind::Button { id, selected, .. } if id == target => Some(selected),
                    _ => None,
                })
                .expect("speed button")
        };
        let off_items = layout([800, 600], &s, &controls);
        assert!(speed_selected(&off_items, id::NORMAL));
        assert!(!speed_selected(&off_items, id::ULTRA));

        controls.ultra_lit = true;
        let on_items = layout([800, 600], &s, &controls);
        assert!(speed_selected(&on_items, id::NORMAL));
        assert!(speed_selected(&on_items, id::ULTRA));

        // Paused or non-Live suppresses ultra blink even if ultra_lit were true.
        s.paused = true;
        let paused_items = layout([800, 600], &s, &controls);
        assert!(!speed_selected(&paused_items, id::ULTRA));
        s.paused = false;

        // Duplicate live instance matching by GUID (`0x51c5ef -> 0x4e97dc`):
        // object 999 shares resident 0's GUID (21) -> still Human & highlights portrait.
        s.selected = Some(999);
        s.selected_guid = Some(21);
        assert_eq!(Selected::of(&s), Selected::Human);
        let dup_items = layout([800, 600], &s, &controls);
        let first_portrait_selected = dup_items
            .iter()
            .find_map(|i| match i.kind {
                Kind::Portrait { guid: 21, selected } => Some(selected),
                _ => None,
            })
            .expect("first portrait");
        assert!(first_portrait_selected);

        // Non-residential House panel suppression (`0x51d3f9..0x51d40f`):
        s.residential = false;
        controls.panel = Some(Panel::House);
        let nonres_items = layout([800, 600], &s, &controls);
        assert!(
            !nonres_items
                .iter()
                .any(|i| matches!(i.kind, Kind::House { .. }))
        );
        assert!(!speed_selected(&nonres_items, id::HOUSE));
    }
}
