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
use crate::menu::{self, Listing, Menu};
use crate::pace::{self, Pace};
use crate::people::People;
use crate::pick;
use crate::sound;
use crate::view::{Move, View};
use sims_render::{Camera, Canvas, Selected, Walls, open_house};
use sims_ui::command::{Camera as UiCamera, Command, Speed, Walls as UiWalls};
use sims_vm::SoundEvent;
use sims_vm::boot::save::Saved;
use sims_vm::boot::{Frames, Lot, StepError};
use sims_vm::play::{Recovery, ScriptError};
use std::collections::{BTreeSet, VecDeque};
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

/// The most sound requests kept for the host to take; the oldest go
/// first. A host that plays sound takes them every wake.
const SOUNDS_KEPT: usize = 4096;

/// What the game is started with.
#[derive(Debug, Clone)]
pub struct Options {
    /// The install's root directory.
    pub install: PathBuf,
    /// A house file inside a neighborhood's `Houses` directory.
    pub house: PathBuf,
    /// Answers to the dialogs that offer a choice, in order, as with
    /// `sims boot --answers` (the Goths' house takes `3`, No to the Help
    /// System). A dialog shown once they run out waits for the player
    /// (`Lot::wait_for_answers`).
    pub answers: Vec<u8>,
    /// Step in play mode ([`Lot::step_play`]), which resets an object
    /// after its error and goes on, instead of strict mode ([`Lot::step`]),
    /// which stops the lot at the first error.
    pub play: bool,
    /// After which ticks the house view's frame is drawn, as far as it
    /// writes the simulation's state (`sims_vm::frame`). When the
    /// original draws one is up to its main loop, not the simulation; the
    /// default is the schedule of the captures of the Goth house without
    /// input. A replay of a recording takes the recording's.
    pub frames: Frames,
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

/// How often the window draws the plumb bob's spin when nothing else
/// changes: 20 frames a second, this crate's choice (the original's frame
/// rate is not ported).
pub const SPIN_FRAME_US: u64 = 50_000;

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
    /// The steps' sound requests, in order, until the host takes them.
    sounds: VecDeque<SoundEvent>,
    /// A short message about the last press, until the next one.
    note: Option<String>,
    /// Where the pointer is on the canvas, while it is over the window.
    pointer: Option<[i32; 2]>,
    /// The object the pointer last reached on the lot, if it is still on
    /// it.
    reached: Option<u16>,
    /// The pie menu listed when the pointer reached an object: for whom,
    /// on what, and the listing, until a click opens it.
    listed: Option<(u16, u16, Result<Listing, String>)>,
    /// The entry of the open pie menu the pointer last highlighted
    /// ([`Game::highlight`]).
    highlighted: Option<usize>,
    /// The time the window has passed in, in microseconds: the clock
    /// that spins the plumb bob.
    clock_us: u64,
    /// `clock_us` when the frame was last drawn.
    drawn_us: u64,
    canvas: Canvas,
}

impl Game {
    /// Boots the house and reads everything drawing it needs.
    pub fn open(options: &Options) -> Result<Self, String> {
        let house = &options.house;
        let opened = open_house(&options.install, house).map_err(|e| e.to_string())?;
        let mut lot = Lot::load(
            &options.install,
            house,
            &options.answers,
            options.frames.clone(),
        )
        .map_err(|e| format!("{}: {e}", house.display()))?;
        // The player answers a dialog when it is shown.
        lot.wait_for_answers();
        let stopped = lot
            .load_report()
            .stopped
            .as_ref()
            .map(|s| format!("The load stopped, so the lot cannot run: {s}"));
        // The booted world, not the file as read: some objects (doors,
        // windows) set how they are drawn in their own init scripts.
        let lot_shape = sims_render::Lot::from_world(lot.world()).map_err(|e| e.to_string())?;
        let hud = Hud::open(
            &options.install,
            house,
            &opened.assets,
            lot.world(),
            lot.program().autonomy_constants().friendship_threshold,
        )?;
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
            sounds: VecDeque::new(),
            pointer: None,
            reached: None,
            listed: None,
            highlighted: None,
            note: None,
            clock_us: 0,
            drawn_us: 0,
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

    /// The ambience's camera value (`sims_audio::ambience::view`): the
    /// share, in percent, of the tiles on the screen that are outdoors,
    /// on the floor shown. The original works it out each frame from its
    /// camera (0x570c3e).
    pub fn ambience_view(&self) -> i32 {
        let camera = &self.view.camera;
        // The camera's arithmetic overflows for an origin this far off;
        // no view the controls make comes near it.
        if camera.origin.iter().any(|n| n.unsigned_abs() >= 1 << 26) {
            return 0;
        }
        let corner = camera
            .rotate_fine(camera.unproject_fine([0, 0]))
            .map(|n| n >> 4);
        let level = u8::try_from(camera.level).unwrap_or(0);
        let world = self.lot.world();
        sims_audio::ambience::view(self.view.size, camera.zoom.level(), corner, |tile| {
            let tile = camera.unrotate_tile(tile.map(i32::from));
            world.outside(level, tile).unwrap_or(false)
        })
    }

    /// What of the camera sends the sound's kind 33 when it changes
    /// ([`sound::Shot`]).
    pub fn shot(&self) -> sound::Shot {
        sound::Shot::of(&self.view.camera, self.view.size)
    }

    /// What the sound scripts may ask about the lot's objects, with where
    /// each is on the screen as the camera shows it now
    /// ([`sound::sources`], [`sound::screen`]).
    pub fn sound_sources(&self) -> sims_audio::sources::Snapshot {
        let world = self.lot.world();
        let mut snapshot = sound::sources(world);
        snapshot.screen = sound::screen(world, &self.view.camera, self.view.size);
        snapshot
    }

    /// Why the lot stopped, if it has.
    pub fn stopped(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    /// The errors play mode caught and recovered from, one line each, in
    /// order: the first `LOGGED` (256) different ones (an error again at a
    /// later tick is not listed again), then a line saying more were
    /// caught.
    pub fn errors(&self) -> &[String] {
        &self.errors.lines
    }

    /// The sound requests of the steps run since the last call, in
    /// order, for the sound system ([`crate::sound::event`]).
    pub fn take_sounds(&mut self) -> Vec<SoundEvent> {
        self.sounds.drain(..).collect()
    }

    /// How many errors play mode caught.
    pub fn caught(&self) -> u64 {
        self.errors.count
    }

    /// Problems drawing the people, one line each.
    pub fn problems(&self) -> &[String] {
        &self.people.problems
    }

    /// Sets the game's speed, as the panel's speed buttons do in live
    /// mode (`0x53a62a`, `0x53a6ad`, `0x53a728`): a paused game runs
    /// again (the original applies it after the speed; both restart the
    /// same clocks, so the order does not matter), and the simulation's speed ([`Lot::set_speed`]) and the
    /// time between ticks with it are set, unless the speed is already
    /// the one asked for (`0x4ce588`), when the time owed carries on. A
    /// lot that cannot take it keeps its speed, and the note across the
    /// top says why.
    pub fn set_speed(&mut self, speed: Speed) {
        let level = pace::level(speed);
        let result = if self.paused() {
            self.lot.unpause().map(|_| self.pace.restart())
        } else {
            Ok(())
        };
        let result = result.and_then(|()| {
            if self.lot.world().speed() == level {
                self.pace.speed = speed;
                return Ok(());
            }
            self.lot
                .set_speed(i32::from(level))
                .map(|_| self.pace.set_speed(speed))
        });
        if let Err(e) = result {
            self.note = Some(format!("The speed was not changed: {e}"));
        }
    }

    /// Whether the game is paused (global 17).
    pub fn paused(&self) -> bool {
        self.lot.world().paused()
    }

    /// Pauses the game, or runs it again, as the panel's pause button
    /// does. A paused lot goes on ticking, with its clock standing
    /// ([`Lot::pause`]).
    pub fn toggle_pause(&mut self) {
        let result = if self.paused() {
            self.lot.unpause().map(|_| self.pace.restart())
        } else {
            self.lot.pause().map(|_| ())
        };
        if let Err(e) = result {
            self.note = Some(format!("The pause was not changed: {e}"));
        }
    }

    /// Whether the plumb bob has spun enough since the frame was drawn to
    /// draw it again, though no tick ran (as while a dialog waits): a
    /// plumb bob is on the lot and [`SPIN_FRAME_US`] have passed.
    pub fn spin_due(&self) -> bool {
        self.clock_us.saturating_sub(self.drawn_us) >= SPIN_FRAME_US
            && self
                .lot
                .world()
                .objects
                .values()
                .any(|o| o.guid == sims_people::bob::STANDARD)
    }

    /// Runs the ticks owed after `elapsed_us` more microseconds at the
    /// simulation's speed. Returns how many ran.
    ///
    /// While a dialog waits for the player, no time is owed: the game
    /// waits, as the original does, and the idle loop's speed is not set
    /// (the lot takes nothing between ticks then; **unverified** whether
    /// the original's idle loop sets it while its dialog is up).
    ///
    /// Each call is a pass of the original's idle loop, so it first sets
    /// the idle loop's own speed ([`Lot::idle_speed`]), whether a tick is
    /// owed or not, paused or not: a speed the player picks after that
    /// stands. The same check runs again before each tick owed. Whenever
    /// it calls the setter, even for the speed the game already has, the
    /// time owed starts again and no more ticks run in this call, as the
    /// setter restarts the original's clocks. The original may then run
    /// an update at once, when its second clock (`0x4732de`) is ready
    /// (`0x46da1b`); here the next tick waits a period of the new speed
    /// (**unverified**, and the window's timing, not the simulation's).
    pub fn advance(&mut self, elapsed_us: u64) -> u32 {
        self.clock_us = self.clock_us.saturating_add(elapsed_us);
        if self.lot.asking().is_some() {
            self.pace.restart();
            return 0;
        }
        if self.idle_speed() {
            return 0;
        }
        let due = self.pace.due(elapsed_us, self.lot.world().speed());
        self.run_while(due, true)
    }

    /// Sets the idle loop's own speed between ticks ([`Lot::idle_speed`]).
    /// True if it called the setter, which starts the time owed again.
    fn idle_speed(&mut self) -> bool {
        // A lot that cannot take it (halted, or not loaded) has stopped,
        // and the banner says why.
        let set = self.lot.idle_speed().unwrap_or(false);
        if set {
            self.pace.restart();
        }
        set
    }

    /// The dialog the lot waits on, if it waits for the player.
    pub fn asking(&self) -> Option<&sims_vm::boot::Asking> {
        self.lot.asking()
    }

    /// Where the dialog shown in the last frame drawn is, and its buttons.
    pub fn dialog(&self) -> Option<&sims_ui::dialog::Layout> {
        self.hud.dialog()
    }

    /// Runs up to `ticks` ticks, stopping at the first that fails or that
    /// shows a dialog the player has to answer.
    pub fn run(&mut self, ticks: u32) -> u32 {
        self.run_while(ticks, false)
    }

    /// Runs up to `ticks` ticks, stopping at the first that fails, and,
    /// if `idle`, before a tick the idle loop's speed sets the speed for
    /// ([`Game::idle_speed`]).
    fn run_while(&mut self, ticks: u32, idle: bool) -> u32 {
        let mut ran = 0;
        while ran < ticks && self.stopped.is_none() && !(idle && self.idle_speed()) {
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
                    self.keep_sounds(update.sounds);
                }
                // The tick shows a dialog: it runs once it is answered,
                // and no time is owed from before the wait.
                Err(StepError::Asking { .. }) => {
                    self.pace.restart();
                    break;
                }
                Err(error) => {
                    // A strict step whose frame failed after its tick ran
                    // still made that tick's sounds.
                    if let StepError::Frame {
                        update: Some(update),
                        ..
                    } = &error
                    {
                        self.keep_sounds(update.sounds.clone());
                    }
                    self.stop(&error);
                }
            }
        }
        if ran > 0 || self.stopped.is_some() {
            let world = self.lot.world();
            self.people.refresh(world);
            // The room lights change as lamps and the time of day do.
            match sims_render::Lot::from_world(world) {
                Ok(shape) => self.lot_shape = shape,
                Err(e) => {
                    self.people
                        .note(vec![format!("the lot is not redrawn: {e}")]);
                }
            }
        }
        ran
    }

    /// Keeps a step's sound requests for the host, at most
    /// [`SOUNDS_KEPT`], the oldest dropped first.
    fn keep_sounds(&mut self, sounds: Vec<SoundEvent>) {
        keep(&mut self.sounds, sounds, SOUNDS_KEPT);
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

    /// Draws the frame: the lot, its people, then the panel. When the
    /// frame shows a new object under the pointer (the object moved, or
    /// the view did), its pie menu is listed, as when the pointer moves
    /// onto it, and the frame drawn again if the listing changed the lot.
    pub fn render(&mut self) -> &Canvas {
        self.draw();
        if self.follow() {
            self.draw();
        }
        &self.canvas
    }

    /// Draws the frame.
    fn draw(&mut self) {
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
        // The lot, then the people, then the objects that hold someone
        // (a bench a Sim sits on), as the original's frame does.
        sims_render::draw_lot(
            world,
            &self.lot_shape,
            &mut self.assets,
            &self.view.camera,
            &mut self.canvas,
        );
        // The plumb bob spins by the game's clock in milliseconds (the
        // original's 0x6e54e0); here the time passed in, which runs on
        // while the lot is paused or a dialog waits (**unverified**
        // whether the original's does).
        self.drawn_us = self.clock_us;
        let milliseconds = (self.clock_us / 1000) as u32;
        self.people.set_clock(milliseconds);
        self.people.draw(world, &self.view.camera, &mut self.canvas);
        sims_render::draw_held(
            world,
            &self.lot_shape,
            &mut self.assets,
            &self.view.camera,
            &mut self.canvas,
        );
        // The pie menu shows the head of the Sim it is for, as posed now.
        if let Some(menu) = &mut self.hud.menu {
            menu.head = match self.people.head(world, menu.person, menu.face_size()) {
                Ok(head) => Some(head),
                Err(e) => {
                    self.people
                        .note(vec![format!("the pie menu has no head: {e}")]);
                    None
                }
            };
        }
        let status = self.status();
        // A dialog the lot waits on is in the lot as the tick left it when
        // it was shown.
        let dialogs = self.lot.asking().map_or(world, |a| &a.world);
        self.hud.read_dialog(dialogs, &self.assets);
        self.hud.draw(
            world,
            &self.assets,
            &self.view,
            &self.pace,
            status.as_deref(),
            &mut self.canvas,
        );
        let problems = self.hud.take_problems();
        self.people.note(problems);
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

    /// Moves the camera. A turn is the lot's input too: Generic Sims Call
    /// 12 reads the view's rotation ([`Lot::set_view_rotation`]).
    pub fn apply(&mut self, command: Move) -> bool {
        let moved = self.view.apply(command);
        self.lot.set_view_rotation(self.view.camera.rotation);
        moved
    }

    /// Whether the panel takes presses at `point`.
    pub fn over_panel(&self, point: [i32; 2]) -> bool {
        self.hud.covers(point)
    }

    /// Moves the pointer to `point`. When it first reaches an object on
    /// the lot, the selected Sim's pie menu on it is listed, as the
    /// original lists it then (0x48c834); listing changes the lot.
    pub fn hover(&mut self, point: [i32; 2]) -> bool {
        self.pointer = Some(point);
        let mut redraw = self.hud.hover(point);
        redraw |= self.highlight(point);
        redraw |= self.follow();
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
            // The menu, whose entries a choice is an index into.
            let menu = self.hud.menu.clone();
            let outcome = match self.hud.press(point) {
                Some(command) => self.command(command, menu.as_ref()),
                None => Outcome::redraw(),
            };
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return outcome;
        }
        let Some(target) = self.under(point) else {
            return Outcome {
                redraw: cleared,
                photo: false,
            };
        };
        self.open_menu(target, point);
        Outcome::redraw()
    }

    /// The pointer left the window: it is on no object, button, portrait
    /// or pie entry, so coming back onto an entry checks it again. True
    /// if the frame changes.
    pub fn leave(&mut self) -> bool {
        self.pointer = None;
        self.highlighted = None;
        let redraw = self.hud.leave();
        self.follow() || redraw
    }

    /// Follows the object under the pointer in the frame last drawn: when
    /// it changes to an object, lists the selected Sim's pie menu on it.
    /// Nothing is listed while the pointer is over the panel, a dialog or
    /// a menu, or once the lot has stopped. True if a listing may have
    /// changed the lot.
    fn follow(&mut self) -> bool {
        let under = match self.pointer {
            Some(point) if self.stopped.is_none() && !self.hud.covers(point) => self.under(point),
            _ => None,
        };

        if under == self.reached {
            return false;
        }
        self.reached = under;
        self.listed = None;
        under.is_some_and(|object| self.list(object))
    }

    /// For whom and on what the pie menu was listed when the pointer
    /// reached an object, until a click opens it or the pointer leaves.
    pub fn listed(&self) -> Option<(u16, u16)> {
        self.listed
            .as_ref()
            .map(|(person, object, _)| (*person, *object))
    }

    /// Checks again the action of the open pie menu's entry the pointer
    /// at `point` has reached, as the original does each time the pointer
    /// reaches an entry (0x48df28, `Lot::highlight`), which changes the
    /// lot. The action the check leaves is the one choosing the entry
    /// queues. True if the check ran.
    fn highlight(&mut self, point: [i32; 2]) -> bool {
        let Some(menu) = &self.hud.menu else {
            self.highlighted = None;
            return false;
        };
        let entry = menu.highlighted(point);
        if entry == self.highlighted {
            return false;
        }
        self.highlighted = entry;
        let Some((i, Some(action))) = entry.map(|i| (i, menu.entries[i].action.clone())) else {
            return false;
        };
        if self.stopped.is_some() {
            return false;
        }
        let person = menu.person;
        match self.lot.highlight(person, action) {
            Ok(checked) => {
                self.keep_sounds(checked.sounds);
                if let Some(entry) = self.hud.menu.as_mut().and_then(|m| m.entries.get_mut(i)) {
                    entry.action = Some(checked.action);
                }
            }
            Err(e) => self.note = Some(format!("The entry's check did not run: {e}")),
        }
        true
    }

    /// The person or object shown at `point` on the lot in the frame last
    /// drawn.
    pub fn under(&self, point: [i32; 2]) -> Option<u16> {
        pick::at(self.lot.world(), &self.canvas, point)
    }

    /// Lists the selected Sim's pie menu on `object` (`Lot::pie_menu`,
    /// which changes the lot), keeping its sounds for the host. True if
    /// the simulation listed it, and so may have changed the lot.
    fn list(&mut self, object: u16) -> bool {
        let Some(person) = self.lot.world().selected() else {
            return false;
        };
        let listing = menu::entries(&mut self.lot, &self.assets, person, object);
        let changed = listing.as_ref().is_ok_and(|l| l.unchecked.is_none());
        if let Ok(listing) = &listing {
            self.keep_sounds(listing.sounds.clone());
        }
        self.listed = Some((person, object, listing));
        changed
    }

    /// Opens the pie menu of `object` at `point` for the selected Sim:
    /// the one listed when the pointer reached it, or, when it was not
    /// listed then (the pointer did not move onto it first), one listed
    /// now.
    fn open_menu(&mut self, object: u16, point: [i32; 2]) {
        if self.stopped.is_some() {
            self.note = Some("The lot has stopped, so it takes no orders.".into());
            return;
        }
        let Some(person) = self.lot.world().selected() else {
            self.note = Some("Nobody is selected to do anything.".into());
            return;
        };
        // The press lists the menu again: the live mode's press handler
        // (0x48cbec) first sets the object under the pointer to none
        // (0x48c834 with 0), so the press's own pick lists it anew.
        self.list(object);
        self.reached = Some(object);
        let Some((_, _, listing)) = self.listed.take() else {
            return;
        };
        match listing {
            Ok(listing) => {
                // Where the simulation cannot list the menu yet, the note
                // says so and why.
                let why = listing
                    .unchecked
                    .map(|reason| format!("the simulation cannot list this menu yet: {reason}"));
                if listing.entries.is_empty() {
                    let line = format!("Object {object} offers nothing to do");
                    self.note = Some(match why {
                        Some(why) => format!("{line} ({why})."),
                        None => format!("{line}."),
                    });
                } else {
                    self.note = why.map(|why| format!("Every entry is shown, as {why}"));
                    self.highlighted = None;
                    self.hud.menu = Some(Menu::new(
                        person,
                        object,
                        listing.entries,
                        point,
                        &self.hud.art,
                        self.view.size,
                    ));
                }
            }
            Err(e) => self.note = Some(format!("Object {object}'s menu is not listed: {e}")),
        }
    }

    /// The control panel as it was last laid out, from the world as it
    /// was when the frame was drawn ([`Game::render`]).
    pub fn panel(&self) -> &[sims_ui::hud::Item] {
        self.hud.items()
    }

    /// What the control panel shows of `world`: [`Game::render`] lays
    /// the panel out from this for the lot's world.
    pub fn snapshot(&self, world: &sims_sim::World) -> sims_ui::household::Snapshot {
        self.hud.snapshot(world)
    }

    /// The first name the panel shows for people of type `guid`.
    pub fn name(&self, guid: u32) -> Option<&str> {
        self.hud.name(guid)
    }

    /// The pie menu, while one is open. Its labels are placed when the
    /// game is rendered.
    pub fn menu(&self) -> Option<&Menu> {
        self.hud.menu.as_ref()
    }

    /// Where the window is to move the pointer (canvas pixels), once:
    /// the original moves it to the middle of a pie menu that opened
    /// elsewhere than asked, and of a submenu that opens.
    pub fn take_pointer(&mut self) -> Option<[i32; 2]> {
        self.hud.take_pointer()
    }

    /// Closes the pie menu; false if none was open.
    pub fn close_menu(&mut self) -> bool {
        self.hud.menu.take().is_some()
    }

    /// Queues `entry` of the pie menu on `object` for `person`, as
    /// choosing it does: the action the simulation offered
    /// (`Lot::pick`), or for a fallback entry the entry's menu number
    /// (`Lot::queue_interaction`).
    fn order(&mut self, person: u16, object: u16, entry: &menu::Entry) {
        let label = &entry.label;
        let queued = match &entry.action {
            Some(action) => self.lot.pick(person, action.clone()),
            None => self.lot.queue_interaction(person, object, entry.menu),
        };
        self.note = Some(match queued {
            Ok(Some(_)) => format!("Queued \"{label}\" for person {person}."),
            Ok(None) => format!("Person {person}'s queue is full; \"{label}\" was not queued."),
            Err(e) => format!("\"{label}\" was not queued: {e}"),
        });
    }

    /// Cancels `sim`'s action numbered `action`, as a press on its icon
    /// does (`Lot::cancel`, 0x4ea3e3), keeping the sounds the trees made,
    /// then plays `UI_queue_delete` if the press counts and `UI_error` if
    /// not (0x4cb16e). A cancel the lot refuses, or a stopped lot, leaves
    /// the icon to take another press.
    fn cancel(&mut self, sim: u16, action: i32) {
        if self.stopped.is_some() {
            self.note = Some("The lot has stopped, so it takes no orders.".into());
            self.hud.release(action);
            return;
        }
        match self.lot.cancel(sim, action) {
            Ok(cancelled) => {
                self.keep_sounds(cancelled.sounds);
                self.hud.play(if cancelled.counts {
                    crate::hud::QUEUE_DELETE
                } else {
                    crate::hud::ERROR
                });
            }
            Err(e) => {
                // The lot is as it was, so the icon takes another press.
                self.hud.release(action);
                self.note = Some(format!("The action was not cancelled: {e}"));
            }
        }
    }

    /// The selected Sim's actions as the frame last drawn showed them.
    pub fn queue(&self) -> &sims_ui::queue::Queue {
        self.hud.queue()
    }

    /// Answers the dialog `caller` shows with its button `button` (0 the
    /// first): the answer the button gives (`Shown::answers`) goes to the
    /// lot, and the tick that showed it runs again at once.
    pub fn answer(&mut self, caller: u16, button: u8) {
        let Some(asking) = self.lot.asking() else {
            self.note = Some("No dialog waits for an answer.".into());
            return;
        };
        if asking.shown.object != caller {
            self.note = Some(format!(
                "The dialog shown is object {}'s, not object {caller}'s.",
                asking.shown.object
            ));
            return;
        }
        let Some(&answer) = asking.shown.answers.get(usize::from(button)) else {
            self.note = Some(format!("The dialog has no button {button}."));
            return;
        };
        if let Err(e) = self.lot.answer(answer) {
            self.note = Some(format!("The answer was not taken: {e}"));
            return;
        }
        // The tick that showed the dialog runs again now, with the answer,
        // so nothing is listed or ordered on the lot as it was before it.
        self.run(1);
    }

    /// A press on the portrait of the Sim `sim` (0x4cf9c4). On another
    /// Sim's portrait it selects `sim` (`Lot::select`, through 0x4e989b
    /// and 0x445449), keeping the sounds that made, and drops a menu
    /// listed for the Sim selected before. On the selected Sim's own
    /// portrait it centres the view on the Sim instead
    /// ([`Game::centre_on`]). Either way it then plays `UI_click`
    /// (0x4cfa51).
    ///
    /// On a residential lot 0x4e989b selects the person only when person
    /// word 32 is 0 (it lives here), and nobody otherwise; the portraits
    /// show only the household's people, whose word is 0, so that is not
    /// checked here. Not ported: a person the household's table names
    /// but who is not on the lot (0x4e9652), and the people panel's call
    /// on a press on the selected Sim's portrait (0x51ea7a, behind a
    /// check of global 0x6f6bac) whose effect has not been read.
    pub fn select(&mut self, sim: u16) {
        if self.lot.world().selected() == Some(sim) {
            self.centre_on(sim);
            self.hud.play(crate::hud::CLICK);
            return;
        }
        match self.lot.select(sim) {
            Ok(sounds) => {
                self.keep_sounds(sounds);
                self.hud.play(crate::hud::CLICK);
            }
            Err(e) => self.note = Some(format!("Person {sim} was not selected: {e}")),
        }
        self.listed = None;
        self.reached = None;
    }

    /// Centres the view on the object `id` as 0x434ced does with 1, but
    /// at once: the original slides the view there over a while
    /// (0x4c9a27, the time at 0x6c7788), which is left out.
    ///
    /// Nothing happens unless object data word 34 is 0 and word 29 is not
    /// negative. A person on the floor shown is centred a little above
    /// its feet: its screen point ([`sound::object_point`]) raised by
    /// half of 2 √6 times 2.0 times 2 to the power of the zoom plus one,
    /// cut to a whole number (19, 39 and 78 pixels from far to near),
    /// comes to the middle of the screen (0x4a7777). The original then
    /// holds the view inside its scroll limits (0x4a9fa9), which the
    /// window does not have. Anything else, or a person on another floor,
    /// shows its floor (0x4a80c7) and, unless it stands off the lot
    /// (0x4a7af0), its screen point on that floor comes to the middle
    /// (0x4a7996).
    pub fn centre_on(&mut self, id: u16) {
        let Some(object) = self.lot.world().objects.get(&id) else {
            return;
        };
        let word = |i: usize| object.data.get(i).copied().unwrap_or(0);
        if word(34) != 0 || word(29) < 0 {
            return;
        }
        let position = object.position;
        let level = position[2];
        if object.person.is_some() && level == self.view.camera.level {
            let mut at = lot_point(&self.view.camera, position);
            at[1] -= person_rise(self.view.camera.zoom.level());
            self.view.apply(Move::Centre(at));
            return;
        }
        self.view.apply(Move::Floor(level));
        // The lot's bounds (0x4a7af0 reads them from the scene) are taken
        // as its tiles, which is **unverified** at the edges.
        if position[..2].iter().any(|&n| !(0..64 * 16).contains(&n)) {
            return;
        }
        let at = lot_point(&self.view.camera, position);
        self.view.apply(Move::Centre(at));
    }

    /// The lot as it stands, to go back to later ([`Game::restore`]).
    /// While a dialog waits, the save is of the lot before the tick that
    /// showed it, so the restored lot shows it again.
    pub fn save(&self) -> Result<Saved, String> {
        self.lot.save().map_err(|e| e.to_string())
    }

    /// Keeps the lot in `store` under `slot`, saying across the top how
    /// that went.
    pub fn save_to(&mut self, store: &sims_sim::Snapshots, slot: &str) {
        let saved = self.save().and_then(|saved| {
            store
                .save_lot(slot, &saved.encode())
                .map_err(|e| e.to_string())
        });
        self.note = Some(match saved {
            Ok(()) => format!("Saved after tick {}.", self.lot.ticks()),
            Err(e) => format!("The lot was not saved: {e}"),
        });
    }

    /// Goes back to the lot saved in `store` under `slot`, saying across
    /// the top how that went. False if nothing changed.
    pub fn restore_from(&mut self, store: &sims_sim::Snapshots, slot: &str) -> bool {
        let saved = match store.load_lot(slot) {
            Ok(Some(bytes)) => Saved::decode(&bytes).map_err(|e| e.to_string()),
            Ok(None) => Err("nothing is saved for this house".into()),
            Err(e) => Err(e.to_string()),
        };
        let restored = saved.and_then(|saved| self.restore(saved));
        self.note = Some(match &restored {
            Ok(()) => format!("Back to the save after tick {}.", self.lot.ticks()),
            Err(e) => format!("The save was not restored: {e}"),
        });
        restored.is_ok()
    }

    /// Goes back to `saved`, a save of this house ([`Lot::restore`]). The
    /// lot then steps as the saved one would have. What the window kept
    /// of the lot before is dropped: why it stopped, the errors caught,
    /// the sounds not yet taken, an open or listed pie menu, the icons
    /// pressed and the dialog read. The panel's speed follows the saved
    /// lot's ([`pace::shown`]), the view turns to the saved lot's
    /// rotation, which the lot's code reads, and its people and rooms are
    /// read again. Sounds already playing are not stopped (**unverified** what a long one does once
    /// its object is gone).
    pub fn restore(&mut self, saved: Saved) -> Result<(), String> {
        self.lot.restore(saved).map_err(|e| e.to_string())?;
        self.stopped = None;
        self.errors = Errors::default();
        self.sounds.clear();
        self.reached = None;
        self.listed = None;
        self.highlighted = None;
        self.hud.forget();
        let world = self.lot.world();
        if let Some(speed) = pace::shown(world) {
            self.pace.speed = speed;
        }
        // The view turns as the turn keys do, keeping the middle of the
        // screen, until it shows the lot as the saved lot was turned.
        let rotation = self.lot.view_rotation();
        for _ in 0..4 {
            if self.view.camera.rotation == rotation {
                break;
            }
            self.view.apply(Move::Rotate(true));
        }
        self.pace.restart();
        self.people.refresh(world);
        match sims_render::Lot::from_world(world) {
            Ok(shape) => self.lot_shape = shape,
            Err(e) => self
                .people
                .note(vec![format!("the lot is not redrawn: {e}")]),
        }
        Ok(())
    }

    /// Carries out a panel command. `menu` is the pie menu that was open
    /// when it was pressed.
    fn command(&mut self, command: Command, menu: Option<&Menu>) -> Outcome {
        let moved = |view: &mut View, m: Move| {
            view.apply(m);
            Outcome::redraw()
        };
        match command {
            Command::SetSpeed(speed) => {
                self.set_speed(speed);
                Outcome::redraw()
            }
            Command::TogglePause => {
                self.toggle_pause();
                Outcome::redraw()
            }
            Command::Camera(camera) => match camera {
                UiCamera::ZoomIn => moved(&mut self.view, Move::ZoomIn),
                UiCamera::ZoomOut => moved(&mut self.view, Move::ZoomOut),
                // Which way the original turns for each is unverified.
                UiCamera::RotateRight => {
                    self.apply(Move::Rotate(true));
                    Outcome::redraw()
                }
                UiCamera::RotateLeft => {
                    self.apply(Move::Rotate(false));
                    Outcome::redraw()
                }
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
            Command::ChooseInteraction {
                sim,
                object,
                interaction,
            } => {
                // `interaction` is the entry's index in the open menu.
                match menu.and_then(|m| m.entries.get(usize::from(interaction))) {
                    Some(entry) => self.order(sim, object, entry),
                    None => self.note = Some(format!("No menu entry {interaction} to choose.")),
                }
                Outcome::redraw()
            }
            Command::Select { sim } => {
                self.select(sim);
                Outcome::redraw()
            }
            Command::CancelAction { sim, action } => {
                self.cancel(sim, action);
                Outcome::redraw()
            }
            Command::AnswerDialog { caller, button } => {
                self.answer(caller, button);
                Outcome::redraw()
            }
            Command::TogglePanel
            | Command::OpenPanel(_)
            | Command::ShowRelations(_)
            | Command::RelationsPage(_)
            | Command::Help(_) => Outcome::redraw(),
            other => {
                self.note = Some(format!("Not carried out yet: {other:?}"));
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

/// Adds `sounds` to the requests the host has yet to take, keeping at
/// most `kept`: the oldest go first.
fn keep(queue: &mut VecDeque<SoundEvent>, sounds: Vec<SoundEvent>, kept: usize) {
    queue.extend(sounds);
    let over = queue.len().saturating_sub(kept);
    queue.drain(..over);
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

/// How far above its feet a centred person is (0x434d64 to 0x434da7):
/// 2 √6 (single precision, 0x434df6) times 2.0 (1/3 times 6, single)
/// times 2 to the power of the zoom plus one, cut toward zero, halved.
/// The screen point of a fine lot position ([`sound::object_point`]),
/// worked out from the lot's own corner and then moved by the origin:
/// the original cuts the point to 16 bits, and its scroll limits
/// (0x4a9fa9) keep its origin small enough for that, where the window's
/// origin may go much further.
fn lot_point(camera: &Camera, position: [i32; 3]) -> [i32; 2] {
    let at = sound::object_point(
        &Camera {
            origin: [0, 0],
            ..*camera
        },
        position,
    );
    [at[0] + camera.origin[0], at[1] + camera.origin[1]]
}

fn person_rise(zoom: u32) -> i32 {
    let rise = (2.0 * 6f64.sqrt()) as f32 * 2.0 * (1u32 << (zoom + 1)) as f32;
    rise as i32 / 2
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
    fn a_centred_person_is_raised_by_half_its_zooms_height() {
        // trunc(2 sqrt 6 * 2.0 * 2^(zoom + 1)) / 2 for far, medium, near.
        assert_eq!([1, 2, 3].map(super::person_rise), [19, 39, 78]);
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

    #[test]
    fn the_sound_queue_keeps_the_newest_and_hands_them_over_once() {
        let play = |object: i16| SoundEvent::Play {
            object,
            name: format!("sound {object}"),
        };
        let mut queue = VecDeque::new();
        keep(&mut queue, (0..3).map(play).collect(), 4);
        keep(&mut queue, (3..6).map(play).collect(), 4);
        // The two oldest are dropped; the rest stay in order.
        assert_eq!(
            Vec::from(queue.clone()),
            (2..6).map(play).collect::<Vec<_>>()
        );
        keep(&mut queue, Vec::new(), 4);
        assert_eq!(queue.len(), 4);
        // Taking them empties the queue, as `Game::take_sounds` does.
        let taken: Vec<_> = queue.drain(..).collect();
        assert_eq!(taken.len(), 4);
        assert!(queue.is_empty());
    }
}
