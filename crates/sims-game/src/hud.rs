//! The live-mode control panel (`sims-ui`) over the lot, read from the
//! running world each frame, the selected Sim's queue of actions, and
//! the dialog the world waits on.
//!
//! The panel turns presses into `sims_ui` commands; [`Hud::press`] hands
//! them to the game, which carries out what it can.

use crate::menu::{Menu, Press};
use crate::pace::Pace;
use crate::view::View;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_content::{Install, Neighborhood};
use sims_render::{Assets, Canvas, Walls, Zoom};
use sims_sim::World;
use sims_ui::art::Art;
use sims_ui::command::{Command, Walls as UiWalls};
use sims_ui::dialog::{self, Labels, Message};
use sims_ui::frame::{Frame, Rect};
use sims_ui::household::{Faces, Snapshot};
use sims_ui::hud::{self, Controls, Item};
use sims_ui::pie;
use sims_ui::queue::Queue;
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

pub struct Hud {
    pub art: Art,
    faces: Faces,
    labels: Labels,
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
    /// The actions whose icons were pressed while they show: a second
    /// press on an icon does nothing (0x4cb148).
    pressed: BTreeSet<i32>,
    /// The interface's sounds, until the game takes them.
    sounds: Vec<SoundEvent>,
}

impl Hud {
    /// Reads the art, and the household of `house` from its neighborhood
    /// with the portraits from the people's files.
    pub fn open(install: &Path, house: &Path, assets: &Assets) -> Result<Self, String> {
        let install = Install::detect(install).map_err(|e| e.to_string())?;
        let ui = UiGraphics::open(&install).map_err(|e| e.to_string())?;
        let resources: Vec<i32> = hud::RESOURCES
            .iter()
            .chain(&dialog::RESOURCES)
            .chain(&pie::RESOURCES)
            .chain(&sims_ui::queue::RESOURCES)
            .copied()
            .collect();
        let art = Art::load(&ui, US_ENGLISH, &resources).map_err(|e| e.to_string())?;
        let labels = Labels::load(&install).map_err(|e| e.to_string())?;
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
        let faces =
            Faces::load(assets.index(), assets.catalog(), &members).map_err(|e| e.to_string())?;
        Ok(Self {
            art,
            faces,
            labels,
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
            pressed: BTreeSet::new(),
            sounds: Vec::new(),
        })
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

    /// Draws the panel over the lot in `canvas`, from `world` as it is
    /// now, with the speed of `pace`, and `status` (a line across the
    /// top) if there is one; the dialog is the one [`Hud::read_dialog`]
    /// last read.
    pub fn draw(
        &mut self,
        world: &World,
        view: &View,
        pace: &Pace,
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
        self.controls.speed = pace.speed;
        // The pause button shows global 17, which the game's pause sets.
        let snapshot = Snapshot::read(world, &self.members);
        self.items = hud::layout(view.size, &snapshot, &self.controls);
        self.queue = Queue::read(world, snapshot.selected);
        // An icon gone takes its press with it.
        let shown: BTreeSet<i32> = self.queue.icons.iter().map(|i| i.uid).collect();
        self.pressed.retain(|uid| shown.contains(uid));
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
        self.queue.draw(&mut frame, &self.art, &self.faces);
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
        if cancelled || !self.pressed.insert(uid) {
            return Some(None);
        }
        Some(Some(Command::CancelAction {
            sim: person,
            action: uid,
        }))
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
            button(self.controls.pointer),
            entry(self.controls.pointer),
        );
        self.controls.pointer = Some(point);
        (
            hud::hovered(&self.items, self.controls.pointer),
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

    /// A press at `point`: the command it asks for, if any. Commands that
    /// change only the panel are applied here too.
    pub fn press(&mut self, point: [i32; 2]) -> Option<Command> {
        if let Some(layout) = &self.dialog {
            return dialog::press(layout, point);
        }
        // A press on an entry chooses it, one on a submenu's entry or
        // back from it keeps the menu open, and one elsewhere closes it.
        // `interaction` is the entry's index in the menu; the game reads
        // its menu number from the menu it had open.
        if let Some(mut menu) = self.menu.take() {
            return match menu.press(&self.art, point) {
                Press::Chose(choice) => Some(Command::ChooseInteraction {
                    sim: menu.person,
                    object: menu.object,
                    interaction: u16::try_from(choice).ok()?,
                }),
                Press::Stays => {
                    self.menu = Some(menu);
                    None
                }
                Press::Closed => None,
            };
        }
        if let Some(command) = self.press_queue(point) {
            return command;
        }
        let command = hud::press(&self.items, point)?;
        self.controls.apply(&command);
        Some(command)
    }
}
