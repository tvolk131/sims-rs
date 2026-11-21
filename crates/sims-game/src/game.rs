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
use sims_ui::command::{Camera as UiCamera, Command, Mode, Speed, Walls as UiWalls};
use sims_vm::boot::save::Saved;
use sims_vm::boot::{Frames, Lot, StepError};
use sims_vm::play::{Recovery, ScriptError};
use sims_vm::{BuyCursor, PanelMode, SkippedFrame, SoundEvent};
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
    /// The Buy Mode catalog item currently armed for placement (`0x486f5a`).
    buy_selected: Option<u32>,
    /// The active Buy Mode `"Tile Cursor"` and held catalog object, once the
    /// pointer has entered the house view (`0x487d3d`).
    buy_cursor: Option<BuyCursor>,
    /// Whether the pointer is currently inside the house view in Buy Mode
    /// (`[tool + 0x97]`, `0x487d3d` / `0x487d4e`), so re-entering after
    /// leaving onto the catalog recreates the preview via `0x486f5a`.
    buy_in_house: bool,
    /// Last Buy Mode catalog GUID instantiated by `0x486f5a` (`tool[0x12]`).
    buy_last_guid: Option<u32>,
    /// Tool-stored Buy Mode facing (`tool[0x10]`, initialized by `0x486f5a`
    /// when switching catalog GUIDs and updated on placement commit
    /// `0x486ada`).
    buy_facing: Option<i16>,
    /// The screen anchor of an active Buy Mode left-button press over the lot
    /// (`0x485f98`), for drag-to-rotate (`dx^2 + dy^2 > 36`, `0x485a2e`) and
    /// commit on release (`0x4864d0`).
    buy_dragging: Option<[i32; 2]>,
    /// Whether Shift is currently held (`VK_SHIFT`, `0x4864f9`), which rearms
    /// any Buy Mode item for repeat purchase on placement (`0x48680f`).
    shift_held: bool,
    /// The time the window has passed in, in microseconds: the clock
    /// that spins the plumb bob and drives the pause button's 500 ms blink.
    clock_us: u64,
    /// `clock_us` when the frame was last drawn.
    drawn_us: u64,
    /// Whether the Live-mode pause button's blink phase flipped since the
    /// last frame was drawn.
    blink_dirty: bool,
    canvas: Canvas,
    /// The camera the frame last drawn was drawn through, the depth its
    /// lot left before anything else was drawn, and its IDs once its
    /// people were drawn, before the objects holding them: what the
    /// pointer picks from ([`Game::under`]).
    drawn_camera: Camera,
    lot_depth: Vec<i32>,
    people_ids: sims_render::Ids,
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
        let lot_shape = sims_render::Lot::from_world(lot.world())
            .map_err(|e| e.to_string())?
            .with_floor_rotation(lot.floor_rotation());
        let mut hud = Hud::open(
            &options.install,
            house,
            &opened.assets,
            lot.world(),
            lot.program().autonomy_constants().friendship_threshold,
        )?;
        hud.sync_clock_blink(lot.world(), 0);
        let mut people = People::open(&options.install, house)?;
        people.refresh(lot.world());
        let view = View::new([800, 600], lot_shape.built_center(), lot.view_rotation());
        let drawn_camera = view.camera;
        let mut game = Self {
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
            buy_selected: None,
            buy_cursor: None,
            buy_in_house: false,
            buy_last_guid: None,
            buy_facing: None,
            buy_dragging: None,
            shift_held: false,
            note: None,
            clock_us: 0,
            drawn_us: 0,
            blink_dirty: false,
            canvas: Canvas::new(1, 1, [0, 0, 0]),
            drawn_camera,
            lot_depth: Vec::new(),
            people_ids: sims_render::Ids::default(),
        };
        game.sync_view();
        Ok(game)
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
        snapshot.screen = sound::screen(world, &self.lot_shape, &self.view.camera, self.view.size);
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
        match result {
            Ok(()) => {
                // 0x53a5e3, 0x53a661..0x53a666, 0x53a6dc..0x53a6e1: manual
                // speed presses in Live mode clear `[control_panel + 0x1ec]`.
                if self.hud.controls.mode == sims_ui::command::Mode::Live {
                    self.hud.set_idle_speed(false);
                }
            }
            Err(e) => {
                self.note = Some(format!("The speed was not changed: {e}"));
            }
        }
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
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
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
    }

    /// Whether the pause button's 500 ms blink phase changed since the
    /// frame was last drawn.
    pub fn blink_due(&self) -> bool {
        self.blink_dirty
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
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
        if self.lot.asking().is_some() {
            self.pace.restart();
            return 0;
        }
        if self.idle_speed() {
            return 0;
        }
        let due = self.pace.due(elapsed_us, self.lot.world().speed());
        let ran = self.run_while(due, true);
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
        ran
    }

    /// Sets the idle loop's own speed between ticks ([`Lot::idle_speed`]).
    /// True if it called the setter, which starts the time owed again.
    fn idle_speed(&mut self) -> bool {
        // A lot that cannot take it (halted, or not loaded) has stopped,
        // and the banner says why.
        let set = self.lot.idle_speed().unwrap_or(false);
        if set {
            self.pace.restart();
            // 0x46d7b5 -> 0x109(1) when entering idle speed, 0x46d803 ->
            // 0x109(0) when leaving (`return_speed` reset to -1).
            self.hud
                .set_idle_speed(self.lot.world().return_speed() >= 0);
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
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
    /// ([`Game::idle_speed`]) or after a tick that changes the
    /// simulation's speed or return speed (such as Special Effect resetting
    /// to normal, `0x4f684c` → `0x46d697`). Any mid-tick speed or return-speed
    /// change updates the panel's speed and restarts the time owed, whether
    /// `idle` is set or not.
    fn run_while(&mut self, ticks: u32, idle: bool) -> u32 {
        let mut ran = 0;
        while ran < ticks && self.stopped.is_none() && !(idle && self.idle_speed()) {
            let speed_before = (self.lot.world().speed(), self.lot.world().return_speed());
            let step = if self.play {
                self.lot.step_play()
            } else {
                self.lot.step()
            };
            match step {
                Ok(update) => {
                    ran += 1;
                    let at = self.lot.ticks();
                    for line in in_order(at, &update.errors, &update.frame_errors) {
                        self.errors.push(at, line);
                    }
                    self.view.masks_rebuilt(update.masks_rebuilt);
                    self.keep_sounds(update.sounds);
                }
                // The tick shows a dialog: it runs once it is answered,
                // and no time is owed from before the wait.
                Err(StepError::Asking { .. }) => {
                    self.pace.restart();
                    break;
                }
                Err(error) => {
                    match &error {
                        // A strict step whose frame failed after its tick
                        // ran still made that tick's sounds.
                        StepError::Frame {
                            update: Some(update),
                            ..
                        } => {
                            self.view.masks_rebuilt(update.masks_rebuilt);
                            self.keep_sounds(update.sounds.clone());
                        }
                        // A play step whose tick failed hands back what
                        // it caught and made before the failure.
                        StepError::Tick {
                            tick,
                            update: Some(update),
                            ..
                        } => {
                            for line in in_order(*tick, &update.errors, &update.frame_errors) {
                                self.errors.push(*tick, line);
                            }
                            self.view.masks_rebuilt(update.masks_rebuilt);
                            self.keep_sounds(update.sounds.clone());
                        }
                        _ => {}
                    }
                    self.stop(&error);
                }
            }
            // `Lot::step` runs `World::idle_speed` before the tick (0x46d9d6).
            // Detect `0x109(1)` / `0x109(0)` transitions directly from
            // `return_speed`'s sign change so even a same-tick Special Effect
            // `0x10c` reset (`(NORMAL, -1) -> (IDLE, NORMAL) -> (NORMAL, NORMAL)`)
            // arms `[control_panel + 0x1ec]` under both `Game::run` and
            // `Game::advance`.
            let return_after = self.lot.world().return_speed();
            if speed_before.1 < 0 && return_after >= 0 {
                self.hud.set_idle_speed(true);
            } else if speed_before.1 >= 0 && return_after < 0 {
                self.hud.set_idle_speed(false);
            }
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
            if sync_speed_after_step(&mut self.pace, speed_before, self.lot.world()) && idle {
                break;
            }
        }
        if ran > 0 || self.stopped.is_some() {
            let world = self.lot.world();
            self.people.refresh(world);
            // The room lights change as lamps and the time of day do.
            match sims_render::Lot::from_world(world) {
                Ok(shape) => self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation()),
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
        let viewport = self.house_viewport_size();
        // The lot, then the people, then the objects that hold someone
        // (a bench a Sim sits on), as the original's frame does.
        let preview = self.buy_cursor.as_ref().map(|c| (c.cursor, c.held));
        sims_render::draw_lot_with_preview(
            world,
            &self.lot_shape,
            &mut self.assets,
            &self.view.camera,
            viewport,
            preview,
            &mut self.canvas,
        );
        self.drawn_camera = self.view.camera;
        self.lot_depth.clone_from(&self.canvas.depth);
        // The plumb bob spins by the game's clock in milliseconds (the
        // original's 0x6e54e0); here the time passed in, which runs on
        // while the lot is paused or a dialog waits (**unverified**
        // whether the original's does).
        self.drawn_us = self.clock_us;
        let milliseconds = (self.clock_us / 1000) as u32;
        self.people.set_clock(milliseconds);
        self.people
            .draw_in_viewport(world, &self.view.camera, viewport, &mut self.canvas);
        // Who the pick notes: the people as drawn, before a bench they sit
        // on draws over them.
        self.people_ids.take(&self.canvas);
        sims_render::draw_held(
            world,
            &self.lot_shape,
            &mut self.assets,
            &self.view.camera,
            &mut self.canvas,
        );
        let status = self.status();
        // A dialog the lot waits on is in the lot as the tick left it when
        // it was shown. Read it before the pie menu so a newly opened dialog
        // suppresses the pie's dark backdrop and head on the frame it closes
        // the menu.
        let dialogs = self.lot.asking().map_or(world, |a| &a.world);
        self.hud.read_dialog(dialogs, &self.assets);
        // The pie menu shades the house viewport behind it (0x4a94ec →
        // 0x4ac8ff) and renders the Sim's 3D head tracking the pointer
        // (0x4a953e → 0x462290 → 0x462985) directly into the house viewport
        // before the 2D HUD draws, so oversized headwear is clipped only by
        // the house viewport (not the 201×201 face bitmap), the bottom control
        // strip hides anything below the viewport, and pie labels draw on top.
        if !self.hud.will_show_dialog(self.view.size) {
            self.hud.resize_menu(self.view.size);
            let pointer = self.hud.pointer();
            if let Some(menu) = &mut self.hud.menu {
                if let Some(center) = menu.active_center() {
                    crate::people::shade_pie_face(&mut self.canvas, center, viewport);
                }
                if let Some(center) = menu.main_center() {
                    let look = menu.head_look(pointer);
                    match self.people.head_in_viewport(
                        world,
                        menu.person,
                        viewport,
                        center,
                        self.view.camera.zoom,
                        look,
                    ) {
                        Ok((origin, head, depth)) => {
                            crate::people::composite_pie_head(
                                &mut self.canvas,
                                origin,
                                &head,
                                &depth,
                                viewport,
                            );
                            menu.head_origin = origin;
                            menu.head = Some(head);
                            menu.head_depth = depth;
                        }
                        Err(e) => {
                            self.people
                                .note(vec![format!("the pie menu has no head: {e}")]);
                            menu.head_origin = [0, 0];
                            menu.head = None;
                            menu.head_depth.clear();
                        }
                    }
                }
            }
        }
        self.hud.sync_clock_blink(world, self.clock_us);
        self.blink_dirty = false;
        self.hud.draw(
            world,
            self.lot.residential(),
            &self.assets,
            &self.view,
            &self.pace,
            status.as_deref(),
            &mut self.canvas,
        );
        // The help balloon closed by the selection's going.
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        let problems = self.hud.take_problems();
        self.people.note(problems);
    }

    /// The canvas last drawn.
    pub fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    /// Passes the house view's screen-centre pick (`0x4a7b58` over the active
    /// house viewport [`Game::house_viewport_size`], at the original's origin
    /// column `camera.origin[0] + pick::ORIGIN_COLUMN`, cut to a tile via
    /// `0x433067`) into the simulation.
    fn sync_view_centre(&mut self) {
        let (w, h) = self.house_viewport_size();
        let mut camera = self.view.camera;
        camera.origin[0] = camera.origin[0].saturating_add(pick::ORIGIN_COLUMN);
        let centre =
            sims_render::screen_centre_pick(&self.lot_shape, &camera, [w as i32, h as i32])
                .map(|[x, y]| [(x >> 4) as i8, (y >> 4) as i8]);
        self.lot.set_view_centre(centre);
    }

    /// Passes the house view's rotation and screen-centre pick into the
    /// simulation.
    fn sync_view(&mut self) {
        self.lot.set_view_rotation(self.view.camera.rotation);
        self.sync_view_centre();
    }

    /// Resizes the canvas to `size` of the original's pixels.
    pub fn resize(&mut self, size: [i32; 2]) -> bool {
        if size == self.view.size {
            return false;
        }
        self.view.resize(size);
        self.sync_view_centre();
        self.hud.resize_menu(size);
        if self.hud.controls.mode == Mode::Buy {
            self.hud
                .relayout_panel(size, self.lot.world(), self.lot.residential(), &self.assets);
        }
        true
    }

    /// Moves the camera. The view's rotation and screen-centre pick are the
    /// lot's inputs too ([`Lot::set_view_rotation`], [`Lot::set_view_centre`]),
    /// synced only when the picker's camera geometry actually changes so
    /// no-op or wall-mode commands preserve a restored pick.
    pub fn apply(&mut self, command: Move) -> bool {
        let before = self.view.camera;
        let (w, h) = self.house_viewport_size();
        let viewport = [w as i32, h as i32];
        let moved = match command {
            Move::ZoomIn | Move::ZoomOut | Move::Rotate(_) if self.lot_shape.size > 0 => {
                let changed = native_zoom_or_rotate(
                    &self.lot_shape,
                    &mut self.view.camera,
                    viewport,
                    command,
                );
                self.view.bound();
                changed || self.view.camera != before
            }
            _ => self.view.apply_in(command, viewport, pick::ORIGIN_COLUMN),
        };
        let after = self.view.camera;
        if after.origin != before.origin
            || after.zoom != before.zoom
            || after.rotation != before.rotation
            || after.level != before.level
        {
            self.sync_view();
        }
        moved
    }

    /// Whether the panel takes presses at `point`.
    pub fn over_panel(&self, point: [i32; 2]) -> bool {
        self.hud.covers(point)
    }

    /// Whether `point` lies inside the canvas (`[0..width, 0..height)`).
    fn in_view(&self, point: [i32; 2]) -> bool {
        point[0] >= 0
            && point[1] >= 0
            && point[0] < self.view.size[0]
            && point[1] < self.view.size[1]
    }

    /// Moves the pointer to `point`. When it first reaches an object on
    /// the lot in Live mode, the selected Sim's pie menu on it is listed, as
    /// the original lists it then (0x48c834); listing changes the lot. In Buy
    /// mode, hovering over the house view activates and moves (or drag-rotates)
    /// the placement cursor (`0x487d3d`, `0x485a2e`), while moving onto the
    /// panel hides it (`0x487d4e`). A point outside the canvas is treated as
    /// [`Game::leave`].
    pub fn hover(&mut self, point: [i32; 2]) -> bool {
        if !self.in_view(point) {
            return self.leave();
        }
        self.pointer = Some(point);
        let mut redraw = self.hud.hover(point);
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        redraw |= self.highlight(point);
        if self.hud.controls.mode == Mode::Buy {
            if !self.hud.covers(point) || self.buy_dragging.is_some() {
                redraw |= self.hover_buy(point);
            } else {
                redraw |= self.hide_buy_cursor();
            }
        } else {
            redraw |= self.follow();
        }
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
    ///
    /// While a pie menu is open (with no dialog over it), a press only
    /// updates which entry is under `point` and marks the button held
    /// (0x5eb3d1 with `+0x170` clear); the entry is chosen, or the menu
    /// closed, on the matching [`Game::release`] (0x5eb433).
    pub fn press(&mut self, point: [i32; 2]) -> Outcome {
        if self.hud.dialog().is_none() && self.hud.menu.is_some() {
            self.pointer = Some(point);
            let mut redraw = self.hud.hover(point);
            redraw |= self.highlight(point);
            if let Some(menu) = &mut self.hud.menu {
                redraw |= menu.press();
            }
            return Outcome {
                redraw,
                photo: false,
            };
        }
        let cleared = self.note.take().is_some();
        if self.hud.covers(point) {
            // The menu, whose entries a choice is an index into.
            let menu = self.hud.menu.clone();
            let outcome = match self
                .hud
                .press(point, self.lot.world(), self.lot.residential())
            {
                Some(command) => self.command(command, menu.as_ref()),
                None => Outcome::redraw(),
            };
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return outcome;
        }
        // 0x51f3c5..0x51f3e5, 0x51d312, 0x51d354: pressing outside the live
        // control panel dismisses any open help balloon with `UI_whoosh`.
        let dismissed = self.hud.dismiss_help();
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        if self.hud.controls.mode == Mode::Buy {
            if self.stopped.is_some() {
                self.note = Some("The lot has stopped, so it takes no orders.".into());
                return Outcome::redraw();
            }
            if self.buy_selected.is_some() || self.buy_cursor.is_some() {
                self.pointer = Some(point);
                self.hover_buy(point);
                self.buy_dragging = Some(point);
                return Outcome::redraw();
            }
            return Outcome {
                redraw: cleared || dismissed,
                photo: false,
            };
        }
        if self.hud.controls.mode != Mode::Live {
            return Outcome {
                redraw: cleared,
                photo: false,
            };
        }
        let Some(target) = self.under(point) else {
            return Outcome {
                redraw: cleared || dismissed,
                photo: false,
            };
        };
        self.open_menu(target, point);
        Outcome::redraw()
    }

    /// A left release at `point` (0x5eb433 in Live mode; `0x4864d0` in Buy
    /// mode).
    pub fn release(&mut self, point: [i32; 2]) -> Outcome {
        if !self.in_view(point) {
            return self.release_outside();
        }
        if self.hud.controls.mode == Mode::Buy && self.buy_dragging.is_some() {
            // Native `0x4864f0` invokes `OnMouseMove` with the release
            // coordinates before committing placement.
            self.pointer = Some(point);
            self.hover_buy(point);
            self.buy_dragging = None;
            return self.commit_buy(point);
        }
        if self.hud.dialog().is_some() || self.hud.menu.is_none() {
            return Outcome {
                redraw: false,
                photo: false,
            };
        }
        self.pointer = Some(point);
        let mut redraw = self.hud.hover(point);
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        redraw |= self.highlight(point);
        let will_choose = self.hud.menu.as_ref().is_some_and(|m| m.will_choose(point));
        if !will_choose {
            let was_held = self.hud.menu.as_ref().is_some_and(|m| m.held());
            let (release, command) = self.hud.release_menu(point);
            debug_assert_eq!((release, command), (menu::Release::Settled, None));
            return Outcome {
                redraw: redraw || was_held,
                photo: false,
            };
        }
        let _ = self.note.take();
        // Choosing an entry checks it again first (0x5eb433 -> 0x5eb62a
        // -> event 0xe -> 0x48df28).
        if let Some(entry) = self.hud.menu.as_ref().and_then(|m| m.highlighted(point)) {
            self.check_entry(entry);
        }
        // A failed check's note stays in front of what the choice says.
        let failed = self.note.take();
        let menu = self.hud.menu.clone();
        let (release, command) = self.hud.release_menu(point);
        if release == menu::Release::Stays
            && let Some(warped) = self.hud.menu.as_ref().and_then(Menu::pending_pointer)
        {
            self.pointer = Some(warped);
        }
        self.highlighted = None;
        let outcome = match command {
            Some(command) => self.command(command, menu.as_ref()),
            None => Outcome::redraw(),
        };
        if let Some(failed) = failed {
            self.note = Some(match self.note.take() {
                Some(note) => format!("{failed} {note}"),
                None => failed,
            });
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        outcome
    }

    /// A left release while the pointer is outside the window (after
    /// [`Game::leave`]): settles an opening press on a moved menu, closes
    /// the pie without checking or choosing any entry, or cancels a Buy Mode
    /// drag.
    pub fn release_outside(&mut self) -> Outcome {
        if self.buy_dragging.take().is_some() {
            let redraw = self.hide_buy_cursor();
            return Outcome {
                redraw,
                photo: false,
            };
        }
        if self.hud.dialog().is_some() || self.hud.menu.is_none() {
            return Outcome {
                redraw: false,
                photo: false,
            };
        }
        self.pointer = None;
        self.highlighted = None;
        let redraw = self.hud.leave();
        let was_held = self.hud.menu.as_ref().is_some_and(|m| m.held());
        match self.hud.release_menu_outside() {
            menu::Release::Settled => Outcome {
                redraw: redraw || was_held,
                photo: false,
            },
            menu::Release::Closed => {
                let _ = self.note.take();
                Outcome::redraw()
            }
            menu::Release::Chose(_) | menu::Release::Stays => unreachable!(),
        }
    }

    /// The pointer left the window: it is on no object, button, portrait
    /// or pie entry, so coming back onto an entry checks it again. True
    /// if the frame changes.
    pub fn leave(&mut self) -> bool {
        self.pointer = None;
        self.highlighted = None;
        let mut redraw = self.hud.leave();
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        if self.hud.controls.mode == Mode::Buy {
            if self.buy_dragging.is_none() {
                redraw |= self.hide_buy_cursor();
            }
            redraw
        } else {
            self.follow() || redraw
        }
    }

    /// Follows the object under the pointer in the frame last drawn: when
    /// it changes to an object, lists the selected Sim's pie menu on it.
    /// Nothing is listed while the pointer is over the panel, a dialog or
    /// a menu, or once the lot has stopped. True if a listing may have
    /// changed the lot.
    fn follow(&mut self) -> bool {
        if self.hud.controls.mode != Mode::Live {
            self.reached = None;
            self.listed = None;
            return false;
        }
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
        entry.is_some_and(|i| self.check_entry(i))
    }

    /// Checks again the action of the open pie menu's entry `i`
    /// (0x48df28, `Lot::highlight`), keeping the action the check leaves
    /// for choosing it. True if the check ran.
    fn check_entry(&mut self, i: usize) -> bool {
        let Some(menu) = &self.hud.menu else {
            return false;
        };
        let Some(action) = menu.entries.get(i).and_then(|e| e.action.clone()) else {
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

    /// The person or object the pointer at `point` is on in the frame
    /// last drawn, by the original's pointer pick ([`pick::at`]).
    pub fn under(&self, point: [i32; 2]) -> Option<u16> {
        pick::at(
            &self.lot,
            &self.drawn_camera,
            &self.canvas,
            &self.lot_depth,
            &self.people_ids,
            point,
        )
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
                    let menu = Menu::new(
                        person,
                        object,
                        listing.entries,
                        point,
                        &self.hud.art,
                        self.view.size,
                    );
                    // Native menu setup reads the current cursor (0x5ebd05)
                    // after any on-screen clamp warp (0x5ebab2) and updates
                    // hit state (0x5ebd29), so the opening frame aims the head
                    // at the press or warped center rather than a stale hover.
                    let effective = menu.pending_pointer().unwrap_or(point);
                    self.pointer = Some(effective);
                    self.hud.set_pointer(effective);
                    self.hud.menu = Some(menu);
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
        self.hud.snapshot(world, self.lot.residential())
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
        let closed = self.hud.menu.take().is_some();
        if closed {
            self.highlighted = None;
        }
        closed
    }

    /// Queues `entry` of the pie menu on `object` for `person`, as
    /// choosing it does: the action the simulation offered
    /// (`Lot::pick`, on the action's caller), or for a fallback entry the
    /// entry's menu number (`Lot::queue_interaction`).
    fn order(&mut self, person: u16, object: u16, entry: &menu::Entry) {
        let label = &entry.label;
        // A picked action is queued on its caller: `person`, but for the
        // offer to join `person`'s running interaction.
        let on = entry
            .action
            .as_ref()
            .and_then(|a| u16::try_from(a.caller).ok())
            .unwrap_or(person);
        let queued = match &entry.action {
            Some(action) => self.lot.pick(person, action.clone()),
            None => self.lot.queue_interaction(person, object, entry.menu),
        };
        self.note = Some(match queued {
            Ok(Some(_)) => format!("Queued \"{label}\" for person {on}."),
            // A full queue, an entry gone from the table, or a check
            // that refused an action run at once.
            Ok(None) => format!("\"{label}\" was not queued for person {on}."),
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
        if let Err(e) = self.lot.answer(sims_vm::Answer {
            status: answer,
            bitmap_id: Some(sims_ui::dialog::BUTTON as i16),
        }) {
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
        let world = self.lot.world();
        let same_neighbor = world.selected() == Some(sim)
            || world
                .selected()
                .and_then(|id| world.objects.get(&id))
                .zip(world.objects.get(&sim))
                .is_some_and(|(selected, target)| selected.guid == target.guid);
        if same_neighbor {
            // 0x4cfa0d..0x4cfa47: centres on the first live person of that
            // neighbor in person-registry order (`0x4e9608`).
            let target = world
                .objects
                .get(&sim)
                .and_then(|o| sims_ui::household::first_person_for_guid(world, o.guid))
                .map_or(sim, |o| o.id);
            self.centre_on(target);
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
    /// negative. For a multi-tile object, `0x4071a3` walks the group from
    /// its lead tile through `next_parts` and averages the parts' `x` and
    /// `y` positions before rotating and projecting. The original's house
    /// view sits above the 100-pixel live-mode strip
    /// (`0x49ae5e` → `0x5cf1b6`, `0x4b136b`), so its middle is
    /// `[width / 2, height / 2]` over [`Game::house_viewport_size`] (full
    /// canvas while the strip is collapsed, `height - 100` while expanded).
    /// A person on the floor shown is
    /// centred a little above its feet: its screen point on the lot's
    /// terrain ([`sound::object_point_on_lot`]) raised by half of 2 √6
    /// times 2.0 times 2 to the power of the zoom plus one, cut to a whole
    /// number (19, 39 and 78 pixels from far to near), comes to the house
    /// view's middle (0x4a7777). Anything else, or a person on another
    /// floor, shows its floor (0x4a80c7) and, unless it stands off the
    /// lot's rotated tile bounds (0x4a7af0), its screen point on that
    /// floor comes to the middle (0x4a7996). Both paths then clamp the
    /// origin to the scroll limits (0x4a9fa9) and align its x coordinate
    /// with `(x + 1) & !3` (0x4a81cc).
    pub fn centre_on(&mut self, id: u16) {
        let world = self.lot.world();
        let Some(object) = world.objects.get(&id) else {
            return;
        };
        let word = |i: usize| object.data.get(i).copied().unwrap_or(0);
        if word(34) != 0 || word(29) < 0 {
            return;
        }
        let is_person = object.person.is_some();
        let Some(position) = centre_position(world, id) else {
            return;
        };
        let level = position[2];
        let (w, h) = self.house_viewport_size();
        let view_size = [w as i32, h as i32];
        if is_person && level == self.view.camera.level {
            let mut at = lot_point(&self.view.camera, &self.lot_shape, position);
            at[1] -= person_rise(self.view.camera.zoom.level());
            self.centre_screen_point(at, view_size);
            return;
        }
        if self.view.apply(Move::Floor(level)) {
            self.sync_view();
        }
        if !self.on_lot(position) {
            return;
        }
        let at = lot_point(&self.view.camera, &self.lot_shape, position);
        self.centre_screen_point(at, view_size);
    }

    /// The active 3D house viewport size (`0x49ae5e`, `0x4f4b43`, `0x4d363b`,
    /// `0x53b2eb` → `0x4b136b` → `0x49b0bf` → `0x5cf1b6`): full canvas
    /// `(width, height)` while the bottom strip is closed, and
    /// `(width, height - STRIP)` (`height - 150 + 50` from the shared
    /// 150-pixel panel container at `0x4f4b43`) while the strip is open in
    /// Live, Build, Buy, or Options mode.
    fn house_viewport_size(&self) -> (u32, u32) {
        viewport_for_controls(self.view.size, &self.hud.controls)
    }

    /// Whether `position` lies on the active lot's rotated tile bounds
    /// (`0x4a7af0`: rotated fine position shifted right by 4, narrowed to
    /// signed bytes, checked against `lot + 0x70..+0x7c`).
    fn on_lot(&self, position: [i32; 3]) -> bool {
        if self.lot_shape.size == 0 {
            return false;
        }
        let rotated = sound::rotate_object_fine(&self.view.camera, [position[0], position[1]]);
        let tx = i32::from((rotated[0] >> 4) as i8);
        let ty = i32::from((rotated[1] >> 4) as i8);
        let b = sound::lot_bounds(&self.lot_shape, &self.view.camera);
        tx >= b[0] && tx <= b[1] && ty >= b[2] && ty <= b[3]
    }

    /// Moves the camera origin so screen point `at` comes to the middle
    /// of `view_size`, clamped to the original's scroll limits
    /// (`0x4a9fa9`) and x-aligned via `(x + 1) & !3` (`0x4a81cc`).
    ///
    /// `sims-render` draws walls and floors at `camera.origin[0]` and
    /// objects/people at `camera.origin[0] + 1`, whereas the original
    /// draws walls and floors one column left of `DAT_006f00d0`
    /// (`0x4ac14b`, `0x4ac25d`) and objects/people at it. So `0x4a9fa9`
    /// and `0x4a81cc` compute the original's `origin_x`, and we store
    /// `origin_x - pick::ORIGIN_COLUMN` so both walls and objects land on
    /// the original's exact screen columns.
    fn centre_screen_point(&mut self, at: [i32; 2], view_size: [i32; 2]) {
        let middle = [view_size[0] / 2, view_size[1] / 2];
        let origin = self.view.camera.origin;
        let target = [
            origin[0].saturating_add(middle[0].saturating_sub(at[0])),
            origin[1].saturating_add(middle[1].saturating_sub(at[1])),
        ];
        let ([min_x, max_x], [min_y, max_y]) =
            scroll_limits(&self.lot_shape, &self.view.camera, view_size);
        let native_x = target[0].max(min_x).min(max_x).wrapping_add(1) & !3;
        let clamped_x = native_x.saturating_sub(pick::ORIGIN_COLUMN);
        let clamped_y = target[1].max(min_y).min(max_y);
        if self.view.set_origin([clamped_x, clamped_y]) {
            self.sync_view();
        }
    }

    /// The lot as it stands, to go back to later ([`Game::restore`]).
    /// While a dialog waits, the save is of the lot before the tick that
    /// showed it, so the restored lot shows it again. While an unpaid Buy
    /// Mode preview is held, the preview and its `"Tile Cursor"` are stripped
    /// from the saved snapshot without disturbing the live placement.
    pub fn save(&self) -> Result<Saved, String> {
        match &self.buy_cursor {
            Some(cursor) => self
                .lot
                .save_without_buy_cursor(cursor)
                .map_err(|e| e.to_string()),
            None => self.lot.save().map_err(|e| e.to_string()),
        }
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
        self.buy_selected = None;
        self.buy_cursor = None;
        self.buy_in_house = false;
        self.buy_last_guid = None;
        self.buy_facing = None;
        self.buy_dragging = None;
        self.hud.forget();
        self.hud.controls.mode = ui_mode(self.lot.mode());
        self.blink_dirty = false;
        let world = self.lot.world();
        self.hud.sync_clock_blink(world, self.clock_us);
        if let Some(speed) = pace::shown(world) {
            self.pace.speed = speed;
        }
        // The view turns as the turn keys do, keeping the middle of the
        // screen, until it shows the lot as the saved lot was turned.
        let (w, h) = self.house_viewport_size();
        let viewport = [w as i32, h as i32];
        let rotation = self.lot.view_rotation();
        for _ in 0..4 {
            if self.view.camera.rotation == rotation {
                break;
            }
            if self.lot_shape.size > 0 {
                native_zoom_or_rotate(
                    &self.lot_shape,
                    &mut self.view.camera,
                    viewport,
                    Move::Rotate(true),
                );
                self.view.bound();
            } else {
                self.view
                    .apply_in(Move::Rotate(true), viewport, pick::ORIGIN_COLUMN);
            }
        }
        self.pace.restart();
        self.people.refresh(world);
        match sims_render::Lot::from_world(world) {
            Ok(shape) => self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation()),
            Err(e) => self
                .people
                .note(vec![format!("the lot is not redrawn: {e}")]),
        }
        Ok(())
    }

    /// Carries out a panel command. `menu` is the pie menu that was open
    /// when it was pressed.
    fn command(&mut self, command: Command, menu: Option<&Menu>) -> Outcome {
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
                UiCamera::ZoomIn => {
                    self.apply(Move::ZoomIn);
                    Outcome::redraw()
                }
                UiCamera::ZoomOut => {
                    self.apply(Move::ZoomOut);
                    Outcome::redraw()
                }
                // Which way the original turns for each is unverified.
                UiCamera::RotateRight => {
                    self.apply(Move::Rotate(true));
                    Outcome::redraw()
                }
                UiCamera::RotateLeft => {
                    self.apply(Move::Rotate(false));
                    Outcome::redraw()
                }
                UiCamera::FloorUp => {
                    self.apply(Move::LevelUp);
                    Outcome::redraw()
                }
                UiCamera::FloorDown => {
                    self.apply(Move::LevelDown);
                    Outcome::redraw()
                }
                UiCamera::Walls(w) => {
                    self.apply(Move::Walls(match w {
                        UiWalls::Up => Walls::Up,
                        UiWalls::Cutaway => Walls::Cutaway,
                        UiWalls::Down => Walls::Down,
                    }));
                    Outcome::redraw()
                }
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
            Command::OpenHelp => {
                self.note = Some("The Help System dialog is not ported yet.".into());
                Outcome::redraw()
            }
            Command::SetMode(mode) => {
                self.set_mode(mode);
                Outcome::redraw()
            }
            Command::BuyCategory(_) | Command::BuySubcategory(_) | Command::BuyBack => {
                self.cancel_buy();
                self.hud
                    .layout_buy(self.view.size, self.lot.world(), &self.assets);
                Outcome::redraw()
            }
            Command::BuyPage(_) => {
                self.hud
                    .layout_buy(self.view.size, self.lot.world(), &self.assets);
                Outcome::redraw()
            }
            Command::BuyItem(guid) => {
                self.select_buy_item(guid);
                Outcome::redraw()
            }
            Command::Undo => {
                self.undo_buy();
                Outcome::redraw()
            }
            Command::Redo => {
                self.redo_buy();
                Outcome::redraw()
            }
            Command::TogglePanel => {
                self.sync_view_centre();
                if self.hud.controls.mode == Mode::Buy {
                    self.hud
                        .layout_buy(self.view.size, self.lot.world(), &self.assets);
                }
                Outcome::redraw()
            }
            Command::OpenPanel(_)
            | Command::Refused { .. }
            | Command::ShowRelations(_)
            | Command::RelationsPage(_)
            | Command::Help(_) => Outcome::redraw(),
        }
    }

    /// The active control-panel mode (`Mode::Live`, `Mode::Buy`, etc.).
    pub fn mode(&self) -> Mode {
        self.hud.controls.mode
    }

    /// The Buy Mode catalog master GUID currently armed for placement, if any.
    pub fn buy_selected(&self) -> Option<u32> {
        self.buy_selected
    }

    /// The active Buy Mode placement cursor (once the pointer has entered
    /// the house view), if any.
    pub fn buy_cursor(&self) -> Option<&BuyCursor> {
        self.buy_cursor.as_ref()
    }

    /// Whether a committed Buy Mode purchase can be undone (`0x4cf074`).
    pub fn can_undo(&self) -> bool {
        self.hud.controls.mode == Mode::Buy && self.lot.can_buy_undo()
    }

    /// Whether an undone Buy Mode purchase can be redone (`0x4cf08c`).
    pub fn can_redo(&self) -> bool {
        self.hud.controls.mode == Mode::Buy && self.lot.can_buy_redo()
    }

    /// Runs a Buy Mode Undo (`undo = true`) or Redo (`undo = false`) while
    /// keeping any active placement preview and catalog selection armed
    /// (`0x4a8545`, `0x4a856a`). Dispatches directly to the lot's purchase
    /// history without moving or lifting the preview (matching `0x4a8545` /
    /// `0x4a856a` + `0xee` panel dirtying at `0x4f66eb`), so an off-lot repeat
    /// preview does not jump onto a freed tile and block immediate Redo, while
    /// an already on-lot preview still collides with Redo (`0x4337f6`) and
    /// preserves any manual surface rotation.
    fn apply_buy_history(&mut self, undo: bool) -> bool {
        let allowed = if undo {
            self.can_undo()
        } else {
            self.can_redo()
        };
        if !allowed || self.stopped.is_some() {
            return false;
        }
        let result = if undo {
            self.lot.buy_undo()
        } else {
            self.lot.buy_redo()
        };
        if let Some(cursor) = self.buy_cursor.as_mut() {
            let held_on_lot = self
                .lot
                .world()
                .objects
                .get(&cursor.held)
                .is_some_and(|o| o.position[0] >= 0);
            let affordable =
                cursor.price <= 0 || cursor.price <= sims_vm::available_funds(self.lot.world());
            cursor.valid = held_on_lot && affordable;
        }
        match result {
            Ok(true) => {
                self.refresh_world();
                self.hud
                    .layout_buy(self.view.size, self.lot.world(), &self.assets);
                true
            }
            Ok(false) => {
                self.refresh_world();
                false
            }
            Err(e) => {
                let verb = if undo { "undone" } else { "redone" };
                self.note = Some(format!("The purchase was not {verb}: {e}"));
                self.refresh_world();
                false
            }
        }
    }

    /// Undoes the most recent committed Buy Mode purchase (`0x4cf2e0` / event
    /// `0xec` → `0x4932fa` → `0x48bd21`), keeping any active placement preview
    /// and catalog selection armed (`0x4a8545`). Returns `true` if a purchase
    /// was undone.
    pub fn undo_buy(&mut self) -> bool {
        self.apply_buy_history(true)
    }

    /// Redoes the most recently undone Buy Mode purchase (`0x4cf317` / event
    /// `0xed` → `0x493338` → `0x48bdfe`), keeping any active placement preview
    /// and catalog selection armed (`0x4a856a`). Returns `true` if a purchase
    /// was redone.
    pub fn redo_buy(&mut self) -> bool {
        self.apply_buy_history(false)
    }

    /// Sets whether the Shift modifier is held (`VK_SHIFT`, `0x4864f9`),
    /// which keeps the Buy tool armed for repeat purchases after placing any
    /// affordable item (`0x48680f`).
    pub fn set_shift(&mut self, held: bool) {
        self.shift_held = held;
    }

    /// Switches between Live, Buy, Build, and Options modes (`0x4cea5d`,
    /// `0x4ce44f`). Cancels any unplaced Buy Mode item, closes open pie menus,
    /// updates `global[19]` (calling `go_live` `0x416534` when returning to
    /// Live mode), and refreshes the control panel and Buy catalog strip.
    pub fn set_mode(&mut self, mode: Mode) {
        self.cancel_buy();
        if self.buy_cursor.is_some() {
            return;
        }
        self.close_menu();
        self.reached = None;
        self.listed = None;
        self.highlighted = None;
        if let Err(e) = self.lot.set_mode(panel_mode_for(mode)) {
            self.note = Some(format!("The mode was not changed: {e}"));
            return;
        }
        let was_expanded = self.hud.controls.expanded;
        self.hud.controls.mode = mode;
        self.hud.controls.expanded = true;
        if !was_expanded {
            self.sync_view_centre();
        }
        self.hud.controls.help = None;
        self.hud.buy_state.reset();
        // Native `0x487191` clears the active selection/cursor while retaining
        // `buy_last_guid` (`tool[0x12]`) and `buy_facing` (`tool[0x10]`).
        self.buy_in_house = false;
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
        self.refresh_world();
        self.hud.relayout_panel(
            self.view.size,
            self.lot.world(),
            self.lot.residential(),
            &self.assets,
        );
    }

    /// Arms catalog master `guid` on the Buy Mode placement tool (`0x5065c8`).
    /// Does not create the object until the pointer enters the house view
    /// (`0x487d3d`). Plays `UI_error` if the household cannot afford it.
    fn select_buy_item(&mut self, guid: u32) {
        self.cancel_buy();
        if self.buy_cursor.is_some() {
            return;
        }
        let price = self
            .hud
            .ensure_buy_catalog(&self.assets)
            .get(guid)
            .map(|item| item.price);
        let funds = sims_vm::available_funds(self.lot.world());
        match price {
            Some(price) if price <= 0 || price <= funds => {
                self.buy_selected = Some(guid);
                self.hud.buy_state.selected = Some(guid);
            }
            Some(_) => {
                self.hud.play(crate::hud::ERROR);
                self.buy_selected = None;
                self.hud.buy_state.selected = None;
            }
            None => {
                self.buy_selected = None;
                self.hud.buy_state.selected = None;
            }
        }
        self.hud
            .layout_buy(self.view.size, self.lot.world(), &self.assets);
    }

    /// Cancels any active Buy Mode cursor or armed catalog selection
    /// (`0x487105`), deleting the unplaced object and `"Tile Cursor"` without
    /// charging funds. Returns `true` if anything was cancelled. If deleting
    /// the preview fails, the cursor handle is kept so it can be retried.
    pub fn cancel_buy(&mut self) -> bool {
        self.buy_dragging = None;
        self.buy_in_house = false;
        let had_cursor = if let Some(cursor) = self.buy_cursor.take() {
            match self.lot.buy_cancel(&cursor) {
                Ok(()) => {
                    self.refresh_world();
                    true
                }
                Err(e) => {
                    self.buy_cursor = Some(cursor);
                    self.note = Some(format!("The Buy cursor was not cancelled: {e}"));
                    return false;
                }
            }
        } else {
            false
        };
        let had_selection = self.buy_selected.take().is_some();
        let had_hud = self.hud.buy_state.selected.take().is_some();
        if had_selection || had_hud || had_cursor {
            if self.hud.controls.mode == Mode::Buy {
                self.hud
                    .layout_buy(self.view.size, self.lot.world(), &self.assets);
            }
            true
        } else {
            false
        }
    }

    /// Rotates the held Buy Mode object by one signed turn step (`0x486cb8` →
    /// `0x421be4(±1)`), returning `true` if a Buy cursor was active.
    pub fn rotate_held(&mut self, clockwise: bool) -> bool {
        let Some(mut cursor) = self.buy_cursor.take() else {
            return false;
        };
        let steps = if clockwise { 1 } else { -1 };
        if let Err(e) = self.lot.buy_turn(&mut cursor, steps) {
            self.note = Some(format!("The object was not rotated: {e}"));
        } else {
            self.refresh_world();
        }
        self.buy_cursor = Some(cursor);
        true
    }

    /// Handles pointer movement over the house view in Buy Mode (`0x487d3d`,
    /// `0x485a2e`): on mouse entry (`!self.buy_in_house`), recreates the
    /// armed catalog preview at the tool's stored facing (`0x487d3d` →
    /// `0x486f5a`); then rechecks the previous cursor position (`0x485bda`),
    /// rotates the held object in place while dragging past the 6 px
    /// threshold (`dx^2 + dy^2 > 36`), and otherwise moves the cursor to the
    /// terrain-unprojected tile (including within the 6 px drag radius).
    fn hover_buy(&mut self, point: [i32; 2]) -> bool {
        if self.stopped.is_some() {
            return false;
        }
        if !self.buy_in_house {
            self.buy_in_house = true;
            if let Some(guid) = self.buy_selected {
                // Native `0x487d3d` → `0x486f5a`: destroy any hidden previous
                // preview (`0x487105`) and instantiate a fresh one using the
                // tool's stored facing (`tool[0x10]`), resetting it to
                // `default_buy_direction` only when switching catalog items.
                if let Some(old) = self.buy_cursor.take()
                    && let Err(e) = self.lot.buy_cancel(&old)
                {
                    self.buy_cursor = Some(old);
                    self.buy_in_house = false;
                    self.note = Some(format!("Could not reset Buy preview: {e}"));
                    return true;
                }
                if self.buy_last_guid != Some(guid) {
                    self.buy_facing =
                        Some(sims_vm::default_buy_direction(self.view.camera.rotation));
                    self.buy_last_guid = Some(guid);
                }
                let dir = self
                    .buy_facing
                    .unwrap_or_else(|| sims_vm::default_buy_direction(self.view.camera.rotation));
                match self.lot.buy_start(guid, self.view.camera.level, dir) {
                    Ok(Some(cursor)) => self.buy_cursor = Some(cursor),
                    Ok(None) => {
                        self.hud.play(crate::hud::ERROR);
                        self.buy_selected = None;
                        self.hud.buy_state.selected = None;
                        self.hud
                            .layout_buy(self.view.size, self.lot.world(), &self.assets);
                        return true;
                    }
                    Err(e) => {
                        self.note = Some(format!("Could not start placing 0x{guid:08X}: {e}"));
                        return true;
                    }
                }
            }
        }
        let Some(mut cursor) = self.buy_cursor.take() else {
            return false;
        };
        // Native `0x485b4c..0x485bda`: ALWAYS recheck the `"Tile Cursor"`'s
        // physical on-lot position on the currently displayed floor
        // (`0x485ba2`) BEFORE checking the drag threshold.
        let level = self.view.camera.level;
        if let Err(e) = self.lot.buy_recheck(&mut cursor, level) {
            self.note = Some(format!("Could not recheck Buy placement: {e}"));
        }
        let rotated = if let Some(anchor) = self.buy_dragging {
            let dx = point[0] - anchor[0];
            let dy = point[1] - anchor[1];
            if dx * dx + dy * dy > 36 {
                // Native `0x485d5d..0x485dad`: `iVar6 = dy`, `iVar8 = dx`.
                // Upper-left -> 6, upper-right -> 4, lower-left -> 0,
                // lower-right -> 2. Compare `desired` against post-`0x485bda`
                // current facing FIRST (`0x485da9`), and only if they differ
                // subtract the OBJD `+0x7a` facing offset (`0x434f8d`) before
                // turning.
                let quad = if dy < 0 {
                    if dx < 0 { 6 } else { 4 }
                } else if dx < 0 {
                    0
                } else {
                    2
                };
                let desired = (quad - 2 * i16::from(self.view.camera.rotation & 3)).rem_euclid(8);
                let current = self
                    .lot
                    .world()
                    .objects
                    .get(&cursor.held)
                    .and_then(|o| o.data.get(1).copied())
                    .unwrap_or(0)
                    .rem_euclid(8);
                if current != desired {
                    let target = (desired - cursor.facing_offset).rem_euclid(8);
                    if let Err(e) = self.lot.buy_rotate(&mut cursor, target) {
                        self.note = Some(format!("The object was not rotated: {e}"));
                    }
                }
                // Native `0x485e59` → `0x485e72`: test Tile Cursor fit on the
                // existing preview in place on the displayed floor (without an
                // extra TakeOff) and place if it fits, including when the
                // facing was unchanged.
                if let Err(e) = self.lot.buy_put_if_fits(&mut cursor, level) {
                    self.note = Some(format!("Could not finalize Buy placement: {e}"));
                }
                true
            } else {
                false
            }
        } else {
            false
        };
        if !rotated {
            let at = [point[0] - pick::ORIGIN_COLUMN, point[1]];
            let fine = sims_render::unproject_terrain(&self.lot_shape, &self.view.camera, at)
                .unwrap_or_else(|| self.view.camera.unproject_fine(at));
            if let Err(e) = self.lot.buy_move(&mut cursor, fine, self.view.camera.level) {
                self.note = Some(format!("Could not move Buy cursor: {e}"));
            }
        }
        self.buy_cursor = Some(cursor);
        self.refresh_world();
        true
    }

    /// Hides the active Buy Mode cursor off the lot on mouse exit (`0x487d4e`).
    fn hide_buy_cursor(&mut self) -> bool {
        let was_in = std::mem::replace(&mut self.buy_in_house, false);
        if !was_in {
            return false;
        }
        let Some(mut cursor) = self.buy_cursor.take() else {
            return false;
        };
        if let Err(e) = self.lot.buy_hide(&mut cursor) {
            self.note = Some(format!("Could not hide Buy cursor: {e}"));
        }
        self.buy_cursor = Some(cursor);
        self.refresh_world();
        true
    }

    /// Commits a Buy Mode placement on mouse release (`0x4864d0` → `0x486ada`
    /// → `0x421a0e`, `0x4929fb`). For repeat-buy items (`cursor.keep_buying`,
    /// OBJD word 96 == 1 outside tutorial), immediately starts another copy
    /// with the placed item's facing (`0x4864d0` → `0x486f5a`) if affordable.
    fn commit_buy(&mut self, point: [i32; 2]) -> Outcome {
        let Some(cursor) = self.buy_cursor.take() else {
            return Outcome::default();
        };
        if !cursor.valid {
            self.buy_cursor = Some(cursor);
            self.hud.play(crate::hud::ERROR);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return Outcome::redraw();
        }
        let last_facing = self
            .lot
            .world()
            .objects
            .get(&cursor.held)
            .and_then(|o| o.data.get(1).copied())
            .unwrap_or_else(|| sims_vm::default_buy_direction(self.view.camera.rotation));
        match self.lot.buy_commit(&cursor) {
            Ok(_) => {
                // Native `0x486ada` line `0x486af6`: remember the committed
                // facing on the tool (`tool[0x10]`).
                self.buy_facing = Some(last_facing);
                self.buy_last_guid = Some(cursor.guid);
                if cursor.keep_buying || self.shift_held {
                    let funds = sims_vm::available_funds(self.lot.world());
                    if cursor.price <= 0 || cursor.price <= funds {
                        self.buy_selected = Some(cursor.guid);
                        self.hud.buy_state.selected = Some(cursor.guid);
                        match self
                            .lot
                            .buy_start(cursor.guid, self.view.camera.level, last_facing)
                        {
                            Ok(Some(next)) => {
                                self.buy_cursor = Some(next);
                                self.buy_in_house = !self.hud.covers(point);
                                self.refresh_world();
                                if !self.hud.covers(point) {
                                    self.hover_buy(point);
                                }
                            }
                            Ok(None) => {
                                self.hud.play(crate::hud::ERROR);
                                self.buy_selected = None;
                                self.hud.buy_state.selected = None;
                                self.refresh_world();
                            }
                            Err(e) => {
                                self.note = Some(format!(
                                    "Could not start repeat purchase 0x{:08X}: {e}",
                                    cursor.guid
                                ));
                                self.buy_selected = None;
                                self.hud.buy_state.selected = None;
                                self.refresh_world();
                            }
                        }
                    } else {
                        self.hud.play(crate::hud::ERROR);
                        self.buy_selected = None;
                        self.hud.buy_state.selected = None;
                        self.refresh_world();
                    }
                } else {
                    self.buy_selected = None;
                    self.hud.buy_state.selected = None;
                    self.refresh_world();
                }
                self.hud
                    .layout_buy(self.view.size, self.lot.world(), &self.assets);
            }
            Err(e) => {
                // `Lot::buy_commit` rolled back the world; keep the cursor
                // handle so the preview objects remain manageable.
                self.buy_cursor = Some(cursor);
                self.note = Some(format!("The item was not placed: {e}"));
                self.hud.play(crate::hud::ERROR);
            }
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    /// Refreshes the cached render lot shape and people from `self.lot.world()`.
    fn refresh_world(&mut self) {
        let world = self.lot.world();
        self.people.refresh(world);
        match sims_render::Lot::from_world(world) {
            Ok(shape) => self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation()),
            Err(e) => self
                .people
                .note(vec![format!("the lot is not redrawn: {e}")]),
        }
    }
}

fn panel_mode_for(mode: Mode) -> PanelMode {
    match mode {
        Mode::Live => PanelMode::Live,
        Mode::Buy => PanelMode::Buy,
        Mode::Build => PanelMode::Build,
        Mode::Options => PanelMode::Options,
    }
}

fn ui_mode(mode: PanelMode) -> Mode {
    match mode {
        PanelMode::Live => Mode::Live,
        PanelMode::Build => Mode::Build,
        PanelMode::Buy => Mode::Buy,
        PanelMode::Options | PanelMode::Camera => Mode::Options,
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

/// The lines for the caught errors of the step that runs the lot's tick
/// `tick` ([`Lot::ticks`]), in the order they happened: the frame drawn
/// before the lot's first tick, the objects' errors in the tick, then the
/// frame drawn after it. A frame's line says which of the two it was,
/// and not the tick: an object's line starts with the clock's tick count,
/// which is another count, and the error log takes the same failure at
/// later ticks for a repeat ([`Errors::push`]).
fn in_order(tick: u32, errors: &[ScriptError], frames: &[SkippedFrame]) -> Vec<String> {
    let frame = |frame: &SkippedFrame| match frame.after {
        0 => format!(
            "the frame before the first tick was skipped: {}",
            frame.error
        ),
        _ => format!("the frame after the tick was skipped: {}", frame.error),
    };
    let before = frames.iter().filter(|frame| frame.after < tick);
    let after = frames.iter().filter(|frame| frame.after >= tick);
    before
        .map(frame)
        .chain(errors.iter().map(describe))
        .chain(after.map(frame))
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

/// The fine `[x, y, level]` position `0x434ced` centres on: for a
/// multi-tile object (`0x4071a3`), starts at the lead tile (`parent`, or
/// `id` itself when `parent == 0`), walks `world.next_parts`, and averages
/// the parts' `x` and `y` coordinates by signed integer division toward
/// zero while keeping `object.position[2]` as the level.
fn centre_position(world: &sims_sim::World, id: u16) -> Option<[i32; 3]> {
    let object = world.objects.get(&id)?;
    let level = object.position[2];
    let Some(part) = &object.multipart else {
        return Some(object.position);
    };
    let lead = u16::try_from(part.parent)
        .ok()
        .filter(|&p| p != 0 && world.objects.contains_key(&p))
        .unwrap_or(id);
    let mut sum_x = 0i64;
    let mut sum_y = 0i64;
    let mut count = 0i64;
    let mut visited = Vec::new();
    let mut current = Some(lead);
    while let Some(part_id) = current {
        if visited.contains(&part_id) {
            break;
        }
        let Some(tile) = world.objects.get(&part_id) else {
            break;
        };
        visited.push(part_id);
        sum_x += i64::from(tile.position[0]);
        sum_y += i64::from(tile.position[1]);
        count += 1;
        current = world.next_parts.get(&part_id).copied();
    }
    if count == 0 {
        return Some(object.position);
    }
    Some([(sum_x / count) as i32, (sum_y / count) as i32, level])
}

/// The screen point of a fine lot position (`0x4a9dc7` → `0x4a9c50`),
/// narrowing rotated tile coordinates to signed bytes (`0x4a9c5c..0x4a9c71`)
/// while preserving the low 4-bit sub-tile offset, worked out from the lot's
/// own corner and then moved by the origin.
fn lot_point(camera: &Camera, lot: &sims_render::Lot, position: [i32; 3]) -> [i32; 2] {
    let rotated = sound::rotate_object_fine(camera, [position[0], position[1]]);
    let tile = [
        i32::from((rotated[0] >> 4) as i8),
        i32::from((rotated[1] >> 4) as i8),
    ];
    let narrowed = [
        (tile[0] << 4) | (rotated[0] & 15),
        (tile[1] << 4) | (rotated[1] & 15),
    ];
    let corners = sound::tile_corners(lot, camera, tile);
    let at = sound::fine_point_with_heights(
        &Camera {
            origin: [0, 0],
            ..*camera
        },
        narrowed,
        position[2],
        corners,
    );
    [
        at[0].saturating_add(camera.origin[0]),
        at[1].saturating_add(camera.origin[1]),
    ]
}

/// The house view's camera origin scroll limits `([min_x, max_x], [min_y, max_y])`
/// for `lot`, `camera`, and viewport `view_size` (`0x4a9fa9`, using the
/// diagonal height extrema `0x4b0615` and rotated lot bounds `0x4791a6`).
pub(crate) fn scroll_limits(
    lot: &sims_render::Lot,
    camera: &Camera,
    view_size: [i32; 2],
) -> ([i32; 2], [i32; 2]) {
    let size = lot.size as i32;
    let zoom = camera.zoom.level();
    let tile_w = camera.zoom.tile_width();
    let half_w = tile_w / 2;
    let tile_h = camera.zoom.tile_height();
    let half_h = tile_h / 2;
    let floor_h = camera.zoom.floor_height();
    let [view_w, view_h] = view_size;

    let inner_tiles = (size - 2).max(0);
    let span_x = inner_tiles * tile_w;
    let (mut min_x, mut max_x) = if view_w < span_x {
        ((span_x / -2 - half_w) + view_w, span_x / 2 + tile_w)
    } else {
        (view_w / 2, view_w / 2)
    };

    // Diagonal height extrema across the active lot (0x4b0615): indexed by
    // rotated `rx + ry` in `0..128`.
    let b = sound::lot_bounds(lot, camera);
    let (min_rx, min_ry) = (b[0], b[2]);
    let mut max_h = [0u8; 128];
    let mut min_h = [0x40u8; 128];
    for y in 0..size {
        for x in 0..size {
            let rx = x + min_rx;
            let ry = y + min_ry;
            let diag = (rx + ry) as usize;
            if diag < 128 {
                for h in sound::raw_tile_corners(lot, camera, [rx, ry]) {
                    max_h[diag] = max_h[diag].max(h);
                    min_h[diag] = min_h[diag].min(h);
                }
            }
        }
    }

    let mut top_adjust = 0i32;
    if size > 2 {
        let (mut rx, mut ry) = (1 + min_rx, 1 + min_ry);
        for step in 1..(size - 1) {
            let diag = (rx + ry) as usize;
            let val = (i32::from(max_h[diag]) << (zoom + 1)) - half_h * step;
            top_adjust = top_adjust.max(val);
            rx += 1;
            ry += 1;
        }
    }
    let mut max_y = top_adjust + floor_h * 2;

    let mut min_y = (view_h - tile_h * 2) - inner_tiles * tile_h;
    if size > 2 {
        let mut bottom_adjust = i32::MAX;
        let (mut rx, mut ry) = (size - 2 + min_rx, size - 2 + min_ry);
        for step in 1..(size - 1) {
            let diag = (rx + ry) as usize;
            let val = half_h * step - (i32::from(min_h[diag]) << (zoom + 1));
            bottom_adjust = bottom_adjust.min(val);
            rx -= 1;
            ry -= 1;
        }
        min_y -= bottom_adjust;
    }

    let shift_x = (min_rx - min_ry) * half_w;
    let shift_y = (min_ry + min_rx) * half_h;
    min_x -= shift_x;
    max_x -= shift_x;
    min_y -= shift_y;
    max_y -= shift_y;
    ([min_x, max_x], [min_y, max_y])
}

/// How far above its feet a centred person is (0x434d64 to 0x434da7):
/// 2 √6 (single precision, 0x434df6) times 2.0 (1/3 times 6, single)
/// times 2 to the power of the zoom plus one, cut toward zero, halved.
fn person_rise(zoom: u32) -> i32 {
    let rise = (2.0 * 6f64.sqrt()) as f32 * 2.0 * (1u32 << (zoom + 1)) as f32;
    rise as i32 / 2
}

/// Syncs the panel's speed and restarts the time owed when a tick changes
/// the simulation's speed or return speed (such as Special Effect resetting
/// to normal, `0x4f684c` → `0x46d697`, or `World::idle_speed` clearing
/// global 18 when someone wakes during manual stepping, `0x46d7db` →
/// `0x46d697`). Returns whether either changed.
fn sync_speed_after_step(
    pace: &mut Pace,
    speed_before: (i16, i16),
    world: &sims_sim::World,
) -> bool {
    if (world.speed(), world.return_speed()) == speed_before {
        return false;
    }
    if let Some(speed) = pace::shown(world) {
        pace.speed = speed;
    }
    pace.restart();
    true
}

/// Rotates a fine lot/view coordinate about `1023` (`0x47bc1f` with `0x400`).
fn rotate_fine_1023([x, y]: [i32; 2], turns: u8) -> [i32; 2] {
    let m: i32 = 1023;
    match turns & 3 {
        1 => [m.wrapping_sub(y), x],
        2 => [m.wrapping_sub(x), m.wrapping_sub(y)],
        3 => [y, m.wrapping_sub(x)],
        _ => [x, y],
    }
}

/// Applies `Move::ZoomIn`, `Move::ZoomOut`, or `Move::Rotate` using the
/// original's terrain-aware screen-centre anchor on the displayed floor
/// (`0x4a7c1f..0x4a7d2e` for zoom, `0x4a7e89..0x4a7f94` for rotation):
///
/// 1. Picks the view-rotated fine ground point at the active house viewport's
///    midpoint (`0x4a7b58`) using the native origin column (`origin[0] + 1`).
/// 2. Cuts it to a signed-byte tile (`0x433067`) and checks the rotated lot's
///    interior bounds (`0x434cb7`); if outside, falls back to the tile
///    `[(size / 4) + min_x, (size / 4) + min_y]` converted to its fine centre
///    `((t << 4) | 8)` (`0x4a7dec`, `0x434c98`).
/// 3. Updates zoom (or un-rotates by `(-old) & 3`, updates rotation, and
///    re-rotates by `new` via `0x47bc1f` about `1023`).
/// 4. Recenters via `0x4a7996` (`on_lot` check `0x4a7af0`, terrain-aware
///    projection on `camera.level` via `fine_point_with_heights`, clamping to
///    `scroll_limits` and `(x + 1) & !3` alignment at `0x4a81cc`).
fn native_zoom_or_rotate(
    lot: &sims_render::Lot,
    camera: &mut Camera,
    viewport: [i32; 2],
    command: Move,
) -> bool {
    let before = *camera;
    let next_zoom = match command {
        Move::ZoomIn => {
            let next = camera.zoom.zoom_in();
            if next == camera.zoom {
                return false;
            }
            Some(next)
        }
        Move::ZoomOut => {
            let next = camera.zoom.zoom_out();
            if next == camera.zoom {
                return false;
            }
            Some(next)
        }
        Move::Rotate(_) => None,
        _ => return false,
    };

    let mut native = *camera;
    native.origin[0] = native.origin[0].saturating_add(pick::ORIGIN_COLUMN);
    let mut fine = match sims_render::screen_centre_pick(lot, &native, viewport) {
        Some(fine) => fine,
        None => return false,
    };
    let [tx, ty] = [
        i32::from((fine[0] >> 4) as i8),
        i32::from((fine[1] >> 4) as i8),
    ];
    let b = sound::lot_bounds(lot, &native);
    let inside = tx > b[0] && tx < b[1] && ty > b[2] && ty < b[3];
    if !inside {
        let quarter = (lot.size as i32 / 4) as i8;
        let fx = i32::from(quarter.wrapping_add(b[0] as i8));
        let fy = i32::from(quarter.wrapping_add(b[2] as i8));
        fine = [(fx << 4) | 8, (fy << 4) | 8];
    }

    if let Some(zoom) = next_zoom {
        native.zoom = zoom;
    } else if let Move::Rotate(clockwise) = command {
        let unrotated = rotate_fine_1023(fine, (4 - (native.rotation & 3)) & 3);
        native.rotation = (native.rotation + if clockwise { 1 } else { 3 }) & 3;
        fine = rotate_fine_1023(unrotated, native.rotation);
    }

    // 0x4a7af0 narrows the rotated tile coordinates to signed bytes (`movsx al/dl`)
    // before checking lot bounds, and 0x4a9dc7 → 0x4a9c50 projects the narrowed tile
    // plus the low 4-bit fractional offset.
    let b = sound::lot_bounds(lot, &native);
    let [rx, ry] = [
        i32::from((fine[0] >> 4) as i8),
        i32::from((fine[1] >> 4) as i8),
    ];
    if rx >= b[0] && rx <= b[1] && ry >= b[2] && ry <= b[3] {
        let narrowed_fine = [(rx << 4) | (fine[0] & 15), (ry << 4) | (fine[1] & 15)];
        let corners = sound::tile_corners(lot, &native, [rx, ry]);
        // 0x4a79fd projects using the current native origin and sign-extends the
        // 16-bit screen coordinates before `0x4a7a23..0x4a7a47` computes
        // `current_origin + (middle - screen_point)`.
        let at = sound::fine_point_with_heights(&native, narrowed_fine, native.level, corners);
        let middle = [viewport[0] / 2, viewport[1] / 2];
        let target = [
            native.origin[0].saturating_add(middle[0].saturating_sub(at[0])),
            native.origin[1].saturating_add(middle[1].saturating_sub(at[1])),
        ];
        let ([min_x, max_x], [min_y, max_y]) = scroll_limits(lot, &native, viewport);
        let native_x = target[0].max(min_x).min(max_x).wrapping_add(1) & !3;
        let clamped_y = target[1].max(min_y).min(max_y);
        native.origin = [native_x, clamped_y];
    }

    camera.zoom = native.zoom;
    camera.rotation = native.rotation;
    camera.origin = [
        native.origin[0].saturating_sub(pick::ORIGIN_COLUMN),
        native.origin[1],
    ];
    if camera.zoom != before.zoom {
        let built = before.focus.masks_built_at.unwrap_or(before.zoom);
        camera.focus.masks_built_at = (built != camera.zoom).then_some(built);
    }
    if camera.rotation != before.rotation {
        camera.focus.masks_built_at = None;
    }
    *camera != before
}

/// The active 3D house viewport size (`0x49ae5e`, `0x4f4b43`, `0x4d363b`,
/// `0x53b2eb` → `0x4b136b` → `0x49b0bf` → `0x5cf1b6`): full canvas when the
/// bottom strip is closed, and `height - STRIP` (`height - 150 + 50` from the
/// shared 150-pixel panel container at `0x4f4b43`) when the strip is open in
/// Live, Build, Buy, or Options mode.
fn viewport_for_controls(size: [i32; 2], controls: &sims_ui::hud::Controls) -> (u32, u32) {
    let [w, h] = size.map(|n| n.max(1));
    let strip = if controls.expanded {
        sims_ui::hud::STRIP
    } else {
        0
    };
    (w as u32, (h - strip).max(1) as u32)
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
    fn scroll_limits_match_0x4a9fa9_across_rotations_view_widths_and_terrain() {
        use sims_render::camera::{Walls, Zoom};
        let mut world = sims_sim::World::empty();
        world.size = 52;
        let full = sims_render::lot::FULL * sims_render::lot::FULL;
        for (id, stride) in [(0, 4), (1, 1), (2, 8), (101, 1), (102, 8)] {
            world.arrays.insert(
                id,
                sims_formats::lot::Array {
                    width: sims_render::lot::FULL,
                    height: sims_render::lot::FULL,
                    stride,
                    encoding: 0,
                    data: vec![0; full * stride],
                },
            );
        }
        let flat = sims_render::Lot::from_world(&world).unwrap();
        let cam = |zoom: Zoom, rotation: u8| Camera {
            zoom,
            rotation,
            level: 1,
            origin: [0, 0],
            walls: Walls::Up,
            focus: sims_render::Focus::default(),
        };

        // 52x52 flat lot at far zoom, 800x500 house view:
        // span_x = 50 * 32 = 1600, min_x = (-800 - 16) + 800 = -16,
        // max_x = 800 + 32 = 832;
        // max_y = 0 + 58 * 2 = 116;
        // min_y = (500 - 32) - 50 * 16 - 8 = -340.
        assert_eq!(
            scroll_limits(&flat, &cam(Zoom::Far, 0), [800, 500]),
            ([-16, 832], [-340, 116])
        );
        // Rotation 1 on a 52x52 lot shifts min_rx = 12, min_ry = 0:
        // shift_x = 12 * 16 = 192, shift_y = 12 * 8 = 96.
        assert_eq!(
            scroll_limits(&flat, &cam(Zoom::Far, 1), [800, 500]),
            ([-16 - 192, 832 - 192], [-340 - 96, 116 - 96])
        );
        // Rotation 2 shifts min_rx = 12, min_ry = 12:
        // shift_x = 0, shift_y = 24 * 8 = 192.
        assert_eq!(
            scroll_limits(&flat, &cam(Zoom::Far, 2), [800, 500]),
            ([-16, 832], [-340 - 192, 116 - 192])
        );
        // Viewport wider than span_x (1600) centres x at view_w / 2.
        let ([min_x, max_x], _) = scroll_limits(&flat, &cam(Zoom::Far, 0), [2000, 500]);
        assert_eq!((min_x, max_x), (1000, 1000));

        // Raising the top corner tile (1, 1) to height 10 raises max_y by
        // max(0, (10 << 2) - 8 * 1) = 32 -> 116 + 32 = 148.
        let cell = |x: usize, y: usize| (x * sims_render::lot::FULL + y) * 4;
        world.arrays.get_mut(&0).unwrap().data[cell(1, 1)..cell(1, 1) + 4]
            .copy_from_slice(&[10, 10, 10, 10]);
        let raised = sims_render::Lot::from_world(&world).unwrap();
        let (_, [min_y, max_y]) = scroll_limits(&raised, &cam(Zoom::Far, 0), [800, 500]);
        assert_eq!((min_y, max_y), (-340, 148));
    }

    #[test]
    fn centre_position_averages_linked_multipart_tiles() {
        let mut world = sims_sim::World::empty();
        let make = |id: u16, position: [i32; 3], parent: Option<i16>| sims_sim::Object {
            id,
            guid: 0,
            footprint: [0; 4],
            position,
            unknown_short: 0,
            attributes: Vec::new(),
            temps: Vec::new(),
            data: Vec::new(),
            obsolete_word: 0,
            stack_capacity_bytes: 0,
            stack: Vec::new(),
            relationship_flag: 0,
            relationships: Vec::new(),
            slots: Vec::new(),
            dynamic_sprites: Vec::new(),
            multipart: parent.map(|p| sims_formats::saved_objects::Multipart {
                offset: [0; 3],
                group: [0; 3],
                parent: p,
            }),
            portal: None,
            person: None,
            registry_word: None,
            last_result: false,
            removal_skips_resets: false,
            selected: false,
        };
        world.objects.insert(1, make(1, [100, 200, 1], None));
        world.objects.insert(10, make(10, [384, 416, 1], Some(0)));
        world.objects.insert(11, make(11, [400, 416, 1], Some(10)));
        world.objects.insert(12, make(12, [416, 449, 2], Some(10)));
        world.next_parts.insert(10, 11);
        world.next_parts.insert(11, 12);
        assert_eq!(centre_position(&world, 1), Some([100, 200, 1]));
        // (384+400+416)/3 = 400, (416+416+449)/3 = 1281/3 = 427; level comes
        // from the queried object (0x434ced reads param_1 + 0x110).
        assert_eq!(centre_position(&world, 10), Some([400, 427, 1]));
        assert_eq!(centre_position(&world, 11), Some([400, 427, 1]));
        assert_eq!(centre_position(&world, 12), Some([400, 427, 2]));
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
        let frames = |skipped: &[(u32, &str)]| -> Vec<SkippedFrame> {
            skipped
                .iter()
                .map(|&(after, error)| SkippedFrame {
                    after,
                    error: error.to_string(),
                })
                .collect()
        };
        let starts = |lines: Vec<String>| -> Vec<String> {
            lines.into_iter().map(|l| l[..13].to_string()).collect()
        };
        let errors = std::slice::from_ref(&error);
        // The first step: the frame before the tick, the tick, the frame
        // after it.
        let first = in_order(1, errors, &frames(&[(1, "a"), (0, "b")]));
        assert_eq!(
            starts(first.clone()),
            ["the frame bef", "tick 1, objec", "the frame aft"]
        );
        assert_eq!(first[0], "the frame before the first tick was skipped: b");
        assert_eq!(first[2], "the frame after the tick was skipped: a");
        // A single frame on the first step says which one it was.
        assert_eq!(
            starts(in_order(1, errors, &frames(&[(1, "a")]))),
            ["tick 1, objec", "the frame aft"]
        );
        assert_eq!(
            starts(in_order(1, errors, &frames(&[(0, "b")]))),
            ["the frame bef", "tick 1, objec"]
        );
        // A step that failed hands back the frame before its tick only.
        assert_eq!(
            in_order(1, &[], &frames(&[(0, "b")])),
            ["the frame before the first tick was skipped: b"]
        );
        // Later steps draw only after the tick.
        assert_eq!(
            in_order(34, &[], &frames(&[(34, "a")])),
            ["the frame after the tick was skipped: a"]
        );
        assert!(in_order(2, &[], &[]).is_empty());
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

    #[test]
    fn a_mid_tick_speed_reset_updates_the_panel_and_drops_owed_time() {
        use sims_sim::speed::{FAST, IDLE, NORMAL};

        let mut world = sims_sim::World::empty();
        world.globals[18] = -1;
        world.set_speed(i32::from(FAST));

        let mut pace = Pace::default();
        pace.set_speed(Speed::Fast);
        // Unchanged speed keeps both the panel selection and accrued credit.
        assert_eq!(pace.due(13_000, FAST), 0);
        assert!(!sync_speed_after_step(&mut pace, (FAST, -1), &world));
        assert_eq!(pace.speed, Speed::Fast);
        assert_eq!(pace.due(1_000, FAST), 1);

        // Accrue 13 ms at Fast, then drop speed to Normal inside a step:
        // the panel switches to Normal and the 13 ms credit is dropped so
        // the next Normal tick waits a full 42 ms.
        assert_eq!(pace.due(13_000, FAST), 0);
        world.set_speed(i32::from(NORMAL));
        assert!(sync_speed_after_step(&mut pace, (FAST, -1), &world));
        assert_eq!(pace.speed, Speed::Normal);
        assert_eq!(pace.due(29_000, NORMAL), 0);
        assert_eq!(pace.due(12_999, NORMAL), 0);
        assert_eq!(pace.due(1, NORMAL), 1);

        // Idle speed (1000, returning to Fast) dropping to Normal (760, 760)
        // also switches the panel to Normal and drops accrued credit.
        pace.set_speed(Speed::Fast);
        world.globals[16] = IDLE;
        world.globals[18] = FAST;
        assert_eq!(pace.due(500, IDLE), 0);
        world.set_speed(i32::from(NORMAL));
        assert_eq!((world.speed(), world.return_speed()), (NORMAL, NORMAL));
        assert!(sync_speed_after_step(&mut pace, (IDLE, FAST), &world));
        assert_eq!(pace.speed, Speed::Normal);
        assert_eq!(pace.due(41_999, NORMAL), 0);
        assert_eq!(pace.due(1, NORMAL), 1);

        // Wake-up clearing global 18 from (760, 760) to (760, -1) inside a
        // manual step still calls 0x46d697 and must drop accrued credit.
        assert_eq!(pace.due(41_000, NORMAL), 0);
        world.globals[18] = -1;
        assert!(sync_speed_after_step(&mut pace, (NORMAL, NORMAL), &world));
        assert_eq!(pace.speed, Speed::Normal);
        assert_eq!(pace.due(1_000, NORMAL), 0);
        assert_eq!(pace.due(40_999, NORMAL), 0);
        assert_eq!(pace.due(1, NORMAL), 1);
    }

    #[test]
    fn house_viewport_size_follows_the_control_panel_strip() {
        use sims_ui::command::Mode;
        use sims_ui::hud::Controls;

        let mut controls = Controls::default();
        assert!(!controls.expanded);
        assert_eq!(viewport_for_controls([800, 600], &controls), (800, 600));

        controls.expanded = true;
        controls.mode = Mode::Live;
        assert_eq!(viewport_for_controls([800, 600], &controls), (800, 500));

        controls.mode = Mode::Build;
        assert_eq!(viewport_for_controls([800, 600], &controls), (800, 500));

        controls.mode = Mode::Options;
        assert_eq!(viewport_for_controls([800, 600], &controls), (800, 500));

        controls.mode = Mode::Buy;
        assert_eq!(viewport_for_controls([800, 600], &controls), (800, 500));

        // Short windows clamp to at least 1 pixel rather than wrapping.
        assert_eq!(viewport_for_controls([800, 80], &controls), (800, 1));
    }

    #[test]
    fn screen_centre_pick_uses_active_viewport_and_native_origin_column() {
        use sims_render::camera::{Walls, Zoom};
        use sims_ui::hud::Controls;

        let mut world = sims_sim::World::empty();
        world.size = 64;
        let full = sims_render::lot::FULL * sims_render::lot::FULL;
        for (id, stride) in [(0, 4), (1, 1), (2, 8), (101, 1), (102, 8)] {
            world.arrays.insert(
                id,
                sims_formats::lot::Array {
                    width: sims_render::lot::FULL,
                    height: sims_render::lot::FULL,
                    stride,
                    encoding: 0,
                    data: vec![0; full * stride],
                },
            );
        }
        let lot = sims_render::Lot::from_world(&world).unwrap();
        let pick_tile = |game_origin: [i32; 2], expanded: bool| {
            let controls = Controls {
                expanded,
                ..Controls::default()
            };
            let (w, h) = viewport_for_controls([800, 600], &controls);
            let camera = Camera {
                origin: [
                    game_origin[0].saturating_add(pick::ORIGIN_COLUMN),
                    game_origin[1],
                ],
                zoom: Zoom::Far,
                rotation: 0,
                level: 1,
                walls: Walls::Cutaway,
                focus: sims_render::Focus::default(),
            };
            sims_render::screen_centre_pick(&lot, &camera, [w as i32, h as i32])
                .map(|[x, y]| [(x >> 4) as i8, (y >> 4) as i8])
        };
        // Native origin [400, 0] (game origin [399, 0]): expanded 800x500 picks
        // [15, 15], collapsed 800x600 picks [18, 18].
        assert_eq!(pick_tile([399, 0], true), Some([15, 15]));
        assert_eq!(pick_tile([399, 0], false), Some([18, 18]));
        // Expanded 800x500: game origin [398, -6] (native [399, -6]) picks
        // [16, 15], while game origin [399, -6] (native [400, -6]) picks [16, 16].
        assert_eq!(pick_tile([398, -6], true), Some([16, 15]));
        assert_eq!(pick_tile([399, -6], true), Some([16, 16]));

        // Zooming from Far to Medium with the panel expanded (800x500 active
        // viewport) uses native 0x4a7b58 + 0x434cb7 + 0x4a7996 anchoring:
        // - from native [400, -6] (inside lot): stays on [16, 16];
        // - from native [400, 240] (tile [0, 0], outside interior): falls back
        //   to quarter-lot tile [16, 16] (0x4a7c56..0x4a7c96) instead of [32, 32].
        let controls = Controls {
            expanded: true,
            ..Controls::default()
        };
        let (w, h) = viewport_for_controls([800, 600], &controls);
        let viewport = [w as i32, h as i32];
        let pick_unrotated = |camera: Camera| {
            let mut native = camera;
            native.origin[0] = native.origin[0].saturating_add(pick::ORIGIN_COLUMN);
            let tile = sims_render::screen_centre_pick(&lot, &native, viewport)
                .map(|[x, y]| [i32::from((x >> 4) as i8), i32::from((y >> 4) as i8)])?;
            Some(native.unrotate_tile(tile))
        };

        for start_origin in [[399, -6], [399, 240]] {
            let mut cam = Camera {
                origin: start_origin,
                zoom: Zoom::Far,
                rotation: 0,
                level: 1,
                walls: Walls::Cutaway,
                focus: sims_render::Focus::default(),
            };
            assert!(native_zoom_or_rotate(
                &lot,
                &mut cam,
                viewport,
                Move::ZoomIn
            ));
            assert_eq!(
                pick_unrotated(cam),
                Some([16, 16]),
                "zoom from game origin {start_origin:?}"
            );
        }

        // On floor 2 at native origin [400, -6] (game [399, -6]), clockwise
        // rotation anchors on the displayed floor's pick ([19, 19]) and keeps
        // the unrotated search start at [19, 19].
        let mut floor2 = Camera {
            origin: [399, -6],
            zoom: Zoom::Far,
            rotation: 0,
            level: 2,
            walls: Walls::Cutaway,
            focus: sims_render::Focus::default(),
        };
        assert_eq!(pick_unrotated(floor2), Some([19, 19]));
        assert!(native_zoom_or_rotate(
            &lot,
            &mut floor2,
            viewport,
            Move::Rotate(true)
        ));
        assert_eq!(pick_unrotated(floor2), Some([19, 19]));

        // Off-lot wrapped pick: on a flat 64x64 lot at 800x600 (collapsed),
        // Far -> Medium from native origin [381, -3997] (game [380, -3997])
        // wraps tile [269, 268] to signed bytes [13, 12] at 0x4a7af0 and
        // recenters to native origin [392, -117] (game [391, -117]).
        let mut wrapped = Camera {
            origin: [380, -3997],
            zoom: Zoom::Far,
            rotation: 0,
            level: 1,
            walls: Walls::Cutaway,
            focus: sims_render::Focus::default(),
        };
        assert!(native_zoom_or_rotate(
            &lot,
            &mut wrapped,
            [800, 600],
            Move::ZoomIn
        ));
        assert_eq!(
            [wrapped.origin[0] + pick::ORIGIN_COLUMN, wrapped.origin[1]],
            [392, -117]
        );

        // 16-bit screen wrap during zoom recentring (0x4a79fd..0x4a7a47): from
        // game origin [380, 61539] (whose low 16 bits wrap to -3997), Far ->
        // Medium projects with the current origin and clamps to [391, 232].
        let mut wrapped_high = Camera {
            origin: [380, 61539],
            zoom: Zoom::Far,
            rotation: 0,
            level: 1,
            walls: Walls::Cutaway,
            focus: sims_render::Focus::default(),
        };
        assert!(native_zoom_or_rotate(
            &lot,
            &mut wrapped_high,
            [800, 600],
            Move::ZoomIn
        ));
        assert_eq!(wrapped_high.origin, [391, 232]);

        // Wrapped lot position projection in `lot_point`: [424, 392, 1] and
        // [4520, 392, 1] (differing by 256 tiles = 4096 fine units) project
        // identically after signed-byte tile narrowing (0x4a9c5c..0x4a9c71).
        let near = Camera {
            origin: [0, 0],
            zoom: Zoom::Near,
            rotation: 0,
            level: 1,
            walls: Walls::Cutaway,
            focus: sims_render::Focus::default(),
        };
        assert_eq!(
            lot_point(&near, &lot, [424, 392, 1]),
            lot_point(&near, &lot, [4520, 392, 1])
        );
    }
}
