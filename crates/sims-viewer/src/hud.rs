//! The original's live-mode control panel over the lot (`--hud`), from
//! `sims-ui`.
//!
//! The panel reads the world and turns presses into `sims_ui` commands.
//! The viewer carries out those that change only the view (the camera,
//! the open panel). The simulation has no command intake yet, so a
//! command for it (choosing a Sim, the speed, a mode) is printed and
//! dropped.
//!
//! A dialog the world is waiting on is drawn over the lot and takes every
//! press until it is answered, as the original's dialogs do. Its answer
//! is printed and dropped too, and the viewer stops showing that dialog.

use crate::view::{Command as ViewCommand, View};
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_content::{Install, Neighborhood};
use sims_render::{Assets, Canvas, Walls, Zoom};
use sims_ui::balloon::Texts;
use sims_ui::command::{Camera, Command, Panel, Walls as UiWalls};
use sims_ui::dialog::{self, Labels, Message};
use sims_ui::frame::Frame;
use sims_ui::house;
use sims_ui::household::{Acquaintances, Careers, Faces, Snapshot};
use sims_ui::hud::{self, Controls, Item};
use std::path::Path;

pub struct Hud {
    art: sims_ui::art::Art,
    faces: Faces,
    /// The careers of `work.iff`, for what the job panel shows the next
    /// level needs.
    careers: Careers,
    /// The curves and constants the house panel's ratings are worked out
    /// with.
    house: house::Tuning,
    /// Which of the lot's object types the Upkeep rating counts, read
    /// when the house panel first shows: the viewer's world does not
    /// change, so once.
    kinds: Option<house::Kinds>,
    /// The household's member GUIDs, in the neighborhood's order.
    members: Vec<u32>,
    /// What the relationship panel needs from outside the world.
    known: Acquaintances,
    /// The help balloons' texts.
    texts: Texts,
    pub controls: Controls,
    items: Vec<Item>,
    /// The dialog the world waits on, until it is answered here.
    message: Option<Message>,
    dialog: Option<dialog::Layout>,
}

impl Hud {
    /// Reads the art, the household of `house` from its neighborhood,
    /// with the portraits from the people's files (everyone `world`'s
    /// neighborhood records hold, for the relationship panel), and the
    /// dialog `world` waits on. The viewer never runs the world, so the dialog
    /// is read once.
    pub fn open(
        install: &Path,
        house: &Path,
        assets: &Assets,
        world: &sims_sim::World,
    ) -> Result<Self, String> {
        let install = Install::detect(install).map_err(|e| e.to_string())?;
        let ui = UiGraphics::open(&install).map_err(|e| e.to_string())?;
        let resources: Vec<i32> = hud::RESOURCES
            .iter()
            .chain(&dialog::RESOURCES)
            .copied()
            .collect();
        let mut art =
            sims_ui::art::Art::load(&ui, US_ENGLISH, &resources).map_err(|e| e.to_string())?;
        art.load_optional(&ui, US_ENGLISH, sims_ui::balloon::BACKGROUND);
        let message = match dialog::pending(world) {
            Some(pending) => {
                let labels = Labels::load(&install).map_err(|e| e.to_string())?;
                Message::read(world, assets.index(), assets.catalog(), &labels, pending)
                    .map_err(|e| e.to_string())?
            }
            None => None,
        };
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
            friendship(assets)?,
        );
        // Without the balloons' strings the HUD still opens; the balloons
        // show what text they have.
        let texts = Texts::load(&install).unwrap_or_default();
        Ok(Self {
            art,
            faces,
            careers,
            house,
            kinds: None,
            members,
            known,
            texts,
            // The strip opens as if Live had been pressed.
            controls: Controls {
                expanded: true,
                ..Controls::default()
            },
            items: Vec::new(),
            message,
            dialog: None,
        })
    }

    /// Draws the panel over the lot in `canvas`.
    pub fn draw(
        &mut self,
        world: &sims_sim::World,
        view: &View,
        assets: &Assets,
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
        let mut snapshot = Snapshot::read(world, &self.members).with_careers(&self.careers);
        // The viewer plays no sounds, so a balloon this closes closes
        // silently.
        self.controls.follow(snapshot.selected_resident().is_some());
        // The lookup goes through every neighborhood record, so only
        // while the panel shows it.
        if self.controls.panel == Some(Panel::Relationships) {
            snapshot = snapshot.with_relations(world, &self.members, &self.known);
        }
        // The house's ratings only while its panel shows.
        if self.controls.expanded && self.controls.panel == Some(Panel::House) {
            let kinds = self.kinds.get_or_insert_with(|| {
                let mut kinds = house::Kinds::default();
                kinds.learn(world, assets.index(), assets.catalog());
                kinds
            });
            snapshot = snapshot.with_house(world, &self.house, kinds);
        }
        self.items = hud::layout(view.size, &snapshot, &self.controls);
        hud::fit(&mut self.items, &self.art, &self.texts);
        let (width, height) = (
            u32::try_from(canvas.width).unwrap_or(u32::MAX),
            u32::try_from(canvas.height).unwrap_or(u32::MAX),
        );
        let mut frame = match Frame::from_rgb(width, height, std::mem::take(&mut canvas.color)) {
            Ok(frame) => frame,
            // A canvas the frame cannot take: leave it as drawn.
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
        self.dialog = self
            .message
            .as_ref()
            .and_then(|m| dialog::layout(view.size, m, &self.art));
        if let (Some(message), Some(layout)) = (&self.message, &self.dialog) {
            dialog::draw(
                &mut frame,
                &self.art,
                message,
                layout,
                self.controls.pointer,
            );
        }
        canvas.color = frame.into_rgb();
    }

    /// Whether the panel covers `point` (canvas pixels), or a dialog is
    /// shown and takes every press.
    pub fn covers(&self, point: [i32; 2]) -> bool {
        self.dialog.is_some() || hud::covers(&self.items, point)
    }

    /// Moves the pointer; true if what it hovers changed.
    pub fn hover(&mut self, point: [i32; 2]) -> bool {
        let button = |pointer: Option<[i32; 2]>| {
            let layout = self.dialog.as_ref()?;
            let [x, y] = pointer?;
            layout.buttons.iter().position(|b| b.contains(x, y))
        };
        let before = (
            hud::hovered(&self.items, self.controls.pointer),
            hud::lit_relation(&self.items, self.controls.pointer),
            button(self.controls.pointer),
        );
        self.controls.pointer = Some(point);
        (
            hud::hovered(&self.items, self.controls.pointer),
            hud::lit_relation(&self.items, self.controls.pointer),
            button(self.controls.pointer),
        ) != before
    }

    /// A press at `point`: the view command to carry out, if any, and
    /// whether the panel changed. A gadget button's press is checked
    /// against `world` as it is at the press ([`hud::check_press`]). The
    /// viewer does not read the lot's zoning, so the lot counts as
    /// residential, and a refusal is silent: the viewer plays no sounds.
    pub fn press(
        &mut self,
        point: [i32; 2],
        world: &sims_sim::World,
    ) -> (Option<ViewCommand>, bool) {
        if let Some(layout) = &self.dialog {
            let Some(command) = dialog::press(layout, point) else {
                return (None, false);
            };
            eprintln!("not carried out, the simulation takes no commands yet: {command:?}");
            self.message = None;
            self.dialog = None;
            return (None, true);
        }
        let Some(command) = hud::press(&self.items, point) else {
            return (None, false);
        };
        let selected = Snapshot::read(world, &self.members)
            .selected_resident()
            .is_some();
        let command = hud::check_press(command, selected, true);
        if command.changes_simulation() {
            eprintln!("not carried out, the simulation takes no commands yet: {command:?}");
            return (None, false);
        }
        let view = match command {
            Command::Camera(Camera::ZoomIn) => Some(ViewCommand::ZoomIn),
            Command::Camera(Camera::ZoomOut) => Some(ViewCommand::ZoomOut),
            // Which way the original turns for each is unverified.
            Command::Camera(Camera::RotateRight) => Some(ViewCommand::Rotate(true)),
            Command::Camera(Camera::RotateLeft) => Some(ViewCommand::Rotate(false)),
            Command::Camera(Camera::FloorUp) => Some(ViewCommand::LevelUp),
            Command::Camera(Camera::FloorDown) => Some(ViewCommand::LevelDown),
            Command::Camera(Camera::Walls(w)) => Some(ViewCommand::Walls(match w {
                UiWalls::Up => Walls::Up,
                UiWalls::Cutaway => Walls::Cutaway,
                UiWalls::Down => Walls::Down,
            })),
            Command::Camera(Camera::Photo) => Some(ViewCommand::Save),
            Command::Camera(Camera::Roof) => {
                eprintln!("the viewer draws no roofs yet");
                None
            }
            _ => None,
        };
        let before = self.controls;
        self.controls.apply(&command);
        (view, self.controls != before)
    }
}

/// The autonomy tuning's friendship threshold, which the relationship
/// panel's smileys need. The viewer may show a snapshot or a house file
/// it did not boot, so it links the global file alone, which holds the
/// tuning, and reads it as the lot does
/// (`sims_vm::Program::autonomy_constants`).
fn friendship(assets: &Assets) -> Result<i32, String> {
    let program =
        sims_vm::Program::link(assets.index(), assets.catalog(), []).map_err(|e| e.to_string())?;
    Ok(program.autonomy_constants().friendship_threshold)
}
