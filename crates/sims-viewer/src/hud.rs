//! The original's live-mode control panel over the lot (`--hud`), from
//! `sims-ui`.
//!
//! The panel reads the world and turns presses into `sims_ui` commands.
//! The viewer carries out those that change only the view (the camera,
//! the open panel). The simulation has no command intake yet, so a
//! command for it (choosing a Sim, the speed, a mode) is printed and
//! dropped.

use crate::view::{Command as ViewCommand, View};
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_content::{Install, Neighborhood};
use sims_render::{Assets, Canvas, Walls, Zoom};
use sims_ui::command::{Camera, Command, Walls as UiWalls};
use sims_ui::frame::Frame;
use sims_ui::household::{Faces, Snapshot};
use sims_ui::hud::{self, Controls, Item};
use std::path::Path;

pub struct Hud {
    art: sims_ui::art::Art,
    faces: Faces,
    /// The household's member GUIDs, in the neighborhood's order.
    members: Vec<u32>,
    pub controls: Controls,
    items: Vec<Item>,
}

impl Hud {
    /// Reads the art, and the household of `house` from its
    /// neighborhood, with the portraits from the people's files.
    pub fn open(install: &Path, house: &Path, assets: &Assets) -> Result<Self, String> {
        let install = Install::detect(install).map_err(|e| e.to_string())?;
        let ui = UiGraphics::open(&install).map_err(|e| e.to_string())?;
        let art =
            sims_ui::art::Art::load(&ui, US_ENGLISH, &hud::RESOURCES).map_err(|e| e.to_string())?;
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
            members,
            // The strip opens as if Live had been pressed.
            controls: Controls {
                expanded: true,
                ..Controls::default()
            },
            items: Vec::new(),
        })
    }

    /// Draws the panel over the lot in `canvas`.
    pub fn draw(&mut self, world: &sims_sim::World, view: &View, canvas: &mut Canvas) {
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
        let snapshot = Snapshot::read(world, &self.members);
        self.items = hud::layout(view.size, &snapshot, &self.controls);
        let mut frame = Frame {
            width: canvas.width as u32,
            height: canvas.height as u32,
            rgb: std::mem::take(&mut canvas.color),
        };
        hud::draw(
            &mut frame,
            &self.art,
            &self.faces,
            &self.items,
            &self.controls,
        );
        canvas.color = frame.rgb;
    }

    /// Whether the panel covers `point` (canvas pixels).
    pub fn covers(&self, point: [i32; 2]) -> bool {
        hud::covers(&self.items, point)
    }

    /// Moves the pointer; true if what it hovers changed.
    pub fn hover(&mut self, point: [i32; 2]) -> bool {
        let before = hud::hovered(&self.items, self.controls.pointer);
        self.controls.pointer = Some(point);
        hud::hovered(&self.items, self.controls.pointer) != before
    }

    /// A press at `point`: the view command to carry out, if any, and
    /// whether the panel changed.
    pub fn press(&mut self, point: [i32; 2]) -> (Option<ViewCommand>, bool) {
        let Some(command) = hud::press(&self.items, point) else {
            return (None, false);
        };
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
