//! The game without its window: a booted lot, stepped as the player's
//! speed asks, drawn with its people and the live-mode panel, and the
//! player's presses turned into what they ask for.
//!
//! The simulation runs in strict mode ([`Lot::step`]) unless
//! [`Options::play`] asks for play mode. In strict mode the first step
//! that fails stops the lot, which stays on screen as the failure left
//! it, with the failure written across the top. In play mode
//! ([`Lot::step_play`]) an object whose tick fails, usually on something
//! the simulation does not do yet, is reset as the original resets an
//! object after a SimAntics error, and the lot goes on; each new error
//! caught is logged and shown across the top for a while, and only a
//! failure play mode does not catch stops the lot.

use crate::hud::Hud;
use crate::pace::Pace;
use crate::people::People;
use crate::pick;
use crate::view::{Move, View};
use sims_render::{Canvas, Selected, Walls, open_house};
use sims_ui::command::{Camera as UiCamera, Command, Walls as UiWalls};
use sims_vm::boot::{Frames, Lot, StepError};
use sims_vm::play::{Recovery, ScriptError};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The room word of an object in no room.
const NO_ROOM: i16 = -5;

/// How many ticks an error caught stays across the top: 10 seconds at
/// normal speed.
const SHOWN_TICKS: u32 = 200;

/// The most errors kept in the log; later ones are only counted.
const LOGGED: usize = 256;

/// The most different errors remembered, to show each once. Errors
/// after that are only counted.
const SEEN: usize = 4096;

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
    /// Step in play mode ([`Lot::step_play`]), which resets an object
    /// after its error and goes on, instead of strict mode ([`Lot::step`]),
    /// which stops the lot at the first error.
    pub play: bool,
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
    play: bool,
    /// Why the lot stopped, once it has.
    stopped: Option<String>,
    errors: Errors,
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
            play: options.play,
            stopped,
            errors: Errors::default(),
            note: None,
            canvas: Canvas::new(1, 1, [0, 0, 0]),
        })
    }

    /// The lot, for reading.
    pub fn lot(&self) -> &Lot {
        &self.lot
    }

    /// The hour of the game's clock (global 0), 0 to 23.
    pub fn hour(&self) -> u32 {
        let hour = self.lot.world().globals.first().copied().unwrap_or(0);
        u32::try_from(hour.rem_euclid(24)).unwrap_or(0)
    }

    /// Why the lot stopped, if it has.
    pub fn stopped(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    /// The errors play mode caught and recovered from, one line each, in
    /// order: the first [`LOGGED`] different ones (an error again at a
    /// later tick is not listed again), then a line saying more were
    /// caught.
    pub fn errors(&self) -> &[String] {
        &self.errors.lines
    }

    /// How many errors play mode caught.
    pub fn caught(&self) -> u64 {
        self.errors.count
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
            let step = if self.play {
                self.lot.step_play()
            } else {
                self.lot.step()
            };
            match step {
                Ok(update) => {
                    ran += 1;
                    let at = self.lot.ticks();
                    for line in in_order(at == 1, &update.errors, &update.frame_errors) {
                        self.errors.push(at, line);
                    }
                }
                Err(error) => self.stop(&error),
            }
        }
        if ran > 0 || self.stopped.is_some() {
            let world = self.lot.world();
            self.people.refresh(world);
            // The room lights change as lamps and the time of day do.
            match sims_render::Lot::from_world(world) {
                Ok(shape) => self.lot_shape = shape,
                Err(e) => {
                    let line = format!("the lot is not redrawn: {e}");
                    if !self.people.problems.contains(&line) {
                        self.people.problems.push(line);
                    }
                }
            }
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

    /// The status line across the top: why the lot stopped, else the last
    /// error caught for a while after it, then the last press's note.
    fn status(&self) -> Option<String> {
        let stopped = self
            .stopped
            .clone()
            .or_else(|| self.errors.recent(self.lot.ticks()));
        match (&stopped, &self.note) {
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
        // The cutaway also opens the room the selected Sim is in.
        self.view.camera.focus.selected = world.selected_whereabouts().map(|w| Selected {
            tile: [i32::from(w.tile[0]), i32::from(w.tile[1])],
            level: w.level,
            room: w.room.unwrap_or(NO_ROOM),
        });
        sims_render::draw(
            world,
            &self.lot_shape,
            &mut self.assets,
            &self.view.camera,
            &mut self.canvas,
        );
        self.people
            .draw(world, &self.lot_shape, &self.view.camera, &mut self.canvas);
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
        let cleared = self.note.take().is_some();
        if self.hud.covers(point) {
            return match self.hud.press(point) {
                Some(command) => self.command(command),
                None => Outcome::redraw(),
            };
        }
        let world = self.lot.world();
        let camera = &self.view.camera;
        if let Some(id) = pick::person(world, &self.lot_shape, camera, point) {
            self.note = Some(format!(
                "Person {id}: choosing a Sim needs the simulation's player orders."
            ));
        } else if let Some(id) = pick::object(world, &self.lot_shape, camera, point) {
            let guid = world.objects.get(&id).map_or(0, |o| o.guid);
            self.note = Some(format!(
                "Object {id} (type {guid:#010x}): its pie menu needs the simulation's player orders."
            ));
        } else {
            return Outcome {
                redraw: cleared,
                photo: false,
            };
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

/// The errors play mode caught.
#[derive(Debug, Default)]
struct Errors {
    /// One line each for the first [`LOGGED`] different errors, then one
    /// saying more came. An error that differs from one listed only in
    /// its tick is counted, not listed again.
    lines: Vec<String>,
    /// The different errors seen, without their ticks, the first
    /// [`SEEN`].
    seen: BTreeSet<String>,
    /// How many were caught, listed or not.
    count: u64,
    /// The last new error and the tick count after the step that caught
    /// it. Repeats are not shown again.
    last: Option<(u32, String)>,
}

impl Errors {
    fn push(&mut self, at: u32, line: String) {
        self.count += 1;
        // A line starts with its tick: "tick 95647, object 5: ...".
        let what = line
            .strip_prefix("tick ")
            .and_then(|rest| rest.split_once(", "))
            .map_or(line.as_str(), |(_, what)| what);
        // Once SEEN different errors are remembered, a new one cannot be
        // told from its repeats, so it is only counted.
        if self.seen.contains(what) || self.seen.len() >= SEEN {
            return;
        }
        self.seen.insert(what.to_owned());
        if self.lines.len() < LOGGED {
            self.lines.push(line.clone());
        } else if self.lines.len() == LOGGED {
            self.lines
                .push("further errors are counted, not listed".into());
        }
        self.last = Some((at, line));
    }

    /// The line across the top at tick count `now`: the last error, for
    /// [`SHOWN_TICKS`] after it was caught.
    fn recent(&self, now: u32) -> Option<String> {
        let (at, line) = self.last.as_ref()?;
        if now.saturating_sub(*at) >= SHOWN_TICKS {
            return None;
        }
        let count = match self.count {
            1 => String::new(),
            n => format!(" ({n} so far)"),
        };
        Some(format!("Error caught{count}: {line}"))
    }
}

/// The lines for one update's caught errors, in the order they happened.
/// An update lists the frame drawn before the lot's first tick (only on
/// the `first` step) ahead of the one drawn after its tick, and the
/// objects' errors happen between the two. On the first step a single
/// skipped frame could be either; it is taken to be the one before the
/// tick (the update does not say).
fn in_order(first: bool, errors: &[ScriptError], frames: &[String]) -> Vec<String> {
    let before = if first { frames.len().min(1) } else { 0 };
    let frame = |error: &String| format!("a frame was skipped: {error}");
    frames[..before]
        .iter()
        .map(frame)
        .chain(errors.iter().map(describe))
        .chain(frames[before..].iter().map(frame))
        .collect()
}

/// One line for an error play mode caught: when, what failed, and what
/// was done about it.
fn describe(error: &ScriptError) -> String {
    let place = if error.in_thread {
        ""
    } else {
        " outside its thread"
    };
    let then = error
        .then
        .as_ref()
        .map(|then| format!("; then, outside its thread, {then}"))
        .unwrap_or_default();
    let recovery = match &error.recovery {
        Recovery::Reset => "it was reset".to_string(),
        Recovery::Restarted(fault) => {
            format!("its reset failed ({fault}), so its thread restarted at Main")
        }
        Recovery::Parked(None) => "it failed again after a reset, so it stops".to_string(),
        Recovery::Parked(Some(fault)) => {
            format!("restarting its thread failed ({fault}), so it stops")
        }
    };
    format!(
        "tick {}, object {}{place}: {}{then}; {recovery}",
        error.tick, error.object, error.fault
    )
}

/// The Goths' house in an install at `root`, the house the boot goal
/// checks against the original.
pub fn goth_house(root: &Path) -> PathBuf {
    root.join("UserData/Houses/House05.iff")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_vm::{Fault, Location, Trap};

    fn fault(object: u16) -> Fault {
        Fault {
            object,
            at: Some(Location {
                tree: 0x1002,
                node: 7,
            }),
            trap: Trap::Unimplemented { opcode: 0x2a },
        }
    }

    #[test]
    fn an_error_says_what_failed_and_what_was_done() {
        let mut error = ScriptError {
            tick: 900,
            object: 12,
            fault: fault(12),
            in_thread: true,
            then: None,
            recovery: Recovery::Reset,
        };
        let line = describe(&error);
        assert!(line.starts_with("tick 900, object 12: object 12, tree 0x1002 node 7"));
        assert!(line.ends_with("; it was reset"), "{line}");
        // A caught thread fault, then one in the rest of the tick.
        error.then = Some(fault(13));
        error.recovery = Recovery::Restarted(fault(12));
        let line = describe(&error);
        assert!(
            line.starts_with("tick 900, object 12: object 12,"),
            "{line}"
        );
        assert!(line.contains("; then, outside its thread, object 13"));
        assert!(line.ends_with("its thread restarted at Main"), "{line}");
        error.then = None;
        error.in_thread = false;
        error.recovery = Recovery::Parked(None);
        let line = describe(&error);
        assert!(line.starts_with("tick 900, object 12 outside its thread:"));
        assert!(line.ends_with("it failed again after a reset, so it stops"));
        error.recovery = Recovery::Parked(Some(fault(12)));
        let line = describe(&error);
        assert!(
            line.contains("restarting its thread failed (object 12,"),
            "{line}"
        );
        assert!(line.ends_with("so it stops"));
    }

    #[test]
    fn an_updates_errors_are_logged_in_the_order_they_happened() {
        let error = ScriptError {
            tick: 1,
            object: 12,
            fault: fault(12),
            in_thread: true,
            then: None,
            recovery: Recovery::Reset,
        };
        let frames =
            |names: &[&str]| -> Vec<String> { names.iter().map(|n| (*n).to_string()).collect() };
        let starts = |lines: Vec<String>| -> Vec<String> {
            lines.into_iter().map(|l| l[..9].to_string()).collect()
        };
        let errors = std::slice::from_ref(&error);
        // The first step: the frame before the tick, the tick, the frame
        // after it.
        assert_eq!(
            starts(in_order(true, errors, &frames(&["b", "a"]))),
            ["a frame w", "tick 1, o", "a frame w"]
        );
        assert!(in_order(true, errors, &frames(&["b", "a"]))[2].ends_with(": a"));
        assert_eq!(
            starts(in_order(true, errors, &frames(&["b"]))),
            ["a frame w", "tick 1, o"]
        );
        // Later steps draw only after the tick.
        assert_eq!(
            starts(in_order(false, errors, &frames(&["a"]))),
            ["tick 1, o", "a frame w"]
        );
        assert!(in_order(false, &[], &[]).is_empty());
    }

    #[test]
    fn errors_show_for_a_while_and_the_log_is_bounded() {
        let mut errors = Errors::default();
        assert_eq!(errors.recent(0), None);
        errors.push(100, "first".into());
        assert_eq!(errors.recent(100).as_deref(), Some("Error caught: first"));
        errors.push(110, "second".into());
        assert_eq!(
            errors.recent(110 + SHOWN_TICKS - 1).as_deref(),
            Some("Error caught (2 so far): second")
        );
        assert_eq!(errors.recent(110 + SHOWN_TICKS), None);
        // The same error at another tick is counted, not listed or shown
        // again.
        errors.push(300, "tick 7, object 2: x".into());
        errors.push(310 + SHOWN_TICKS, "tick 8, object 2: x".into());
        assert_eq!(errors.lines.len(), 3);
        assert_eq!(errors.lines[2], "tick 7, object 2: x");
        assert_eq!(errors.count, 4);
        assert_eq!(errors.recent(310 + SHOWN_TICKS), None);
        for n in 0..LOGGED * 2 {
            errors.push(200, n.to_string());
        }
        assert_eq!(errors.lines.len(), LOGGED + 1);
        assert_eq!(errors.count, LOGGED as u64 * 2 + 4);
        assert_eq!(
            errors.lines.last().unwrap(),
            "further errors are counted, not listed"
        );
        // Once SEEN different errors are remembered, a new one is only
        // counted, so one recurring every tick does not hold the line open.
        for n in errors.seen.len()..SEEN {
            errors.push(400, format!("tick 9, filler {n}"));
        }
        let count = errors.count;
        errors.push(400 + SHOWN_TICKS, "tick 10, object 3: new".into());
        assert_eq!(errors.count, count + 1);
        assert_eq!(errors.recent(400 + SHOWN_TICKS), None);
        assert_eq!(errors.lines.len(), LOGGED + 1);
    }
}
