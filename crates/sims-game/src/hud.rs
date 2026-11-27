//! The live-mode control panel (`sims-ui`) over the lot, read from the
//! running world each frame, the selected Sim's queue of actions, and
//! the dialog the world waits on.
//!
//! The panel turns presses into `sims_ui` commands; [`Hud::press`] hands
//! them to the game, which carries out what it can.

use crate::menu::{Menu, Release};
use crate::pace::Pace;
use crate::view::View;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_content::{ArchivePool, BuyLabels, Catalog, Index, Install, Neighborhood};
use sims_render::{Assets, Canvas, Walls, Zoom};
use sims_sim::World;
use sims_ui::art::Art;
use sims_ui::balloon::{self, Balloon, BalloonFadeState, Texts};
use sims_ui::build::{
    self, BuildCatalog, BuildLayout, BuildState, BuildSubtoolAction, BuildSwatches,
    BuildWidgetKind, TerrainSubtool,
};
use sims_ui::buy::{
    self, BuyCatalog, BuyLayout, BuyState, BuyWidgetKind, RatingLabels, SellInfo, SellLabels,
};
use sims_ui::camera::{
    self, CameraButton, CameraCommand, CameraLayout, CameraPanel, CameraState, CameraStrings,
};
use sims_ui::cheat_console::CheatConsole;
use sims_ui::command::{
    BuildItemId, Command, Mode, OptionsAction, Panel as UiPanel, VolumeChannel, Walls as UiWalls,
};
use sims_ui::dialog::{self, DialogFadeState, DialogState, Labels, Message};
use sims_ui::dialog_timer::{self, AutoAnswer};
use sims_ui::frame::{Frame, Rect};
use sims_ui::house;
use sims_ui::household::{Acquaintances, Careers, Faces, Fame, Snapshot};
use sims_ui::hud::{self, Controls, Item, Selected};
use sims_ui::needs_history::{NeedsTrend, PanelFrame};
use sims_ui::options::{self, OptionsTexts};
use sims_ui::pie;
use sims_ui::queue::{Queue, QueueSlideState};
use sims_ui::thumbnail::{self, Thumbnails};
use sims_vm::SoundEvent;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

/// The status line's font size and colors.
const STATUS_SIZE: u8 = 10;
const STATUS_TEXT: [u8; 3] = [255, 255, 255];
const STATUS_BACK: [u8; 3] = [96, 24, 24];

/// The object the interface's own sounds are played for (0x4cb171).
pub const INTERFACE: i16 = -4;
/// What a press on an action's icon plays when it counts, and when it
/// does not or is a pet's (0x4cb16e).
pub const QUEUE_DELETE: &str = "UI_queue_delete";
pub const ERROR: &str = "UI_error";
/// A press on a portrait (0x4cfa51).
pub const CLICK: &str = "UI_click";
/// A help balloon opening or closing (0x50fa95, 0x50fb7f).
pub const WHOOSH: &str = "UI_whoosh";

/// A help balloon the HUD fades: a Live one or an Options one. Both are
/// `cWinBalloon` windows (`0x50fc30`), with the same 250 ms ramp.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FadingBalloon {
    Live(Balloon),
    Options(options::OptionsBalloon),
}

impl FadingBalloon {
    fn of(item: &Item) -> Option<Self> {
        match &item.kind {
            hud::Kind::Balloon(b) => Some(Self::Live(b.clone())),
            hud::Kind::OptionsBalloon(b) => Some(Self::Options(b.clone())),
            _ => None,
        }
    }

    fn rect(&self) -> Rect {
        match self {
            Self::Live(b) => b.rect,
            Self::Options(b) => b.rect,
        }
    }

    fn is_options(&self) -> bool {
        matches!(self, Self::Options(_))
    }

    /// Draws it moved down by `dy` at `alpha`.
    fn draw(&self, frame: &mut Frame, art: &Art, thumbnails: &Thumbnails, dy: i32, alpha: u8) {
        match self {
            Self::Live(b) => {
                let mut b = b.clone();
                b.rect.y = b.rect.y.saturating_add(dy);
                balloon::draw_with_alpha(frame, art, Some(thumbnails), &b, alpha);
            }
            Self::Options(b) => {
                let mut b = b.clone();
                b.rect.y = b.rect.y.saturating_add(dy);
                options::draw_balloon_with_alpha(frame, art, &b, alpha);
            }
        }
    }
}
/// Selling an object with nonzero refund (`0x487e18` → `0x492ad4`).
pub const MONEYBACK: &str = "UI_moneyback";
/// Disposing of a zero-value object (`0x487e18` → `0x492b05`).
pub const ARCHTOOL_DELETE: &str = "UI_archtool_delete";
/// Build Mode drag tool press (`0x48b000`, `0x488800`, `0x48a100`).
pub const BLD_DRAG_DOWN: &str = "UI_bld_dragtool_mousedown";
/// Build Mode drag tool commit (`0x48b550`, `0x488a80`, `0x48a480`).
pub const BLD_DRAG_UP: &str = "UI_bld_dragtool_mouseup";

/// Keyboard keys handled by native modal dialogs (`cTSWinGenDlg::OnKeyDown` /
/// `OnChar` at `0x5e5b3b..0x5e5bfd`, `0x6d7530`, plus Tab on multi-field cards).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKey {
    /// Enter (`0x0d`): activates the primary/default button (button 0).
    Enter,
    /// Escape (`0x1b`): activates the cancel button (last button on confirmations,
    /// or Cancel on multi-button entry cards).
    Escape,
    /// Tab: cycles keyboard focus between Lot Name and Lot Description on the
    /// edit-mode Save Card.
    Tab,
    /// Printable character: matches localized Yes/No shortcut letters on
    /// confirmation dialogs (`STR# 142[3]` / `STR# 142[4]`, `0x5e5b80..0x5e5bfd`).
    Char(char),
}

/// Which native Options confirmation dialog is currently open (`0x516809`,
/// `0x516874`, `0x5170f0`, `0x517c03 -> 0x50d68a`, `0x4f290d`, `0x4f1a11`,
/// `0x4f25b4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionsDialog {
    /// Edit-mode Save card (`0x516874 -> 0x517c03(0)` when `[lot + 0x50] == 1`):
    /// `PopupInfoTiles.bmp` (3008) + `SAVE_CARD` (4994) + OK.
    SaveCardSave,
    /// Edit-mode dirty Quit card (`0x516815 -> 0x517c03(1)`):
    /// `PopupInfoTiles.bmp` (3008) + `SAVE_CARD` (4994) + Yes/No/Cancel.
    SaveCardQuit,
    /// Edit-mode dirty Neighborhood card (`0x51710c -> 0x517c03(1)`):
    /// `PopupInfoTiles.bmp` (3008) + `SAVE_CARD` (4994) + Yes/No/Cancel.
    SaveCardNeighborhood,
    /// Clean or non-savable Quit confirmation (`0x4f29f1..0x4f2a7a`):
    /// `STR# 153[1..2]` (`confirm_quit`) + Yes/No.
    ConfirmQuit,
    /// Dirty savable Quit confirmation (`0x4f298e..0x4f29be`):
    /// `STR# 153[5..6]` (`confirm_save_before_quit`) + Yes/No/Cancel.
    ConfirmSaveBeforeQuit,
    /// Dirty savable Neighborhood confirmation (`0x51712d -> 0x4f1a11`):
    /// `STR# 153[3..4]` (`confirm_save`) + Yes/No/Cancel.
    ConfirmSaveBeforeNeighborhood,
    /// Reset Tutorial confirmation (`0x4f25b4`):
    /// `STR# 153[7..8]` (`confirm_reset_tutorial`) + Yes/No.
    ConfirmResetTutorial,
    /// Edit-mode dirty Reset Tutorial card (`0x4f25ea -> 0x4f1a11 -> 0x517c03(1)`):
    /// `PopupInfoTiles.bmp` (3008) + `SAVE_CARD` (4994) + Yes/No/Cancel.
    SaveCardResetTutorial,
    /// Dirty savable Reset Tutorial confirmation (`0x4f25ea -> 0x4f1a11`):
    /// `STR# 153[3..4]` (`confirm_save`) + Yes/No/Cancel.
    ConfirmSaveBeforeResetTutorial,
    /// Genuine user-facing error or confirmation notice (`cTSWinMsgBox` + OK).
    Notice,
    /// Inline reference book modal (`Dialog` kind 12 Spellbook or kind 15 Cookbook,
    /// `0x4518e2`/`0x451946 -> 0x5107dc`), dismissed with OK / Enter / Escape.
    SpecialBook,
}

/// Duration in milliseconds for native modal dialog fade-in (`cTSWinFade`,
/// `0x5e8c93..0x5e8e10`).
pub const DIALOG_FADE_DURATION_MS: u32 = 200;

pub struct Hud {
    pub art: Art,
    faces: Faces,
    /// Boot-time faces/names snapshot, restored before re-applying saved
    /// runtime character names on [`Game::restore`].
    boot_faces: Faces,
    /// The catalog pictures of the object types the queue has shown.
    thumbnails: Thumbnails,
    /// What could not be read while drawing, until the game takes it.
    problems: Vec<String>,
    /// The careers of `work.iff`, for what the job panel shows the next
    /// level needs and for Child Report Card grade strings.
    careers: Careers,
    /// Superstar fame progression data (`GameData/fame.iff`).
    fame: Fame,
    /// The curves and constants the house panel's ratings are worked out
    /// with.
    house: house::Tuning,
    /// Which of the lot's object types the Upkeep rating counts, read as
    /// they show up while the house panel shows.
    kinds: house::Kinds,
    /// What the relationship panel needs from outside the world: the
    /// autonomy tuning's friendship threshold (0x6c4398) for its
    /// smileys, the neighborhood's characters, and the Superstar pack.
    known: Acquaintances,
    /// The UI archive, for a dialog's named icon outside `art`.
    ui: Option<UiGraphics>,
    labels: Labels,
    /// The help balloons' texts.
    texts: Texts,
    /// The Options panel's coordinates, labels, and confirmation strings
    /// (`GameData/UIText.iff STR# 145` & `153`).
    options_texts: OptionsTexts,
    /// The household's member GUIDs, in the neighborhood's order.
    members: Vec<u32>,
    pub controls: Controls,
    items: Vec<Item>,
    /// The dialog the world waits on, and the record it was read from
    /// (its caller and the operands), so it is read once.
    message: Option<(Message, (u16, [u8; 8]))>,
    /// A UI-owned modal confirmation dialog (e.g. Save / Neighborhood / Quit /
    /// Reset Tutorial from Options mode), shown using the same native
    /// `cTSWinMsgBox` chrome (`PopupInfoTiles.bmp` 3008).
    ui_dialog: Option<(OptionsDialog, Message)>,
    /// Interactive state (focus, caret, 500 ms blink, pressed button) of the
    /// active modal dialog.
    dialog_state: DialogState,
    /// The press held now landed on a non-modal dialog
    /// ([`Hud::take_non_modal_press`]): its release is the dialog's too.
    non_modal_press: bool,
    /// Whether Shift is held, so a press in a dialog's text field extends
    /// its selection (`0x5f2522`).
    shift_held: bool,
    /// The simulation dialog's 40 s auto-answer timer (`0x436c89`), and
    /// the answer it gives when it runs out.
    auto_answer: AutoAnswer,
    auto_answer_status: Option<u8>,
    /// Each non-modal dialog still shown, read at the end of the update
    /// that showed it, while its caller's frame (and so its `$Local`
    /// text) was still there: one queued behind another keeps its text.
    non_modal_read: Vec<((u16, [u8; 8]), Message)>,
    dialog: Option<dialog::Layout>,
    /// Active Sound volume slider being dragged (`0x5f6245..0x5f6336`).
    dragging_slider: Option<VolumeChannel>,
    /// Options action button (`Save` / `Neighborhood` / `Quit` / `Reset Tutorial`)
    /// currently held down for mouse-up dispatch (`0x514170..0x514176`,
    /// `0x5e45d2..0x5e45f3`). Hit-tested against its live layout rectangle on
    /// release so a mid-press resize cannot fire an unhovered button.
    pressed_options_action: Option<OptionsAction>,
    /// Whether the Options-strip "Saves" (Load / Manage Saves) button is currently pressed.
    pressed_saves_button: bool,
    /// Last laid-out screen rect of the Options-strip "Saves" button (present only
    /// while `Mode::Options` is expanded).
    saves_button_rect: Option<Rect>,
    /// Whether the Options-strip "Co-op" multiplayer button is currently pressed.
    pressed_coop_button: bool,
    /// Last laid-out screen rect of the Options-strip "Co-op" button (present only
    /// while `Mode::Options` is expanded).
    coop_button_rect: Option<Rect>,
    /// Whether the Camera subpanel's Photo Album action (`CamViewAlbum`, `1105`)
    /// is enabled (true in `NeighborhoodShell` lots, false in standalone launches).
    album_button_enabled: bool,
    /// Whether the native Complete Collection Camera subpanel (`cWinCamera`)
    /// is currently toggled open inside the bottom panel strip (`x >= hud::CLUSTER[0]`).
    camera_strip_open: bool,
    /// Native `cWinCamera` interactive controller (`sims_ui::camera::CameraPanel`).
    camera_panel: CameraPanel,
    /// Snapshot size, JPEG quality, custom dimensions, album availability, and Quick Tips state.
    camera_state: CameraState,
    /// Localized coordinates, tooltips, and Quick Tips help balloons from `UIText.iff STR# 140`.
    camera_strings: CameraStrings,
    /// Last laid-out native `CameraLayout` while Camera Mode is open.
    camera_layout: Option<CameraLayout>,
    /// Last laid-out bounding rect of the Camera subpanel strip when open.
    camera_strip_rect: Option<Rect>,
    /// Whether the Roof camera toggle (`Camera::Roof`) is active.
    roof_view: bool,
    /// Classic Sims 1 top-left cheat console (`Ctrl+Shift+C` / `Cmd+Shift+C`).
    pub cheat_console: CheatConsole,
    /// The pie menu, while one is open.
    pub menu: Option<Menu>,
    /// The status band across the top, as last drawn.
    band: Option<Rect>,
    /// The selected Sim's actions, as last drawn.
    queue: Queue,
    /// The actions whose icons were pressed while they show.
    pressed: Pressed,
    /// 500 ms action-queue icon slide-in & slot-shift animator (`cWinActionQueueIcon`).
    queue_slide: QueueSlideState,
    /// Pending pie-menu screen click origin for newly queued icons `(person, click_pos, clock_us)`.
    pending_queue_origin: Option<(Option<u16>, [i32; 2], u64)>,
    /// 250 ms fade-in / fade-out animator for the help balloon (`cWinBalloon`,
    /// Live and Options alike).
    balloon_fade: BalloonFadeState,
    /// Most recently laid-out help balloon retained during 250 ms fade-out.
    last_balloon: Option<FadingBalloon>,
    /// The co-op player's own selection the panel was last laid out for
    /// (`None`: the world's), so a press checks against what is shown.
    drawn_selection: Option<Option<u16>>,
    /// Opacity fade-in state for native modal dialogs (`cTSWinFade`).
    dialog_fade: DialogFadeState,
    /// Last presentation clock timestamp in microseconds (`0` for static/headless frames).
    last_clock_us: u64,
    /// The live panel's need histories and trend arrows (`0x51ce79`,
    /// `0x513b41`, `0x50c2fa`), run on the presentation clock.
    needs_trend: NeedsTrend,
    /// Whether the last frame drew the need bars (the Needs panel shown).
    needs_trend_shown: bool,
    /// The clock controls' shared 500 ms blink timer (`0x53b4b0..0x53b7e7`):
    /// state 2 blinks Pause (row 0 <-> row 1) and state 3 blinks Ultra
    /// (`selected` false <-> true) during idle speed when return speed != Ultra.
    clock_blink: ClockBlink,
    /// The job panel's performance blink (`0x5306df`, `0x52c659`).
    performance_blink: PerformanceBlink,
    /// Native `[control_panel + 0x1ec]` idle-blink flag: set on `0x109(1)`
    /// (`0x46d7b5 -> 0x4f688e`), cleared on `0x109(0)` (`0x46d803`) or a
    /// Live-mode manual speed press (`0x53a5e3..0x53a6e1`), and preserved
    /// across scripted Special Effect `0x10c` speed resets
    /// (`0x4f6856..0x4f687a` -> `0x53b663..0x53b6ac`).
    idle_speed: bool,
    /// The Buy Mode residential catalog, loaded lazily on first Buy Mode entry.
    buy_catalog: Option<BuyCatalog>,
    /// Buy Mode popup rating labels (`GameData/UIText.iff STR# 160`, `0x508168`).
    rating_labels: RatingLabels,
    /// Buy Mode sell prompt templates (`GameData/UIText.iff STR# 136`, `0x50a343`).
    sell_labels: SellLabels,
    /// The Buy Mode category buttons' text (`GameData/UIText.iff` STR# 150,
    /// 200–207 and 210, 0x5057a2–0x5058cb), if `UIText.iff` has the tables.
    category_labels: Option<BuyLabels>,
    /// The Buy Mode catalog strip state (sort, category, subcategory, page,
    /// selected GUID).
    pub buy_state: BuyState,
    /// The Buy Mode strip widgets and popup card as last laid out.
    buy_layout: BuyLayout,
    /// Install handle retained for lazy Build Mode catalog loading.
    install: Install,
    /// The Build Mode catalog, loaded lazily on first Build Mode entry.
    build_catalog: Option<BuildCatalog>,
    /// Cached 45×45 surface swatch thumbnails for walls, floors, and wallpapers.
    build_swatches: BuildSwatches,
    /// The Build Mode panel state (active tool, page, selected item).
    pub build_state: BuildState,
    /// The Build Mode strip widgets and popup card as last laid out.
    build_layout: BuildLayout,
    /// Neighborhood families (`FAMI` / `FAMs`) cached for ordinary VM dialog
    /// `$Family` (`0x44c034`) and `$FamilyAssets:N` (`0x44bf66`) formatting.
    families: std::collections::BTreeMap<i32, dialog::FamilyInfo>,
    /// The interface's sounds, until the game takes them.
    sounds: Vec<SoundEvent>,
}

/// Install-only HUD assets (art, UI strings, options defaults) that depend
/// solely on [`Install`] and can be loaded in parallel with lot boot.
#[derive(Clone)]
pub(crate) struct HudStatic {
    art: Art,
    /// The UI archive, for a dialog's named icon outside `art` (`gz N`).
    ui: Option<UiGraphics>,
    labels: Labels,
    rating_labels: RatingLabels,
    sell_labels: SellLabels,
    category_labels: Option<BuyLabels>,
    texts: Texts,
    fame: Fame,
    options_texts: OptionsTexts,
    options_state: options::OptionsState,
    camera_strings: CameraStrings,
}

impl HudStatic {
    pub(crate) fn load(install: &Install, pool: Arc<ArchivePool>) -> Result<Self, String> {
        let (art_res, text_res) = std::thread::scope(|scope| {
            let text_handle = scope.spawn(|| -> Result<_, String> {
                let labels = Labels::load(install).map_err(|e| e.to_string())?;
                let rating_labels = RatingLabels::load(install);
                let sell_labels = SellLabels::load(install);
                // Without the category tables the Buy buttons have no text.
                let category_labels = BuyLabels::load(install, US_ENGLISH).ok();
                // Without the balloons', options', or camera strings the HUD still opens with defaults.
                let texts = Texts::load(install).unwrap_or_default();
                let fame = Fame::load(install).unwrap_or_default();
                let options_texts = OptionsTexts::load(install).unwrap_or_default();
                let options_state =
                    options::OptionsState::load_install(install).unwrap_or_default();
                let camera_strings = CameraStrings::load(install, US_ENGLISH).unwrap_or_default();
                Ok((
                    labels,
                    rating_labels,
                    sell_labels,
                    category_labels,
                    texts,
                    fame,
                    options_texts,
                    options_state,
                    camera_strings,
                ))
            });
            let art_res = (|| -> Result<(Art, UiGraphics), String> {
                let ui = UiGraphics::open_with_pool(install, pool).map_err(|e| e.to_string())?;
                let resources: Vec<i32> = hud::RESOURCES
                    .iter()
                    .chain(&options::RESOURCES)
                    .chain(&dialog::RESOURCES)
                    .chain(&pie::RESOURCES)
                    .chain(&sims_ui::queue::RESOURCES)
                    .chain(&thumbnail::RESOURCES)
                    .copied()
                    .collect();
                let optional: Vec<i32> = hud::OPTIONAL_RESOURCES
                    .iter()
                    .chain(&options::HELP_RESOURCES)
                    .chain(&camera::RESOURCES)
                    .chain(&camera::HELP_RESOURCES)
                    .chain(&[
                        options::id::SAVE_CARD,
                        sims_ui::neighborhood::art_id::POPUP_TILER,
                        sims_ui::album::CAM_VIEW_ALBUM,
                        sims_ui::tutorial::MARKER,
                    ])
                    .copied()
                    .chain(buy::resource_ids())
                    .chain(build::resource_ids())
                    .collect();
                Art::load_with_optional(&ui, US_ENGLISH, &resources, &optional)
                    .map(|art| (art, ui))
                    .map_err(|e| e.to_string())
            })();
            (
                art_res,
                text_handle.join().expect("hud text worker panicked"),
            )
        });
        let (art, ui) = art_res?;
        let (
            labels,
            rating_labels,
            sell_labels,
            category_labels,
            texts,
            fame,
            options_texts,
            options_state,
            camera_strings,
        ) = text_res?;
        Ok(Self {
            art,
            ui: Some(ui),
            labels,
            rating_labels,
            sell_labels,
            category_labels,
            texts,
            fame,
            options_texts,
            options_state,
            camera_strings,
        })
    }

    /// Clean-room procedural HUD chrome & defaults for standalone `open-pack/`
    /// mode (zero EA/Maxis `UIGraphics.far` or `UIText.iff` assets required).
    pub(crate) fn clean_room_procedural() -> Self {
        Self {
            art: Art::clean_room_procedural(),
            ui: None,
            labels: Labels::default(),
            rating_labels: RatingLabels::default(),
            sell_labels: SellLabels::default(),
            category_labels: None,
            texts: Texts::default(),
            fame: Fame::default(),
            options_texts: OptionsTexts::default(),
            options_state: options::OptionsState::default(),
            camera_strings: CameraStrings::default(),
        }
    }
}

/// Index/Catalog-dependent HUD tables (`work.iff` careers and house rating
/// tuning) that can be loaded in parallel with VM linking and animations.
#[derive(Clone)]
pub(crate) struct HudShared {
    careers: Careers,
    house: house::Tuning,
}

impl HudShared {
    pub(crate) fn load(
        index: &Index,
        catalog: &Catalog,
        pool: &ArchivePool,
    ) -> Result<Self, String> {
        let (careers_res, house_res) = std::thread::scope(|scope| {
            let house_handle = scope.spawn(|| {
                house::Tuning::load_with_pool(index, catalog, pool).map_err(|e| e.to_string())
            });
            let careers_res = Careers::load_with_pool(index, pool).map_err(|e| e.to_string());
            (
                careers_res,
                house_handle.join().expect("hud tuning worker panicked"),
            )
        });
        Ok(Self {
            careers: careers_res?,
            house: house_res?,
        })
    }
}

impl Hud {
    /// Reads the art, and the household of `house` from its neighborhood
    /// with the portraits from the people's files. `world` is the booted
    /// lot's: the faces of everyone its neighborhood records hold are
    /// read, for the relationship panel. `friendship` is the autonomy
    /// tuning's friendship threshold.
    pub fn open(
        install: &Path,
        house: &Path,
        assets: &Assets,
        world: &World,
        friendship: i32,
    ) -> Result<Self, String> {
        let neighborhood = house
            .parent()
            .and_then(Path::parent)
            .ok_or("the house file is not in a Houses folder")?;
        let neighborhood = Neighborhood::open(neighborhood).map_err(|e| e.to_string())?;
        let number = neighborhood
            .houses()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|(_, path)| path.file_name() == house.file_name())
            .map(|(n, _)| n);
        let members = number
            .and_then(|n| neighborhood.family_in(n))
            .map(|(_, f)| f.members.clone())
            .unwrap_or_default();
        let mut hud = Self::open_with_members(install, members, assets, world, friendship)?;
        hud.families = load_neighborhood_families(&neighborhood);
        Ok(hud)
    }

    /// Reads the art and faces for a lot whose occupying household members
    /// come from a `NeighborhoodStore` rather than a `Houses/HouseXX.iff` path.
    pub fn open_with_members(
        install: &Path,
        members: Vec<u32>,
        assets: &Assets,
        world: &World,
        friendship: i32,
    ) -> Result<Self, String> {
        let install = Install::detect(install).map_err(|e| e.to_string())?;
        let pool = Arc::new(ArchivePool::new());
        let static_part = HudStatic::load(&install, Arc::clone(&pool))?;
        // Load Careers + Tuning in parallel with Faces while deferring their
        // error checks until after `Faces::load`.
        let (shared_res, faces_res) = std::thread::scope(|scope| {
            let shared_handle =
                scope.spawn(|| HudShared::load(assets.index(), assets.catalog(), &pool));
            let faces_res = {
                let mut people = members.clone();
                for record in world.neighbors.slots.iter().flatten() {
                    if !people.contains(&record.guid) {
                        people.push(record.guid);
                    }
                }
                Faces::load_with_pool(assets.index(), assets.catalog(), &people, &pool)
                    .map_err(|e| e.to_string())
            };
            (
                shared_handle.join().expect("hud shared worker panicked"),
                faces_res,
            )
        });
        let faces = faces_res?;
        let shared_part = shared_res?;
        Ok(Self::assemble(
            &install,
            assets,
            friendship,
            static_part,
            shared_part,
            members,
            faces,
        ))
    }

    /// Builds a [`Hud`] from preloaded [`HudStatic`] and [`HudShared`] tables
    /// plus the shared [`Install`], [`Neighborhood`], and [`ArchivePool`],
    /// preserving original error precedence (`houses` -> `Faces` -> `HudShared`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_preloaded(
        install: &Install,
        house: &Path,
        neighborhood: &Neighborhood,
        assets: &Assets,
        world: &World,
        friendship: i32,
        static_part: HudStatic,
        shared_part: Result<HudShared, String>,
        pool: &ArchivePool,
    ) -> Result<Self, String> {
        let number = neighborhood
            .houses()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|(_, path)| path.file_name() == house.file_name())
            .map(|(n, _)| n);
        let members = number
            .and_then(|n| neighborhood.family_in(n))
            .map(|(_, f)| f.members.clone())
            .unwrap_or_default();
        let mut hud = Self::from_preloaded_with_members(
            install,
            members,
            assets,
            world,
            friendship,
            static_part,
            shared_part,
            pool,
        )?;
        hud.families = load_neighborhood_families(neighborhood);
        Ok(hud)
    }

    /// Builds a [`Hud`] from preloaded [`HudStatic`] and [`HudShared`] tables
    /// for a lot whose occupying household members are already known (e.g. from
    /// a REDB `NeighborhoodStore` snapshot).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_preloaded_with_members(
        install: &Install,
        members: Vec<u32>,
        assets: &Assets,
        world: &World,
        friendship: i32,
        static_part: HudStatic,
        shared_part: Result<HudShared, String>,
        pool: &ArchivePool,
    ) -> Result<Self, String> {
        let mut people = members.clone();
        for record in world.neighbors.slots.iter().flatten() {
            if !people.contains(&record.guid) {
                people.push(record.guid);
            }
        }
        let faces = Faces::load_with_pool(assets.index(), assets.catalog(), &people, pool)
            .map_err(|e| e.to_string())?;
        let shared_part = shared_part?;
        Ok(Self::assemble(
            install,
            assets,
            friendship,
            static_part,
            shared_part,
            members,
            faces,
        ))
    }

    /// Builds a clean-room standalone [`Hud`] backed by
    /// [`Art::clean_room_procedural`] and procedural starter Sim portraits with
    /// zero commercial EA/Maxis UI assets.
    pub(crate) fn open_open_pack(
        install: &Install,
        members: Vec<u32>,
        starter_sims: &[sims_antics::house::StarterSimSpec],
        assets: &Assets,
        world: &World,
        friendship: i32,
        pool: &ArchivePool,
    ) -> Result<Self, String> {
        let mut people = members.clone();
        for record in world.neighbors.slots.iter().flatten() {
            if !people.contains(&record.guid) {
                people.push(record.guid);
            }
        }
        let mut faces = Faces::load_with_pool(assets.index(), assets.catalog(), &people, pool)
            .unwrap_or_default();
        for (idx, sim) in starter_sims.iter().enumerate() {
            if faces.name(sim.guid).is_none() {
                faces.insert_name(sim.guid, &sim.name);
            }
            if faces.face(sim.guid).is_none() {
                faces.insert(sim.guid, crate::open_pack::procedural_sim_portrait(idx));
            }
        }
        let shared_part = HudShared {
            careers: Careers::default(),
            house: house::Tuning::default(),
        };
        Ok(Self::assemble(
            install,
            assets,
            friendship,
            HudStatic::clean_room_procedural(),
            shared_part,
            members,
            faces,
        ))
    }

    fn assemble(
        install: &Install,
        assets: &Assets,
        friendship: i32,
        static_part: HudStatic,
        shared_part: HudShared,
        members: Vec<u32>,
        faces: Faces,
    ) -> Self {
        let known = Acquaintances::read(
            assets.index(),
            assets.catalog(),
            install.expansions,
            friendship,
        );
        let thumbnails = Thumbnails::with_template(&static_part.art);
        Self {
            art: static_part.art,
            ui: static_part.ui,
            boot_faces: faces.clone(),
            faces,
            thumbnails,
            problems: Vec::new(),
            careers: shared_part.careers,
            fame: static_part.fame,
            house: shared_part.house,
            kinds: house::Kinds::default(),
            known,
            labels: static_part.labels,
            texts: static_part.texts,
            options_texts: static_part.options_texts,
            members,
            // The strip opens as if Live had been pressed, with preferences
            // loaded from `UserData/Options.iff` (`Optn` 128, `0x4e6505`).
            controls: Controls {
                expanded: true,
                options: static_part.options_state,
                ..Controls::default()
            },
            items: Vec::new(),
            message: None,
            ui_dialog: None,
            dialog_state: DialogState::default(),
            non_modal_press: false,
            shift_held: false,
            auto_answer: AutoAnswer::default(),
            auto_answer_status: None,
            non_modal_read: Vec::new(),
            dialog: None,
            dragging_slider: None,
            pressed_options_action: None,
            pressed_saves_button: false,
            saves_button_rect: None,
            pressed_coop_button: false,
            coop_button_rect: None,
            album_button_enabled: false,
            camera_strip_open: false,
            camera_panel: CameraPanel::new(),
            camera_state: CameraState {
                album_enabled: false,
                ..CameraState::default()
            },
            camera_strings: static_part.camera_strings,
            camera_layout: None,
            camera_strip_rect: None,
            roof_view: false,
            cheat_console: CheatConsole::new(),
            menu: None,
            band: None,
            queue: Queue::default(),
            pressed: Pressed::default(),
            queue_slide: QueueSlideState::new(),
            pending_queue_origin: None,
            balloon_fade: BalloonFadeState::new(),
            last_balloon: None,
            drawn_selection: None,
            dialog_fade: DialogFadeState::hidden(),
            last_clock_us: 0,
            needs_trend: NeedsTrend::default(),
            needs_trend_shown: false,
            clock_blink: ClockBlink::default(),
            performance_blink: PerformanceBlink::default(),
            idle_speed: false,
            buy_catalog: None,
            rating_labels: static_part.rating_labels,
            sell_labels: static_part.sell_labels,
            category_labels: static_part.category_labels,
            buy_state: BuyState::default(),
            buy_layout: BuyLayout::default(),
            install: install.clone(),
            build_catalog: None,
            build_swatches: BuildSwatches::default(),
            build_state: BuildState {
                last_wall_style: 1,
                ..BuildState::default()
            },
            build_layout: BuildLayout::default(),
            families: std::collections::BTreeMap::new(),
            sounds: Vec::new(),
        }
    }

    /// Replaces the cached neighborhood family table used for ordinary VM dialog
    /// `$Family` / `$FamilyAssets:N` formatting and Spellbook/Cookbook titles.
    pub fn set_families(&mut self, families: std::collections::BTreeMap<i32, dialog::FamilyInfo>) {
        self.families = families;
    }

    /// Returns the household's localized family surname (`FAMs`) for `family_id`,
    /// if present and non-empty.
    pub fn family_surname(&self, family_id: i32) -> Option<&str> {
        self.families
            .get(&family_id)
            .map(|f| f.name.trim())
            .filter(|s| !s.is_empty())
    }

    /// Forgets what the panel kept of the lot it showed, for a lot put
    /// back to a save: the pie menu, the dialog read, the icons pressed,
    /// the Buy/Build Mode catalog state, and the pause button's blink timer. The
    /// rest is read from the lot at the next frame.
    pub fn forget(&mut self) {
        self.menu = None;
        self.message = None;
        self.disarm_auto_answer();
        self.non_modal_read.clear();
        self.ui_dialog = None;
        self.dialog_state = DialogState::default();
        self.dialog = None;
        self.dragging_slider = None;
        self.pressed_options_action = None;
        self.pressed_saves_button = false;
        self.saves_button_rect = None;
        self.pressed_coop_button = false;
        self.coop_button_rect = None;
        self.camera_strip_open = false;
        self.camera_panel.clear_pointer();
        self.camera_layout = None;
        self.camera_strip_rect = None;
        self.roof_view = false;
        self.controls.pressed_options_action = None;
        self.controls.help = None;
        self.controls.options.tab = None;
        self.controls.options.help = None;
        self.items.clear();
        self.pressed = Pressed::default();
        self.queue_slide = QueueSlideState::new();
        self.pending_queue_origin = None;
        self.balloon_fade = BalloonFadeState::new();
        self.last_balloon = None;
        self.dialog_fade = DialogFadeState::hidden();
        self.clock_blink.reset();
        self.performance_blink = PerformanceBlink::default();
        self.controls.performance_lit = false;
        self.idle_speed = false;
        self.controls.pause_lit = false;
        self.controls.ultra_lit = false;
        self.buy_state.reset();
        self.buy_layout = BuyLayout::default();
        self.build_state.reset();
        self.build_layout = BuildLayout::default();
        // A restored or newly entered lot gets a new live panel, so its
        // need histories and bars start empty.
        self.needs_trend = NeedsTrend::default();
        self.needs_trend_shown = false;
        self.controls.trends = [None; 8];
    }

    /// Loads the residential Buy Mode catalog on first use and returns it.
    pub fn ensure_buy_catalog(&mut self, assets: &Assets) -> &BuyCatalog {
        self.buy_catalog.get_or_insert_with(|| {
            load_buy_catalog(
                assets,
                &self.rating_labels,
                &self.sell_labels,
                self.category_labels.as_ref(),
            )
        })
    }

    /// The residential Buy Mode catalog, once loaded.
    pub fn buy_catalog(&self) -> Option<&BuyCatalog> {
        self.buy_catalog.as_ref()
    }

    /// The Buy Mode catalog strip layout as last built.
    pub fn buy_layout(&self) -> &BuyLayout {
        &self.buy_layout
    }

    /// Whether `point` lies over the expanded Buy Mode catalog / sell strip
    /// (`cWinCatalog`, `x >= 240` and `y >= height - STRIP`), where clicking or
    /// dropping a held object sells it (`0x50d04d` → `0x4cf97a` → `0x487e18`).
    /// Uses the 100 px strip height (`hud::STRIP`), NOT the 150 px left-cluster
    /// container (`hud::TALL_STRIP`), so releasing in the house viewport above
    /// `y = height - 100` never triggers a sale.
    pub fn over_buy_catalog(&self, size: [i32; 2], point: [i32; 2]) -> bool {
        if self.controls.mode != Mode::Buy || !self.controls.expanded {
            return false;
        }
        if let Some(prompt) = &self.buy_layout.sell_prompt {
            return prompt.rect.contains(point[0], point[1]);
        }
        let top = size[1] - hud::STRIP;
        point[0] >= 240 && point[0] < size[0] && point[1] >= top && point[1] < size[1]
    }

    /// Lays out the Buy Mode catalog strip (or active Hand Tool sell prompt)
    /// for `size` and `world` (or clears it outside expanded Buy Mode),
    /// automatically selecting the lot's residential or expansion commercial
    /// catalog zone (`0x4cb8e7..0x4cbaa8`) and syncing `self.buy_state.page`.
    pub fn layout_buy(
        &mut self,
        size: [i32; 2],
        world: &World,
        assets: &Assets,
        sell: Option<&SellInfo>,
    ) {
        self.layout_buy_for_lot(size, world, None, assets, sell);
    }

    /// Like [`Hud::layout_buy`], using the lot's neighborhood zoning (`Some(1)`
    /// residential, `Some(2)` community) to distinguish Old Town community lots
    /// (`BuyZone::Community`) from residential lots (`BuyZone::Residential`).
    pub fn layout_buy_for_lot(
        &mut self,
        size: [i32; 2],
        world: &World,
        lot_zoning: Option<i32>,
        assets: &Assets,
        sell: Option<&SellInfo>,
    ) {
        if self.controls.mode != Mode::Buy || !self.controls.expanded {
            self.buy_layout = BuyLayout::default();
            return;
        }
        let lot_id = world.globals.get(10).copied().unwrap_or(0);
        let zone = sims_ui::buy::BuyZone::for_lot(lot_id, lot_zoning);
        self.buy_state.set_zone(zone);
        let catalog = self.buy_catalog.get_or_insert_with(|| {
            load_buy_catalog(
                assets,
                &self.rating_labels,
                &self.sell_labels,
                self.category_labels.as_ref(),
            )
        });
        let funds = sims_vm::available_funds(world);
        let layout = buy::layout_with_sell(size, catalog, &self.buy_state, &self.art, funds, sell);
        self.buy_state.page = layout.page;
        self.buy_layout = layout;
    }

    /// Clears the cached Buy Mode strip layout (e.g. when leaving Buy Mode).
    pub fn clear_buy_layout(&mut self) {
        self.buy_layout = BuyLayout::default();
    }

    /// Loads the Build Mode catalog on first use and returns it.
    pub fn ensure_build_catalog(&mut self, assets: &Assets) -> &BuildCatalog {
        let install = &self.install;
        self.build_catalog.get_or_insert_with(|| {
            let builtin_floors = assets.builtin_floor_ids();
            let builtin_walls = assets.builtin_wall_ids();
            BuildCatalog::load(
                install,
                assets.index(),
                assets.catalog(),
                &builtin_floors,
                &builtin_walls,
                assets.floor_plugins(),
                assets.wall_plugins(),
            )
        })
    }

    /// Returns Architecture pricing tables for `sims_vm` Build Mode edits.
    pub fn build_prices(&mut self, assets: &Assets) -> sims_vm::BuildPrices {
        let catalog = self.ensure_build_catalog(assets);
        sims_vm::BuildPrices {
            styles: catalog.style_prices.clone(),
            floors: catalog.floor_prices.clone(),
            walls: catalog.wall_prices.clone(),
        }
    }

    /// Returns Terrain / Water / Pool pricing tables for `sims_vm` Build Mode Part 2 edits.
    pub fn tool_prices(&mut self, assets: &Assets) -> sims_vm::ToolPrices {
        let catalog = self.ensure_build_catalog(assets);
        sims_vm::ToolPrices {
            terrain_raise: catalog.terrain_price(TerrainSubtool::Raise),
            terrain_lower: catalog.terrain_price(TerrainSubtool::Lower),
            terrain_level: catalog.terrain_price(TerrainSubtool::Level),
            grass: catalog.terrain_price(TerrainSubtool::Grass),
            pool: catalog.pool_price(),
            water: catalog.water_price(),
        }
    }

    /// The Build Mode catalog, once loaded.
    pub fn build_catalog(&self) -> Option<&BuildCatalog> {
        self.build_catalog.as_ref()
    }

    /// Install handle owned by the HUD (used for lazy `RoofCatalog` and shop strings).
    pub fn install(&self) -> &Install {
        &self.install
    }

    /// Hit-tests Build Mode Part 2 subtools (Terrain / Water / Roof pitch &
    /// pattern slots and narrow-viewport tool pagination).
    pub fn press_build_subtool(&self, point: [i32; 2]) -> Option<BuildSubtoolAction> {
        if self.controls.mode != Mode::Build || !self.controls.expanded {
            return None;
        }
        self.build_layout.press_subtool(point)
    }

    /// Enables or disables the Camera subpanel's Photo Album action (`CamViewAlbum`, `1105`).
    pub fn set_album_button_enabled(&mut self, enabled: bool) {
        self.album_button_enabled = enabled;
        self.camera_state.album_enabled = enabled;
        if !enabled {
            self.camera_panel.clear_pointer();
        }
    }

    /// Current native Camera Mode settings (`SnapshotSize`, `SnapshotQuality`,
    /// `album_enabled`, `quick_tips`).
    pub fn camera_state(&self) -> CameraState {
        let mut state = self.camera_state;
        state.album_enabled = self.album_button_enabled;
        state.quick_tips = self.controls.options.play.quick_tips;
        state
    }

    /// Mutable access to the Camera Mode settings (for tests or Quick Tips overrides).
    pub fn camera_state_mut(&mut self) -> &mut CameraState {
        &mut self.camera_state
    }

    /// Active native `cWinCamera` panel controller.
    pub fn camera_panel(&self) -> &CameraPanel {
        &self.camera_panel
    }

    /// Loaded `STR# 140` Camera Mode strings (button coordinates, tooltips, Quick Tips).
    pub fn camera_strings(&self) -> &CameraStrings {
        &self.camera_strings
    }

    /// Last laid-out native [`CameraLayout`] when Camera Mode is open.
    pub fn camera_layout(&self) -> Option<&CameraLayout> {
        self.camera_layout.as_ref()
    }

    /// Screen rectangle of a specific native `cWinCamera` button when Camera Mode is open.
    pub fn camera_button_rect(&self, button: CameraButton) -> Option<Rect> {
        self.camera_layout
            .as_ref()
            .and_then(|layout| layout.button(button))
            .map(|placement| placement.rect)
    }

    /// Whether the bottom native Camera subpanel (`cWinCamera`) is currently open.
    pub fn camera_strip_open(&self) -> bool {
        self.camera_strip_open
    }

    /// Toggles the bottom native Camera subpanel open or closed, returning the new state.
    pub fn toggle_camera_strip(&mut self, size: [i32; 2]) -> bool {
        self.camera_strip_open = !self.camera_strip_open;
        if self.camera_strip_open {
            self.layout_camera_strip(size);
            if self.camera_strip_open {
                // Native `0x4d1734..0x4d1746` / `0x53a543`: opening Camera Mode
                // expands the control panel strip so `BackPatch.bmp` (`2036`)
                // fills the curved cluster wedge next to the Camera/Options
                // buttons. Only expand once layout accepts the viewport size.
                self.controls.expanded = true;
                self.dismiss_help();
            }
        } else {
            self.clear_camera_strip_layout();
        }
        self.camera_strip_open
    }

    /// Closes the bottom Camera subpanel if open. Returns `true` if it was open.
    pub fn close_camera_strip(&mut self) -> bool {
        if !self.camera_strip_open {
            return false;
        }
        self.camera_strip_open = false;
        self.clear_camera_strip_layout();
        true
    }

    fn clear_camera_strip_layout(&mut self) {
        self.camera_panel.clear_pointer();
        self.camera_layout = None;
        self.camera_strip_rect = None;
    }

    /// Bounding rectangle of the Camera subpanel inside the bottom panel strip
    /// area, when open.
    pub fn camera_strip_rect(&self) -> Option<Rect> {
        self.camera_strip_rect
    }

    /// Screen rectangle of the native `CamViewAlbum` (`1105`) button, when the
    /// Camera subpanel is open and Photo Album is enabled.
    pub fn album_button_rect(&self) -> Option<Rect> {
        if !self.album_button_enabled {
            return None;
        }
        self.camera_button_rect(CameraButton::ViewAlbum)
    }

    /// Releases a held native `cWinCamera` button, returning
    /// `(changed, Option<CameraCommand>)`.
    ///
    /// Size and quality toggles update `self.camera_state` in place while
    /// keeping the Camera subpanel open; `OpenPhotoAlbum` closes the
    /// subpanel before returning. Photos are taken by clicking the lot.
    pub fn release_camera_strip(
        &mut self,
        point: Option<[i32; 2]>,
    ) -> (bool, Option<CameraCommand>) {
        if self.camera_panel.pressed().is_none() {
            return (false, None);
        }
        let Some([x, y]) = point else {
            self.camera_panel.clear_pointer();
            return (true, None);
        };
        let Some(strip) = self.camera_strip_rect else {
            self.camera_panel.clear_pointer();
            return (true, None);
        };
        // Keep Quick Tips hover balloons active in `draw()`/`hover()`, while
        // allowing clicks on size/quality controls to apply the chosen setting
        // even when `Options.iff` enables `play.quick_tips` by default.
        let mut release_state = self.camera_state();
        release_state.quick_tips = false;
        let cmd =
            self.camera_panel
                .release(x, y, strip, &release_state, &self.camera_strings, &self.art);
        match cmd {
            Some(CameraCommand::SetSize(size)) => {
                self.camera_state.size = size;
                self.play(CLICK);
                (true, Some(CameraCommand::SetSize(size)))
            }
            Some(CameraCommand::SetQuality(quality)) => {
                self.camera_state.quality = quality;
                self.play(CLICK);
                (true, Some(CameraCommand::SetQuality(quality)))
            }
            Some(CameraCommand::OpenPhotoAlbum) => {
                self.play(CLICK);
                self.close_camera_strip();
                (true, Some(CameraCommand::OpenPhotoAlbum))
            }
            Some(CameraCommand::Help(topic)) => {
                self.play(WHOOSH);
                (true, Some(CameraCommand::Help(topic)))
            }
            None => (true, None),
        }
    }

    /// Updates the HUD's cached presentation clock in microseconds (`0` for
    /// static/headless deterministic frames).
    pub fn set_clock_us(&mut self, clock_us: u64) {
        self.last_clock_us = clock_us;
    }

    /// Records the screen click origin of a pie-menu interaction choice so the
    /// newly queued action icon slides smoothly into its slot over 500 ms.
    pub fn set_queue_origin(&mut self, person: Option<u16>, origin: [i32; 2]) {
        self.pending_queue_origin = Some((person, origin, self.last_clock_us));
    }

    /// Whether any temporal UI animation (250 ms help-balloon fade, modal
    /// dialog fade-in, 500 ms action-queue icon slide, or a shown need bar's
    /// 500 ms trend-arrow fade) is actively in flight
    /// at `clock_us`. Always `false` when `clock_us == 0`.
    pub fn is_ui_animating(&self, clock_us: u64) -> bool {
        if clock_us == 0 {
            return false;
        }
        let now_ms = (clock_us / 1_000) as u32;
        self.balloon_fade.is_animating(now_ms)
            || self.dialog_fade.is_animating(now_ms)
            || self.queue_slide.is_animating(now_ms)
            || (self.needs_trend_shown && self.needs_trend.animating(now_ms))
    }

    /// One frame of the live panel's need sampling and its bars' draw
    /// (`0x4ce390` → `0x51ce79`, then `0x50c2fa`), at the presentation
    /// clock: the arrows the layout shows go to `controls.trends`. Static
    /// frames (`clock_us == 0`) neither sample nor show arrows.
    fn step_needs_trend(&mut self, world: &World, household: &Snapshot, clock_us: u64) {
        self.controls.trends = [None; 8];
        self.needs_trend_shown = false;
        if clock_us == 0 {
            return;
        }
        let now = (clock_us / 1_000) as u32;
        let members: Vec<[f32; 16]> = household.residents.iter().map(|r| r.motives).collect();
        let shown = household.selected_resident();
        let member =
            shown.and_then(|s| household.residents.iter().position(|r| std::ptr::eq(r, s)));
        let live = self.controls.mode == Mode::Live;
        let open = self.controls.panel == Some(UiPanel::Needs);
        self.needs_trend.update(&PanelFrame {
            now,
            speed: world.speed(),
            live,
            open,
            members: &members,
            selected: household.selected,
            member,
            motives: hud::NEEDS.map(|(_, motive)| motive),
        });
        // The bars draw only while the Needs panel shows (the layout's
        // condition in `live_strip`).
        if live && open && self.controls.expanded && shown.is_some() {
            self.controls.trends = self.needs_trend.draw(now);
            self.needs_trend_shown = true;
        }
    }

    /// Whether the live panel's next need sample is due at `clock_us`, so
    /// a frame should be drawn to take it.
    pub fn needs_sample_due(&self, clock_us: u64) -> bool {
        clock_us > 0
            && self.controls.mode == Mode::Live
            && self.needs_trend.due_in((clock_us / 1_000) as u32) == Some(0)
    }

    /// Current Live help-balloon fade state (`BalloonFadeState`).
    pub fn balloon_fade(&self) -> &BalloonFadeState {
        &self.balloon_fade
    }

    /// Current modal dialog fade state (`DialogFadeState`).
    pub fn dialog_fade(&self) -> &DialogFadeState {
        &self.dialog_fade
    }

    /// Current action-queue icon slide state (`QueueSlideState`).
    pub fn queue_slide(&self) -> &QueueSlideState {
        &self.queue_slide
    }

    fn start_dialog_fade(&mut self) {
        if self.last_clock_us > 0 {
            let now_ms = (self.last_clock_us / 1_000) as u32;
            self.dialog_fade = DialogFadeState::hidden();
            self.dialog_fade
                .fade_to(1.0, now_ms, DIALOG_FADE_DURATION_MS);
        } else {
            self.dialog_fade = DialogFadeState::opaque();
        }
    }

    /// Updates the highlighted state of the Roof camera toggle (`Camera::Roof`).
    pub fn set_roof_view(&mut self, on: bool) {
        self.roof_view = on;
    }

    /// The Build Mode catalog strip layout as last built.
    pub fn build_layout(&self) -> &BuildLayout {
        &self.build_layout
    }

    /// Lays out the Build Mode strip (`cWinArch`) for `size` and `world`,
    /// syncing `self.build_state.page` and `tool_page` to the clamped page numbers.
    pub fn layout_build(
        &mut self,
        size: [i32; 2],
        world: &World,
        assets: &Assets,
        can_undo: bool,
        can_redo: bool,
    ) {
        if self.controls.mode != Mode::Build || !self.controls.expanded {
            self.build_layout = BuildLayout::default();
            return;
        }
        let install = &self.install;
        let catalog = self.build_catalog.get_or_insert_with(|| {
            let builtin_floors = assets.builtin_floor_ids();
            let builtin_walls = assets.builtin_wall_ids();
            BuildCatalog::load(
                install,
                assets.index(),
                assets.catalog(),
                &builtin_floors,
                &builtin_walls,
                assets.floor_plugins(),
                assets.wall_plugins(),
            )
        });
        let funds = sims_vm::available_funds(world);
        let layout = build::layout(
            size,
            catalog,
            &self.build_state,
            &self.art,
            funds,
            can_undo,
            can_redo,
            self.controls.pointer,
        );
        self.build_state.page = layout.page;
        self.build_state.tool_page = layout.tool_page;
        self.build_layout = layout;
    }

    /// Decodes and caches 45×45 swatch thumbnails for any visible Build Mode
    /// floor, wallpaper, or wall/fence slots in `self.build_layout`.
    pub fn ensure_build_swatches(&mut self, assets: &mut Assets) {
        if self.controls.mode != Mode::Build || !self.controls.expanded {
            return;
        }
        let mut needed: Vec<BuildItemId> = self
            .build_layout
            .widgets
            .iter()
            .filter_map(|w| match w.kind {
                BuildWidgetKind::Slot { item, .. } => Some(item),
                BuildWidgetKind::Button { .. } => None,
            })
            .collect();
        if let Some(card) = &self.build_layout.popup {
            needed.push(card.item);
        }
        for item in needed {
            if self.build_swatches.contains(item) {
                continue;
            }
            let image = match item {
                BuildItemId::Floor(key) => {
                    if let Some(img) = assets.floor(key, 0) {
                        Some(render_image_to_swatch(img))
                    } else {
                        assets.floor(key, 1).map(render_image_to_swatch)
                    }
                }
                BuildItemId::Wallpaper(key) => [2, 1, 0].into_iter().find_map(|zoom| {
                    assets
                        .pattern(key, zoom)
                        .map(|frames| render_wallpaper_swatch(&frames[2]))
                }),
                BuildItemId::WallStyle(style_id) => {
                    let pat = sims_vm::WallStyle::from_style_id(style_id)
                        .map(sims_vm::WallStyle::default_pattern)
                        .unwrap_or(0);
                    if let Some(frames) = assets.pattern(pat, 0) {
                        let pat_img = frames[0].clone();
                        if let Some(shapes) = assets.shape(style_id, false, 0) {
                            Some(render_wall_swatch(&pat_img, &shapes[0]))
                        } else {
                            Some(render_image_to_swatch(&pat_img))
                        }
                    } else {
                        None
                    }
                }
                BuildItemId::Object(_) => None,
            };
            if let Some(img) = image {
                self.build_swatches.insert(item, img);
            }
        }
    }

    /// Re-lays out the control panel items and Buy/Build strip for `size`
    /// after a mode or panel toggle, so consecutive clicks work even before
    /// the next [`Hud::draw`].
    #[allow(clippy::too_many_arguments)]
    pub fn relayout_panel(
        &mut self,
        size: [i32; 2],
        world: &World,
        residential: bool,
        lot_zoning: Option<i32>,
        visibility: options::LotVisibility,
        assets: &Assets,
        sell: Option<&SellInfo>,
    ) {
        let snapshot = self.snapshot(world, residential);
        self.items = hud::layout_with_options_lot(
            size,
            &snapshot,
            &self.controls,
            &self.options_texts,
            visibility,
            Some(&self.art),
        );
        hud::fit(&mut self.items, &self.art, &self.texts);
        self.sync_options_extra_button_rects(size);
        self.layout_buy_for_lot(size, world, lot_zoning, assets, sell);
        self.layout_build(size, world, assets, false, false);
        self.layout_camera_strip(size);
    }

    /// Opens a UI-owned Options confirmation dialog using the install's
    /// `UIText.iff STR# 153` strings and `STR# 142` OK/Yes/No/Cancel labels
    /// (`cWinOptions::OnCommand`, `0x516809..0x516883`, `0x5170f0..0x517136`,
    /// `0x517c03 -> 0x50d68a`, `0x4f290d`, `0x4f1a11`, `0x4f25b4`).
    ///
    /// - Edit-mode save cards (`0x517c21`: `[lot + 0x50] == 1`) build the
    ///   dedicated `cWinEditLotInfo` layout (`PopupInfoTiles.bmp` `3008` +
    ///   `SAVE_CARD` `4994` + editable Lot Name & Lot Description fields).
    /// - Ordinary confirmations (`0x4f290d`, `0x4f1a11`, `0x4f25b4`) use
    ///   `cTSWinGenDlg` (`FrameStyle::Generic`, slot 5 → `PopupInfoTiles.bmp`
    ///   `3008` at `0x5e5441`, `396×135` for 3-button Neighborhood Save prompts).
    ///
    /// Immediately lays out the dialog against `view_size` so pointer hit
    /// testing and modal blocking apply before the next frame draws.
    pub fn open_options_dialog(
        &mut self,
        kind: OptionsDialog,
        view_size: [i32; 2],
        lot_name: &str,
        lot_description: &str,
    ) {
        let ok = self.labels.get(1).to_owned();
        let yes = self.labels.get(3).to_owned();
        let no = self.labels.get(4).to_owned();
        let cancel = self.labels.get(2).to_owned();
        let save_card = self.art.image(options::id::SAVE_CARD).cloned();
        let message = match kind {
            OptionsDialog::SaveCardSave => {
                Message::save_card(lot_name, lot_description, None, vec![ok], save_card)
            }
            OptionsDialog::SaveCardQuit
            | OptionsDialog::SaveCardNeighborhood
            | OptionsDialog::SaveCardResetTutorial => {
                // `0x50dcc8..0x50dcec`: 3-button Save Card loads `STR# 153`
                // 1-based entry 5 (`confirm_save.1`) and draws it centered
                // above the buttons (`0x50dfb2..0x50e056`).
                Message::save_card(
                    lot_name,
                    lot_description,
                    Some(self.options_texts.confirm_save.1.clone()),
                    vec![yes, no, cancel],
                    save_card,
                )
            }
            OptionsDialog::ConfirmQuit => Message::confirm(
                self.options_texts.confirm_quit.0.clone(),
                self.options_texts.confirm_quit.1.clone(),
                vec![yes, no],
            )
            .with_frame(dialog::FrameStyle::Generic),
            OptionsDialog::ConfirmSaveBeforeQuit => Message::confirm(
                self.options_texts.confirm_save_before_quit.0.clone(),
                self.options_texts.confirm_save_before_quit.1.clone(),
                vec![yes, no, cancel],
            )
            .with_frame(dialog::FrameStyle::Generic),
            OptionsDialog::ConfirmSaveBeforeNeighborhood
            | OptionsDialog::ConfirmSaveBeforeResetTutorial => Message::confirm(
                self.options_texts.confirm_save.0.clone(),
                self.options_texts.confirm_save.1.clone(),
                vec![yes, no, cancel],
            )
            .with_frame(dialog::FrameStyle::Generic),
            OptionsDialog::ConfirmResetTutorial => Message::confirm(
                self.options_texts.confirm_reset_tutorial.0.clone(),
                self.options_texts.confirm_reset_tutorial.1.clone(),
                vec![yes, no],
            )
            .with_frame(dialog::FrameStyle::Generic),
            OptionsDialog::Notice | OptionsDialog::SpecialBook => {
                Message::notice("", "", ok).with_frame(dialog::FrameStyle::Generic)
            }
        };
        self.dialog_state = DialogState::for_message(&message);
        self.dialog = dialog::layout(view_size, &message, &self.art);
        self.ui_dialog = Some((kind, message));
        self.start_dialog_fade();
        self.menu = None;
        self.close_camera_strip();
    }

    /// Opens a native Sims 1 modal notice dialog (`cTSWinMsgBox`, OK button)
    /// for genuine user-facing errors or confirmations (e.g., save error,
    /// travel unavailable, co-op error, or strict lot halt).
    pub fn open_notice_dialog(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        view_size: [i32; 2],
    ) {
        let ok = self.labels.get(1).to_owned();
        let message = Message::notice(title, body, ok).with_frame(dialog::FrameStyle::Generic);
        self.dialog_state = DialogState::for_message(&message);
        self.dialog = dialog::layout(view_size, &message, &self.art);
        self.ui_dialog = Some((OptionsDialog::Notice, message));
        self.start_dialog_fade();
        self.menu = None;
        self.close_camera_strip();
    }

    /// Opens a native inline reference book modal (`Dialog` kind 12 Spellbook or
    /// kind 15 Cookbook) displaying real caller spell/recipe data until the
    /// player dismisses it with OK / Enter / Escape.
    pub fn open_special_book_dialog(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        view_size: [i32; 2],
    ) {
        let ok = self.labels.get(1).to_owned();
        let message = Message::notice(title, body, ok).with_frame(dialog::FrameStyle::Popup);
        self.dialog_state = DialogState::for_message(&message);
        self.dialog = dialog::layout(view_size, &message, &self.art);
        self.ui_dialog = Some((OptionsDialog::SpecialBook, message));
        self.start_dialog_fade();
        self.menu = None;
        self.close_camera_strip();
    }

    /// Closes an open `OptionsDialog::SpecialBook` modal without emitting a button command
    /// (used when a co-op peer dismisses the book first or on snapshot restore).
    pub fn close_special_book_dialog(&mut self) {
        if matches!(self.ui_dialog, Some((OptionsDialog::SpecialBook, _))) {
            self.ui_dialog = None;
            self.dialog = None;
            if let Some((vm_msg, _)) = &self.message {
                self.dialog_state = DialogState::for_message(vm_msg);
                self.start_dialog_fade();
            } else {
                self.dialog_state = DialogState::default();
                self.dialog_fade = DialogFadeState::hidden();
            }
        }
    }

    /// The current text inside the active modal dialog's primary text entry
    /// field, if any.
    pub fn active_dialog_entry(&self) -> Option<&str> {
        self.active_message().and_then(|m| m.entry.as_deref())
    }

    /// The current text inside the active modal dialog's secondary text entry
    /// field (Lot Description on the edit-mode Save Card), if any.
    pub fn active_dialog_secondary_entry(&self) -> Option<&str> {
        self.active_message()
            .and_then(|m| m.secondary_entry.as_deref())
    }

    /// Switches keyboard focus between Lot Name and Lot Description on an open
    /// edit-mode Save Card (`Tab`).
    pub fn switch_dialog_field(&mut self) -> bool {
        self.active_message()
            .cloned()
            .is_some_and(|m| self.dialog_state.switch_field(&m))
    }

    /// Whether the active modal dialog's focused text field is multiline
    /// (Lot Description on an edit-mode Save Card, `0x50db98..0x50dba0`), where
    /// Return inserts `'\n'` instead of submitting (`0x5f28b6..0x5f28f9`).
    pub fn is_multiline_entry(&self) -> bool {
        self.active_message()
            .is_some_and(|m| self.dialog_state.is_multiline(m))
    }

    /// The currently active modal dialog (UI confirmation or simulation
    /// prompt), if any.
    pub fn active_message(&self) -> Option<&Message> {
        self.ui_dialog
            .as_ref()
            .map(|(_, m)| m)
            .or_else(|| self.message.as_ref().map(|(m, _)| m))
    }

    /// Mutable access to the currently active modal dialog.
    fn active_message_mut(&mut self) -> Option<&mut Message> {
        if let Some((_, m)) = &mut self.ui_dialog {
            Some(m)
        } else {
            self.message.as_mut().map(|(m, _)| m)
        }
    }

    /// Advances the active dialog's `cTSWinTextEdit2` 500 ms caret blink timer
    /// (and the open cheat console's caret blink timer).
    pub fn sync_dialog_blink(&mut self, clock_us: u64) -> bool {
        let dialog_changed = if self.active_message().is_some_and(|m| m.entry.is_some()) {
            self.dialog_state.sync_blink(clock_us)
        } else {
            false
        };
        let cheat_changed = self.cheat_console.sync_blink(clock_us);
        dialog_changed || cheat_changed
    }

    /// Screen rectangle of the Options-strip "Saves" button (present only while
    /// Options mode is expanded).
    pub fn options_saves_button_rect(&self) -> Option<Rect> {
        self.saves_button_rect
    }

    /// Releases a held Options-strip "Saves" button, returning `(changed, activated)`.
    pub fn release_saves_button(&mut self, point: Option<[i32; 2]>) -> (bool, bool) {
        if !self.pressed_saves_button {
            return (false, false);
        }
        self.pressed_saves_button = false;
        let activated = point.is_some_and(|[x, y]| {
            self.saves_button_rect
                .is_some_and(|rect| rect.contains(x, y))
        });
        if activated {
            self.play(CLICK);
        }
        (true, activated)
    }

    /// Screen rectangle of the Options-strip "Co-op" button (present only while
    /// Options mode is expanded).
    pub fn options_coop_button_rect(&self) -> Option<Rect> {
        self.coop_button_rect
    }

    /// Releases a held Options-strip "Co-op" button, returning `(changed, activated)`.
    pub fn release_coop_button(&mut self, point: Option<[i32; 2]>) -> (bool, bool) {
        if !self.pressed_coop_button {
            return (false, false);
        }
        self.pressed_coop_button = false;
        let activated = point.is_some_and(|[x, y]| {
            self.coop_button_rect
                .is_some_and(|rect| rect.contains(x, y))
        });
        if activated {
            self.play(CLICK);
        }
        (true, activated)
    }

    /// Types `ch` into the active dialog's text entry field.
    pub fn type_dialog_char(&mut self, ch: char) -> bool {
        let mut state = self.dialog_state.clone();
        let changed = self
            .active_message_mut()
            .is_some_and(|m| state.insert(m, ch));
        self.dialog_state = state;
        changed
    }

    /// Deletes the character before the caret in the active dialog's text entry.
    pub fn dialog_backspace(&mut self) -> bool {
        let mut state = self.dialog_state.clone();
        let changed = self
            .active_message_mut()
            .is_some_and(|m| state.backspace(m));
        self.dialog_state = state;
        changed
    }

    /// Deletes the character at the caret in the active dialog's text entry.
    pub fn dialog_delete(&mut self) -> bool {
        let mut state = self.dialog_state.clone();
        let changed = self.active_message_mut().is_some_and(|m| state.delete(m));
        self.dialog_state = state;
        changed
    }

    /// Sets whether Shift is held, for a press in a dialog's text field.
    pub fn set_shift(&mut self, held: bool) {
        self.shift_held = held;
    }

    /// Moves the active dialog's text-entry caret a character left, or a
    /// word with `word` (Ctrl), extending the selection with `extend`
    /// (Shift; `0x5f29d8`).
    pub fn dialog_move_left(&mut self, word: bool, extend: bool) -> bool {
        self.active_message().cloned().is_some_and(|m| {
            if word {
                self.dialog_state.move_word(&m, -1, extend)
            } else {
                self.dialog_state.move_left(&m, extend)
            }
        })
    }

    /// Moves the active dialog's text-entry caret a character (or word)
    /// right, as [`Hud::dialog_move_left`] does.
    pub fn dialog_move_right(&mut self, word: bool, extend: bool) -> bool {
        self.active_message().cloned().is_some_and(|m| {
            if word {
                self.dialog_state.move_word(&m, 1, extend)
            } else {
                self.dialog_state.move_right(&m, extend)
            }
        })
    }

    /// Moves the active dialog's text-entry caret one visual row up (multiline
    /// fields, `0x5f29f2..0x5f2a90`).
    pub fn dialog_move_up(&mut self, extend: bool) -> bool {
        self.active_message().cloned().is_some_and(|m| {
            self.dialog_state
                .move_up(&m, self.dialog.as_ref(), &self.art, extend)
        })
    }

    /// Moves the active dialog's text-entry caret one visual row down (multiline
    /// fields, `0x5f2a90..0x5f2b40`).
    pub fn dialog_move_down(&mut self, extend: bool) -> bool {
        self.active_message().cloned().is_some_and(|m| {
            self.dialog_state
                .move_down(&m, self.dialog.as_ref(), &self.art, extend)
        })
    }

    /// Moves the active dialog's text-entry caret to the start of the current
    /// line (or field, `0x5f2c13`), or of the whole text with `whole` (Ctrl).
    pub fn dialog_home(&mut self, whole: bool, extend: bool) -> bool {
        self.active_message().cloned().is_some_and(|m| {
            if whole {
                self.dialog_state.text_home(&m, extend)
            } else {
                self.dialog_state
                    .home_in_layout(&m, self.dialog.as_ref(), Some(&self.art), extend)
            }
        })
    }

    /// Moves the active dialog's text-entry caret to the end of the current
    /// line (or field, `0x5f2c13`), or of the whole text with `whole` (Ctrl).
    pub fn dialog_end(&mut self, whole: bool, extend: bool) -> bool {
        self.active_message().cloned().is_some_and(|m| {
            if whole {
                self.dialog_state.text_end(&m, extend)
            } else {
                self.dialog_state
                    .end_in_layout(&m, self.dialog.as_ref(), Some(&self.art), extend)
            }
        })
    }

    /// Toggles the active dialog's text field between inserting and
    /// overwriting (Insert, `0x5f29d8`).
    pub fn dialog_toggle_overwrite(&mut self) -> bool {
        self.active_message().is_some_and(|m| m.entry.is_some())
            && self.dialog_state.toggle_overwrite()
    }

    /// Pastes `text` into the active dialog's text field (`0x5f473f`).
    pub fn dialog_paste(&mut self, text: &str) -> bool {
        let mut state = self.dialog_state.clone();
        let changed = self
            .active_message_mut()
            .is_some_and(|m| state.paste(m, text));
        self.dialog_state = state;
        changed
    }

    /// The selected text of the active dialog's field (Copy, `0x5f4699`).
    pub fn dialog_selected_text(&self) -> Option<String> {
        self.dialog_state.selected_text(self.active_message()?)
    }

    /// Cuts the active dialog's selected text (`0x5f4724`), returning it.
    pub fn dialog_cut(&mut self) -> Option<String> {
        let mut state = self.dialog_state.clone();
        let taken = self.active_message_mut().and_then(|m| state.cut(m));
        self.dialog_state = state;
        taken
    }

    /// Selects the word under a double-click at `point` in the active
    /// dialog's text field (`0x5f2c21`), after the first press of it put
    /// the caret there.
    pub fn dialog_double_click(&mut self, point: [i32; 2]) -> bool {
        let Some(layout) = self.dialog.as_ref() else {
            return false;
        };
        let field = if self.dialog_state.active_field == 1 {
            layout.secondary_entry
        } else {
            layout.entry
        };
        // Only when the press that made this a double click went to the
        // field: a press another window took leaves no drag captured.
        if self.dialog_state.dragging_entry.is_none()
            || !field.is_some_and(|r| r.contains(point[0], point[1]))
        {
            return false;
        }
        self.active_message()
            .cloned()
            .is_some_and(|m| self.dialog_state.select_word(&m))
    }

    /// Whether a Sound volume slider drag is currently captured (`0x5f6245`).
    pub fn dragging_slider(&self) -> bool {
        self.dragging_slider.is_some()
    }

    /// Updates an active Sound slider drag using canvas x-coordinate `x`
    /// (including out-of-window coordinates while captured, `0x5f6245..0x5f6336`).
    pub fn drag_slider(&mut self, x: i32) -> bool {
        let Some(channel) = self.dragging_slider else {
            return false;
        };
        let Some(rect) = hud::slider_rect(&self.items, channel) else {
            return false;
        };
        let step = options::slider_value_at(rect, x);
        self.controls.options.sound.set(channel, step)
    }

    /// Ends any active Sound slider drag (`0x5f62ba..0x5f62de`), evaluating
    /// the release x-coordinate through `slider_value_at` before releasing
    /// pointer capture.
    pub fn release_slider(&mut self, point: Option<[i32; 2]>) -> bool {
        let Some(channel) = self.dragging_slider.take() else {
            return false;
        };
        if let (Some([x, _]), Some(rect)) = (point, hud::slider_rect(&self.items, channel)) {
            let step = options::slider_value_at(rect, x);
            self.controls.options.sound.set(channel, step);
        }
        true
    }

    /// Syncs runtime character names assigned by Make New Character
    /// (`0x44b0a8..0x44b0fc`, `0x44b18a..0x44b1cc`) into the HUD's `Faces`
    /// cache so newly created children and pets display their submitted names.
    pub fn sync_character_names(&mut self, names: &std::collections::BTreeMap<u32, String>) {
        for (&guid, name) in names {
            self.faces.insert_name(guid, name);
        }
    }

    /// Resets the HUD `Faces` cache back to its boot-time character names and
    /// applies only `names` from the restored snapshot, dropping runtime names
    /// created after that snapshot.
    pub fn reset_character_names(&mut self, names: &std::collections::BTreeMap<u32, String>) {
        self.faces = self.boot_faces.clone();
        self.sync_character_names(names);
    }

    /// Recomputes the active modal dialog's layout for a resized canvas
    /// (`0x5e45d2..0x5e45f3`) so mid-press resizing hit-tests live button bounds.
    pub fn relayout_dialog(&mut self, size: [i32; 2]) {
        self.dialog = self
            .active_message()
            .and_then(|m| dialog::layout(size, m, &self.art));
    }

    /// Releases a held Options action button (`0x514170..0x514176`,
    /// `0x5e45d2..0x5e45f3`), activating it only when `point` is still inside
    /// that button's current layout rectangle.
    pub fn release_options_action(&mut self, point: Option<[i32; 2]>) -> (bool, Option<Command>) {
        self.controls.pressed_options_action = None;
        let Some(action) = self.pressed_options_action.take() else {
            return (false, None);
        };
        let Some([x, y]) = point else {
            return (true, None);
        };
        let Some(rect) = self
            .items
            .iter()
            .find(|i| i.command == Some(Command::OptionsAction(action)))
            .map(|i| i.rect)
        else {
            return (true, None);
        };
        if !rect.contains(x, y) {
            return (true, None);
        }
        let command = Command::OptionsAction(action);
        self.play(CLICK);
        let open = self.controls.help.is_some() || self.controls.options.help.is_some();
        self.controls.apply(&command);
        self.balloon_whoosh(open);
        (true, Some(command))
    }

    /// Whether a `cTSWinTextEdit2` mouse-drag selection is currently captured
    /// (`[esi + 0x157]` at `0x5f27a6..0x5f2887`).
    pub fn dragging_dialog_entry(&self) -> bool {
        self.dialog_state.dragging_entry.is_some()
    }

    /// Updates a captured `cTSWinTextEdit2` mouse-drag selection (`0x5f27ae..0x5f2887`).
    pub fn drag_dialog_entry(&mut self, point: [i32; 2]) -> bool {
        let Some(layout) = self.dialog.as_ref() else {
            return false;
        };
        let Some(message) = self
            .ui_dialog
            .as_ref()
            .map(|(_, m)| m)
            .or_else(|| self.message.as_ref().map(|(m, _)| m))
        else {
            return false;
        };
        self.dialog_state
            .drag_entry(layout, message, &self.art, point)
    }

    /// Releases a held modal dialog button (`cTSWinBtn`, `0x5e4599..0x5e45fc`)
    /// or captured text-entry drag (`cTSWinTextEdit2`, `0x5f27a6`), activating
    /// a button only if `point` is still inside that same enabled button.
    pub fn release_dialog(&mut self, point: Option<[i32; 2]>) -> (bool, Option<Command>) {
        // `0x5f27a6`: mouse-up only clears capture (`[ecx + 0x157] = 0`);
        // drag movement updates the selection at `0x5f27ae..0x5f2887`.
        // Re-hit-testing on mouse-up would misread a stationary click after
        // mouse-down shifted the field's scroll offset.
        let released_entry = self.dialog_state.release_entry();
        // 0x524a13: the button coming up starts the timer a press on the
        // dialog stopped again, with the whole 40 s.
        self.auto_answer.release(self.last_clock_us);
        let Some(pressed) = self.dialog_state.pressed_button.take() else {
            return (released_entry, None);
        };
        let command = point.and_then(|[x, y]| {
            let layout = self.dialog.as_ref()?;
            let rect = layout.buttons.get(pressed)?;
            if rect.contains(x, y) && !self.dialog_state.is_disabled(pressed) {
                Some(Command::AnswerDialog {
                    caller: layout.caller,
                    button: u8::try_from(pressed).ok()?,
                })
            } else {
                None
            }
        });
        (true, command)
    }

    /// Submits the active modal dialog via Enter (activates button 0 if enabled).
    pub fn submit_dialog(&mut self) -> Option<Command> {
        self.confirm_key(DialogKey::Enter)
    }

    /// Handles a native modal confirmation key (`0x5e5b3b..0x5e5bfd`, `0x6d7530`):
    /// - `Enter` activates button 0 (OK / Yes) when enabled.
    /// - `Escape` activates the cancel button (last button on non-entry
    ///   confirmations, or Cancel on multi-button entry dialogs such as Save Card).
    /// - `Char(ch)` activates localized Yes (button 0) or No (button 1) on
    ///   non-entry confirmations whose buttons match `STR# 142[3]` / `STR# 142[4]`.
    pub fn confirm_key(&mut self, key: DialogKey) -> Option<Command> {
        // A non-modal dialog is answered only through its dismiss box,
        // unless it asks for text (kind 3): its field has the keys, and
        // Enter answers it as on a modal one. Other keys go on to whatever
        // else has them.
        if !self.has_dialog() && !self.has_non_modal_dialog_entry() {
            return None;
        }
        let message = self.active_message()?;
        let count = message.buttons.len();
        if count == 0 {
            return None;
        }
        let caller = message.caller;
        let button = match key {
            DialogKey::Enter => Some(0usize),
            DialogKey::Escape => {
                if message.entry.is_some() && count <= 1 {
                    None
                } else {
                    Some(count - 1)
                }
            }
            DialogKey::Tab => None,
            DialogKey::Char(ch) if message.entry.is_none() && count >= 2 => {
                // `0x4f370d..0x4f374e` -> `[0x6d7530]`: Yes/No confirmation
                // hotkeys come from `UIText.iff STR# 161` entries 7 and 8,
                // checked at `0x5e5b80..0x5e5bfd` on Yes/No and Yes/No/Cancel
                // dialogs.
                let yes_label = self.labels.get(3);
                let no_label = self.labels.get(4);
                let yes_key = self.labels.yes_key();
                let no_key = self.labels.no_key();
                if message.buttons.first().is_some_and(|b| b == yes_label)
                    && yes_key.to_lowercase().eq(ch.to_lowercase())
                {
                    Some(0)
                } else if message.buttons.get(1).is_some_and(|b| b == no_label)
                    && no_key.to_lowercase().eq(ch.to_lowercase())
                {
                    Some(1)
                } else {
                    None
                }
            }
            DialogKey::Char(_) => None,
        }?;
        if button >= count || self.dialog_state.is_disabled(button) {
            return None;
        }
        self.dialog_state.pressed_button = None;
        Some(Command::AnswerDialog {
            caller,
            button: u8::try_from(button).ok()?,
        })
    }

    /// Answers and closes the open UI confirmation dialog with `button`,
    /// returning `(kind, button, primary_entry, secondary_entry)` if one was open.
    pub fn take_ui_dialog_answer(
        &mut self,
        button: u8,
    ) -> Option<(OptionsDialog, u8, Option<String>, Option<String>)> {
        let (kind, message) = self.ui_dialog.take()?;
        self.dialog = None;
        if let Some((vm_msg, _)) = &self.message {
            self.dialog_state = DialogState::for_message(vm_msg);
            self.start_dialog_fade();
        } else {
            self.dialog_state = DialogState::default();
            self.dialog_fade = DialogFadeState::hidden();
        }
        Some((kind, button, message.entry, message.secondary_entry))
    }

    /// Whether a UI confirmation dialog (outside the simulation) is currently open.
    pub fn has_ui_dialog(&self) -> bool {
        self.ui_dialog.is_some()
    }

    /// Which UI-owned dialog (`OptionsDialog`) is currently open, if any.
    pub fn ui_dialog_kind(&self) -> Option<OptionsDialog> {
        self.ui_dialog.as_ref().map(|(kind, _)| *kind)
    }

    /// Whether any modal dialog (UI confirmation, loaded VM dialog, or drawn
    /// layout) is currently active, even before the next [`Hud::draw`]. A
    /// non-modal simulation dialog ([`dialog::Presentation::modal`] clear)
    /// is not one: it takes only the presses on it
    /// ([`Hud::has_non_modal_dialog`]).
    pub fn has_dialog(&self) -> bool {
        self.ui_dialog.is_some()
            || match &self.message {
                Some((message, _)) => message.presentation.modal,
                None => self.dialog.is_some(),
            }
    }

    /// Whether the dialog shown is a non-modal simulation dialog: drawn
    /// with a dismiss box, taking presses on it only, so the game goes on
    /// being played around it.
    pub fn has_non_modal_dialog(&self) -> bool {
        self.ui_dialog.is_none()
            && self
                .message
                .as_ref()
                .is_some_and(|(message, _)| !message.presentation.modal)
    }

    /// Whether the dialog shown is a non-modal simulation dialog that asks
    /// for text (kind 3): its field takes the keys typed, as a modal
    /// one's does. A modal dialog the lot waits on is shown before it
    /// ([`Hud::read_dialog`]), so it has the keys first.
    pub fn has_non_modal_dialog_entry(&self) -> bool {
        self.has_non_modal_dialog() && self.active_dialog_entry().is_some()
    }

    /// The rect of the non-modal dialog drawn last, if one is shown.
    fn non_modal_rect(&self) -> Option<Rect> {
        self.has_non_modal_dialog()
            .then(|| self.dialog.as_ref().map(|l| l.rect))
            .flatten()
    }

    /// Whether `point` is on a non-modal dialog: a press there is the
    /// dialog's, before any pie menu, held object or tool under it.
    pub fn over_non_modal_dialog(&self, point: [i32; 2]) -> bool {
        self.non_modal_rect()
            .is_some_and(|r| r.contains(point[0], point[1]))
    }

    /// Whether the press being released landed on a non-modal dialog
    /// (so its release must not reach what is under the dialog), clearing
    /// it. Call once per release, after [`Hud::release_dialog`].
    pub fn take_non_modal_press(&mut self) -> bool {
        std::mem::take(&mut self.non_modal_press)
    }

    /// Drops a non-modal dialog's capture left by a press whose release
    /// never came, and the button it held. A modal dialog's held button is
    /// left as it is.
    pub fn cancel_non_modal_press(&mut self) {
        if std::mem::take(&mut self.non_modal_press) {
            self.dialog_state.pressed_button = None;
            self.dialog_state.dragging_entry = None;
        }
    }

    /// Updates the native `[control_panel + 0x1ec]` idle-blink flag
    /// (`0x109(1/0)` at `0x4f688e`, or cleared by manual Live-mode speed
    /// clicks at `0x53a5e3..0x53a6e1`).
    pub fn set_idle_speed(&mut self, active: bool) {
        self.idle_speed = active;
    }

    /// Resolves `[control_panel + 0x1ec]` against `world`: `return_speed < 0`
    /// means idle speed has exited (`0x109(0)`), `speed == IDLE` with
    /// `return_speed >= 0` means idle speed is active (`0x109(1)`), and
    /// `return_speed >= 0` with `speed != IDLE` preserves `self.idle_speed`
    /// so scripted `0x10c` speed resets keep state 3 armed while manual
    /// Live-mode speed clicks keep it cleared.
    fn effective_idle_speed(&self, world: &World) -> bool {
        if world.return_speed() < 0 {
            false
        } else if world.speed() == sims_sim::speed::IDLE {
            true
        } else {
            self.idle_speed
        }
    }

    /// Microseconds until the next HUD blink timer (`ClockBlink`, modal
    /// text-entry caret, or cheat-console caret) needs a wakeup, or `None`
    /// when no blink is active.
    pub fn next_blink_us(&self, clock_us: u64) -> Option<u64> {
        let mut best: Option<u64> = None;
        let mut consider = |us: u64| {
            best = Some(best.map_or(us, |prev| prev.min(us)));
        };
        if let Some(us) = self.clock_blink.next_blink_us(clock_us) {
            consider(us);
        }
        if let Some(us) = self.performance_blink.next_blink_us(clock_us) {
            consider(us);
        }
        if self.active_message().is_some_and(|m| m.entry.is_some())
            && let Some(us) = self.dialog_state.next_blink_us(clock_us)
        {
            consider(us);
        }
        if let Some(us) = self.cheat_console.next_blink_us(clock_us) {
            consider(us);
        }
        // The live panel samples the needs on its own clock, paused or not
        // (`0x4ce390`). A sample already due made the advance at this clock
        // mark the frame dirty (`needs_sample_due`), so the redraw pacer owns
        // that deadline; only a later one needs a wakeup.
        if clock_us > 0
            && self.controls.mode == Mode::Live
            && let Some(ms) = self.needs_trend.due_in((clock_us / 1_000) as u32)
            && ms > 0
        {
            let ms = u64::from(ms);
            consider((ms * 1_000).saturating_sub(clock_us % 1_000).max(1_000));
        }
        best
    }

    /// Syncs the clock controls' shared 500 ms blink timer (`0x53b4b0..0x53b7e7`)
    /// with `world` and the monotonic `clock_us`:
    /// - state 2 (`Paused`): blinks `id::PAUSE` row 0 <-> row 1 whenever
    ///   paused (including outside Live mode, `0x53b58a -> 0x53b5e8`);
    /// - state 3 (`IdleUltra`): armed on `0x109(1)` from Live (`0x53b56f..0x53b57b`,
    ///   even when `return_speed == ULTRA`) or when returning from Pause/non-Live
    ///   with `return_speed != ULTRA` (`0x53b627..0x53b645`); toggles `id::ULTRA`
    ///   selected state every 500 ms when `speed != Ultra` (`0x53b4c9..0x53b4fe`),
    ///   and clears `ultra_lit` (without resetting the timer) on a speed refresh
    ///   such as Special Effect `0x10c` (`0x53b0d3`, `0x53b121..0x53b128`).
    ///
    /// Returns true if either `controls.pause_lit` or `controls.ultra_lit` changed.
    pub fn sync_clock_blink(&mut self, world: &World, clock_us: u64) -> bool {
        self.idle_speed = self.effective_idle_speed(world);
        let speed = hud::speed_for(world);
        let speed_pair = (world.speed(), world.return_speed());
        let changed = self.clock_blink.sync(
            world.paused(),
            self.controls.mode,
            self.idle_speed,
            speed,
            speed_pair,
            clock_us,
        );
        self.controls.pause_lit = self.clock_blink.pause_lit();
        self.controls.ultra_lit = self.clock_blink.ultra_lit();
        // The job panel shown last blinks its performance while the Sim
        // has a job and performance is below 0 (`0x530530..0x5306c6`).
        let blinks = self.items.iter().any(|i| hud::performance_blinks(&i.kind));
        let flipped = self.performance_blink.sync(blinks, clock_us);
        self.controls.performance_lit = self.performance_blink.lit;
        changed || flipped
    }

    /// Clears the cached VM dialog and its input state after answering so a
    /// consecutive identical prompt (`(caller, operands)`) initializes fresh
    /// text state (`0x523825..0x523887`).
    pub fn clear_vm_dialog(&mut self) {
        self.message = None;
        self.disarm_auto_answer();
        if self.ui_dialog.is_none() {
            self.dialog = None;
            self.dialog_state = DialogState::default();
            self.dialog_fade = DialogFadeState::hidden();
        }
    }

    /// The non-modal dialog of `key` (its caller and operands) was answered
    /// (in co-op, when the command ran): its widget and the text read for
    /// it go, as `0x4370be` destroys it, so the caller's next identical
    /// prompt is a new widget with fresh input state.
    pub fn close_non_modal(&mut self, key: (u16, [u8; 8])) {
        self.non_modal_read.retain(|(k, _)| *k != key);
        if self.message.as_ref().is_some_and(|(_, k)| *k == key) {
            self.clear_vm_dialog();
        }
    }

    /// Reads the dialog `world` waits on, if it changed. The lot's world,
    /// or, while the lot waits for the player, the world as the tick left
    /// it when it showed the dialog.
    ///
    /// A modal dialog comes first. Otherwise it is the first non-modal
    /// dialog still shown whose text was read ([`Hud::read_non_modal_shown`])
    /// or can be read now, so one whose caller's frame is gone does not
    /// hide the others.
    pub fn read_dialog(&mut self, world: &World, assets: &Assets) {
        // What was read of a dialog no longer shown is dropped.
        self.non_modal_read.retain(|(key, _)| {
            world.dialogs.records.iter().any(|r| {
                r.status <= 1 && r.operands[7] & 0x80 != 0 && (r.caller, r.operands) == *key
            })
        });
        let waiting = || world.dialogs.records.iter().filter(|r| r.status <= 1);
        let modal = waiting().find(|r| r.operands[7] & 0x80 == 0);
        let candidates: Vec<&sims_sim::Dialog> = match modal {
            Some(record) => vec![record],
            None => waiting().collect(),
        };
        if candidates.is_empty() {
            self.disarm_auto_answer();
            if self.message.take().is_some() && self.ui_dialog.is_none() {
                self.dialog_fade = DialogFadeState::hidden();
            }
            return;
        }
        for pending in candidates {
            let key = (pending.caller, pending.operands);
            if self.message.as_ref().is_some_and(|(_, k)| *k == key) {
                return;
            }
            let read = match self.non_modal_read.iter().find(|(k, _)| *k == key) {
                Some((_, m)) => Some(m.clone()),
                None => self.read_message(world, assets, pending),
            };
            if let Some(m) = read {
                self.show_message(m, key);
                return;
            }
        }
        self.disarm_auto_answer();
        self.message = None;
    }

    /// Reads `record`'s dialog from `world`, or None when it cannot be
    /// read (its caller has no frame, its strings are missing).
    fn read_message(
        &mut self,
        world: &World,
        assets: &Assets,
        record: &sims_sim::Dialog,
    ) -> Option<Message> {
        let mut ctx = dialog::DialogContext {
            faces: Some(&self.faces),
            thumbnails: Some(&mut self.thumbnails),
            art: Some(&self.art),
            careers: Some(&self.careers),
            fame: Some(&self.fame),
            families: Some(&self.families),
            ui: self.ui.as_ref(),
            known: Some(&self.known),
            members: &self.members,
        };
        Message::read_with_context(
            world,
            assets.index(),
            assets.catalog(),
            &self.labels,
            record,
            &mut ctx,
        )
        .ok()
        .flatten()
    }

    /// Shows `m`, the dialog of `key` (its caller and operands), as a new
    /// widget: fresh input state and, at `0x436c89`, its timer.
    fn show_message(&mut self, m: Message, key: (u16, [u8; 8])) {
        if self.ui_dialog.is_none() {
            self.dialog_state = DialogState::for_message(&m);
            self.start_dialog_fade();
        }
        self.auto_answer_status = dialog_timer::auto_answer_status(&key.1, m.presentation.anchored);
        match self.auto_answer_status {
            Some(_) => self.auto_answer.arm(self.last_clock_us),
            None => self.auto_answer.disarm(),
        }
        self.message = Some((m, key));
    }

    /// After an update that showed the non-modal dialogs of `callers`
    /// (`sims_vm::Update::non_modal_shown`), reads each one from `world`
    /// as that update left it, as the original makes the widget then
    /// (`0x436da0`), and then the dialog to show ([`Hud::read_dialog`]).
    ///
    /// A caller shows its choice again each tick it runs, closing the old
    /// widget and making a new one (`0x451552` → `0x437058`). The new one's
    /// timer starts over. When its text differs from the one shown (the
    /// same instruction with other `$Local` values), it replaces it with
    /// fresh input state, so a press held on the old text answers nothing.
    pub fn read_non_modal_shown(&mut self, world: &World, assets: &Assets, callers: &[u16]) {
        for record in &world.dialogs.records {
            if record.status != 1
                || record.operands[7] & 0x80 == 0
                || !callers.contains(&record.caller)
            {
                continue;
            }
            let key = (record.caller, record.operands);
            let Some(m) = self.read_message(world, assets, record) else {
                continue;
            };
            if self
                .message
                .as_ref()
                .is_some_and(|(shown, k)| *k == key && !same_dialog(shown, &m))
            {
                self.show_message(m.clone(), key);
            }
            match self.non_modal_read.iter_mut().find(|(k, _)| *k == key) {
                Some((_, kept)) => *kept = m,
                None => self.non_modal_read.push((key, m)),
            }
        }
        self.read_dialog(world, assets);
        for &caller in callers {
            self.restart_auto_answer(caller);
        }
    }

    fn disarm_auto_answer(&mut self) {
        self.auto_answer.disarm();
        self.auto_answer_status = None;
    }

    /// The simulation showed `caller`'s dialog again (a non-modal choice,
    /// which its caller makes again each tick it runs: `0x451552` →
    /// `0x437058`): the new widget's timer starts over with the whole 40 s.
    pub fn restart_auto_answer(&mut self, caller: u16) {
        if self.auto_answer_status.is_some()
            && self
                .message
                .as_ref()
                .is_some_and(|(message, _)| message.caller == caller)
        {
            // A press held on the widget kept holds the timer: its release
            // starts the whole 40 s (`0x524a13`).
            self.auto_answer.restart(self.last_clock_us);
        }
    }

    /// Whether the simulation dialog shown has its 40 s timer running.
    pub fn auto_answer_running(&self) -> bool {
        self.auto_answer.running()
    }

    /// How long until the simulation dialog's timer runs out at
    /// `clock_us`, or None while it does not run: when the host must wake
    /// to press its button, drawn or not.
    pub fn auto_answer_remaining_us(&self, clock_us: u64) -> Option<u64> {
        if self.ui_dialog.is_some() || self.message.is_none() || self.auto_answer_status.is_none() {
            return None;
        }
        self.auto_answer.remaining_us(clock_us)
    }

    /// The simulation dialog shown, as its caller and the Dialog
    /// instruction's operands, if one is read.
    pub fn vm_dialog_key(&self) -> Option<(u16, [u8; 8])> {
        self.message.as_ref().map(|(_, key)| *key)
    }

    /// The answer the simulation dialog's timer gives if it has run out
    /// at `clock_us` (`0x5248a8` presses the stored button): its caller,
    /// the answer's status and whether the dialog is modal. Taken once.
    /// A shell dialog in front holds it.
    pub fn take_auto_answer(&mut self, clock_us: u64) -> Option<(u16, u8, bool)> {
        if self.ui_dialog.is_some() {
            return None;
        }
        let (message, _) = self.message.as_ref()?;
        let status = self.auto_answer_status?;
        let answer = (message.caller, status, message.presentation.modal);
        self.auto_answer.take_due(clock_us).then_some(answer)
    }

    /// What the panel shows of `world`: the household with what the next
    /// level of each one's career needs, zoning-aware family friends
    /// (`0x4cfcdb`), and, while the relationship panel is open, the people
    /// the selected Sim knows (read only then: the lookup goes through
    /// every neighborhood record), or while the house panel shows (the
    /// strip open on it), the house's ratings.
    pub fn snapshot(&self, world: &World, residential: bool) -> Snapshot {
        self.for_panel(self.household(world, residential, None), world)
    }

    /// The active household's live member roster (`0x4e97dc`): reads the
    /// family named by global 9 from `world.families` when families are
    /// loaded, falling back to the neighborhood roster opened from disk.
    /// Global 9 at 0 names the family numbered 0 (0x428041 at
    /// 0x42827c..0x42830d), which a solo traveler leads
    /// (0x4f1c87..0x4f1cec).
    fn members<'a>(&'a self, world: &'a World) -> &'a [u32] {
        if world.families.is_empty() {
            return &self.members;
        }
        let id = i32::from(world.globals.get(9).copied().unwrap_or(0));
        world
            .families
            .iter()
            .find(|f| f.id == id)
            .map_or(&[], |f| f.members.as_slice())
    }

    /// Household resident object IDs in roster order (`Snapshot::residents`),
    /// used by the `Space` hotkey (`cWinLive::OnKeyDown`, `0x51ba2d`) to cycle
    /// to the next Sim.
    pub fn resident_object_ids(&self, world: &World) -> Vec<u16> {
        Snapshot::read(world, self.members(world))
            .residents
            .into_iter()
            .map(|r| r.object)
            .collect()
    }

    /// The household with zoning-aware family friends, career needs/grades,
    /// and Superstar fame progression data. When `selected_override` is `Some`,
    /// overrides `selected` and `selected_guid` in-place without cloning `World`.
    fn household(
        &self,
        world: &World,
        residential: bool,
        selected_override: Option<Option<u16>>,
    ) -> Snapshot {
        let mut snap = Snapshot::read(world, self.members(world));
        if let Some(sel) = selected_override {
            let selected_obj =
                sel.and_then(|id| world.objects.get(&id).filter(|o| o.person.is_some()));
            snap.selected = selected_obj.map(|o| o.id);
            snap.selected_guid = selected_obj.map(|o| o.guid);
        }
        snap.with_idle_speed(self.effective_idle_speed(world))
            .with_zoning(world, residential, self.known.friendship)
            .with_careers(&self.careers)
            .with_fame(world, &self.known, &self.fame)
    }

    /// `snapshot` with what the open panel reads besides.
    fn for_panel(&self, snapshot: Snapshot, world: &World) -> Snapshot {
        match self.controls.panel {
            Some(UiPanel::Relationships) => {
                snapshot.with_relations(world, self.members(world), &self.known)
            }
            Some(UiPanel::Gifts) if self.controls.expanded => {
                snapshot.with_inventory(world, |guid| {
                    self.buy_catalog
                        .as_ref()
                        .map(|cat| cat.display_for_object(guid).1)
                        .unwrap_or_default()
                })
            }
            Some(UiPanel::House) if self.controls.expanded && snapshot.residential => {
                snapshot.with_house(world, &self.house, &self.kinds)
            }
            _ => snapshot,
        }
    }

    /// Draws the panel over the lot in `canvas`, from `world` as it is
    /// now (reading the highlighted speed button directly from `world` via
    /// [`hud::speed_for`]), `residential` (`sims_vm::Lot::residential`),
    /// and `status` (a line across the top) if there is one; the dialog is
    /// the one [`Hud::read_dialog`] last read. The catalog pictures the
    /// queue shows are read from `assets` the first time each type shows.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        world: &World,
        residential: bool,
        assets: &Assets,
        view: &View,
        pace: &Pace,
        status: Option<&str>,
        canvas: &mut Canvas,
    ) {
        self.draw_for_lot(
            world,
            residential,
            options::LotVisibility::from_residential(residential),
            assets,
            view,
            pace,
            status,
            None,
            canvas,
        );
    }

    /// Like [`Hud::draw`], using exact per-lot [`options::LotVisibility`]
    /// (`0x4f2490..0x4f24e7`, `0x5177aa..0x517833`) and optionally showing the
    /// Buy Mode Hand Tool sell prompt (`0x487d7a` → `0x4cf90b` → `0x50a343`)
    /// when `sell` is `Some`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_for_lot(
        &mut self,
        world: &World,
        residential: bool,
        visibility: options::LotVisibility,
        assets: &Assets,
        view: &View,
        pace: &Pace,
        status: Option<&str>,
        sell: Option<&SellInfo>,
        canvas: &mut Canvas,
    ) {
        self.draw_for_lot_at(
            world,
            residential,
            visibility,
            assets,
            view,
            pace,
            status,
            sell,
            0,
            canvas,
        );
    }

    /// Like [`Hud::draw_for_lot`], advancing temporal UI animations
    /// (`BalloonFadeState`, `DialogFadeState`, `QueueSlideState`) at
    /// presentation clock `clock_us`. When `clock_us == 0`, all animations
    /// snap instantaneously so headless and static pixel-ratchet frames remain
    /// 100% bit-exact.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_for_lot_at(
        &mut self,
        world: &World,
        residential: bool,
        visibility: options::LotVisibility,
        assets: &Assets,
        view: &View,
        pace: &Pace,
        status: Option<&str>,
        sell: Option<&SellInfo>,
        clock_us: u64,
        canvas: &mut Canvas,
    ) {
        self.draw_for_lot_at_with_selection(
            world,
            residential,
            Some(if residential { 1 } else { 2 }),
            visibility,
            assets,
            view,
            pace,
            status,
            sell,
            clock_us,
            None,
            canvas,
        );
    }

    /// Like [`Hud::draw_for_lot_at`], optionally overriding the HUD's displayed
    /// selected Sim (`selected_override`: `Some(local_sel)`) in co-op mode
    /// without cloning `world` and passing exact neighborhood `lot_zoning`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_for_lot_at_with_selection(
        &mut self,
        world: &World,
        residential: bool,
        lot_zoning: Option<i32>,
        visibility: options::LotVisibility,
        assets: &Assets,
        view: &View,
        _pace: &Pace,
        status: Option<&str>,
        sell: Option<&SellInfo>,
        clock_us: u64,
        selected_override: Option<Option<u16>>,
        canvas: &mut Canvas,
    ) {
        self.last_clock_us = clock_us;
        let now_ms = (clock_us / 1_000) as u32;
        let camera = &view.camera;
        self.controls.camera = hud::CameraState {
            floor: u8::try_from(camera.level).unwrap_or(1),
            walls: match camera.walls {
                Walls::Up => UiWalls::Up,
                Walls::Cutaway => UiWalls::Cutaway,
                Walls::Down => UiWalls::Down,
            },
            roof: self.roof_view,
            zoom_in: camera.zoom != Zoom::Near,
            zoom_out: camera.zoom != Zoom::Far,
        };
        self.controls.speed = hud::speed_for(world);
        // The panel follows the selection first, so that the snapshot
        // reads what the open panel needs.
        self.drawn_selection = selected_override;
        let household = self.household(world, residential, selected_override);
        if self.controls.follow(Selected::of(&household)) {
            self.play(WHOOSH);
        }
        // 0x51d3f9..0x51d412: rejecting House on a non-residential lot stores
        // panel 0 (`None`) so a reused HUD does not reopen House on a later
        // residential lot.
        if !residential && self.controls.panel == Some(UiPanel::House) {
            self.controls.panel = None;
            if self.controls.help.take().is_some() {
                self.play(WHOOSH);
            }
        }
        self.step_needs_trend(world, &household, clock_us);
        if self.controls.expanded && residential && self.controls.panel == Some(UiPanel::House) {
            self.kinds.learn(world, assets.index(), assets.catalog());
        }
        if self.controls.expanded && self.controls.panel == Some(UiPanel::Gifts) {
            self.ensure_buy_catalog(assets);
        }
        // The pause button shows global 17, which the game's pause sets.
        let snapshot = self.for_panel(household, world);
        if self.controls.expanded && self.controls.panel == Some(UiPanel::Gifts) {
            for entry in &snapshot.inventory {
                let _ = self
                    .thumbnails
                    .read(assets.index(), assets.catalog(), entry.guid);
            }
        }
        if let Some(help) = self.controls.help
            && !hud::help_matches_snapshot(help, &snapshot, self.controls.panel)
        {
            self.controls.help = None;
            self.play(WHOOSH);
        }
        self.items = hud::layout_with_options_lot(
            view.size,
            &snapshot,
            &self.controls,
            &self.options_texts,
            visibility,
            Some(&self.art),
        );
        hud::fit(&mut self.items, &self.art, &self.texts);
        self.sync_options_extra_button_rects(view.size);
        // A job panel that blinks starts its timer the frame it is first laid
        // out, not at the next clock sync.
        if clock_us > 0
            && self.performance_blink.since.is_none()
            && self.items.iter().any(|i| hud::performance_blinks(&i.kind))
        {
            self.performance_blink.since = Some(clock_us);
        }
        self.queue = Queue::read(world, snapshot.selected);
        if clock_us > 0 {
            let origin = self.pending_queue_origin.and_then(|(person, pos, ts)| {
                if person == self.queue.person && clock_us.saturating_sub(ts) <= 2_000_000 {
                    Some(pos)
                } else {
                    None
                }
            });
            let has_new_icon = self
                .queue
                .icons
                .iter()
                .any(|icon| self.queue_slide.position(icon.uid, now_ms).is_none());
            self.queue_slide.sync(&self.queue, origin, now_ms);
            if has_new_icon || origin.is_none() {
                self.pending_queue_origin = None;
            }
            self.queue_slide.apply_to(&mut self.queue, now_ms);
        } else {
            self.pending_queue_origin = None;
            self.queue_slide.sync(&self.queue, None, 0);
        }
        self.pressed.keep(&self.queue);
        let queue_objects: Vec<u32> = self.queue.objects().collect();
        for guid in queue_objects {
            // A type whose picture fails is kept as the error left it
            // (`Thumbnails::read`), and not read again.
            if let Err(e) = self.thumbnails.read(assets.index(), assets.catalog(), guid) {
                self.problems
                    .push(format!("the catalog picture of type 0x{guid:08X}: {e}"));
            }
        }
        let balloon_objects: Vec<u32> = hud::balloon_objects(&self.items).collect();
        for guid in balloon_objects {
            if let Err(e) = self
                .thumbnails
                .read_balloon(assets.index(), assets.catalog(), guid)
            {
                self.problems
                    .push(format!("the balloon picture of type 0x{guid:08X}: {e}"));
            }
        }
        let (width, height) = (
            u32::try_from(canvas.width).unwrap_or(u32::MAX),
            u32::try_from(canvas.height).unwrap_or(u32::MAX),
        );
        let mut frame = match Frame::from_packed(width, height, std::mem::take(&mut canvas.color)) {
            Ok(frame) => frame,
            Err(pixels) => {
                canvas.color = pixels;
                return;
            }
        };
        let current_balloon = self.items.iter().find_map(FadingBalloon::of);
        if clock_us == 0 {
            if let Some(b) = current_balloon {
                self.balloon_fade = BalloonFadeState::visible();
                self.last_balloon = Some(b);
            } else {
                self.balloon_fade = BalloonFadeState::new();
                self.last_balloon = None;
            }
            hud::draw_with_thumbnails(
                &mut frame,
                &self.art,
                &self.faces,
                Some(&self.thumbnails),
                &self.items,
                &self.controls,
            );
        } else {
            // A balloon of the other kind does not take over this one's
            // ramp: an Options balloon is the Options panel's child and goes
            // with it (out of Options, or with the panel collapsed), and a
            // new kind starts its own fade.
            let kind_changed = self.last_balloon.as_ref().is_some_and(|last| {
                current_balloon.as_ref().map_or(
                    last.is_options()
                        && (self.controls.mode != Mode::Options || !self.controls.expanded),
                    |b| b.is_options() != last.is_options(),
                )
            });
            if kind_changed {
                self.balloon_fade = BalloonFadeState::new();
                self.last_balloon = None;
            }
            if let Some(b) = &current_balloon {
                self.last_balloon = Some(b.clone());
                if !self.balloon_fade.is_open() {
                    self.balloon_fade.open(now_ms);
                }
            } else if self.balloon_fade.is_open() {
                self.balloon_fade.close(now_ms);
            }
            let animate_balloon = self.balloon_fade.is_animating(now_ms)
                || (current_balloon.is_none() && self.balloon_fade.is_visible(now_ms));
            if animate_balloon {
                let non_balloons: Vec<Item> = self
                    .items
                    .iter()
                    .filter(|item| FadingBalloon::of(item).is_none())
                    .cloned()
                    .collect();
                hud::draw_with_thumbnails(
                    &mut frame,
                    &self.art,
                    &self.faces,
                    Some(&self.thumbnails),
                    &non_balloons,
                    &self.controls,
                );
                if let Some(balloon) = current_balloon.as_ref().or(self.last_balloon.as_ref()) {
                    if self.controls.options.graphics.interface_effects {
                        let alpha = self.balloon_fade.alpha(now_ms);
                        balloon.draw(&mut frame, &self.art, &self.thumbnails, 0, alpha);
                    } else {
                        let dy = self
                            .balloon_fade
                            .slide_offset_y(now_ms, balloon.rect().height);
                        balloon.draw(&mut frame, &self.art, &self.thumbnails, dy, 255);
                    }
                }
            } else {
                if !self.balloon_fade.is_visible(now_ms) {
                    self.last_balloon = None;
                }
                hud::draw_with_thumbnails(
                    &mut frame,
                    &self.art,
                    &self.faces,
                    Some(&self.thumbnails),
                    &self.items,
                    &self.controls,
                );
            }
        }
        if let Some(rect) = self.saves_button_rect {
            self.draw_options_strip_button(&mut frame, rect, "Saves", self.pressed_saves_button);
        }
        if let Some(rect) = self.coop_button_rect {
            self.draw_options_strip_button(&mut frame, rect, "Co-op", self.pressed_coop_button);
        }
        if self.controls.mode == Mode::Buy && self.controls.expanded {
            self.layout_buy_for_lot(view.size, world, lot_zoning, assets, sell);
            let mut guids: Vec<u32> = self
                .buy_layout
                .widgets
                .iter()
                .filter_map(|w| match w.kind {
                    BuyWidgetKind::Slot { guid, .. } => Some(guid),
                    BuyWidgetKind::Button { .. } => None,
                })
                .collect();
            if let Some(card) = &self.buy_layout.popup {
                guids.push(card.guid);
            }
            if let Some(prompt) = &self.buy_layout.sell_prompt {
                guids.push(prompt.guid);
                let _ = self
                    .thumbnails
                    .read_balloon(assets.index(), assets.catalog(), prompt.guid);
            }
            for guid in guids {
                let _ = self.thumbnails.read(assets.index(), assets.catalog(), guid);
            }
            buy::draw(
                &mut frame,
                &self.art,
                &self.thumbnails,
                &self.buy_layout,
                self.controls.pointer,
            );
        } else {
            self.buy_layout = BuyLayout::default();
        }
        if self.controls.mode == Mode::Build && self.controls.expanded {
            let mut guids: Vec<u32> = self
                .build_layout
                .widgets
                .iter()
                .filter_map(|w| match w.kind {
                    BuildWidgetKind::Slot {
                        item: BuildItemId::Object(guid),
                        ..
                    } => Some(guid),
                    _ => None,
                })
                .collect();
            if let Some(card) = &self.build_layout.popup
                && let BuildItemId::Object(guid) = card.item
            {
                guids.push(guid);
            }
            for guid in guids {
                let _ = self.thumbnails.read(assets.index(), assets.catalog(), guid);
            }
            build::draw_with_catalog(
                &mut frame,
                &self.art,
                &self.thumbnails,
                &self.build_swatches,
                self.build_catalog.as_ref(),
                &self.build_layout,
                self.controls.pointer,
            );
        } else {
            self.build_layout = BuildLayout::default();
        }
        self.layout_camera_strip(view.size);
        if self.camera_strip_open {
            self.draw_camera_strip(&mut frame, view.size);
        }
        if self.controls.mode == Mode::Live {
            self.queue
                .draw(&mut frame, &self.art, &self.faces, &self.thumbnails);
        }
        self.dialog = self
            .active_message()
            .and_then(|m| dialog::layout(view.size, m, &self.art));
        // A modal dialog takes every press until it is answered, so a menu
        // open under it is closed rather than left to be chosen from after.
        // A non-modal one leaves the game to be played around it.
        if self.dialog.is_some() && !self.has_non_modal_dialog() {
            self.menu = None;
            self.close_camera_strip();
        }
        // A modal dialog closes any pie menu (above), so only a non-modal
        // one is ever drawn with a menu open: it goes over the menu's
        // labels, as presses on it go to it first (`Game::dialog_takes`).
        if let Some(menu) = &mut self.menu {
            menu.layout(&self.art, view.size);
            menu.draw_labels(&mut frame, &self.art, self.controls.pointer);
        }
        if let (Some(message), Some(layout)) = (self.active_message(), &self.dialog) {
            let alpha = if clock_us == 0 {
                255
            } else {
                self.dialog_fade.alpha(now_ms)
            };
            dialog::draw_with_state_alpha(
                &mut frame,
                &self.art,
                message,
                layout,
                self.controls.pointer,
                &self.dialog_state,
                alpha,
            );
        }
        self.band = status.and_then(|status| self.status(&mut frame, status));
        if self.cheat_console.is_open() {
            self.cheat_console.draw(&mut frame, &self.art);
        }
        canvas.color = frame.into_packed();
    }

    /// Computes non-overlapping `(coop_rect, saves_rect)` placements for the
    /// synthetic Options-strip `"Co-op"` and `"Saves"` buttons across all
    /// resolutions (`640×480`, `800×600`, `1024×768`, `1280×720`, etc.) and all
    /// Options subpanel tabs (`None`, `Graphics`, `Sound`, `Play`).
    ///
    /// Guarantees pairwise disjointness against `CLUSTER`, every laid-out
    /// Options strip widget (actions, tabs, dividers, checkboxes, radios,
    /// sliders, labels, Reset Tutorial), and any open Options help balloon or
    /// Quick Tips tooltip.
    fn compute_options_extra_button_rects(&self, size: [i32; 2]) -> Option<(Rect, Rect)> {
        if self.controls.mode != Mode::Options || !self.controls.expanded {
            return None;
        }
        let divider_x = self
            .items
            .iter()
            .find(|i| {
                matches!(
                    i.kind,
                    hud::Kind::Image(id) if id == options::id::TOOTHPICK_VERT
                )
            })
            .map(|i| i.rect.x)?;
        let strip_y = (size[1] - hud::STRIP).max(0);
        const BTN_W: i32 = 48;
        const BTN_H: i32 = 22;
        const GAP: i32 = 4;
        const PAIR_W: i32 = BTN_W * 2 + GAP; // 100

        let pair_at = |x: i32, y: i32| {
            (
                Rect::new(x, y, BTN_W, BTN_H),
                Rect::new(x + BTN_W + GAP, y, BTN_W, BTN_H),
            )
        };
        let rects_overlap = |a: Rect, b: Rect| {
            a.width > 0
                && a.height > 0
                && b.width > 0
                && b.height > 0
                && a.x < b.x + b.width
                && a.x + a.width > b.x
                && a.y < b.y + b.height
                && a.y + a.height > b.y
        };
        let cluster_rect = Rect::new(
            0,
            (size[1] - hud::CLUSTER[1]).max(0),
            hud::CLUSTER[0],
            hud::CLUSTER[1],
        );
        let is_clear = |coop: Rect, saves: Rect| -> bool {
            let inside = |r: Rect| {
                r.x >= 0 && r.y >= 0 && r.x + r.width <= size[0] && r.y + r.height <= size[1]
            };
            if !inside(coop) || !inside(saves) {
                return false;
            }
            if rects_overlap(coop, cluster_rect) || rects_overlap(saves, cluster_rect) {
                return false;
            }
            for item in &self.items {
                if matches!(
                    item.kind,
                    hud::Kind::Stretched(_)
                        | hud::Kind::Image(hud::id::STRIP | hud::id::BACK_PATCH)
                ) {
                    continue;
                }
                let obstacle = match &item.kind {
                    hud::Kind::OptionsTooltip(text) => {
                        options::tooltip_rect(size, &self.art, item.rect, text).unwrap_or(item.rect)
                    }
                    _ => item.rect,
                };
                if rects_overlap(coop, obstacle) || rects_overlap(saves, obstacle) {
                    return false;
                }
            }
            true
        };

        // 1. When no subpanel tab is open, the entire right side of the Options
        //    strip (`x > divider_x`) is empty: dock neatly beside the divider.
        if self.controls.options.tab.is_none() {
            let (coop, saves) = pair_at(divider_x + 12, strip_y + (hud::STRIP - BTN_H) / 2);
            if is_clear(coop, saves) {
                return Some((coop, saves));
            }
        }

        // 2. When a subpanel tab IS open and there is room to the right of all
        //    subpanel controls inside the strip (e.g. 1024×768, 1280×720, or
        //    Sound at 800×600), right-align inside the strip with >= 8 px margin.
        let subpanel_right = self
            .items
            .iter()
            .filter(|item| {
                !matches!(
                    item.kind,
                    hud::Kind::Stretched(_)
                        | hud::Kind::Image(hud::id::STRIP | hud::id::BACK_PATCH)
                        | hud::Kind::OptionsBalloon(_)
                        | hud::Kind::OptionsTooltip(_)
                ) && item.rect.x >= divider_x
            })
            .map(|item| item.rect.x + item.rect.width)
            .max()
            .unwrap_or(divider_x + 2);
        let right_x = size[0] - PAIR_W - 6;
        if right_x >= subpanel_right + 8 {
            let (coop, saves) = pair_at(right_x, size[1] - BTN_H - 8);
            if is_clear(coop, saves) {
                return Some((coop, saves));
            }
        }

        // 3. Compact / full-width subpanel fallback (e.g. Graphics & Play at
        //    800×600 or 640×480): dock cleanly right ABOVE the Options strip
        //    (`y = strip_y - BTN_H - 4`), strictly right of `CLUSTER` so it can
        //    never cover `"Export HTML"`, `"Character Detail"`, or any other
        //    Options subpanel control.
        let above_y = (strip_y - BTN_H - 4).max(4);
        let min_x = hud::CLUSTER[0] + 8;
        let max_x = (size[0] - PAIR_W - 6).max(min_x);
        for candidate_x in [min_x, max_x, divider_x + 4, (min_x + max_x) / 2] {
            let x = candidate_x.clamp(min_x, max_x);
            let (coop, saves) = pair_at(x, above_y);
            if is_clear(coop, saves) {
                return Some((coop, saves));
            }
        }
        // If an Options help balloon or Quick Tips tooltip occupies the row
        // immediately above the strip, step up cleanly above their actual drawn boxes.
        let top_float = self
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                hud::Kind::OptionsBalloon(_) => Some(i.rect.y),
                hud::Kind::OptionsTooltip(text) => {
                    options::tooltip_rect(size, &self.art, i.rect, text).map(|r| r.y)
                }
                _ => None,
            })
            .min()
            .unwrap_or(above_y);
        let lifted_y = (top_float - BTN_H - 4).max(4);
        for candidate_x in [min_x, max_x, divider_x + 4, (min_x + max_x) / 2] {
            let x = candidate_x.clamp(min_x, max_x);
            let (coop, saves) = pair_at(x, lifted_y);
            if is_clear(coop, saves) {
                return Some((coop, saves));
            }
        }
        None
    }

    /// Synchronizes stored `"Saves"` and `"Co-op"` hitboxes with the latest
    /// laid-out Options items so pointer hits immediately after resize or
    /// Options tab switches never target stale rectangles.
    fn sync_options_extra_button_rects(&mut self, size: [i32; 2]) {
        if let Some((coop, saves)) = self.compute_options_extra_button_rects(size) {
            self.coop_button_rect = Some(coop);
            self.saves_button_rect = Some(saves);
        } else {
            self.coop_button_rect = None;
            self.saves_button_rect = None;
        }
    }

    fn draw_options_strip_button(&self, frame: &mut Frame, rect: Rect, label: &str, pressed: bool) {
        let hovered = self
            .controls
            .pointer
            .is_some_and(|[px, py]| rect.contains(px, py));
        let (bg, border, text_rgb) = if pressed && hovered {
            ([30, 88, 176], [0, 255, 255], [0, 255, 255])
        } else if hovered {
            ([26, 62, 146], [136, 178, 232], [255, 255, 255])
        } else {
            ([16, 36, 108], [88, 120, 204], [195, 205, 205])
        };
        frame.fill(rect, bg);
        frame.fill(Rect::new(rect.x, rect.y, rect.width, 1), border);
        frame.fill(
            Rect::new(rect.x, rect.y + rect.height - 1, rect.width, 1),
            border,
        );
        frame.fill(Rect::new(rect.x, rect.y, 1, rect.height), border);
        frame.fill(
            Rect::new(rect.x + rect.width - 1, rect.y, 1, rect.height),
            border,
        );
        if let Some(font) = self.art.font(10).or_else(|| self.art.font(12)) {
            let tw = font.width(label);
            let tx = rect.x + (rect.width - tw) / 2;
            let ty = rect.y + (rect.height - font.line_height) / 2;
            font.draw(frame, label, [tx, ty], text_rgb);
        }
    }

    /// Computes the native Complete Collection Camera subpanel (`cWinCamera`)
    /// layout strictly inside the bottom panel strip area (`x >= hud::CLUSTER[0]`),
    /// guaranteeing zero overlap with the Live Mode cluster, clock readout,
    /// speed buttons, or Camera icon.
    pub fn layout_camera_strip(&mut self, size: [i32; 2]) {
        if !self.camera_strip_open {
            self.clear_camera_strip_layout();
            return;
        }
        let avail_w = (size[0] - hud::CLUSTER[0]).max(0);
        if avail_w < 84 || size[1] < hud::STRIP {
            self.camera_strip_open = false;
            self.clear_camera_strip_layout();
            return;
        }
        let strip = CameraPanel::strip_rect([size[0].max(0) as u32, size[1].max(0) as u32]);
        let state = self.camera_state();
        let layout = CameraLayout::compute(strip, &state, &self.camera_strings, &self.art);
        // Verified directly against Sims.exe 1.91 `camera-strip.png`: native
        // `cWinCamera` displays only its 8 controls (3 sizes, 3 qualities,
        // divider, and Photo Album) — snapshots are taken by clicking the lot
        // viewport, with no synthetic button injected into the strip.
        self.camera_strip_rect = Some(strip);
        self.camera_layout = Some(layout);
    }

    fn draw_camera_strip(&mut self, frame: &mut Frame, size: [i32; 2]) {
        let Some(strip) = self.camera_strip_rect else {
            return;
        };
        // Native `0x53a1a9..0x53a1c3`: draw `kCameraPatch` (`CameraPatch.bmp`,
        // `2035`) over the control-panel cluster at `(165, 83)` while Camera Mode
        // is active.
        let cluster_origin = [0, size[1].saturating_sub(hud::CLUSTER[1])];
        CameraPanel::draw_cluster_patch(frame, cluster_origin, &self.art);

        let state = self.camera_state();
        let layout =
            self.camera_panel
                .draw(frame, strip, &state, &self.camera_strings, &self.art, true);
        self.camera_layout = Some(layout);
    }

    /// A line of text on a band across the top of the frame; the band.
    fn status(&self, frame: &mut Frame, text: &str) -> Option<Rect> {
        let font = self.art.font(STATUS_SIZE)?;
        let width = i32::try_from(frame.width()).unwrap_or(i32::MAX);
        let lines = font.wrap(text, (width - 16).max(16));
        let line_height = i32::from(STATUS_SIZE) + 4;
        let height = line_height * i32::try_from(lines.len()).unwrap_or(1) + 8;
        let band = Rect::new(0, 0, width, height);
        frame.fill(band, STATUS_BACK);
        for (i, line) in lines.iter().enumerate() {
            let y = 4 + line_height * i32::try_from(i).unwrap_or(0);
            font.draw(frame, line, [8, y], STATUS_TEXT);
        }
        Some(band)
    }

    /// The panel as it was last laid out.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The first name of people of type `guid`, from their file.
    pub fn name(&self, guid: u32) -> Option<&str> {
        self.faces.name(guid)
    }

    /// The cached portrait bitmap of people of type `guid`, if loaded.
    pub fn face(&self, guid: u32) -> Option<&sims_formats::bitmap::Image> {
        self.faces.face(guid)
    }

    /// Where the dialog drawn last is, and its buttons.
    pub fn dialog(&self) -> Option<&dialog::Layout> {
        self.dialog.as_ref()
    }

    /// Whether the panel or the status band covers `point` (canvas
    /// pixels), or a dialog or a pie menu is shown and takes every press.
    pub fn covers(&self, point: [i32; 2]) -> bool {
        self.has_dialog()
            || self
                .non_modal_rect()
                .is_some_and(|r| r.contains(point[0], point[1]))
            || self.menu.is_some()
            || self.band.is_some_and(|b| b.contains(point[0], point[1]))
            || self
                .camera_strip_rect
                .is_some_and(|r| r.contains(point[0], point[1]))
            || self
                .saves_button_rect
                .is_some_and(|r| r.contains(point[0], point[1]))
            || self
                .coop_button_rect
                .is_some_and(|r| r.contains(point[0], point[1]))
            || hud::covers(&self.items, point)
            || (self.controls.mode == Mode::Buy
                && self.controls.expanded
                && self.buy_layout.covers(point))
            || (self.controls.mode == Mode::Build
                && self.controls.expanded
                && self.build_layout.covers(point))
            || (self.controls.mode == Mode::Live && self.queue.at(point).is_some())
    }

    /// The selected Sim's actions as last drawn.
    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    /// The catalog pictures read so far.
    pub fn thumbnails(&self) -> &Thumbnails {
        &self.thumbnails
    }

    /// What could not be read while drawing since this was last asked.
    pub fn take_problems(&mut self) -> Vec<String> {
        std::mem::take(&mut self.problems)
    }

    /// The interface's sounds since the last call.
    pub fn take_sounds(&mut self) -> Vec<SoundEvent> {
        std::mem::take(&mut self.sounds)
    }

    /// Plays one of the interface's sounds.
    pub fn play(&mut self, name: &str) {
        self.sounds.push(SoundEvent::Play {
            object: INTERFACE,
            name: name.into(),
        });
    }

    /// A press on the icon of the selected Sim's action at `point`
    /// (0x4cb126): the cancel it asks for, if any. A pet's icon only
    /// plays `UI_error`; an icon already pressed, or whose action is
    /// already cancelled, does nothing.
    fn press_queue(&mut self, point: [i32; 2]) -> Option<Option<Command>> {
        let icon = if self.last_clock_us > 0 {
            self.queue.at_clickable(
                point,
                &self.queue_slide,
                (self.last_clock_us / 1_000) as u32,
            )?
        } else {
            self.queue.at(point)?
        };
        let (uid, cancelled) = (icon.uid, icon.cancelled);
        let person = self.queue.person?;
        if self.queue.pet {
            self.play(ERROR);
            return Some(None);
        }
        if cancelled || !self.pressed.press(uid) {
            return Some(None);
        }
        Some(Some(Command::CancelAction {
            sim: person,
            action: uid,
        }))
    }

    /// Forgets the press on `uid`'s icon, whose cancel never reached the
    /// simulation (the lot refused it, or has stopped), so that the icon
    /// takes the next press. In the original a cancel cannot fail.
    pub fn release(&mut self, uid: i32) {
        self.pressed.release(uid);
    }

    /// The pointer's current position in canvas pixels, if inside the window.
    pub fn pointer(&self) -> Option<[i32; 2]> {
        self.controls.pointer
    }

    /// Sets the pointer position directly (used when opening a pie menu so the
    /// opening frame seeds its head pose and hit state from the press or
    /// on-screen clamp warp rather than a stale hover, `0x5ebd05` / `0x5ebd29`).
    pub fn set_pointer(&mut self, point: [i32; 2]) {
        self.controls.pointer = Some(point);
    }

    /// Whether a dialog message is loaded and will cover/close any open pie
    /// menu on this frame (`Hud::draw`).
    pub fn will_show_dialog(&self, size: [i32; 2]) -> bool {
        self.active_message()
            .filter(|m| self.ui_dialog.is_some() || m.presentation.modal)
            .and_then(|m| dialog::layout(size, m, &self.art))
            .is_some()
    }

    /// Moves the pointer; true if what it hovers changed, a pie menu is open
    /// (so the Sim's 3D head follows the pointer), or an open help balloon was
    /// dismissed by moving outside the subpanel (including onto the balloon
    /// itself, which attaches outside `LivePanel` at `0x51a0a2..0x51a0b5`,
    /// `0x51f1f2..0x51f3e5`, `0x51d2ee..0x51d359`).
    pub fn hover(&mut self, point: [i32; 2]) -> bool {
        let moved_for_pie =
            self.menu.is_some() && self.dialog.is_none() && self.controls.pointer != Some(point);
        let mut dragged = false;
        if let Some(channel) = self.dragging_slider
            && let Some(rect) = hud::slider_rect(&self.items, channel)
        {
            let step = options::slider_value_at(rect, point[0]);
            dragged = self.controls.options.sound.set(channel, step);
        }
        if self.dialog_state.dragging_entry.is_some() {
            dragged |= self.drag_dialog_entry(point);
        }
        let button = |pointer: Option<[i32; 2]>| {
            let layout = self.dialog.as_ref()?;
            let [x, y] = pointer?;
            layout.buttons.iter().position(|b| b.contains(x, y))
        };
        let entry = |pointer: Option<[i32; 2]>| {
            let menu = self.menu.as_ref()?;
            Some(menu.hovered(pointer?))
        };
        let buy_widget = |pointer: Option<[i32; 2]>| {
            let [x, y] = pointer?;
            self.buy_layout
                .widgets
                .iter()
                .rposition(|w| w.rect.contains(x, y))
        };
        let build_widget = |pointer: Option<[i32; 2]>| {
            let [x, y] = pointer?;
            let w = self
                .build_layout
                .widgets
                .iter()
                .rposition(|w| w.rect.contains(x, y));
            let s = self
                .build_layout
                .subtools
                .iter()
                .rposition(|w| w.rect.contains(x, y));
            Some((w, s))
        };
        let over_saves = |pointer: Option<[i32; 2]>| {
            let s = pointer
                .is_some_and(|[x, y]| self.saves_button_rect.is_some_and(|r| r.contains(x, y)));
            let c = pointer
                .is_some_and(|[x, y]| self.coop_button_rect.is_some_and(|r| r.contains(x, y)));
            (s, c)
        };
        let prev_camera_hover = self.camera_panel.hovered();
        let before = (
            hud::hovered(&self.items, self.controls.pointer),
            hud::lit_relation(&self.items, self.controls.pointer),
            button(self.controls.pointer),
            entry(self.controls.pointer),
            buy_widget(self.controls.pointer),
            build_widget(self.controls.pointer),
            over_saves(self.controls.pointer),
        );
        self.controls.pointer = Some(point);
        let camera_hover_changed = if self.camera_strip_open
            && let Some(strip) = self.camera_strip_rect
        {
            let state = self.camera_state();
            let next = self.camera_panel.hover(
                point[0],
                point[1],
                strip,
                &state,
                &self.camera_strings,
                &self.art,
            );
            next != prev_camera_hover
        } else {
            false
        };
        let hovered_changed = (
            hud::hovered(&self.items, self.controls.pointer),
            hud::lit_relation(&self.items, self.controls.pointer),
            button(self.controls.pointer),
            entry(self.controls.pointer),
            buy_widget(self.controls.pointer),
            build_widget(self.controls.pointer),
            over_saves(self.controls.pointer),
        ) != before
            || camera_hover_changed;
        let dismissed_live = self.controls.help.is_some()
            && !hud::in_subpanel(&self.items, point)
            && self.dismiss_live_help();
        // `0x517957..0x5179cd`: Options help balloons also dismiss when the
        // pointer leaves the Options panel (`cWinOptions`).
        let dismissed_options = self.controls.options.help.is_some()
            && !hud::in_options_panel(&self.items, point)
            && self.dismiss_options_help();
        moved_for_pie || dragged || hovered_changed || dismissed_live || dismissed_options
    }

    /// The pointer left the window: no button is hovered, no name shows, and
    /// any open Live or Options help balloon is dismissed (`0x51d2ee..0x51d317`,
    /// `0x517995..0x5179bd`). Active slider drag capture (`0x5f6245..0x5f625b`)
    /// is preserved until mouse release. True if the frame changes.
    pub fn leave(&mut self) -> bool {
        let left = self.controls.pointer.take().is_some();
        let cam_hovered = self.camera_panel.hovered().is_some();
        self.camera_panel.clear_pointer();
        let dismissed = self.dismiss_help();
        left || cam_hovered || dismissed
    }

    /// Where the pointer is to be moved, as the original moves it when a
    /// pie menu opens; the panel's pointer is moved there.
    pub fn take_pointer(&mut self) -> Option<[i32; 2]> {
        let point = self.menu.as_mut()?.take_pointer()?;
        self.controls.pointer = Some(point);
        Some(point)
    }

    /// Re-lays out an open pie menu for a new canvas `size`.
    pub fn resize_menu(&mut self, size: [i32; 2]) {
        if let Some(menu) = &mut self.menu {
            menu.layout(&self.art, size);
        }
    }

    /// Starts the help balloon's 250 ms fade-out (`0x50fb7f`), or drops it at
    /// once on a static frame.
    fn close_balloon_fade(&mut self) {
        if self.last_clock_us > 0 {
            self.balloon_fade.close((self.last_clock_us / 1_000) as u32);
        } else {
            self.balloon_fade = BalloonFadeState::new();
            self.last_balloon = None;
        }
    }

    /// Whether a closed help balloon of the Options kind (`options`) or the
    /// Live kind is still fading out at the last clock.
    fn balloon_fading_out(&self, options: bool) -> bool {
        self.last_clock_us > 0
            && !self.balloon_fade.is_open()
            && self
                .balloon_fade
                .is_visible((self.last_clock_us / 1_000) as u32)
            && self
                .last_balloon
                .as_ref()
                .is_some_and(|b| b.is_options() == options)
    }

    /// After a command that may open or close a help balloon (`open`: one
    /// was open before): a close starts the fade-out at once and whooshes;
    /// an open whooshes, unless a balloon of its kind is still fading out
    /// (0x50fad3..0x50fb2f: the ramp turns around silently).
    pub(crate) fn balloon_whoosh(&mut self, open: bool) {
        let live = self.controls.help.is_some();
        let options = self.controls.options.help.is_some();
        let now_open = live || options;
        if open && !now_open {
            self.close_balloon_fade();
            self.play(WHOOSH);
        } else if !open && now_open && !self.balloon_fading_out(options && !live) {
            self.play(WHOOSH);
        }
    }

    /// Dismisses any open live-mode help balloon (`0x50fb7f`) and plays `UI_whoosh`.
    fn dismiss_live_help(&mut self) -> bool {
        if self.controls.help.take().is_some() {
            self.close_balloon_fade();
            self.play(WHOOSH);
            true
        } else {
            false
        }
    }

    /// Dismisses any open Options help balloon (`0x517995..0x5179bd` -> `0x50fb7f`)
    /// and plays `UI_whoosh`.
    pub(crate) fn dismiss_options_help(&mut self) -> bool {
        if self.controls.options.help.take().is_some() {
            self.close_balloon_fade();
            self.play(WHOOSH);
            true
        } else {
            false
        }
    }

    /// The HUD's artwork, for tests that add a bitmap.
    #[cfg(test)]
    pub(crate) fn art_mut(&mut self) -> &mut Art {
        &mut self.art
    }

    /// Draws the tutorial highlight marker over `target` on `canvas`,
    /// `elapsed_us` after it was shown ([`sims_ui::tutorial`]). Returns
    /// whether it drew (not without its bitmap).
    pub fn draw_tutorial_marker(&self, canvas: &mut Canvas, target: Rect, elapsed_us: u64) -> bool {
        let (width, height) = (
            u32::try_from(canvas.width).unwrap_or(u32::MAX),
            u32::try_from(canvas.height).unwrap_or(u32::MAX),
        );
        match Frame::from_packed(width, height, std::mem::take(&mut canvas.color)) {
            Ok(mut frame) => {
                let drawn = sims_ui::tutorial::draw(&mut frame, &self.art, target, elapsed_us);
                canvas.color = frame.into_packed();
                drawn
            }
            Err(pixels) => {
                canvas.color = pixels;
                false
            }
        }
    }

    /// Dismisses any open help balloon (Live or Options, `0x50fb7f`) and plays
    /// `UI_whoosh`. Returns true if a balloon was open.
    pub fn dismiss_help(&mut self) -> bool {
        let live = self.controls.help.take().is_some();
        let opts = self.controls.options.help.take().is_some();
        if live || opts {
            self.close_balloon_fade();
            self.play(WHOOSH);
            true
        } else {
            false
        }
    }

    /// A press at `point`: the command it asks for, if any. Commands that
    /// change only the panel are applied here too. A gadget button's
    /// press is checked against `world` and `residential` (whether the lot
    /// is, `sims_vm::Lot::residential`) as they are at the press, not as
    /// the frame drew them ([`hud::check_press`]).
    pub fn press(&mut self, point: [i32; 2], world: &World, residential: bool) -> Option<Command> {
        // A non-modal dialog takes only the presses on it; any other goes on
        // to what is under it.
        let non_modal = self.has_non_modal_dialog();
        self.non_modal_press = false;
        if let Some(layout) = self
            .dialog
            .clone()
            .filter(|l| !non_modal || l.rect.contains(point[0], point[1]))
        {
            self.non_modal_press = non_modal;
            // 0x5249f9: a mouse button going down on the simulation
            // dialog stops its timer until the release.
            if self.ui_dialog.is_none() && layout.rect.contains(point[0], point[1]) {
                self.auto_answer.press();
            }
            if let Some(message) = self.active_message().cloned()
                && self.dialog_state.click_entry(
                    &layout,
                    &message,
                    &self.art,
                    point,
                    self.shift_held,
                )
            {
                self.dialog_state.pressed_button = None;
                return None;
            }
            // 0x5e44b4..0x5e45fc: cTSWinBtn captures on mouse-down and marks
            // the button pressed; activation happens on matching mouse-up
            // (`Hud::release_dialog`) if the pointer is still inside it.
            let hit = layout
                .buttons
                .iter()
                .position(|r| r.contains(point[0], point[1]))
                .filter(|&idx| !self.dialog_state.is_disabled(idx));
            self.dialog_state.dragging_entry = None;
            self.dialog_state.pressed_button = hit;
            return None;
        }
        // A modal dialog that has not yet been drawn still captures every
        // press so underlying controls cannot fire through it.
        if self.has_dialog() {
            return None;
        }
        // A press on an open pie menu only marks the button held
        // (0x5eb3d1 with +0x170 clear); choosing happens on release
        // (`Hud::release_menu`, 0x5eb433).
        if let Some(menu) = &mut self.menu {
            menu.press();
            return None;
        }
        // The status band is drawn over everything else on the lot,
        // the icons too, and takes the press.
        if self.band.is_some_and(|b| b.contains(point[0], point[1])) {
            self.dismiss_help();
            return None;
        }
        if self.camera_strip_open
            && let Some(strip) = self.camera_strip_rect
        {
            let state = self.camera_state();
            if self.camera_panel.press(
                point[0],
                point[1],
                strip,
                &state,
                &self.camera_strings,
                &self.art,
            ) {
                self.dismiss_help();
                return None;
            }
        }
        if self
            .saves_button_rect
            .is_some_and(|r| r.contains(point[0], point[1]))
        {
            self.pressed_saves_button = true;
            return None;
        }
        if self
            .coop_button_rect
            .is_some_and(|r| r.contains(point[0], point[1]))
        {
            self.pressed_coop_button = true;
            return None;
        }
        if self.controls.mode == Mode::Live
            && let Some(command) = self.press_queue(point)
        {
            self.dismiss_help();
            return command;
        }
        if self.controls.mode == Mode::Buy
            && self.controls.expanded
            && let Some(command) = self.buy_layout.press(point)
        {
            self.buy_state.apply(&command);
            return Some(command);
        }
        if self.controls.mode == Mode::Build
            && self.controls.expanded
            && let Some(command) = self.build_layout.press(point)
        {
            return Some(command);
        }
        if let Some((channel, _)) = hud::slider_at(&self.items, point) {
            self.dragging_slider = Some(channel);
        }
        let command = match hud::press(&self.items, point) {
            Some(command) => command,
            None => {
                // If Interests/Gifts was laid out disabled (e.g. while a pet
                // was selected) and selection switched to a human before
                // redraw, the press still opens the panel (0x51c5a6).
                if let Some(panel) = hud::disabled_panel(&self.items, point) {
                    let snapshot = Snapshot::read(world, self.members(world));
                    if Selected::of(&snapshot) == Selected::Human {
                        Command::OpenPanel(panel)
                    } else {
                        return None;
                    }
                } else {
                    if !hud::in_subpanel(&self.items, point) && !hud::on_balloon(&self.items, point)
                    {
                        self.dismiss_help();
                    }
                    return None;
                }
            }
        };
        let command = match command {
            Command::OpenPanel(_) => {
                let snapshot = Snapshot::read(world, self.members(world));
                let selected = Selected::of(&snapshot);
                hud::check_press(command, selected, residential)
            }
            // Guard stale subpanel hit rects before redraw so a hidden control
            // (e.g. zodiac on a pet, Report Card on an adult, or Fame on a
            // regular career) does not open a balloon or play `UI_whoosh`.
            Command::Help(help) => {
                let snapshot = self.household(world, residential, self.drawn_selection);
                if !hud::help_matches_snapshot(help, &snapshot, self.controls.panel) {
                    return None;
                }
                command
            }
            other => other,
        };
        if command == (Command::Refused { error: true }) {
            self.play(ERROR);
        }
        // `0x514170..0x514176` + `0x5e45d2..0x5e45f3`: Options action buttons
        // (`Save`, `Neighborhood`, `Quit`, `Reset Tutorial`) capture on press
        // and dispatch on mouse-up inside the same button so dragging off
        // cancels them cleanly.
        if let Command::OptionsAction(action) = command {
            self.pressed_options_action = Some(action);
            self.controls.pressed_options_action = Some(action);
            return None;
        }
        if matches!(
            command,
            Command::OpenOptionsTab(_) | Command::ToggleOption(_) | Command::SetDetail { .. }
        ) {
            self.play(CLICK);
        }
        let open = self.controls.help.is_some() || self.controls.options.help.is_some();
        self.controls.apply(&command);
        self.balloon_whoosh(open);
        Some(command)
    }

    /// A left release on an open pie menu at `point` (0x5eb433): settles
    /// the opening press, opens or leaves a submenu, chooses an entry, or
    /// closes the menu.
    pub fn release_menu(&mut self, point: [i32; 2]) -> (Release, Option<Command>) {
        self.release_menu_at(point, 0)
    }

    /// Like [`Hud::release_menu`], driving submenu transitions on
    /// [`sims_people::pie_head::PieAnimState`] at `now_ms`.
    pub fn release_menu_at(&mut self, point: [i32; 2], now_ms: u32) -> (Release, Option<Command>) {
        let Some(mut menu) = self.menu.take() else {
            return (Release::Closed, None);
        };
        let release = menu.release_at(&self.art, point, now_ms);
        match release {
            Release::Chose(choice) => {
                let command =
                    u16::try_from(choice)
                        .ok()
                        .map(|interaction| Command::ChooseInteraction {
                            sim: menu.person,
                            object: menu.object,
                            interaction,
                        });
                (release, command)
            }
            Release::Settled => {
                // Settling does not create a new cursor warp (`0x5eb53c`);
                // leave the pointer at the release coordinate rather than
                // re-applying an unconsumed opening warp.
                self.menu = Some(menu);
                (release, None)
            }
            Release::Stays => {
                if let Some(point) = menu.pending_pointer() {
                    self.controls.pointer = Some(point);
                }
                self.menu = Some(menu);
                (release, None)
            }
            Release::Closed => (release, None),
        }
    }

    /// A left release on an open pie menu while the pointer is outside the
    /// window: settles a moved opening menu or closes the pie.
    pub fn release_menu_outside(&mut self) -> Release {
        let Some(mut menu) = self.menu.take() else {
            return Release::Closed;
        };
        let release = menu.release_outside();
        if release == Release::Settled {
            self.menu = Some(menu);
        }
        release
    }
}

/// The residential Buy Mode catalog with its popup rating, sell prompt and
/// category button labels, loaded on first use.
fn load_buy_catalog(
    assets: &Assets,
    rating_labels: &RatingLabels,
    sell_labels: &SellLabels,
    category_labels: Option<&BuyLabels>,
) -> BuyCatalog {
    let catalog = BuyCatalog::load(assets.index(), assets.catalog())
        .with_rating_labels(rating_labels.clone())
        .with_sell_labels(sell_labels.clone());
    match category_labels {
        Some(labels) => catalog.with_category_labels(labels.clone()),
        None => catalog,
    }
}

/// The actions whose icons were pressed while they show: a second press
/// on an icon does nothing (0x4cb148, the icon's +0xf1).
#[derive(Debug, Default)]
struct Pressed(BTreeSet<i32>);

impl Pressed {
    /// Notes a press on `uid`'s icon; false if it was pressed already.
    fn press(&mut self, uid: i32) -> bool {
        self.0.insert(uid)
    }

    /// Forgets the press on `uid`'s icon.
    fn release(&mut self, uid: i32) {
        self.0.remove(&uid);
    }

    /// Keeps the presses of the icons `queue` shows: an icon gone takes
    /// its press with it.
    fn keep(&mut self, queue: &Queue) {
        self.0
            .retain(|uid| queue.icons.iter().any(|icon| icon.uid == *uid));
    }
}

/// Native clock-control blink state (`[control_panel + 0x1e4]`,
/// `0x53b4b0..0x53b7e7`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ClockState {
    /// State 0: unpaused in Live mode, no 500 ms timer running.
    #[default]
    Live,
    /// State 1: unpaused outside Live mode (steady red Pause button).
    OtherMode,
    /// State 2: paused; toggles Pause row 0 <-> row 1 every 500 ms.
    Paused,
    /// State 3: unpaused in Live mode with `[control_panel + 0x1ec]` set;
    /// toggles Ultra selected every 500 ms whenever `speed != Some(Speed::Ultra)`
    /// (and re-arms without toggling while `speed == Some(Speed::Ultra)`).
    IdleUltra,
}

/// The job panel performance's 333 ms blink (`0x5306df`, `0x52c659`,
/// `0x53070d`): it starts unlit, turns lit one period after it starts and
/// flips each period after, and stops unlit as soon as it is no longer
/// wanted. While it stays wanted its phase is kept (`0x5306df` does
/// nothing when `[gadget + 0x1a5]` already says it runs).
#[derive(Debug, Default)]
struct PerformanceBlink {
    since: Option<u64>,
    lit: bool,
}

impl PerformanceBlink {
    /// Advances the blink to `clock_us`, `active` while it is wanted;
    /// returns whether the color changed.
    fn sync(&mut self, active: bool, clock_us: u64) -> bool {
        if !active {
            self.since = None;
            return std::mem::take(&mut self.lit);
        }
        let Some(since) = self.since else {
            self.since = Some(clock_us);
            return false;
        };
        if clock_us.saturating_sub(since) < hud::PERFORMANCE_BLINK_US {
            return false;
        }
        self.lit = !self.lit;
        self.since = Some(clock_us);
        true
    }

    fn next_blink_us(&self, clock_us: u64) -> Option<u64> {
        let since = self.since?;
        Some(
            hud::PERFORMANCE_BLINK_US
                .saturating_sub(clock_us.saturating_sub(since))
                .max(1_000),
        )
    }
}

/// Shared 500 ms clock blink timer (`0x53b4b0..0x53b7e7`, `0x53b0d3..0x53b16b`).
#[derive(Debug, Default)]
struct ClockBlink {
    state: ClockState,
    since: Option<u64>,
    lit: bool,
    was_idle_speed: bool,
    last_speed_pair: Option<(i16, i16)>,
    ultra_lit: bool,
}

impl ClockBlink {
    fn pause_lit(&self) -> bool {
        self.state == ClockState::Paused && self.lit
    }

    fn ultra_lit(&self) -> bool {
        self.ultra_lit
    }

    fn next_blink_us(&self, clock_us: u64) -> Option<u64> {
        if !matches!(self.state, ClockState::Paused | ClockState::IdleUltra) {
            return None;
        }
        match self.since {
            None => Some(hud::PAUSE_BLINK_US),
            Some(since) => {
                let rem = hud::PAUSE_BLINK_US.saturating_sub(clock_us.saturating_sub(since));
                Some(rem.max(1_000))
            }
        }
    }

    fn reset(&mut self) {
        self.state = ClockState::Live;
        self.since = None;
        self.lit = false;
        self.was_idle_speed = false;
        self.last_speed_pair = None;
        self.ultra_lit = false;
    }

    fn sync(
        &mut self,
        paused: bool,
        mode: sims_ui::command::Mode,
        idle_speed: bool,
        speed: Option<sims_ui::command::Speed>,
        speed_pair: (i16, i16),
        clock_us: u64,
    ) -> bool {
        let before = (self.pause_lit(), self.ultra_lit);
        let entered_idle = !self.was_idle_speed && idle_speed;
        self.was_idle_speed = idle_speed;
        let speed_changed = self.last_speed_pair.is_some_and(|prev| prev != speed_pair);
        self.last_speed_pair = Some(speed_pair);

        let next = if paused {
            ClockState::Paused
        } else if mode != sims_ui::command::Mode::Live {
            ClockState::OtherMode
        } else if !idle_speed {
            ClockState::Live
        } else {
            match self.state {
                // 0x53b6a6..0x53b6ac: stay in state 3 while `[+0x1ec] != 0`.
                ClockState::IdleUltra => ClockState::IdleUltra,
                // 0x53b56f..0x53b57b: `0x109(1)` from state 0 enters state 3
                // unconditionally (even if return speed is already Ultra);
                // a speed-only refresh in state 0 (`0x53b547`) stays in state 0.
                ClockState::Live => {
                    if entered_idle {
                        ClockState::IdleUltra
                    } else {
                        ClockState::Live
                    }
                }
                // 0x53b5ce -> 0x53b627..0x53b645: returning from Pause or
                // non-Live enters state 3 only when return speed != Ultra.
                ClockState::Paused | ClockState::OtherMode => {
                    if speed != Some(sims_ui::command::Speed::Ultra) {
                        ClockState::IdleUltra
                    } else {
                        ClockState::Live
                    }
                }
            }
        };

        if self.state != next {
            self.state = next;
            self.lit = false;
            self.since =
                matches!(next, ClockState::Paused | ClockState::IdleUltra).then_some(clock_us);
            self.ultra_lit = false;
            return (self.pause_lit(), self.ultra_lit) != before;
        }

        // 0x53b0d3 -> 0x53b121..0x53b128: a speed refresh (such as Special
        // Effect `0x10c`) preserves state 3 and its running 500 ms timer,
        // while clearing Ultra's selected bit before the next callback.
        if self.state == ClockState::IdleUltra && speed_changed {
            self.lit = false;
        }

        if matches!(self.state, ClockState::Paused | ClockState::IdleUltra) {
            match self.since {
                None => {
                    self.since = Some(clock_us);
                    self.lit = false;
                }
                Some(since) if clock_us.saturating_sub(since) >= hud::PAUSE_BLINK_US => {
                    // 0x53b4c9..0x53b52e -> 0x5daf1c..0x5daf31: a single timer
                    // callback toggles once (skipping toggle while state 3 has
                    // Ultra selected, 0x53b4e4 -> 0x53b521) and re-arms 500 ms
                    // from the current clock, even after a long stall.
                    if self.state == ClockState::Paused
                        || speed != Some(sims_ui::command::Speed::Ultra)
                    {
                        self.lit = !self.lit;
                    } else {
                        self.lit = false;
                    }
                    self.since = Some(clock_us);
                }
                Some(_) => {}
            }
        }
        self.ultra_lit = self.state == ClockState::IdleUltra
            && speed != Some(sims_ui::command::Speed::Ultra)
            && self.lit;
        (self.pause_lit(), self.ultra_lit) != before
    }
}

/// Scales/crops a `sims_render::Image` (RGBA) to fit cleanly inside a 41×41
/// Build Mode catalog swatch (`sims_formats::bitmap::Image`).
fn render_image_to_swatch(src: &sims_render::Image) -> sims_formats::bitmap::Image {
    let max_dim = 41usize;
    let sw = src.width.max(1);
    let sh = src.height.max(1);
    let scale = (max_dim as f32 / sw as f32)
        .min(max_dim as f32 / sh as f32)
        .min(1.0);
    let dw = ((sw as f32 * scale).round() as usize).clamp(1, max_dim);
    let dh = ((sh as f32 * scale).round() as usize).clamp(1, max_dim);
    let mut pixels = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        let sy = ((y * sh) / dh).min(sh - 1);
        for x in 0..dw {
            let sx = ((x * sw) / dw).min(sw - 1);
            let si = (sy * sw + sx) * 4;
            let di = (y * dw + x) * 4;
            pixels[di..di + 4].copy_from_slice(&src.rgba[si..si + 4]);
        }
    }
    sims_formats::bitmap::Image {
        width: dw as u32,
        height: dh as u32,
        rgba: pixels,
    }
}

/// Side of the square a wallpaper swatch is fitted into: the 41 px slot
/// interior less a 2 px margin (oracle `build-wallpaper.png`: 19.7×37 at
/// 615.6..635.4, 677..713 in a slot at 603, 673).
const WALLPAPER_SWATCH_BOX: f32 = 37.0;

/// A wallpaper's catalog swatch: the pattern's flat, camera-facing frame
/// (frame 2, the diagonal wall) scaled to fit [`WALLPAPER_SWATCH_BOX`],
/// centred in a [`sims_ui::thumbnail::SIZE`] slot, with area-averaged
/// sampling so its edges blend like the original's.
fn render_wallpaper_swatch(src: &sims_render::Image) -> sims_formats::bitmap::Image {
    let cell = sims_ui::thumbnail::SIZE as usize;
    let (sw, sh) = (src.width.max(1) as f32, src.height.max(1) as f32);
    let scale = (WALLPAPER_SWATCH_BOX / sw).min(WALLPAPER_SWATCH_BOX / sh);
    let ox = (cell as f32 - sw * scale) / 2.0;
    let oy = (cell as f32 - sh * scale) / 2.0;
    let area = 1.0 / (scale * scale);
    let mut rgba = vec![0u8; cell * cell * 4];
    for v in 0..cell {
        let y0 = ((v as f32 - oy) / scale).max(0.0);
        let y1 = ((v as f32 + 1.0 - oy) / scale).min(sh);
        if y1 <= y0 {
            continue;
        }
        for u in 0..cell {
            let x0 = ((u as f32 - ox) / scale).max(0.0);
            let x1 = ((u as f32 + 1.0 - ox) / scale).min(sw);
            if x1 <= x0 {
                continue;
            }
            let (mut a_sum, mut rgb) = (0.0f32, [0.0f32; 3]);
            for sy in y0.floor() as usize..(y1.ceil() as usize).min(src.height) {
                let wy = (y1.min(sy as f32 + 1.0) - y0.max(sy as f32)).max(0.0);
                for sx in x0.floor() as usize..(x1.ceil() as usize).min(src.width) {
                    let wx = (x1.min(sx as f32 + 1.0) - x0.max(sx as f32)).max(0.0);
                    let i = (sy * src.width + sx) * 4;
                    let a = f32::from(src.rgba[i + 3]) / 255.0 * wx * wy;
                    a_sum += a;
                    for (c, value) in rgb.iter_mut().enumerate() {
                        *value += f32::from(src.rgba[i + c]) * a;
                    }
                }
            }
            if a_sum <= 0.0 {
                continue;
            }
            let di = (v * cell + u) * 4;
            for c in 0..3 {
                rgba[di + c] = (rgb[c] / a_sum).round().clamp(0.0, 255.0) as u8;
            }
            rgba[di + 3] = (a_sum / area * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    sims_formats::bitmap::Image {
        width: cell as u32,
        height: cell as u32,
        rgba,
    }
}

/// Masks a wall pattern sprite with a wall shape sprite and scales it into a
/// Build Mode catalog swatch.
fn render_wall_swatch(
    pat: &sims_render::Image,
    shape: &sims_render::Image,
) -> sims_formats::bitmap::Image {
    let w = pat.width.min(shape.width);
    let h = pat.height.min(shape.height);
    if w == 0 || h == 0 {
        return render_image_to_swatch(pat);
    }
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let pi = (y * pat.width + x) * 4;
            let si = (y * shape.width + x) * 4;
            let di = (y * w + x) * 4;
            let alpha = shape.rgba[si + 3];
            rgba[di] = pat.rgba[pi];
            rgba[di + 1] = pat.rgba[pi + 1];
            rgba[di + 2] = pat.rgba[pi + 2];
            rgba[di + 3] = ((u16::from(pat.rgba[pi + 3]) * u16::from(alpha)) / 255) as u8;
        }
    }
    let masked = sims_render::Image {
        width: w,
        height: h,
        rgba,
        depth: None,
    };
    render_image_to_swatch(&masked)
}

/// Reads neighborhood family surnames (`FAMs`) and net assets (`FAMI` budget +
/// architecture value, `0x410241` / `0x410271`) from `Neighborhood.iff` so
/// ordinary VM dialog `$Family` / `$FamilyAssets:N` tokens have live data.
fn load_neighborhood_families(
    neighborhood: &Neighborhood,
) -> std::collections::BTreeMap<i32, dialog::FamilyInfo> {
    let mut out = std::collections::BTreeMap::new();
    let mut surnames: std::collections::BTreeMap<u16, String> = std::collections::BTreeMap::new();
    let iff_path = neighborhood.directory.join("Neighborhood.iff");
    if let Ok(bytes) = std::fs::read(&iff_path)
        && let Ok(iff) = sims_formats::iff::Iff::parse(bytes)
    {
        for chunk in &iff.chunks {
            if chunk.kind() == "FAMs"
                && let Ok(table) = sims_formats::strings::StringTable::parse(iff.data(chunk))
                && let Some(name) = table
                    .get(0, US_ENGLISH)
                    .or_else(|| table.entries.first().map(|e| e.text.as_str()))
                && !name.is_empty()
            {
                surnames.insert(chunk.id, name.to_owned());
            }
        }
    }
    for (&id, fam) in &neighborhood.families {
        let name = surnames.remove(&id).unwrap_or_default();
        let assets = fam.budget.saturating_add(fam.architecture_value);
        out.insert(i32::from(id), dialog::FamilyInfo { name, assets });
    }
    out
}

/// Whether `fresh`, a dialog read again, says what `shown` says: the
/// same text, buttons and look. What the player typed into `shown`'s
/// fields is theirs, not the dialog's, so it is not compared.
fn same_dialog(shown: &Message, fresh: &Message) -> bool {
    let mut typed = shown.clone();
    typed.entry.clone_from(&fresh.entry);
    typed.secondary_entry.clone_from(&fresh.secondary_entry);
    typed == *fresh
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_ui::command::{Mode, Speed};
    use sims_ui::frame::Rect;
    use sims_ui::queue::{Icon, Picture};

    #[test]
    fn the_performance_blinks_green_every_333_ms_and_stops_unlit() {
        let ms = |n: u64| n * 1_000;
        let mut blink = PerformanceBlink::default();
        assert_eq!(blink.next_blink_us(0), None);
        assert!(!blink.sync(true, ms(10)));
        assert_eq!(blink.next_blink_us(ms(10)), Some(ms(333)));
        assert!(!blink.sync(true, ms(342)));
        assert!(blink.sync(true, ms(343)), "lit one period after it starts");
        assert!(blink.lit);
        assert!(!blink.sync(true, ms(500)));
        assert!(blink.sync(true, ms(676)));
        assert!(!blink.lit);
        assert!(blink.sync(true, ms(1009)));
        // When it is no longer wanted it goes out at once, and starts
        // unlit again later.
        assert!(blink.sync(false, ms(1010)));
        assert!(!blink.lit);
        assert_eq!(blink.next_blink_us(ms(1010)), None);
        assert!(!blink.sync(false, ms(2000)));
        assert!(!blink.sync(true, ms(2000)));
        assert!(!blink.lit);
    }

    #[test]
    fn pause_blink_toggles_every_500ms_and_resets_unlit() {
        let mut blink = ClockBlink::default();
        let normal = (sims_sim::speed::NORMAL, -1);
        let idle_normal = (sims_sim::speed::IDLE, sims_sim::speed::NORMAL);
        let idle_ultra = (sims_sim::speed::IDLE, sims_sim::speed::ULTRA);
        let reset_normal = (sims_sim::speed::NORMAL, sims_sim::speed::NORMAL);
        assert!(!blink.pause_lit());
        // Entering Live-mode pause starts unlit (cyan row 0, 0x53b707..0x53b74d).
        assert!(!blink.sync(
            true,
            Mode::Live,
            false,
            Some(Speed::Normal),
            normal,
            100_000
        ));
        assert!(!blink.pause_lit());
        // 499.999 ms later: still unlit.
        assert!(!blink.sync(
            true,
            Mode::Live,
            false,
            Some(Speed::Normal),
            normal,
            599_999
        ));
        assert!(!blink.pause_lit());
        // At 500 ms: flips to lit (red row 1, 0x53b500..0x53b528).
        assert!(blink.sync(
            true,
            Mode::Live,
            false,
            Some(Speed::Normal),
            normal,
            600_000
        ));
        assert!(blink.pause_lit());
        // At 1000 ms: flips back to unlit.
        assert!(blink.sync(
            true,
            Mode::Live,
            false,
            Some(Speed::Normal),
            normal,
            1_100_000
        ));
        assert!(!blink.pause_lit());
        // A 1-second stall (2x 500 ms) toggles once and re-arms 500 ms from
        // the current clock (0x53b500..0x53b52e -> 0x5daf1c..0x5daf31).
        assert!(blink.sync(
            true,
            Mode::Live,
            false,
            Some(Speed::Normal),
            normal,
            2_100_000
        ));
        assert!(blink.pause_lit());
        assert!(!blink.sync(
            true,
            Mode::Live,
            false,
            Some(Speed::Normal),
            normal,
            2_599_999
        ));
        assert!(blink.sync(
            true,
            Mode::Live,
            false,
            Some(Speed::Normal),
            normal,
            2_600_000
        ));
        assert!(!blink.pause_lit());
        // Unpausing clears lit and kills the timer; re-pausing starts fresh unlit.
        assert!(blink.sync(
            true,
            Mode::Live,
            false,
            Some(Speed::Normal),
            normal,
            3_100_000
        ));
        assert!(blink.pause_lit());
        assert!(blink.sync(
            false,
            Mode::Live,
            false,
            Some(Speed::Normal),
            normal,
            3_200_000
        ));
        assert!(!blink.pause_lit());
        assert!(!blink.sync(
            true,
            Mode::Live,
            false,
            Some(Speed::Normal),
            normal,
            3_400_000
        ));
        assert!(!blink.pause_lit());
        assert!(!blink.sync(
            true,
            Mode::Live,
            false,
            Some(Speed::Normal),
            normal,
            3_899_999
        ));
        assert!(blink.sync(
            true,
            Mode::Live,
            false,
            Some(Speed::Normal),
            normal,
            3_900_000
        ));
        assert!(blink.pause_lit());

        // Switching directly from lit Pause (state 2) to Ultra (state 3)
        // clears pause_lit, starts ultra_lit unlit, and toggles ultra_lit at
        // 500 ms (`0x53b6bc -> 0x53b757`, `0x53b4c9..0x53b4fe`).
        assert!(blink.sync(
            false,
            Mode::Live,
            true,
            Some(Speed::Normal),
            idle_normal,
            4_000_000
        ));
        assert!(!blink.pause_lit());
        assert!(!blink.ultra_lit());
        assert!(!blink.sync(
            false,
            Mode::Live,
            true,
            Some(Speed::Normal),
            idle_normal,
            4_499_999
        ));
        assert!(blink.sync(
            false,
            Mode::Live,
            true,
            Some(Speed::Normal),
            idle_normal,
            4_500_000
        ));
        assert!(blink.ultra_lit());

        // Scripted `0x10c` speed refresh at 4_700_000 clears Ultra's selected
        // bit (`0x53b121..0x53b128`) without resetting the running 500 ms
        // timer (`since = 4_500_000`), so the next callback at 5_000_000 turns
        // Ultra back on.
        assert!(blink.sync(
            false,
            Mode::Live,
            true,
            Some(Speed::Normal),
            reset_normal,
            4_700_000
        ));
        assert!(!blink.ultra_lit());
        assert!(!blink.sync(
            false,
            Mode::Live,
            true,
            Some(Speed::Normal),
            reset_normal,
            4_999_999
        ));
        assert!(blink.sync(
            false,
            Mode::Live,
            true,
            Some(Speed::Normal),
            reset_normal,
            5_000_000
        ));
        assert!(blink.ultra_lit());
        // Leaving idle speed clears state 3.
        assert!(blink.sync(
            false,
            Mode::Live,
            false,
            Some(Speed::Normal),
            normal,
            5_100_000
        ));
        assert!(!blink.ultra_lit());

        // Entering idle speed from state 0 with Ultra selected arms state 3
        // (`0x53b56f..0x53b57b`); callback at 5_700_000 skips toggle and
        // re-arms (`0x53b4e4 -> 0x53b521`). A scripted reset to Normal at
        // 5_900_000 then flips Ultra on at 6_200_000 (existing cadence, not
        // delayed by a fresh 500 ms).
        assert!(!blink.sync(
            false,
            Mode::Live,
            true,
            Some(Speed::Ultra),
            idle_ultra,
            5_200_000
        ));
        assert!(!blink.ultra_lit());
        assert!(!blink.sync(
            false,
            Mode::Live,
            true,
            Some(Speed::Ultra),
            idle_ultra,
            5_700_000
        ));
        assert!(!blink.ultra_lit());
        assert!(!blink.sync(
            false,
            Mode::Live,
            true,
            Some(Speed::Normal),
            reset_normal,
            5_900_000
        ));
        assert!(!blink.ultra_lit());
        assert!(!blink.sync(
            false,
            Mode::Live,
            true,
            Some(Speed::Normal),
            reset_normal,
            6_199_999
        ));
        assert!(blink.sync(
            false,
            Mode::Live,
            true,
            Some(Speed::Normal),
            reset_normal,
            6_200_000
        ));
        assert!(blink.ultra_lit());
    }

    #[test]
    fn an_icon_takes_one_press_until_released_or_gone() {
        let icon = |uid| Icon {
            uid,
            picture: Picture::Object(1),
            yellow: false,
            cancelled: false,
            rect: Rect::new(32, 32, 45, 45),
        };
        let mut pressed = Pressed::default();
        assert!(pressed.press(7));
        assert!(!pressed.press(7));
        // A cancel that failed lets the icon be pressed again.
        pressed.release(7);
        assert!(pressed.press(7));
        // Shown again, the press stays; gone, it goes.
        let mut queue = Queue {
            person: Some(1),
            pet: false,
            icons: vec![icon(7)],
        };
        pressed.keep(&queue);
        assert!(!pressed.press(7));
        queue.icons.clear();
        pressed.keep(&queue);
        assert!(pressed.press(7));
    }
}
