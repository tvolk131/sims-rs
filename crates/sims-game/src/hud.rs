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
use sims_ui::command::{Command, Mode as UiMode, Panel as UiPanel, Walls as UiWalls};
use sims_ui::dialog::{self, Labels, Message};
use sims_ui::frame::{Frame, Rect};
use sims_ui::house;
use sims_ui::household::{Acquaintances, Careers, Faces, Snapshot};
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
    /// level needs.
    careers: Careers,
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
    /// The Live-mode pause button's 500 ms blink timer (`0x53b4b0..0x53b7e7`).
    pause_blink: PauseBlink,
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
        art.load_optional(&ui, US_ENGLISH, sims_ui::balloon::BACKGROUND);
        let labels = Labels::load(&install).map_err(|e| e.to_string())?;
        // Without the balloons' strings the HUD still opens; the balloons
        // show what text they have.
        let texts = Texts::load(&install).unwrap_or_default();
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
            pause_blink: PauseBlink::default(),
            sounds: Vec::new(),
        })
    }

    /// Forgets what the panel kept of the lot it showed, for a lot put
    /// back to a save: the pie menu, the dialog read, the icons pressed,
    /// and the pause button's blink timer. The rest is read from the lot
    /// at the next frame.
    pub fn forget(&mut self) {
        self.menu = None;
        self.message = None;
        self.dialog = None;
        self.pressed = Pressed::default();
        self.pause_blink.reset();
        self.controls.pause_lit = false;
    }

    /// Syncs the Live-mode pause button's 500 ms blink timer
    /// (`0x53b4b0..0x53b7e7`) with `paused` and the monotonic `clock_us`.
    /// Returns true if the lit phase changed.
    pub fn sync_pause_blink(&mut self, paused: bool, clock_us: u64) -> bool {
        let blinking = paused && self.controls.mode == UiMode::Live;
        let changed = self.pause_blink.sync(blinking, clock_us);
        self.controls.pause_lit = self.pause_blink.lit();
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

    /// The household with zoning-aware family friends and what the next
    /// level of each one's career needs.
    fn household(&self, world: &World, residential: bool) -> Snapshot {
        Snapshot::read(world, &self.members)
            .with_zoning(world, residential, self.known.friendship)
            .with_careers(&self.careers)
    }

    /// `snapshot` with what the open panel reads besides.
    fn for_panel(&self, snapshot: Snapshot, world: &World) -> Snapshot {
        match self.controls.panel {
            Some(UiPanel::Relationships) => {
                snapshot.with_relations(world, &self.members, &self.known)
            }
            Some(UiPanel::House) if self.controls.expanded => {
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
        if self.controls.expanded && self.controls.panel == Some(UiPanel::House) {
            self.kinds.learn(world, assets.index(), assets.catalog());
        }
        // The pause button shows global 17, which the game's pause sets.
        let snapshot = self.for_panel(household, world);
        self.items = hud::layout(view.size, &snapshot, &self.controls);
        hud::fit(&mut self.items, &self.art, &self.texts);
        self.queue = Queue::read(world, snapshot.selected);
        self.pressed.keep(&self.queue);
        let objects: Vec<u32> = self.queue.objects().collect();
        for guid in objects {
            // A type whose picture fails is kept as the error left it
            // (`Thumbnails::read`), and not read again.
            if let Err(e) = self.thumbnails.read(assets.index(), assets.catalog(), guid) {
                self.problems
                    .push(format!("the catalog picture of type 0x{guid:08X}: {e}"));
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
        hud::draw(
            &mut frame,
            &self.art,
            &self.faces,
            &self.items,
            &self.controls,
        );
        self.queue
            .draw(&mut frame, &self.art, &self.faces, &self.thumbnails);
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
            menu.draw(&mut frame, &self.art, self.controls.pointer);
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
            || self.queue.at(point).is_some()
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

    /// Moves the pointer; true if what it hovers changed.
    pub fn hover(&mut self, point: [i32; 2]) -> bool {
        let button = |pointer: Option<[i32; 2]>| {
            let layout = self.dialog.as_ref()?;
            let [x, y] = pointer?;
            layout.buttons.iter().position(|b| b.contains(x, y))
        };
        let entry = |pointer: Option<[i32; 2]>| {
            let menu = self.menu.as_ref()?;
            Some(menu.hovered(pointer?))
        };
        let before = (
            hud::hovered(&self.items, self.controls.pointer),
            hud::lit_relation(&self.items, self.controls.pointer),
            button(self.controls.pointer),
            entry(self.controls.pointer),
        );
        self.controls.pointer = Some(point);
        (
            hud::hovered(&self.items, self.controls.pointer),
            hud::lit_relation(&self.items, self.controls.pointer),
            button(self.controls.pointer),
            entry(self.controls.pointer),
        ) != before
    }

    /// The pointer left the window: no button is hovered and no name
    /// shows. True if the pointer was in it, so the frame changes.
    pub fn leave(&mut self) -> bool {
        self.controls.pointer.take().is_some()
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
            return None;
        }
        if let Some(command) = self.press_queue(point) {
            return command;
        }
        let command = hud::press(&self.items, point)?;
        let command = match command {
            Command::OpenPanel(_) => {
                let snapshot = Snapshot::read(world, &self.members);
                let selected = Selected::of(&snapshot);
                hud::check_press(command, selected, residential)
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
            Release::Settled | Release::Stays => {
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

/// The Live-mode pause button's 500 ms blink timer (`0x53b4b0..0x53b7e7`):
/// starts unlit (cyan row 0, `0x53b707..0x53b74d`) on entry and toggles row
/// every [`hud::PAUSE_BLINK_US`] (`0x53b500..0x53b528`).
#[derive(Debug, Default)]
struct PauseBlink {
    since: Option<u64>,
    lit: bool,
}

impl PauseBlink {
    fn lit(&self) -> bool {
        self.lit
    }

    fn reset(&mut self) {
        self.since = None;
        self.lit = false;
    }

    fn sync(&mut self, blinking: bool, clock_us: u64) -> bool {
        let before = self.lit;
        if !blinking {
            self.reset();
            return self.lit != before;
        }
        match self.since {
            None => {
                self.since = Some(clock_us);
                self.lit = false;
            }
            Some(since) if clock_us.saturating_sub(since) >= hud::PAUSE_BLINK_US => {
                // 0x53b500..0x53b52e -> 0x5daf1c..0x5daf31: a single timer
                // callback toggles the row once and re-arms 500 ms from the
                // current clock, even after a long stall.
                self.lit = !self.lit;
                self.since = Some(clock_us);
            }
            Some(_) => {}
        }
        self.lit != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_ui::frame::Rect;
    use sims_ui::queue::{Icon, Picture};

    #[test]
    fn pause_blink_toggles_every_500ms_and_resets_unlit() {
        let mut blink = PauseBlink::default();
        assert!(!blink.lit());
        // Entering Live-mode pause starts unlit (cyan row 0, 0x53b707..0x53b74d).
        assert!(!blink.sync(true, 100_000));
        assert!(!blink.lit());
        // 499.999 ms later: still unlit.
        assert!(!blink.sync(true, 599_999));
        assert!(!blink.lit());
        // At 500 ms: flips to lit (red row 1, 0x53b500..0x53b528).
        assert!(blink.sync(true, 600_000));
        assert!(blink.lit());
        // At 1000 ms: flips back to unlit.
        assert!(blink.sync(true, 1_100_000));
        assert!(!blink.lit());
        // A 1-second stall (2x 500 ms) toggles once and re-arms 500 ms from
        // the current clock (0x53b500..0x53b52e -> 0x5daf1c..0x5daf31).
        assert!(blink.sync(true, 2_100_000));
        assert!(blink.lit());
        assert!(!blink.sync(true, 2_599_999));
        assert!(blink.sync(true, 2_600_000));
        assert!(!blink.lit());
        // Unpausing clears lit and kills the timer; re-pausing starts fresh unlit.
        assert!(blink.sync(true, 3_100_000));
        assert!(blink.lit());
        assert!(blink.sync(false, 3_200_000));
        assert!(!blink.lit());
        assert!(!blink.sync(true, 3_400_000));
        assert!(!blink.lit());
        assert!(!blink.sync(true, 3_899_999));
        assert!(blink.sync(true, 3_900_000));
        assert!(blink.lit());
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
