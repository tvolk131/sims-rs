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
use sims_content::{ArchivePool, Catalog, Index, Install, Neighborhood};
use sims_render::{Assets, Canvas, Walls, Zoom};
use sims_sim::World;
use sims_ui::art::Art;
use sims_ui::balloon::Texts;
use sims_ui::build::{
    self, BuildCatalog, BuildLayout, BuildState, BuildSubtoolAction, BuildSwatches,
    BuildWidgetKind, TerrainSubtool,
};
use sims_ui::buy::{
    self, BuyCatalog, BuyLayout, BuyState, BuyWidgetKind, RatingLabels, SellInfo, SellLabels,
};
use sims_ui::cheat_console::CheatConsole;
use sims_ui::command::{
    BuildItemId, Command, Mode, OptionsAction, Panel as UiPanel, VolumeChannel, Walls as UiWalls,
};
use sims_ui::dialog::{self, DialogState, Labels, Message};
use sims_ui::frame::{Frame, Rect};
use sims_ui::house;
use sims_ui::household::{Acquaintances, Careers, Faces, Fame, Snapshot};
use sims_ui::hud::{self, Controls, Item, Selected};
use sims_ui::options::{self, OptionsTexts};
use sims_ui::pie;
use sims_ui::queue::Queue;
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
}

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
    /// Whether the Camera-panel Photo Album button (`CamViewAlbum`, resource `1105`)
    /// is shown beside the snapshot button (enabled in `NeighborhoodShell` lots,
    /// disabled for standalone/ratchet renders so static Live frames stay bit-exact).
    album_button_enabled: bool,
    /// Whether the Camera-panel Photo Album button is currently pressed.
    pressed_album_button: bool,
    /// Last laid-out screen rect of the Camera-panel Photo Album button.
    album_button_rect: Option<Rect>,
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
    /// The clock controls' shared 500 ms blink timer (`0x53b4b0..0x53b7e7`):
    /// state 2 blinks Pause (row 0 <-> row 1) and state 3 blinks Ultra
    /// (`selected` false <-> true) during idle speed when return speed != Ultra.
    clock_blink: ClockBlink,
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
    /// The interface's sounds, until the game takes them.
    sounds: Vec<SoundEvent>,
}

/// Install-only HUD assets (art, UI strings, options defaults) that depend
/// solely on [`Install`] and can be loaded in parallel with lot boot.
pub(crate) struct HudStatic {
    art: Art,
    labels: Labels,
    rating_labels: RatingLabels,
    sell_labels: SellLabels,
    texts: Texts,
    fame: Fame,
    options_texts: OptionsTexts,
    options_state: options::OptionsState,
}

impl HudStatic {
    pub(crate) fn load(install: &Install, pool: Arc<ArchivePool>) -> Result<Self, String> {
        let (art_res, text_res) = std::thread::scope(|scope| {
            let text_handle = scope.spawn(|| -> Result<_, String> {
                let labels = Labels::load(install).map_err(|e| e.to_string())?;
                let rating_labels = RatingLabels::load(install);
                let sell_labels = SellLabels::load(install);
                // Without the balloons' or options' strings the HUD still opens with defaults.
                let texts = Texts::load(install).unwrap_or_default();
                let fame = Fame::load(install).unwrap_or_default();
                let options_texts = OptionsTexts::load(install).unwrap_or_default();
                let options_state =
                    options::OptionsState::load_install(install).unwrap_or_default();
                Ok((
                    labels,
                    rating_labels,
                    sell_labels,
                    texts,
                    fame,
                    options_texts,
                    options_state,
                ))
            });
            let art_res = (|| -> Result<Art, String> {
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
                    .chain(&[
                        options::id::SAVE_CARD,
                        sims_ui::neighborhood::art_id::POPUP_TILER,
                        sims_ui::album::CAM_VIEW_ALBUM,
                    ])
                    .copied()
                    .chain(buy::resource_ids())
                    .chain(build::resource_ids())
                    .collect();
                Art::load_with_optional(&ui, US_ENGLISH, &resources, &optional)
                    .map_err(|e| e.to_string())
            })();
            (
                art_res,
                text_handle.join().expect("hud text worker panicked"),
            )
        });
        let art = art_res?;
        let (labels, rating_labels, sell_labels, texts, fame, options_texts, options_state) =
            text_res?;
        Ok(Self {
            art,
            labels,
            rating_labels,
            sell_labels,
            texts,
            fame,
            options_texts,
            options_state,
        })
    }
}

/// Index/Catalog-dependent HUD tables (`work.iff` careers and house rating
/// tuning) that can be loaded in parallel with VM linking and animations.
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
        Self::open_with_members(install, members, assets, world, friendship)
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
        Self::from_preloaded_with_members(
            install,
            members,
            assets,
            world,
            friendship,
            static_part,
            shared_part,
            pool,
        )
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
        Self {
            art: static_part.art,
            boot_faces: faces.clone(),
            faces,
            thumbnails: Thumbnails::default(),
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
            dialog: None,
            dragging_slider: None,
            pressed_options_action: None,
            pressed_saves_button: false,
            saves_button_rect: None,
            pressed_coop_button: false,
            coop_button_rect: None,
            album_button_enabled: false,
            pressed_album_button: false,
            album_button_rect: None,
            roof_view: false,
            cheat_console: CheatConsole::new(),
            menu: None,
            band: None,
            queue: Queue::default(),
            pressed: Pressed::default(),
            clock_blink: ClockBlink::default(),
            idle_speed: false,
            buy_catalog: None,
            rating_labels: static_part.rating_labels,
            sell_labels: static_part.sell_labels,
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
            sounds: Vec::new(),
        }
    }

    /// Forgets what the panel kept of the lot it showed, for a lot put
    /// back to a save: the pie menu, the dialog read, the icons pressed,
    /// the Buy/Build Mode catalog state, and the pause button's blink timer. The
    /// rest is read from the lot at the next frame.
    pub fn forget(&mut self) {
        self.menu = None;
        self.message = None;
        self.ui_dialog = None;
        self.dialog_state = DialogState::default();
        self.dialog = None;
        self.dragging_slider = None;
        self.pressed_options_action = None;
        self.pressed_saves_button = false;
        self.saves_button_rect = None;
        self.pressed_coop_button = false;
        self.coop_button_rect = None;
        self.pressed_album_button = false;
        self.album_button_rect = None;
        self.roof_view = false;
        self.controls.pressed_options_action = None;
        self.controls.help = None;
        self.controls.options.tab = None;
        self.controls.options.help = None;
        self.items.clear();
        self.pressed = Pressed::default();
        self.clock_blink.reset();
        self.idle_speed = false;
        self.controls.pause_lit = false;
        self.controls.ultra_lit = false;
        self.buy_state.reset();
        self.buy_layout = BuyLayout::default();
        self.build_state.reset();
        self.build_layout = BuildLayout::default();
    }

    /// Loads the residential Buy Mode catalog on first use and returns it.
    pub fn ensure_buy_catalog(&mut self, assets: &Assets) -> &BuyCatalog {
        let rating_labels = self.rating_labels.clone();
        let sell_labels = self.sell_labels.clone();
        self.buy_catalog.get_or_insert_with(|| {
            BuyCatalog::load(assets.index(), assets.catalog())
                .with_rating_labels(rating_labels)
                .with_sell_labels(sell_labels)
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
    /// syncing `self.buy_state.page` to the clamped page number.
    pub fn layout_buy(
        &mut self,
        size: [i32; 2],
        world: &World,
        assets: &Assets,
        sell: Option<&SellInfo>,
    ) {
        if self.controls.mode != Mode::Buy || !self.controls.expanded {
            self.buy_layout = BuyLayout::default();
            return;
        }
        let rating_labels = self.rating_labels.clone();
        let sell_labels = self.sell_labels.clone();
        let catalog = self.buy_catalog.get_or_insert_with(|| {
            BuyCatalog::load(assets.index(), assets.catalog())
                .with_rating_labels(rating_labels)
                .with_sell_labels(sell_labels)
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

    /// Enables or disables the Camera-panel Photo Album button (`CamViewAlbum`, `1105`).
    pub fn set_album_button_enabled(&mut self, enabled: bool) {
        self.album_button_enabled = enabled;
        if !enabled {
            self.album_button_rect = None;
            self.pressed_album_button = false;
        }
    }

    /// Screen rectangle of the Camera-panel Photo Album button, when shown.
    pub fn album_button_rect(&self) -> Option<Rect> {
        self.album_button_rect
    }

    /// Releases a held Camera-panel Photo Album button, returning `(changed, activated)`.
    pub fn release_album_button(&mut self, point: Option<[i32; 2]>) -> (bool, bool) {
        if !self.pressed_album_button {
            return (false, false);
        }
        self.pressed_album_button = false;
        let activated = point.is_some_and(|[x, y]| {
            self.album_button_rect
                .is_some_and(|rect| rect.contains(x, y))
        });
        if activated {
            self.play(CLICK);
        }
        (true, activated)
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
                BuildItemId::Wallpaper(key) => {
                    if let Some(frames) = assets.pattern(key, 0) {
                        Some(render_image_to_swatch(&frames[0]))
                    } else {
                        assets
                            .pattern(key, 1)
                            .map(|frames| render_image_to_swatch(&frames[0]))
                    }
                }
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
    pub fn relayout_panel(
        &mut self,
        size: [i32; 2],
        world: &World,
        residential: bool,
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
        self.layout_buy(size, world, assets, sell);
        self.layout_build(size, world, assets, false, false);
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
        };
        self.dialog_state = DialogState::for_message(&message);
        self.dialog = dialog::layout(view_size, &message, &self.art);
        self.ui_dialog = Some((kind, message));
        self.menu = None;
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

    /// Moves the active dialog's text-entry caret left.
    pub fn dialog_move_left(&mut self) -> bool {
        self.active_message()
            .cloned()
            .is_some_and(|m| self.dialog_state.move_left(&m))
    }

    /// Moves the active dialog's text-entry caret right.
    pub fn dialog_move_right(&mut self) -> bool {
        self.active_message()
            .cloned()
            .is_some_and(|m| self.dialog_state.move_right(&m))
    }

    /// Moves the active dialog's text-entry caret one visual row up (multiline
    /// fields, `0x5f29f2..0x5f2a90`).
    pub fn dialog_move_up(&mut self) -> bool {
        self.active_message().cloned().is_some_and(|m| {
            self.dialog_state
                .move_up(&m, self.dialog.as_ref(), &self.art)
        })
    }

    /// Moves the active dialog's text-entry caret one visual row down (multiline
    /// fields, `0x5f2a90..0x5f2b40`).
    pub fn dialog_move_down(&mut self) -> bool {
        self.active_message().cloned().is_some_and(|m| {
            self.dialog_state
                .move_down(&m, self.dialog.as_ref(), &self.art)
        })
    }

    /// Moves the active dialog's text-entry caret to the start of the current
    /// line (or field, `0x5f2c13`).
    pub fn dialog_home(&mut self) -> bool {
        self.active_message().cloned().is_some_and(|m| {
            self.dialog_state
                .home_in_layout(&m, self.dialog.as_ref(), Some(&self.art))
        })
    }

    /// Moves the active dialog's text-entry caret to the end of the current
    /// line (or field, `0x5f2c13`).
    pub fn dialog_end(&mut self) -> bool {
        self.active_message().cloned().is_some_and(|m| {
            self.dialog_state
                .end_in_layout(&m, self.dialog.as_ref(), Some(&self.art))
        })
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
        let now_open = self.controls.help.is_some() || self.controls.options.help.is_some();
        if now_open != open {
            self.play(WHOOSH);
        }
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
        self.dialog_state = DialogState::default();
        Some((kind, button, message.entry, message.secondary_entry))
    }

    /// Whether a UI confirmation dialog (outside the simulation) is currently open.
    pub fn has_ui_dialog(&self) -> bool {
        self.ui_dialog.is_some()
    }

    /// Whether any modal dialog (UI confirmation, loaded VM dialog, or drawn
    /// layout) is currently active, even before the next [`Hud::draw`].
    pub fn has_dialog(&self) -> bool {
        self.ui_dialog.is_some() || self.message.is_some() || self.dialog.is_some()
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
        changed
    }

    /// Clears the cached VM dialog and its input state after answering so a
    /// consecutive identical prompt (`(caller, operands)`) initializes fresh
    /// text state (`0x523825..0x523887`).
    pub fn clear_vm_dialog(&mut self) {
        self.message = None;
        if self.ui_dialog.is_none() {
            self.dialog = None;
            self.dialog_state = DialogState::default();
        }
    }

    /// Reads the dialog `world` waits on, if it changed. The lot's world,
    /// or, while the lot waits for the player, the world as the tick left
    /// it when it showed the dialog.
    pub fn read_dialog(&mut self, world: &World, assets: &Assets) {
        let Some(pending) = dialog::pending(world) else {
            self.message = None;
            return;
        };
        let key = (pending.caller, pending.operands);
        if self.message.as_ref().is_some_and(|(_, k)| *k == key) {
            return;
        }
        self.message = Message::read(
            world,
            assets.index(),
            assets.catalog(),
            &self.labels,
            pending,
        )
        .ok()
        .flatten()
        .map(|m| {
            if self.ui_dialog.is_none() {
                self.dialog_state = DialogState::for_message(&m);
            }
            (m, key)
        });
    }

    /// What the panel shows of `world`: the household with what the next
    /// level of each one's career needs, zoning-aware family friends
    /// (`0x4cfcdb`), and, while the relationship panel is open, the people
    /// the selected Sim knows (read only then: the lookup goes through
    /// every neighborhood record), or while the house panel shows (the
    /// strip open on it), the house's ratings.
    pub fn snapshot(&self, world: &World, residential: bool) -> Snapshot {
        self.for_panel(self.household(world, residential), world)
    }

    /// The active household's live member roster (`0x4e97dc`): reads the
    /// family named by global 9 from `world.families` when families are
    /// loaded, falling back to the neighborhood roster opened from disk.
    fn members<'a>(&'a self, world: &'a World) -> &'a [u32] {
        if world.families.is_empty() {
            return &self.members;
        }
        let id = i32::from(world.globals.get(9).copied().unwrap_or(0));
        if id == 0 {
            return &[];
        }
        world
            .families
            .iter()
            .find(|f| f.id == id)
            .map_or(&[], |f| f.members.as_slice())
    }

    /// The household with zoning-aware family friends, career needs/grades,
    /// and Superstar fame progression data.
    fn household(&self, world: &World, residential: bool) -> Snapshot {
        Snapshot::read(world, self.members(world))
            .with_idle_speed(self.effective_idle_speed(world))
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
        _pace: &Pace,
        status: Option<&str>,
        sell: Option<&SellInfo>,
        canvas: &mut Canvas,
    ) {
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
        let household = self.household(world, residential);
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
        if self.controls.expanded && residential && self.controls.panel == Some(UiPanel::House) {
            self.kinds.learn(world, assets.index(), assets.catalog());
        }
        // The pause button shows global 17, which the game's pause sets.
        let snapshot = self.for_panel(household, world);
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
        self.queue = Queue::read(world, snapshot.selected);
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
        let mut frame = match Frame::from_rgb(width, height, std::mem::take(&mut canvas.color)) {
            Ok(frame) => frame,
            Err(rgb) => {
                canvas.color = rgb;
                return;
            }
        };
        hud::draw_with_thumbnails(
            &mut frame,
            &self.art,
            &self.faces,
            Some(&self.thumbnails),
            &self.items,
            &self.controls,
        );
        self.saves_button_rect = self.compute_options_saves_button_rect(view.size);
        if let Some(rect) = self.saves_button_rect {
            self.draw_options_strip_button(&mut frame, rect, "Saves", self.pressed_saves_button);
        }
        self.coop_button_rect = self.compute_options_coop_button_rect(view.size);
        if let Some(rect) = self.coop_button_rect {
            self.draw_options_strip_button(&mut frame, rect, "Co-op", self.pressed_coop_button);
        }
        self.album_button_rect = self.compute_camera_album_button_rect();
        if let Some(rect) = self.album_button_rect {
            self.draw_camera_album_button(&mut frame, rect);
        }
        if self.controls.mode == Mode::Buy && self.controls.expanded {
            self.layout_buy(view.size, world, assets, sell);
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
        if self.controls.mode == Mode::Live {
            self.queue
                .draw(&mut frame, &self.art, &self.faces, &self.thumbnails);
        }
        self.dialog = self
            .active_message()
            .and_then(|m| dialog::layout(view.size, m, &self.art));
        // A dialog takes every press until it is answered, so a menu open
        // under it is closed rather than left to be chosen from after.
        if self.dialog.is_some() {
            self.menu = None;
        }
        if let (Some(message), Some(layout)) = (self.active_message(), &self.dialog) {
            dialog::draw_with_state(
                &mut frame,
                &self.art,
                message,
                layout,
                self.controls.pointer,
                &self.dialog_state,
            );
        }
        if let Some(menu) = &mut self.menu {
            menu.layout(&self.art, view.size);
            menu.draw_labels(&mut frame, &self.art, self.controls.pointer);
        }
        self.band = status.and_then(|status| self.status(&mut frame, status));
        if self.cheat_console.is_open() {
            self.cheat_console.draw(&mut frame, &self.art);
        }
        canvas.color = frame.into_rgb();
    }

    fn compute_options_saves_button_rect(&self, size: [i32; 2]) -> Option<Rect> {
        if self.controls.mode != Mode::Options || !self.controls.expanded {
            return None;
        }
        let quit = self
            .items
            .iter()
            .find(|i| i.command == Some(Command::OptionsAction(OptionsAction::Quit)))?;
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
        let left = quit.rect.x + quit.rect.width + 4;
        let avail = divider_x - left - 3;
        if avail >= 98 {
            Some(Rect::new(left, quit.rect.y + 6, 46, 28))
        } else if avail >= 42 {
            Some(Rect::new(left, quit.rect.y + 2, avail.min(48), 22))
        } else {
            // Compact window fallback: sit neatly in the bottom-right corner of the strip.
            let x = (size[0] - 54).max(divider_x + 4);
            let y = (size[1] - 26).max(0);
            Some(Rect::new(x, y, 48, 22))
        }
    }

    fn compute_options_coop_button_rect(&self, size: [i32; 2]) -> Option<Rect> {
        if self.controls.mode != Mode::Options || !self.controls.expanded {
            return None;
        }
        let quit = self
            .items
            .iter()
            .find(|i| i.command == Some(Command::OptionsAction(OptionsAction::Quit)))?;
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
        let left = quit.rect.x + quit.rect.width + 4;
        let avail = divider_x - left - 3;
        if avail >= 98 {
            Some(Rect::new(left + 50, quit.rect.y + 6, 46, 28))
        } else if avail >= 42 {
            Some(Rect::new(left, quit.rect.y + 26, avail.min(48), 22))
        } else {
            let saves_x = (size[0] - 54).max(divider_x + 4);
            let y = (size[1] - 26).max(0);
            Some(Rect::new((saves_x - 52).max(0), y, 48, 22))
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

    fn compute_camera_album_button_rect(&self) -> Option<Rect> {
        if !self.album_button_enabled || !self.controls.expanded {
            return None;
        }
        let photo = self
            .items
            .iter()
            .find(|i| i.command == Some(Command::Camera(sims_ui::command::Camera::Photo)))?;
        Some(Rect::new(
            (photo.rect.x - 33).max(0),
            photo.rect.y,
            photo.rect.width,
            photo.rect.height,
        ))
    }

    fn draw_camera_album_button(&self, frame: &mut Frame, rect: Rect) {
        let hovered = self
            .controls
            .pointer
            .is_some_and(|[px, py]| rect.contains(px, py));
        if let Some(strip) = self.art.image(sims_ui::album::CAM_VIEW_ALBUM)
            && strip.width >= 4
            && strip.height > 0
        {
            let cell_w = strip.width / 4;
            let state = if self.pressed_album_button && hovered {
                2
            } else if hovered {
                3
            } else {
                1
            };
            let src = Rect::new(
                (cell_w * state) as i32,
                0,
                cell_w as i32,
                strip.height as i32,
            );
            let dx = rect.x + (rect.width - cell_w as i32) / 2;
            let dy = rect.y + (rect.height - strip.height as i32) / 2;
            frame.draw(strip, src, [dx, dy]);
            return;
        }
        let (bg, border, fg) = if self.pressed_album_button && hovered {
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
        if let Some(font) = self.art.font(10) {
            let label = "A";
            let tx = rect.x + (rect.width - font.width(label)) / 2;
            let ty = rect.y + (rect.height - font.line_height) / 2;
            font.draw(frame, label, [tx, ty], fg);
        }
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

    /// Where the dialog drawn last is, and its buttons.
    pub fn dialog(&self) -> Option<&dialog::Layout> {
        self.dialog.as_ref()
    }

    /// Whether the panel or the status band covers `point` (canvas
    /// pixels), or a dialog or a pie menu is shown and takes every press.
    pub fn covers(&self, point: [i32; 2]) -> bool {
        self.has_dialog()
            || self.menu.is_some()
            || self.band.is_some_and(|b| b.contains(point[0], point[1]))
            || self
                .saves_button_rect
                .is_some_and(|r| r.contains(point[0], point[1]))
            || self
                .coop_button_rect
                .is_some_and(|r| r.contains(point[0], point[1]))
            || self
                .album_button_rect
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
        let icon = self.queue.at(point)?;
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
            let a = pointer
                .is_some_and(|[x, y]| self.album_button_rect.is_some_and(|r| r.contains(x, y)));
            (s, c, a)
        };
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
        let hovered_changed = (
            hud::hovered(&self.items, self.controls.pointer),
            hud::lit_relation(&self.items, self.controls.pointer),
            button(self.controls.pointer),
            entry(self.controls.pointer),
            buy_widget(self.controls.pointer),
            build_widget(self.controls.pointer),
            over_saves(self.controls.pointer),
        ) != before;
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
        let dismissed = self.dismiss_help();
        left || dismissed
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

    /// Dismisses any open live-mode help balloon (`0x50fb7f`) and plays `UI_whoosh`.
    fn dismiss_live_help(&mut self) -> bool {
        if self.controls.help.take().is_some() {
            self.play(WHOOSH);
            true
        } else {
            false
        }
    }

    /// Dismisses any open Options help balloon (`0x517995..0x5179bd` -> `0x50fb7f`)
    /// and plays `UI_whoosh`.
    fn dismiss_options_help(&mut self) -> bool {
        if self.controls.options.help.take().is_some() {
            self.play(WHOOSH);
            true
        } else {
            false
        }
    }

    /// Dismisses any open help balloon (Live or Options, `0x50fb7f`) and plays
    /// `UI_whoosh`. Returns true if a balloon was open.
    pub fn dismiss_help(&mut self) -> bool {
        let live = self.controls.help.take().is_some();
        let opts = self.controls.options.help.take().is_some();
        if live || opts {
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
        if let Some(layout) = self.dialog.clone() {
            if let Some(message) = self.active_message().cloned()
                && self
                    .dialog_state
                    .click_entry(&layout, &message, &self.art, point)
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
        if self
            .album_button_rect
            .is_some_and(|r| r.contains(point[0], point[1]))
        {
            self.pressed_album_button = true;
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
                let snapshot = self.household(world, residential);
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
        let now_open = self.controls.help.is_some() || self.controls.options.help.is_some();
        if now_open != open {
            self.play(WHOOSH);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use sims_ui::command::{Mode, Speed};
    use sims_ui::frame::Rect;
    use sims_ui::queue::{Icon, Picture};

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
