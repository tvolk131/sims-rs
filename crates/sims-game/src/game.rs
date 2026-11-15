//! The game without its window: a booted lot, stepped as the player's
//! speed asks, drawn with its people and the live-mode panel, and the
//! player's presses turned into what they ask for.
//!
//! The simulation runs in strict mode: the first step that fails stops
//! the lot, which stays on screen as the failure left it, with the
//! failure written across the top. Usually that is the first thing the
//! simulation does not do yet.

use crate::hud::Hud;
use crate::pace::Pace;
use crate::people::People;
use crate::pick;
use crate::view::{Move, View};
use sims_render::{Canvas, Walls, open_house};
use sims_ui::command::{Camera as UiCamera, Command, Walls as UiWalls};
use sims_vm::boot::{Frames, Lot, StepError};
use std::path::{Path, PathBuf};

/// What the game is started with.
#[derive(Debug, Clone)]
pub struct Options {
    /// The install's root directory.
    pub install: PathBuf,
    /// A house file inside a neighborhood's `Houses` directory.
    pub house: PathBuf,
    /// Answers to the dialogs that offer a choice, in order, as with
    /// `sims boot --answers` (the Goths' house takes `3`, No to the Help
    /// System). The player cannot answer them yet.
    pub answers: Vec<u8>,
}

/// What the host should do after an input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The picture changed.
    pub redraw: bool,
    /// The player asked for a photo of the view.
    pub photo: bool,
}

impl Outcome {
    fn redraw() -> Self {
        Self {
            redraw: true,
            photo: false,
        }
    }
}

pub struct Game {
    lot: Lot,
    lot_shape: sims_render::Lot,
    assets: sims_render::Assets,
    pub view: View,
    people: People,
    hud: Hud,
    pub pace: Pace,
    /// Why the lot stopped, once it has.
    stopped: Option<String>,
    /// A short message about the last press, until the next one.
    note: Option<String>,
    canvas: Canvas,
}

impl Game {
    /// Boots the house and reads everything drawing it needs.
    pub fn open(options: &Options) -> Result<Self, String> {
        let house = &options.house;
        let opened = open_house(&options.install, house).map_err(|e| e.to_string())?;
        let lot = Lot::load(&options.install, house, &options.answers, Frames::default())
            .map_err(|e| format!("{}: {e}", house.display()))?;
        let stopped = lot
            .load_report()
            .stopped
            .as_ref()
            .map(|s| format!("The load stopped, so the lot cannot run: {s}"));
        // The booted world, not the file as read: some objects (doors,
        // windows) set how they are drawn in their own init scripts.
        let lot_shape = sims_render::Lot::from_world(lot.world()).map_err(|e| e.to_string())?;
        let hud = Hud::open(&options.install, house, &opened.assets)?;
        let mut people = People::open(&options.install, house)?;
        people.refresh(lot.world());
        let view = View::new([800, 600], lot_shape.built_center(), opened.rotation);
        Ok(Self {
            lot,
            lot_shape,
            assets: opened.assets,
            view,
            people,
            hud,
            pace: Pace::default(),
            stopped,
            note: None,
            canvas: Canvas::new(1, 1, [0, 0, 0]),
        })
    }

    /// The lot, for reading.
    pub fn lot(&self) -> &Lot {
        &self.lot
    }

    /// Why the lot stopped, if it has.
    pub fn stopped(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    /// Problems drawing the people, one line each.
    pub fn problems(&self) -> &[String] {
        &self.people.problems
    }

    /// Runs the ticks owed after `elapsed_us` more microseconds. Returns
    /// how many ran.
    pub fn advance(&mut self, elapsed_us: u64) -> u32 {
        let due = self.pace.due(elapsed_us);
        self.run(due)
    }

    /// Runs up to `ticks` ticks, stopping at the first that fails.
    pub fn run(&mut self, ticks: u32) -> u32 {
        let mut ran = 0;
        while ran < ticks && self.stopped.is_none() {
            match self.lot.step() {
                Ok(_) => ran += 1,
                Err(error) => self.stop(&error),
            }
        }
        if ran > 0 {
            self.people.refresh(self.lot.world());
        }
        ran
    }

    fn stop(&mut self, error: &StepError) {
        let at = self.lot.ticks();
        self.stopped = Some(match error {
            StepError::Incomplete => "The load did not complete, so the lot cannot run.".into(),
            _ => format!("The lot stopped after tick {at}: {error}"),
        });
    }

    /// The status line across the top: why the lot stopped, else the
    /// last press's note.
    fn status(&self) -> Option<String> {
        match (&self.stopped, &self.note) {
            (Some(stop), Some(note)) => Some(format!("{stop}  —  {note}")),
            (Some(line), None) | (None, Some(line)) => Some(line.clone()),
            (None, None) => None,
        }
    }

    /// Draws the frame: the lot, its people, then the panel.
    pub fn render(&mut self) -> &Canvas {
        let [w, h] = self.view.size.map(|n| n.max(1) as usize);
        if (self.canvas.width, self.canvas.height) != (w, h) {
            self.canvas = Canvas::new(w, h, [0, 0, 0]);
        }
        let world = self.lot.world();
        sims_render::draw(
            world,
            &self.lot_shape,
            &mut self.assets,
            &self.view.camera,
            &mut self.canvas,
        );
        self.people.draw(world, &self.view.camera, &mut self.canvas);
        let status = self.status();
        self.hud.draw(
            world,
            &self.assets,
            &self.view,
            &self.pace,
            status.as_deref(),
            &mut self.canvas,
        );
        &self.canvas
    }

    /// The canvas last drawn.
    pub fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    /// Resizes the canvas to `size` of the original's pixels.
    pub fn resize(&mut self, size: [i32; 2]) -> bool {
        if size == self.view.size {
            return false;
        }
        self.view.resize(size);
        true
    }

    /// Moves the camera.
    pub fn apply(&mut self, command: Move) -> bool {
        self.view.apply(command)
    }

    /// Whether the panel takes presses at `point`.
    pub fn over_panel(&self, point: [i32; 2]) -> bool {
        self.hud.covers(point)
    }

    /// Moves the pointer to `point`.
    pub fn hover(&mut self, point: [i32; 2]) -> bool {
        let mut redraw = self.hud.hover(point);
        // The cutaway follows the rooms the pointer passes over, while
        // the cutaway is on and the pointer is over the lot.
        if !self.hud.covers(point) && self.view.camera.walls == Walls::Cutaway {
            let camera = self.view.camera;
            self.view
                .camera
                .focus
                .point(&self.lot_shape, &camera, point);
            redraw |= self.view.camera.focus != camera.focus;
        }
        redraw
    }

    /// A left press at `point`, which is over the panel or the lot.
    pub fn press(&mut self, point: [i32; 2]) -> Outcome {
        self.note = None;
        if self.hud.covers(point) {
            return match self.hud.press(point) {
                Some(command) => self.command(command),
                None => Outcome::redraw(),
            };
        }
        let world = self.lot.world();
        let camera = &self.view.camera;
        if let Some(id) = pick::person(world, camera, point) {
            self.note = Some(format!(
                "Person {id}: choosing a Sim needs the simulation's player orders."
            ));
        } else if let Some(id) = pick::object(world, camera, point) {
            let guid = world.objects.get(&id).map_or(0, |o| o.guid);
            self.note = Some(format!(
                "Object {id} (type {guid:#010x}): its pie menu needs the simulation's player orders."
            ));
        } else {
            return Outcome::default();
        }
        Outcome::redraw()
    }

    /// Carries out a panel command.
    fn command(&mut self, command: Command) -> Outcome {
        let moved = |view: &mut View, m: Move| {
            view.apply(m);
            Outcome::redraw()
        };
        match command {
            Command::SetSpeed(speed) => {
                self.pace.set_speed(speed);
                Outcome::redraw()
            }
            Command::TogglePause => {
                self.pace.toggle_pause();
                Outcome::redraw()
            }
            Command::Camera(camera) => match camera {
                UiCamera::ZoomIn => moved(&mut self.view, Move::ZoomIn),
                UiCamera::ZoomOut => moved(&mut self.view, Move::ZoomOut),
                // Which way the original turns for each is unverified.
                UiCamera::RotateRight => moved(&mut self.view, Move::Rotate(true)),
                UiCamera::RotateLeft => moved(&mut self.view, Move::Rotate(false)),
                UiCamera::FloorUp => moved(&mut self.view, Move::LevelUp),
                UiCamera::FloorDown => moved(&mut self.view, Move::LevelDown),
                UiCamera::Walls(w) => moved(
                    &mut self.view,
                    Move::Walls(match w {
                        UiWalls::Up => Walls::Up,
                        UiWalls::Cutaway => Walls::Cutaway,
                        UiWalls::Down => Walls::Down,
                    }),
                ),
                UiCamera::Photo => Outcome {
                    redraw: false,
                    photo: true,
                },
                UiCamera::Roof => {
                    self.note = Some("Roofs are not drawn yet.".into());
                    Outcome::redraw()
                }
            },
            Command::TogglePanel | Command::OpenPanel(_) => Outcome::redraw(),
            other => {
                self.note = Some(format!(
                    "Not carried out, the simulation takes no orders yet: {other:?}"
                ));
                Outcome::redraw()
            }
        }
    }
}

/// The Goths' house in an install at `root`, the house the boot goal
/// checks against the original.
pub fn goth_house(root: &Path) -> PathBuf {
    root.join("UserData/Houses/House05.iff")
}
