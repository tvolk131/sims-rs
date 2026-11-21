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
use sims_content::{Install, Neighborhood};
use sims_render::{Assets, Canvas, Walls, Zoom};
use sims_sim::World;
use sims_ui::art::Art;
use sims_ui::balloon::Texts;
use sims_ui::buy::{self, BuyCatalog, BuyLayout, BuyState, BuyWidgetKind, RatingLabels};
use sims_ui::command::{Command, Mode, Panel as UiPanel, Walls as UiWalls};
use sims_ui::dialog::{self, Labels, Message};
use sims_ui::frame::{Frame, Rect};
use sims_ui::house;
use sims_ui::household::{Acquaintances, Careers, Faces, Fame, Snapshot};
use sims_ui::hud::{self, Controls, Item, Selected};
use sims_ui::pie;
use sims_ui::queue::Queue;
use sims_ui::thumbnail::{self, Thumbnails};
use sims_vm::SoundEvent;
use std::collections::BTreeSet;
use std::path::Path;

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

pub struct Hud {
    pub art: Art,
    faces: Faces,
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
    /// The household's member GUIDs, in the neighborhood's order.
    members: Vec<u32>,
    pub controls: Controls,
    items: Vec<Item>,
    /// The dialog the world waits on, and the record it was read from
    /// (its caller and the operands), so it is read once.
    message: Option<(Message, (u16, [u8; 8]))>,
    dialog: Option<dialog::Layout>,
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
    /// The Buy Mode catalog strip state (sort, category, subcategory, page,
    /// selected GUID).
    pub buy_state: BuyState,
    /// The Buy Mode strip widgets and popup card as last laid out.
    buy_layout: BuyLayout,
    /// The interface's sounds, until the game takes them.
    sounds: Vec<SoundEvent>,
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
        let install = Install::detect(install).map_err(|e| e.to_string())?;
        let ui = UiGraphics::open(&install).map_err(|e| e.to_string())?;
        let resources: Vec<i32> = hud::RESOURCES
            .iter()
            .chain(&dialog::RESOURCES)
            .chain(&pie::RESOURCES)
            .chain(&sims_ui::queue::RESOURCES)
            .chain(&thumbnail::RESOURCES)
            .copied()
            .collect();
        let mut art = Art::load(&ui, US_ENGLISH, &resources).map_err(|e| e.to_string())?;
        for id in hud::OPTIONAL_RESOURCES {
            art.load_optional(&ui, US_ENGLISH, id);
        }
        for id in buy::resource_ids() {
            art.load_optional(&ui, US_ENGLISH, id);
        }
        let labels = Labels::load(&install).map_err(|e| e.to_string())?;
        let rating_labels = RatingLabels::load(&install);
        // Without the balloons' strings the HUD still opens; the balloons
        // show what text they have.
        let texts = Texts::load(&install).unwrap_or_default();
        let fame = Fame::load(&install).unwrap_or_default();
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
        let mut people = members.clone();
        for record in world.neighbors.slots.iter().flatten() {
            if !people.contains(&record.guid) {
                people.push(record.guid);
            }
        }
        let faces =
            Faces::load(assets.index(), assets.catalog(), &people).map_err(|e| e.to_string())?;
        let careers = Careers::load(assets.index()).map_err(|e| e.to_string())?;
        let house =
            house::Tuning::load(assets.index(), assets.catalog()).map_err(|e| e.to_string())?;
        let known = Acquaintances::read(
            assets.index(),
            assets.catalog(),
            install.expansions,
            friendship,
        );
        Ok(Self {
            art,
            faces,
            thumbnails: Thumbnails::default(),
            problems: Vec::new(),
            careers,
            fame,
            house,
            kinds: house::Kinds::default(),
            known,
            labels,
            texts,
            members,
            // The strip opens as if Live had been pressed.
            controls: Controls {
                expanded: true,
                ..Controls::default()
            },
            items: Vec::new(),
            message: None,
            dialog: None,
            menu: None,
            band: None,
            queue: Queue::default(),
            pressed: Pressed::default(),
            clock_blink: ClockBlink::default(),
            idle_speed: false,
            buy_catalog: None,
            rating_labels,
            buy_state: BuyState::default(),
            buy_layout: BuyLayout::default(),
            sounds: Vec::new(),
        })
    }

    /// Forgets what the panel kept of the lot it showed, for a lot put
    /// back to a save: the pie menu, the dialog read, the icons pressed,
    /// the Buy Mode catalog state, and the pause button's blink timer. The
    /// rest is read from the lot at the next frame.
    pub fn forget(&mut self) {
        self.menu = None;
        self.message = None;
        self.dialog = None;
        self.pressed = Pressed::default();
        self.clock_blink.reset();
        self.idle_speed = false;
        self.controls.pause_lit = false;
        self.controls.ultra_lit = false;
        self.buy_state.reset();
        self.buy_layout = BuyLayout::default();
    }

    /// Loads the residential Buy Mode catalog on first use and returns it.
    pub fn ensure_buy_catalog(&mut self, assets: &Assets) -> &BuyCatalog {
        let rating_labels = self.rating_labels.clone();
        self.buy_catalog.get_or_insert_with(|| {
            BuyCatalog::load(assets.index(), assets.catalog()).with_rating_labels(rating_labels)
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

    /// Whether `point` lies over the expanded Buy Mode catalog strip
    /// (`cWinCatalog`, `x >= CLUSTER[0]` and `y >= height - TALL_STRIP`), where
    /// clicking or dropping a held object sells it (`0x50d04d` → `0x4cf97a` →
    /// `0x487e18`).
    pub fn over_buy_catalog(&self, size: [i32; 2], point: [i32; 2]) -> bool {
        if self.controls.mode != Mode::Buy || !self.controls.expanded {
            return false;
        }
        let top = size[1] - hud::TALL_STRIP;
        point[0] >= hud::CLUSTER[0] && point[0] < size[0] && point[1] >= top && point[1] < size[1]
    }

    /// Lays out the Buy Mode catalog strip for `size` and `world` (or clears
    /// it outside expanded Buy Mode), syncing `self.buy_state.page` to the
    /// clamped page number.
    pub fn layout_buy(&mut self, size: [i32; 2], world: &World, assets: &Assets) {
        if self.controls.mode != Mode::Buy || !self.controls.expanded {
            self.buy_layout = BuyLayout::default();
            return;
        }
        let rating_labels = self.rating_labels.clone();
        let catalog = self.buy_catalog.get_or_insert_with(|| {
            BuyCatalog::load(assets.index(), assets.catalog()).with_rating_labels(rating_labels)
        });
        let funds = sims_vm::available_funds(world);
        let layout = buy::layout(size, catalog, &self.buy_state, &self.art, funds);
        self.buy_state.page = layout.page;
        self.buy_layout = layout;
    }

    /// Clears the cached Buy Mode strip layout (e.g. when leaving Buy Mode).
    pub fn clear_buy_layout(&mut self) {
        self.buy_layout = BuyLayout::default();
    }

    /// Re-lays out the control panel items and Buy Mode strip for `size`
    /// after a mode or panel toggle, so consecutive clicks work even before
    /// the next [`Hud::draw`].
    pub fn relayout_panel(
        &mut self,
        size: [i32; 2],
        world: &World,
        residential: bool,
        assets: &Assets,
    ) {
        let snapshot = self.snapshot(world, residential);
        self.items = hud::layout(size, &snapshot, &self.controls);
        hud::fit(&mut self.items, &self.art, &self.texts);
        self.layout_buy(size, world, assets);
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
        .map(|m| (m, key));
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
        _pace: &Pace,
        status: Option<&str>,
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
            roof: false,
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
        self.items = hud::layout(view.size, &snapshot, &self.controls);
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
        if self.controls.mode == Mode::Buy && self.controls.expanded {
            self.layout_buy(view.size, world, assets);
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
        if self.controls.mode == Mode::Live {
            self.queue
                .draw(&mut frame, &self.art, &self.faces, &self.thumbnails);
        }
        self.dialog = self
            .message
            .as_ref()
            .and_then(|(m, _)| dialog::layout(view.size, m, &self.art));
        // A dialog takes every press until it is answered, so a menu open
        // under it is closed rather than left to be chosen from after.
        if self.dialog.is_some() {
            self.menu = None;
        }
        if let (Some((message, _)), Some(layout)) = (&self.message, &self.dialog) {
            dialog::draw(
                &mut frame,
                &self.art,
                message,
                layout,
                self.controls.pointer,
            );
        }
        if let Some(menu) = &mut self.menu {
            menu.layout(&self.art, view.size);
            menu.draw_labels(&mut frame, &self.art, self.controls.pointer);
        }
        self.band = status.and_then(|status| self.status(&mut frame, status));
        canvas.color = frame.into_rgb();
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
        self.dialog.is_some()
            || self.menu.is_some()
            || self.band.is_some_and(|b| b.contains(point[0], point[1]))
            || hud::covers(&self.items, point)
            || (self.controls.mode == Mode::Buy
                && self.controls.expanded
                && self.buy_layout.covers(point))
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
        self.message
            .as_ref()
            .and_then(|(m, _)| dialog::layout(size, m, &self.art))
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
        let before = (
            hud::hovered(&self.items, self.controls.pointer),
            hud::lit_relation(&self.items, self.controls.pointer),
            button(self.controls.pointer),
            entry(self.controls.pointer),
            buy_widget(self.controls.pointer),
        );
        self.controls.pointer = Some(point);
        let hovered_changed = (
            hud::hovered(&self.items, self.controls.pointer),
            hud::lit_relation(&self.items, self.controls.pointer),
            button(self.controls.pointer),
            entry(self.controls.pointer),
            buy_widget(self.controls.pointer),
        ) != before;
        let dismissed = self.controls.help.is_some()
            && !hud::in_subpanel(&self.items, point)
            && self.dismiss_help();
        moved_for_pie || hovered_changed || dismissed
    }

    /// The pointer left the window: no button is hovered, no name shows, and
    /// any open help balloon is dismissed (`0x51d2ee..0x51d317`). True if the
    /// frame changes.
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

    /// Dismisses any open help balloon (`0x50fb7f`) and plays `UI_whoosh`.
    /// Returns true if a balloon was open.
    pub fn dismiss_help(&mut self) -> bool {
        if self.controls.help.take().is_some() {
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
        if let Some(layout) = &self.dialog {
            return dialog::press(layout, point);
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
        let open = self.controls.help.is_some();
        self.controls.apply(&command);
        if self.controls.help.is_some() != open {
            self.play(WHOOSH);
        }
        Some(command)
    }

    /// A left release on an open pie menu at `point` (0x5eb433): settles
    /// the opening press, opens or leaves a submenu, chooses an entry, or
    /// closes the menu.
    pub fn release_menu(&mut self, point: [i32; 2]) -> (Release, Option<Command>) {
        let Some(mut menu) = self.menu.take() else {
            return (Release::Closed, None);
        };
        let release = menu.release(&self.art, point);
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
