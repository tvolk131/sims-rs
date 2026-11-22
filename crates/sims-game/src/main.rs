//! The game's window.
//!
//! ```text
//! sims-game [<install>] [<house file>] [--neighborhood] [--answers A,B,...] [--mute]
//!           [--play] [--saves <file>] [--png <file> [--ticks N] [--size WxH]]
//! ```
//!
//! The install defaults to `SIMS_INSTALL`. Without a house file (or with
//! `--neighborhood`), the window opens on the Neighborhood map screen
//! (`NeighborhoodShell`), importing `UserData`..`UserData8` into the REDB
//! `--saves` file on demand without modifying `SIMS_INSTALL`. Giving an
//! explicit `<house file>` (or `--png`) boots directly into that house
//! (defaulting `--png` to the Goths' `UserData/Houses/House05.iff` with
//! `--answers 3`), keeping direct house launch and headless PNG rendering
//! backwards-compatible.
//!
//! The lot runs at the original's pace: a tick every 42 milliseconds at
//! normal speed, 14 at fast and 2 at ultra. Visual presentation uses a hybrid
//! redraw cadence: matching the physical monitor's refresh rate during active
//! user input/interaction and capping passive simulation-only redraws to 60 FPS
//! while keeping simulation ticks running at full speed inside `SIM_BUDGET`.

// A window shell: it reads the wall clock to pace the lot.
#![allow(clippy::disallowed_methods)]

use sims_game::game::goth_house;
use sims_game::hud::DialogKey;
use sims_game::neighborhood::{NeighborhoodShell, ScreenKind, ShellOutcome};
use sims_game::sound::{self, Input};
use sims_game::view::Move;
use sims_game::{Game, Options};
use sims_render::{Walls, png};
use softbuffer::{Context, Surface};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

/// How often the loop wakes to step the lot when nothing else happens.
const WAKE: Duration = Duration::from_millis(5);

/// Wall-clock simulation budget per event-loop wake. Keeps Ultra/Idle speeds
/// responsive on the UI thread instead of running a full catch-up burst before
/// yielding to draw a frame.
const SIM_BUDGET: Duration = Duration::from_millis(8);

/// Minimum interval between window title bar updates while running steadily,
/// avoiding per-frame AppKit/WM `set_title` IPC churn at Fast/Ultra speed.
const TITLE_INTERVAL: Duration = Duration::from_millis(250);

/// Target redraw interval for passive simulation-only updates (60 FPS).
const PASSIVE_FPS_INTERVAL: Duration = Duration::from_nanos(16_666_667);

/// Fallback monitor refresh rate (60 Hz in millihertz) when winit reports none.
const FALLBACK_REFRESH_MILLIHERTZ: u32 = 60_000;

/// Minimum sane monitor refresh rate accepted from the OS (10 Hz).
const MIN_REFRESH_MILLIHERTZ: u32 = 10_000;

/// Maximum sane monitor refresh rate accepted from the OS (500 Hz).
const MAX_REFRESH_MILLIHERTZ: u32 = 500_000;

/// How long after the last discrete input event (pointer move, click, wheel,
/// key press) the redraw pacer stays at the physical display refresh rate
/// before settling back to 60 FPS passive cadence.
const INTERACTIVE_HOLD: Duration = Duration::from_millis(250);

/// Double-click detection window (400 ms, 4 canvas pixels).
const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);
const DOUBLE_CLICK_DISTANCE_SQ: i32 = 16;

/// Converts a monitor refresh rate in millihertz (`window.current_monitor()
/// .and_then(|m| m.refresh_rate_millihertz())`) into a frame interval,
/// falling back to 60 Hz when absent or outside `[10 Hz, 500 Hz]`.
fn refresh_interval_from_millihertz(millihertz: Option<u32>) -> Duration {
    let mhz = millihertz
        .filter(|&v| (MIN_REFRESH_MILLIHERTZ..=MAX_REFRESH_MILLIHERTZ).contains(&v))
        .unwrap_or(FALLBACK_REFRESH_MILLIHERTZ);
    let nanos = (1_000_000_000_000u64 / u64::from(mhz)).max(1);
    Duration::from_nanos(nanos)
}

/// Hybrid visual redraw cadence pacer:
/// - Matches the physical monitor refresh rate (e.g. 120 Hz on ProMotion,
///   60 Hz on 60 Hz displays) during active user input or ongoing interaction
///   (dragging, panning, edge-scrolling, dialogs, Buy/Build cursors).
/// - Caps passive simulation-only redraws to 60 FPS (`PASSIVE_FPS_INTERVAL`),
///   preventing 200+ FPS single-core busy-spin during Fast/Ultra speed while
///   allowing `tick()` to advance the simulation at full pace.
#[derive(Debug, Clone)]
struct RedrawPacer {
    display_interval: Duration,
    last_present: Option<Instant>,
    retry_after: Option<Instant>,
    last_interaction: Option<Instant>,
    redraw_requested: bool,
}

impl RedrawPacer {
    fn new(refresh_millihertz: Option<u32>) -> Self {
        Self {
            display_interval: refresh_interval_from_millihertz(refresh_millihertz),
            last_present: None,
            retry_after: None,
            last_interaction: None,
            redraw_requested: false,
        }
    }

    fn set_refresh_millihertz(&mut self, refresh_millihertz: Option<u32>) {
        self.display_interval = refresh_interval_from_millihertz(refresh_millihertz);
    }

    #[cfg(test)]
    fn display_interval(&self) -> Duration {
        self.display_interval
    }

    fn note_interaction(&mut self, now: Instant) {
        self.last_interaction = Some(now);
    }

    /// Forces the next dirty redraw to be requested immediately (e.g. after
    /// resize, scale change, or screen transition).
    fn request_immediate(&mut self) {
        self.last_present = None;
        self.retry_after = None;
    }

    fn is_interactive(&self, now: Instant, continuous_interaction: bool) -> bool {
        continuous_interaction
            || self
                .last_interaction
                .is_some_and(|t| now.saturating_duration_since(t) < INTERACTIVE_HOLD)
    }

    /// Active target frame interval at `now`.
    fn target_interval(&self, now: Instant, continuous_interaction: bool) -> Duration {
        if self.is_interactive(now, continuous_interaction) {
            self.display_interval
        } else {
            PASSIVE_FPS_INTERVAL.max(self.display_interval)
        }
    }

    fn due_instant(&self, now: Instant, continuous_interaction: bool) -> Option<Instant> {
        let interval = self.target_interval(now, continuous_interaction);
        let present_due = self.last_present.map(|prev| prev + interval);
        match (present_due, self.retry_after) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    /// Returns `true` when `window.request_redraw()` should be emitted now.
    fn should_request_redraw(
        &mut self,
        now: Instant,
        dirty: bool,
        continuous_interaction: bool,
    ) -> bool {
        if !dirty || self.redraw_requested {
            return false;
        }
        let due = self
            .due_instant(now, continuous_interaction)
            .is_none_or(|at| now >= at);
        if due {
            self.redraw_requested = true;
            true
        } else {
            false
        }
    }

    /// Earliest instant when a deferred dirty frame becomes due to present.
    fn next_redraw_deadline(
        &self,
        now: Instant,
        dirty: bool,
        continuous_interaction: bool,
    ) -> Option<Instant> {
        if !dirty || self.redraw_requested {
            return None;
        }
        self.due_instant(now, continuous_interaction)
    }

    /// Clears the pending redraw request as soon as the window event is
    /// consumed, before any early returns (e.g. zero-sized window), so a
    /// skipped redraw can never wedge future presentation.
    fn on_redraw_consumed(&mut self) {
        self.redraw_requested = false;
    }

    /// Paces retries after a surface resize, buffer acquisition, or present
    /// failure separately from `last_present`, preventing an unpaced busy loop
    /// while preserving retry behavior.
    fn on_redraw_failed(&mut self, now: Instant) {
        self.redraw_requested = false;
        self.retry_after = Some(now + PASSIVE_FPS_INTERVAL.max(self.display_interval));
    }

    fn on_present(&mut self, now: Instant) {
        self.last_present = Some(now);
        self.retry_after = None;
        self.redraw_requested = false;
    }
}

/// Whether the interactive wake loop should start another owed simulation tick
/// after `ran` ticks have already completed in `spent` wall-clock time.
///
/// Always permits the first owed tick (`ran == 0`) so a slow lot still makes
/// forward progress, then yields to render once `spent >= budget`.
fn within_step_budget(ran: u32, spent: Duration, budget: Duration) -> bool {
    ran == 0 || spent < budget
}

/// Tracks wall-clock time for the interactive wake loop with separate anchors
/// for presentation (`clock_us`: plumb bob spin, pause-button blink) and
/// simulation pacing (`Pace::due`).
struct WakeClock {
    last_clock: Instant,
    last_step: Instant,
}

impl WakeClock {
    fn new(now: Instant) -> Self {
        Self {
            last_clock: now,
            last_step: now,
        }
    }

    fn reset(&mut self, now: Instant) {
        self.last_clock = now;
        self.last_step = now;
    }

    fn elapsed_us(&self, now: Instant) -> (u64, u64) {
        let to_us = |d: Duration| u64::try_from(d.as_micros()).unwrap_or(u64::MAX);
        (
            to_us(now.saturating_duration_since(self.last_clock)),
            to_us(now.saturating_duration_since(self.last_step)),
        )
    }

    fn finish(&mut self, wake_start: Instant, wake_end: Instant, capped: bool) {
        self.last_clock = wake_start;
        self.last_step = if capped { wake_end } else { wake_start };
    }
}

/// High-level run state reflected in the window title suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TitleState {
    Running,
    Paused,
    Stopped,
    Shell,
}

/// Rate-limits window title updates while running, while flushing immediately
/// on the first draw and whenever the paused/stopped/shell status changes.
#[derive(Default)]
struct TitleThrottle {
    last_title: String,
    last_state: Option<TitleState>,
    last_update: Option<Instant>,
}

impl TitleThrottle {
    fn update(
        &mut self,
        ticks: u32,
        state: TitleState,
        now: Instant,
        interval: Duration,
    ) -> Option<&str> {
        let state_changed = self.last_state != Some(state);
        let interval_elapsed = self
            .last_update
            .is_none_or(|prev| now.saturating_duration_since(prev) >= interval);
        if !state_changed && !interval_elapsed {
            return None;
        }
        let suffix = match state {
            TitleState::Running => "",
            TitleState::Paused => ", paused",
            TitleState::Stopped => ", stopped",
            TitleState::Shell => "",
        };
        let next = format!("sims-game: tick {ticks}{suffix}");
        self.last_state = Some(state);
        if next == self.last_title {
            return None;
        }
        self.last_title = next;
        self.last_update = Some(now);
        Some(&self.last_title)
    }

    fn update_custom(&mut self, next: String, now: Instant) -> Option<&str> {
        self.last_state = Some(TitleState::Shell);
        if next == self.last_title {
            return None;
        }
        self.last_title = next;
        self.last_update = Some(now);
        Some(&self.last_title)
    }
}

/// The sound the window plays, or none.
#[cfg(any(target_os = "macos", feature = "sound"))]
struct Sound(Option<sims_game::output::Output>);

#[cfg(any(target_os = "macos", feature = "sound"))]
impl Sound {
    fn start(install: &std::path::Path, mute: bool) -> Self {
        if mute {
            return Self(None);
        }
        let started =
            sims_game::sound::Mixer::open(install).and_then(sims_game::output::Output::start);
        match started {
            Ok(output) => Self(Some(output)),
            Err(e) => {
                eprintln!("sims-game: playing without sound: {e}");
                Self(None)
            }
        }
    }

    fn send(&self, input: Input) {
        if let Some(output) = &self.0 {
            output.send(input);
        }
    }
}

#[cfg(not(any(target_os = "macos", feature = "sound")))]
struct Sound;

#[cfg(not(any(target_os = "macos", feature = "sound")))]
impl Sound {
    fn start(_: &std::path::Path, mute: bool) -> Self {
        if !mute {
            eprintln!("sims-game: built without sound (the `sound` feature)");
        }
        Self
    }

    fn send(&self, _: Input) {}
}

/// A frame drawn without a window.
struct Still {
    file: PathBuf,
    ticks: u32,
    size: [i32; 2],
}

/// Parsed CLI launch configuration.
struct LaunchConfig {
    install: PathBuf,
    /// `Some(house)` for direct house launch or `--png`; `None` for the
    /// Neighborhood screen shell.
    house: Option<PathBuf>,
    answers: Vec<u8>,
    play: bool,
    still: Option<Still>,
    mute: bool,
    saves: PathBuf,
}

fn parse_launch_args<I>(args: I, env_install: Option<PathBuf>) -> Result<LaunchConfig, String>
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let usage = "usage: sims-game [<install>] [<house file>] [--neighborhood] [--answers A,B,...] \
                 [--mute] [--play] [--saves <file>] [--png <file> [--ticks N] [--size WxH]]";
    let mut paths = Vec::new();
    let mut answers = None;
    let mut png = None;
    let mut ticks = 0;
    let mut size = [800, 600];
    let mut mute = false;
    let mut play = false;
    let mut neighborhood_flag = false;
    let mut saves = PathBuf::from("sims-game-saves.redb");
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut text = |name: &str| -> Result<String, String> {
            args.next()
                .and_then(|a| a.into_string().ok())
                .ok_or_else(|| format!("{name} needs a value"))
        };
        if arg == "--mute" {
            mute = true;
        } else if arg == "--play" {
            play = true;
        } else if arg == "--neighborhood" {
            neighborhood_flag = true;
        } else if arg == "--saves" {
            saves = PathBuf::from(text("--saves")?);
        } else if arg == "--png" {
            png = Some(PathBuf::from(text("--png")?));
        } else if arg == "--ticks" {
            let n = text("--ticks")?;
            ticks = n.parse().map_err(|e| format!("--ticks {n}: {e}"))?;
        } else if arg == "--size" {
            let n = text("--size")?;
            let parsed = n
                .split_once('x')
                .and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))
                .filter(|s: &[i32; 2]| s.iter().all(|&v| (1..=8192).contains(&v)));
            size = parsed.ok_or_else(|| format!("--size {n}: give WIDTHxHEIGHT"))?;
        } else if arg == "--answers" {
            let list = args.next().ok_or(usage)?;
            let list = list.to_str().ok_or("--answers must be Unicode")?;
            answers = Some(
                list.split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::parse)
                    .collect::<Result<Vec<u8>, _>>()
                    .map_err(|e| format!("--answers {list}: {e}"))?,
            );
        } else if arg.to_str().is_some_and(|a| a.starts_with("--")) {
            return Err(usage.into());
        } else {
            paths.push(PathBuf::from(arg));
        }
    }
    let mut paths = paths.into_iter();
    let install = match paths.next() {
        Some(path) => path,
        None => env_install.ok_or("give the install, or set SIMS_INSTALL")?,
    };
    let explicit_house = paths.next();
    if paths.next().is_some() {
        return Err(usage.into());
    }
    let goths = goth_house(&install);
    let house = if neighborhood_flag && png.is_none() {
        if explicit_house.is_some() {
            return Err("--neighborhood cannot be combined with a house file".into());
        }
        None
    } else if png.is_some() {
        Some(explicit_house.unwrap_or_else(|| goths.clone()))
    } else {
        explicit_house
    };
    // In the window the player answers; a still has nobody to.
    let answers = answers.unwrap_or_else(|| {
        if house.as_ref() == Some(&goths) && png.is_some() {
            vec![3]
        } else {
            Vec::new()
        }
    });
    let still = png.map(|file| Still { file, ticks, size });
    Ok(LaunchConfig {
        install,
        house,
        answers,
        play,
        still,
        mute,
        saves,
    })
}

fn options() -> Result<LaunchConfig, String> {
    parse_launch_args(
        std::env::args_os().skip(1),
        std::env::var_os("SIMS_INSTALL").map(PathBuf::from),
    )
}

/// Active desktop session: either a standalone explicit house launch or the
/// full Neighborhood ↔ Family Bin ↔ Lot shell.
enum Session {
    Standalone {
        game: Box<Game>,
        slot: String,
        options_file: PathBuf,
        saved_optn: Vec<u8>,
    },
    Shell(Box<NeighborhoodShell>),
}

impl Session {
    fn game(&self) -> Option<&Game> {
        match self {
            Self::Standalone { game, .. } => Some(game),
            Self::Shell(shell) => shell.game(),
        }
    }

    fn game_mut(&mut self) -> Option<&mut Game> {
        match self {
            Self::Standalone { game, .. } => Some(game),
            Self::Shell(shell) => shell.game_mut(),
        }
    }

    fn screen_kind(&self) -> ScreenKind {
        match self {
            Self::Standalone { .. } => ScreenKind::Lot,
            Self::Shell(shell) => shell.screen(),
        }
    }

    fn has_dialog(&self) -> bool {
        match self {
            Self::Standalone { game, .. } => game.has_dialog(),
            Self::Shell(shell) => shell.has_dialog(),
        }
    }

    fn is_actively_interacting(&self) -> bool {
        match self {
            Self::Standalone { game, .. } => game.is_actively_interacting(),
            Self::Shell(shell) => shell.is_actively_interacting(),
        }
    }

    fn resize(&mut self, size: [i32; 2]) -> bool {
        match self {
            Self::Standalone { game, .. } => game.resize(size),
            Self::Shell(shell) => shell.resize(size),
        }
    }

    fn render(&mut self) {
        match self {
            Self::Standalone { game, .. } => {
                let _ = game.render();
            }
            Self::Shell(shell) => shell.render(),
        }
    }

    fn canvas(&self) -> &sims_render::Canvas {
        match self {
            Self::Standalone { game, .. } => game.canvas(),
            Self::Shell(shell) => shell.canvas(),
        }
    }

    fn persist_options(&mut self) {
        match self {
            Self::Standalone {
                game,
                options_file,
                saved_optn,
                ..
            } => {
                let current = game.options().to_optn();
                if current != *saved_optn {
                    match game.save_options(options_file) {
                        Ok(()) => *saved_optn = current,
                        Err(e) => eprintln!("sims-game: {}: {e}", options_file.display()),
                    }
                }
            }
            Self::Shell(shell) => shell.persist_options(),
        }
    }
}

struct App {
    session: Session,
    last_screen: ScreenKind,
    window: Option<Rc<Window>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    surface_size: Option<(NonZeroU32, NonZeroU32)>,
    title_throttle: TitleThrottle,
    redraw_pacer: RedrawPacer,
    scale: u32,
    cursor: Option<PhysicalPosition<f64>>,
    /// Most recent cursor position reported by winit (including outside the
    /// client rect) so captured Sound slider drags evaluate out-of-window
    /// release coordinates (`0x5f62ba..0x5f62de`).
    last_cursor: Option<PhysicalPosition<f64>>,
    drag: Option<PhysicalPosition<f64>>,
    last_left_press: Option<(Instant, [i32; 2])>,
    wake_clock: WakeClock,
    dirty: bool,
    photos: usize,
    /// The stop already printed, so it is printed once.
    reported: bool,
    /// How many of the game's caught errors are printed.
    printed: usize,
    sound: Sound,
    /// Tracks what the sound has been told about the lot and camera.
    sound_feed: sound::Feed,
    /// Filters winit's synthetic pre-activation focus events at startup
    /// (notably macOS `WindowEvent::Focused(false)` queued during
    /// `create_window` before `makeKeyAndOrderFront`).
    focus_filter: FocusFilter,
    /// The REDB file of saves (used directly in Standalone mode).
    saves: PathBuf,
    /// Modifier keys tracked from `WindowEvent::ModifiersChanged` for Buy Mode
    /// shortcuts (`Ctrl/Cmd+Z` Undo, `Ctrl+Y`/`Ctrl/Cmd+Shift+Z` Redo).
    ctrl_or_cmd_held: bool,
    shift_held: bool,
}

/// Filters startup `WindowEvent::Focused` notifications so ordinary window
/// launch sends no kind 25 (matching `0x4eee14` + `0x4f7610`).
#[derive(Default)]
struct FocusFilter {
    activated: bool,
}

impl FocusFilter {
    fn filter(&mut self, focused: bool) -> Option<bool> {
        if !self.activated {
            if focused {
                self.activated = true;
            }
            return None;
        }
        Some(focused)
    }
}

/// The canvas dimensions for a physical client area `size` at `scale`,
/// matching [`App::redraw`].
fn canvas_size(size: winit::dpi::PhysicalSize<u32>, scale: u32) -> Option<[i32; 2]> {
    if size.width == 0 || size.height == 0 {
        return None;
    }
    let scale = scale.max(1);
    Some(
        [size.width / scale, size.height / scale]
            .map(|n| i32::try_from(n).unwrap_or(i32::MAX).max(1)),
    )
}

/// Maps a physical pointer `position` inside `size` at `scale` to its
/// canvas pixel using the same edge clamp as [`App::redraw`]; `None` when
/// `position` is outside `[0..size.width, 0..size.height)` or the client
/// area is empty.
fn canvas_point(
    position: PhysicalPosition<f64>,
    size: winit::dpi::PhysicalSize<u32>,
    scale: u32,
) -> Option<[i32; 2]> {
    let [w, h] = canvas_size(size, scale)?;
    if position.x < 0.0
        || position.y < 0.0
        || position.x >= f64::from(size.width)
        || position.y >= f64::from(size.height)
    {
        return None;
    }
    let scale = f64::from(scale.max(1));
    let x = ((position.x / scale) as i32).clamp(0, w - 1);
    let y = ((position.y / scale) as i32).clamp(0, h - 1);
    Some([x, y])
}

/// Maps any physical pointer `position` (including out-of-bounds coordinates
/// during captured slider drags, `0x5f6245..0x5f62de`) to canvas pixels.
fn unclamped_canvas_point(
    position: PhysicalPosition<f64>,
    size: winit::dpi::PhysicalSize<u32>,
    scale: u32,
) -> Option<[i32; 2]> {
    let _ = canvas_size(size, scale)?;
    let scale = f64::from(scale.max(1));
    Some([
        (position.x / scale).floor() as i32,
        (position.y / scale).floor() as i32,
    ])
}

/// Queries the live Quartz cursor position relative to `window`'s content area
/// so a macOS Sound slider release outside the window evaluates the true
/// release X coordinate (`0x5f62ba..0x5f62de`) even when winit filters
/// `mouse_motion` after button release.
#[cfg(target_os = "macos")]
fn live_macos_cursor_point(window: &Window, scale: u32) -> Option<[i32; 2]> {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    let inner = window.inner_position().ok()?;
    let size = window.inner_size();
    let winit_scale = window.scale_factor();
    if !(winit_scale.is_finite() && winit_scale > 0.0) {
        return None;
    }
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
    let event = CGEvent::new(source).ok()?;
    let loc = event.location();
    let phys = PhysicalPosition::new(
        loc.x * winit_scale - f64::from(inner.x),
        loc.y * winit_scale - f64::from(inner.y),
    );
    unclamped_canvas_point(phys, size, scale)
}

/// Path of the persisted user `Options.iff` (`Optn` 128) beside `saves`.
fn options_path(saves: &std::path::Path) -> PathBuf {
    saves.with_extension("options.iff")
}

impl App {
    fn refresh_monitor_rate(&mut self) {
        let rate = self
            .window
            .as_ref()
            .and_then(|w| w.current_monitor())
            .and_then(|m| m.refresh_rate_millihertz());
        self.redraw_pacer.set_refresh_millihertz(rate);
    }

    fn continuous_interaction(&self) -> bool {
        self.drag.is_some() || self.session.is_actively_interacting()
    }

    fn window_drawable(&self) -> bool {
        self.window
            .as_ref()
            .is_some_and(|w| canvas_size(w.inner_size(), self.scale.max(1)).is_some())
    }

    fn maybe_request_redraw(&mut self, now: Instant) {
        if !self.window_drawable() {
            return;
        }
        let continuous = self.continuous_interaction();
        if self
            .redraw_pacer
            .should_request_redraw(now, self.dirty, continuous)
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
    }

    fn drain_shell_audio(&mut self) {
        let screen_now = self.session.screen_kind();
        if screen_now != self.last_screen {
            self.last_screen = screen_now;
            self.sound_feed.reset_for_lot();
            self.drag = None;
            self.printed = 0;
            self.reported = false;
            self.wake_clock.reset(Instant::now());
            self.redraw_pacer.request_immediate();
        }
        if let Session::Shell(shell) = &mut self.session {
            for input in shell.take_audio_inputs() {
                self.sound.send(input);
            }
        }
    }

    fn persist_options(&mut self) {
        self.session.persist_options();
    }

    fn slider_release_point(&mut self) -> Option<[i32; 2]> {
        #[cfg(target_os = "macos")]
        if let Some(window) = self.window.as_ref()
            && let Some(point) = live_macos_cursor_point(window, self.scale)
        {
            return Some(point);
        }
        if let Some(point) = self.live_point() {
            return Some(point);
        }
        self.window
            .as_ref()
            .and_then(|w| unclamped_canvas_point(self.last_cursor?, w.inner_size(), self.scale))
    }

    fn sync_size(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        if let Some(canvas) = canvas_size(size, self.scale) {
            self.session.resize(canvas);
        }
        if self
            .cursor
            .is_some_and(|c| canvas_point(c, size, self.scale).is_none())
        {
            self.cursor = None;
            let _ = match &mut self.session {
                Session::Standalone { game, .. } => game.leave(),
                Session::Shell(shell) => shell.leave(),
            };
        }
    }

    fn live_point(&mut self) -> Option<[i32; 2]> {
        let size = self.window.as_ref()?.inner_size();
        if let Some(canvas) = canvas_size(size, self.scale) {
            self.session.resize(canvas);
        }
        let cursor = self.cursor?;
        let point = canvas_point(cursor, size, self.scale);
        if point.is_none() {
            self.cursor = None;
        }
        point
    }

    /// Moves the pointer where the game asks, as the original moves it
    /// to the middle of a pie menu.
    fn move_pointer(&mut self) {
        let Some([x, y]) = self.session.game_mut().and_then(Game::take_pointer) else {
            return;
        };
        let scale = f64::from(self.scale.max(1));
        let position =
            PhysicalPosition::new((f64::from(x) + 0.5) * scale, (f64::from(y) + 0.5) * scale);
        if let Some(w) = &self.window
            && w.set_cursor_position(position).is_ok()
        {
            self.cursor = Some(position);
        }
        if let Some(point) = self.live_point() {
            let redraw = match &mut self.session {
                Session::Standalone { game, .. } => game.hover(point),
                Session::Shell(shell) => shell.hover(point),
            };
            self.changed(redraw, true);
        }
    }

    fn changed(&mut self, redraw: bool, interactive: bool) {
        let now = Instant::now();
        if interactive {
            self.redraw_pacer.note_interaction(now);
        }
        if redraw {
            self.dirty = true;
        }
        self.maybe_request_redraw(now);
    }

    fn photo(&mut self) {
        let path = PathBuf::from(format!("sims-game-{}.png", self.photos));
        self.photos += 1;
        match std::fs::write(&path, png::encode(self.session.canvas())) {
            Ok(()) => eprintln!("saved {}", path.display()),
            Err(e) => eprintln!("could not save {}: {e}", path.display()),
        }
    }

    /// Saves the lot (F5) or goes back to its save (F9). Returns `true` when
    /// the operation succeeded (`0x4f26cf`, checked by `0x4f29cc` before quit).
    fn save(&mut self, restore: bool) -> bool {
        let ok = match &mut self.session {
            Session::Standalone { game, slot, .. } => {
                let store = match sims_sim::Snapshots::open(&self.saves) {
                    Ok(store) => store,
                    Err(e) => {
                        eprintln!("sims-game: {}: {e}", self.saves.display());
                        let msg = if restore {
                            format!("The save was not restored: {e}")
                        } else {
                            format!("The lot was not saved: {e}")
                        };
                        game.set_note(msg);
                        self.changed(true, true);
                        return false;
                    }
                };
                if !restore {
                    game.save_to(&store, slot)
                } else if game.restore_from(&store, slot) {
                    self.printed = 0;
                    self.reported = false;
                    self.wake_clock.reset(Instant::now());
                    true
                } else {
                    false
                }
            }
            Session::Shell(shell) => {
                if !restore {
                    shell.save_current_lot().is_ok()
                } else if shell.restore_current_lot().is_ok() {
                    self.printed = 0;
                    self.reported = false;
                    self.wake_clock.reset(Instant::now());
                    true
                } else {
                    false
                }
            }
        };
        self.drain_shell_audio();
        self.changed(true, true);
        ok
    }

    fn handle_standalone_outcome(
        &mut self,
        event_loop: &ActiveEventLoop,
        outcome: sims_game::game::Outcome,
    ) {
        self.persist_options();
        self.changed(outcome.redraw, true);
        self.move_pointer();
        if outcome.photo {
            self.photo();
        }
        let saved_ok = if outcome.save { self.save(false) } else { true };
        if outcome.save
            && !saved_ok
            && let Some(game) = self.session.game_mut()
        {
            game.after_save_failed();
            self.changed(true, true);
        }
        if outcome.neighborhood
            && saved_ok
            && let Some(game) = self.session.game_mut()
        {
            game.set_note(
                "Neighborhood view requires launching without a house file (or with --neighborhood).",
            );
            self.changed(true, true);
        }
        if outcome.quit && saved_ok {
            event_loop.exit();
        }
    }

    fn handle_shell_outcome(&mut self, event_loop: &ActiveEventLoop, outcome: ShellOutcome) {
        self.drain_shell_audio();
        self.changed(outcome.redraw, true);
        self.move_pointer();
        if outcome.photo {
            self.photo();
        }
        if outcome.quit {
            self.persist_options();
            event_loop.exit();
        }
    }

    fn key(&mut self, key: &Key) {
        let Some(game) = self.session.game_mut() else {
            return;
        };
        let step = 64;
        let command = match key {
            Key::Named(NamedKey::ArrowLeft) => Move::Scroll([step, 0]),
            Key::Named(NamedKey::ArrowRight) => Move::Scroll([-step, 0]),
            Key::Named(NamedKey::ArrowUp) => Move::Scroll([0, step]),
            Key::Named(NamedKey::ArrowDown) => Move::Scroll([0, -step]),
            Key::Named(NamedKey::PageUp) => Move::LevelUp,
            Key::Named(NamedKey::PageDown) => Move::LevelDown,
            Key::Named(NamedKey::Space) => {
                game.toggle_pause();
                self.changed(true, true);
                return;
            }
            Key::Named(NamedKey::F1) => {
                game.set_mode(sims_ui::command::Mode::Live);
                self.changed(true, true);
                return;
            }
            Key::Named(NamedKey::F2) => {
                game.set_mode(sims_ui::command::Mode::Buy);
                self.changed(true, true);
                return;
            }
            Key::Named(NamedKey::F3) => {
                game.set_mode(sims_ui::command::Mode::Build);
                self.changed(true, true);
                return;
            }
            Key::Named(NamedKey::F5) => {
                let _ = self.save(false);
                return;
            }
            Key::Named(NamedKey::F9) => {
                let _ = self.save(true);
                return;
            }
            Key::Named(NamedKey::Delete | NamedKey::Backspace) => {
                if game.sell_held() {
                    self.changed(true, true);
                }
                return;
            }
            Key::Character(c) => {
                if self.ctrl_or_cmd_held {
                    match c.as_str() {
                        "z" | "Z" if !self.shift_held => {
                            if game.undo_buy() {
                                self.changed(true, true);
                            }
                            return;
                        }
                        "z" | "Z" | "y" | "Y" => {
                            if game.redo_buy() {
                                self.changed(true, true);
                            }
                            return;
                        }
                        _ => {}
                    }
                }
                match c.as_str() {
                    "+" | "=" => Move::ZoomIn,
                    "-" | "_" => Move::ZoomOut,
                    "[" => Move::Rotate(false),
                    "]" => Move::Rotate(true),
                    "," | "<" => {
                        if game.rotate_held(false) {
                            self.changed(true, true);
                            return;
                        }
                        Move::Rotate(false)
                    }
                    "." | ">" => {
                        if game.rotate_held(true) {
                            self.changed(true, true);
                            return;
                        }
                        Move::Rotate(true)
                    }
                    "x" | "X" => {
                        if game.page_buy(1) {
                            self.changed(true, true);
                        }
                        return;
                    }
                    "z" | "Z" => {
                        if game.page_buy(-1) {
                            self.changed(true, true);
                        }
                        return;
                    }
                    "1" => Move::Walls(Walls::Up),
                    "2" => Move::Walls(Walls::Cutaway),
                    "3" => Move::Walls(Walls::Down),
                    _ => return,
                }
            }
            _ => return,
        };
        let redraw = game.apply(command);
        self.changed(redraw, true);
    }

    /// Steps the active lot (or neighborhood animation timer) for the time
    /// since the last call.
    fn tick(&mut self) {
        let now = Instant::now();
        let (clock_us, step_us) = self.wake_clock.elapsed_us(now);
        self.drain_shell_audio();
        let Some(game) = self.session.game_mut() else {
            self.wake_clock.finish(now, Instant::now(), false);
            let redraw = match &mut self.session {
                Session::Shell(shell) => shell.tick_shell(clock_us),
                Session::Standalone { .. } => false,
            };
            self.changed(redraw, false);
            return;
        };

        let waited = game.asking().is_some();
        let (ran, capped) = game.advance_while(clock_us, step_us, |ran| {
            within_step_budget(ran, now.elapsed(), SIM_BUDGET)
        });
        self.wake_clock.finish(now, Instant::now(), capped);
        let mut redraw = ran > 0 || (!waited && game.asking().is_some());
        let scrolled = self.drag.is_none() && game.edge_scroll(clock_us);
        redraw |= scrolled || game.spin_due() || game.blink_due();
        self.sound_feed.update(game, |input| self.sound.send(input));
        for error in &game.errors()[self.printed..] {
            eprintln!("sims-game: error caught: {error}");
        }
        self.printed = game.errors().len();
        if !self.reported
            && let Some(stop) = game.stopped()
        {
            eprintln!("sims-game: {stop}");
            self.reported = true;
            redraw = true;
        }
        // Edge-scrolling counts as active user interaction (display-refresh cadence),
        // while pure simulation ticks / plumb-bob spin / pause blink are passive (60 FPS cap).
        self.changed(redraw, scrolled);
    }

    fn redraw(&mut self) {
        self.redraw_pacer.on_redraw_consumed();
        let Some(window) = self.window.clone() else {
            return;
        };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return;
        };
        let scale = self.scale.max(1);
        let Some(canvas_size) = canvas_size(size, scale) else {
            return;
        };
        if self.session.resize(canvas_size) {
            self.dirty = true;
        }
        if self.dirty {
            self.session.render();
            self.dirty = false;
            let now = Instant::now();
            match &self.session {
                Session::Standalone { game, .. } => {
                    let state = if game.stopped().is_some() {
                        TitleState::Stopped
                    } else if game.paused() {
                        TitleState::Paused
                    } else {
                        TitleState::Running
                    };
                    if let Some(title) =
                        self.title_throttle
                            .update(game.lot().ticks(), state, now, TITLE_INTERVAL)
                    {
                        window.set_title(title);
                    }
                }
                Session::Shell(shell) => {
                    if let Some(game) = shell.game() {
                        let state = if game.stopped().is_some() {
                            TitleState::Stopped
                        } else if game.paused() {
                            TitleState::Paused
                        } else {
                            TitleState::Running
                        };
                        if let Some(title) = self.title_throttle.update(
                            game.lot().ticks(),
                            state,
                            now,
                            TITLE_INTERVAL,
                        ) {
                            window.set_title(title);
                        }
                    } else if let Some(title) =
                        self.title_throttle.update_custom(shell.window_title(), now)
                    {
                        window.set_title(title);
                    }
                }
            }
        }
        let Some(surface) = &mut self.surface else {
            self.dirty = true;
            self.redraw_pacer.on_redraw_failed(Instant::now());
            return;
        };
        if self.surface_size != Some((w, h)) {
            if surface.resize(w, h).is_err() {
                self.surface_size = None;
                self.dirty = true;
                self.redraw_pacer.on_redraw_failed(Instant::now());
                return;
            }
            self.surface_size = Some((w, h));
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            self.surface_size = None;
            self.dirty = true;
            self.redraw_pacer.on_redraw_failed(Instant::now());
            return;
        };
        let canvas = self.session.canvas();
        let (bw, bh) = (size.width as usize, size.height as usize);
        blit_canvas(canvas, &mut buffer, bw, bh, scale as usize);
        if buffer.present().is_err() {
            self.surface_size = None;
            self.dirty = true;
            self.redraw_pacer.on_redraw_failed(Instant::now());
            return;
        }
        self.redraw_pacer.on_present(Instant::now());
    }
}

#[inline(always)]
fn pack_rgb([r, g, b]: [u8; 3]) -> u32 {
    (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

#[inline]
fn blit_row_1x(src_row: &[[u8; 3]], dst_row: &mut [u32]) {
    let copy_w = dst_row.len().min(src_row.len());
    for (d, &rgb) in dst_row[..copy_w].iter_mut().zip(&src_row[..copy_w]) {
        *d = pack_rgb(rgb);
    }
    if dst_row.len() > copy_w {
        let last = pack_rgb(src_row[copy_w - 1]);
        dst_row[copy_w..].fill(last);
    }
}

#[inline]
fn blit_row_2x(src_row: &[[u8; 3]], dst_row: &mut [u32]) {
    let pairs = (dst_row.len() / 2).min(src_row.len());
    let (head, tail) = dst_row.split_at_mut(pairs * 2);
    for (pair, &rgb) in head
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .zip(&src_row[..pairs])
    {
        let px = pack_rgb(rgb);
        *pair = [px, px];
    }
    if !tail.is_empty() {
        let cx = pairs.min(src_row.len() - 1);
        tail.fill(pack_rgb(src_row[cx]));
    }
}

/// Blits `canvas` into the `bw × bh` `softbuffer` pixel buffer scaled up by
/// integer `scale`, matching `(y / scale).min(h - 1)` / `(x / scale).min(w - 1)`
/// bit-for-bit, with specialized zero-division paths for `scale == 1` and
/// `scale == 2`.
fn blit_canvas(
    canvas: &sims_render::Canvas,
    buffer: &mut [u32],
    bw: usize,
    bh: usize,
    scale: usize,
) {
    let scale = scale.max(1);
    let Some(total) = bw.checked_mul(bh) else {
        return;
    };
    if total == 0 || buffer.len() < total {
        return;
    }
    if canvas.width == 0
        || canvas.height == 0
        || canvas.color.len() < canvas.width.saturating_mul(canvas.height)
    {
        buffer[..total].fill(0);
        return;
    }
    let max_cy = canvas.height - 1;
    let max_cx = canvas.width - 1;
    match scale {
        1 => {
            for y in 0..bh {
                let cy = y.min(max_cy);
                if y > 0 && cy == (y - 1).min(max_cy) {
                    let prev = (y - 1) * bw;
                    buffer.copy_within(prev..prev + bw, y * bw);
                } else {
                    let src_row = &canvas.color[cy * canvas.width..(cy + 1) * canvas.width];
                    blit_row_1x(src_row, &mut buffer[y * bw..(y + 1) * bw]);
                }
            }
        }
        2 => {
            for y in 0..bh {
                let cy = (y >> 1).min(max_cy);
                if y > 0 && cy == ((y - 1) >> 1).min(max_cy) {
                    let prev = (y - 1) * bw;
                    buffer.copy_within(prev..prev + bw, y * bw);
                } else {
                    let src_row = &canvas.color[cy * canvas.width..(cy + 1) * canvas.width];
                    blit_row_2x(src_row, &mut buffer[y * bw..(y + 1) * bw]);
                }
            }
        }
        _ => {
            for y in 0..bh {
                let cy = (y / scale).min(max_cy);
                if y > 0 && cy == ((y - 1) / scale).min(max_cy) {
                    let prev = (y - 1) * bw;
                    buffer.copy_within(prev..prev + bw, y * bw);
                } else {
                    let src_row = &canvas.color[cy * canvas.width..(cy + 1) * canvas.width];
                    let dst_row = &mut buffer[y * bw..(y + 1) * bw];
                    for (x, dst) in dst_row.iter_mut().enumerate() {
                        let cx = (x / scale).min(max_cx);
                        *dst = pack_rgb(src_row[cx]);
                    }
                }
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("sims-game")
            .with_inner_size(LogicalSize::new(1024.0, 768.0));
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Rc::new(w),
            Err(e) => {
                eprintln!("sims-game: could not open a window: {e}");
                event_loop.exit();
                return;
            }
        };
        let surface =
            Context::new(window.clone()).and_then(|context| Surface::new(&context, window.clone()));
        match surface {
            Ok(s) => {
                self.surface = Some(s);
                self.surface_size = None;
            }
            Err(e) => {
                eprintln!("sims-game: could not draw into the window: {e}");
                event_loop.exit();
                return;
            }
        }
        self.scale = window.scale_factor().round().max(1.0) as u32;
        self.window = Some(window.clone());
        self.refresh_monitor_rate();
        self.drain_shell_audio();
        self.wake_clock.reset(Instant::now());
        self.dirty = true;
        self.redraw_pacer.request_immediate();
        self.maybe_request_redraw(Instant::now());
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.tick();
        let now = Instant::now();
        let wake_deadline = now + WAKE;
        let next_deadline = if self.window_drawable() {
            self.redraw_pacer
                .next_redraw_deadline(now, self.dirty, self.continuous_interaction())
                .map_or(wake_deadline, |d| d.min(wake_deadline))
        } else {
            wake_deadline
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(next_deadline));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.persist_options();
                event_loop.exit();
            }
            WindowEvent::Focused(focused) => {
                if focused {
                    self.refresh_monitor_rate();
                }
                if let Some(focused) = self.focus_filter.filter(focused) {
                    if let Some(game) = self.session.game_mut() {
                        if self
                            .sound_feed
                            .set_focused(focused, game, |input| self.sound.send(input))
                        {
                            self.wake_clock.reset(Instant::now());
                            self.changed(true, true);
                        }
                    } else {
                        self.sound_feed
                            .set_shell_focused(focused, |input| self.sound.send(input));
                    }
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Moved(_) => {
                self.refresh_monitor_rate();
            }
            WindowEvent::Resized(size) => {
                self.refresh_monitor_rate();
                self.sync_size(size);
                self.redraw_pacer.request_immediate();
                self.changed(true, true);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor.round().max(1.0) as u32;
                self.surface_size = None;
                self.refresh_monitor_rate();
                if let Some(size) = self.window.as_ref().map(|w| w.inner_size()) {
                    self.sync_size(size);
                }
                self.redraw_pacer.request_immediate();
                self.changed(true, true);
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                self.shift_held = state.shift_key();
                self.ctrl_or_cmd_held = state.control_key() || state.super_key();
                if let Some(game) = self.session.game_mut() {
                    game.set_shift(self.shift_held);
                    game.set_ctrl(state.control_key());
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.session.has_dialog() {
                    let has_entry = self.session.game().is_some_and(Game::has_dialog_entry);
                    if has_entry {
                        let mut changed = false;
                        let mut standalone_out = None;
                        let mut shell_out = None;
                        match &event.logical_key {
                            Key::Named(NamedKey::Backspace) => {
                                changed =
                                    self.session.game_mut().is_some_and(Game::dialog_backspace);
                            }
                            Key::Named(NamedKey::Delete) => {
                                changed = self.session.game_mut().is_some_and(Game::dialog_delete);
                            }
                            Key::Named(NamedKey::ArrowLeft) => {
                                changed =
                                    self.session.game_mut().is_some_and(Game::dialog_move_left);
                            }
                            Key::Named(NamedKey::ArrowRight) => {
                                changed =
                                    self.session.game_mut().is_some_and(Game::dialog_move_right);
                            }
                            Key::Named(NamedKey::ArrowUp) => {
                                changed = self.session.game_mut().is_some_and(Game::dialog_move_up);
                            }
                            Key::Named(NamedKey::ArrowDown) => {
                                changed =
                                    self.session.game_mut().is_some_and(Game::dialog_move_down);
                            }
                            Key::Named(NamedKey::Home) => {
                                changed = self.session.game_mut().is_some_and(Game::dialog_home);
                            }
                            Key::Named(NamedKey::End) => {
                                changed = self.session.game_mut().is_some_and(Game::dialog_end);
                            }
                            Key::Named(NamedKey::Tab) => {
                                changed = self
                                    .session
                                    .game_mut()
                                    .is_some_and(Game::dialog_switch_field);
                            }
                            Key::Named(NamedKey::Enter)
                                if self
                                    .session
                                    .game()
                                    .is_some_and(Game::dialog_enter_is_newline) =>
                            {
                                changed = self
                                    .session
                                    .game_mut()
                                    .is_some_and(|g| g.type_dialog_char('\n'));
                            }
                            Key::Named(NamedKey::Enter) if !event.repeat => match &mut self.session
                            {
                                Session::Standalone { game, .. } => {
                                    standalone_out = Some(game.dialog_submit());
                                }
                                Session::Shell(shell) => {
                                    if let Some(game) = shell.game_mut() {
                                        let o = game.dialog_submit();
                                        shell_out = Some(shell.handle_lot_outcome(o));
                                    }
                                }
                            },
                            Key::Named(NamedKey::Escape) if !event.repeat => {
                                match &mut self.session {
                                    Session::Standalone { game, .. } => {
                                        standalone_out =
                                            Some(game.dialog_confirm_key(DialogKey::Escape));
                                    }
                                    Session::Shell(shell) => {
                                        shell_out = Some(shell.key_dialog(DialogKey::Escape));
                                    }
                                }
                            }
                            Key::Named(NamedKey::Space) => {
                                changed = self
                                    .session
                                    .game_mut()
                                    .is_some_and(|g| g.type_dialog_char(' '));
                            }
                            _ => {
                                if let Some(text) = &event.text
                                    && let Some(game) = self.session.game_mut()
                                {
                                    for ch in text.chars() {
                                        changed |= game.type_dialog_char(ch);
                                    }
                                }
                            }
                        }
                        if let Some(out) = standalone_out {
                            self.handle_standalone_outcome(event_loop, out);
                        } else if let Some(out) = shell_out {
                            self.handle_shell_outcome(event_loop, out);
                        } else {
                            self.changed(changed, true);
                        }
                    } else if !event.repeat {
                        let dialog_key = match &event.logical_key {
                            Key::Named(NamedKey::Enter) => Some(DialogKey::Enter),
                            Key::Named(NamedKey::Escape) => Some(DialogKey::Escape),
                            Key::Character(s) => s.chars().next().map(DialogKey::Char),
                            _ => event
                                .text
                                .as_ref()
                                .and_then(|t| t.chars().next())
                                .map(DialogKey::Char),
                        };
                        if let Some(key) = dialog_key {
                            match &mut self.session {
                                Session::Standalone { game, .. } => {
                                    let out = game.dialog_confirm_key(key);
                                    self.handle_standalone_outcome(event_loop, out);
                                }
                                Session::Shell(shell) => {
                                    let out = shell.key_dialog(key);
                                    self.handle_shell_outcome(event_loop, out);
                                }
                            }
                        }
                    }
                } else if event.logical_key == Key::Named(NamedKey::Escape) {
                    if !event.repeat {
                        match &mut self.session {
                            Session::Standalone { game, .. } => {
                                let closed = game.close_menu();
                                let cancelled = !closed && game.cancel_buy();
                                let escaped_options =
                                    !closed && !cancelled && game.escape_options();
                                if !escaped_options
                                    && escape_exits(closed, cancelled, game.buy_cursor().is_some())
                                {
                                    self.persist_options();
                                    event_loop.exit();
                                } else {
                                    self.changed(true, true);
                                }
                            }
                            Session::Shell(shell) => {
                                if let Some(game) = shell.game_mut() {
                                    let closed = game.close_menu();
                                    let cancelled = !closed && game.cancel_buy();
                                    let escaped_options =
                                        !closed && !cancelled && game.escape_options();
                                    if !escaped_options
                                        && escape_exits(
                                            closed,
                                            cancelled,
                                            game.buy_cursor().is_some(),
                                        )
                                    {
                                        self.persist_options();
                                        event_loop.exit();
                                    } else {
                                        self.changed(true, true);
                                    }
                                } else {
                                    let out = shell.escape();
                                    self.handle_shell_outcome(event_loop, out);
                                }
                            }
                        }
                    }
                } else {
                    self.key(&event.logical_key);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.session.has_dialog() {
                    return;
                }
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => f64::from(y),
                    MouseScrollDelta::PixelDelta(p) => p.y,
                };
                if let Some(game) = self.session.game_mut() {
                    let command = if y > 0.0 {
                        Move::ZoomIn
                    } else if y < 0.0 {
                        Move::ZoomOut
                    } else {
                        return;
                    };
                    let redraw = game.apply(command);
                    self.changed(redraw, true);
                } else if let Session::Shell(shell) = &mut self.session {
                    let redraw = shell.wheel(y);
                    self.changed(redraw, true);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => match (button, state) {
                (MouseButton::Left, ElementState::Pressed) => {
                    if let Some(point) = self.live_point() {
                        let now = Instant::now();
                        let is_double = self.last_left_press.is_some_and(|(prev_t, prev_p)| {
                            let dx = point[0] - prev_p[0];
                            let dy = point[1] - prev_p[1];
                            now.saturating_duration_since(prev_t) <= DOUBLE_CLICK_WINDOW
                                && dx * dx + dy * dy <= DOUBLE_CLICK_DISTANCE_SQ
                        });
                        self.last_left_press = if is_double { None } else { Some((now, point)) };
                        match &mut self.session {
                            Session::Standalone { game, .. } => {
                                let out = game.press(point);
                                self.handle_standalone_outcome(event_loop, out);
                            }
                            Session::Shell(shell) => {
                                let out = if is_double && shell.screen() == ScreenKind::FamilyBin {
                                    shell.double_click(point)
                                } else {
                                    shell.press(point)
                                };
                                self.handle_shell_outcome(event_loop, out);
                            }
                        }
                    }
                }
                (MouseButton::Left, ElementState::Released) => {
                    let dragging_slider = self.session.game().is_some_and(Game::dragging_slider);
                    let point = if dragging_slider {
                        self.slider_release_point()
                    } else {
                        self.live_point()
                    };
                    match &mut self.session {
                        Session::Standalone { game, .. } => {
                            let out = match point {
                                Some(pt) => game.release(pt),
                                None => game.release_outside(),
                            };
                            self.handle_standalone_outcome(event_loop, out);
                        }
                        Session::Shell(shell) => {
                            let out = match point {
                                Some(pt) => shell.release(pt),
                                None => shell.release_outside(),
                            };
                            self.handle_shell_outcome(event_loop, out);
                        }
                    }
                }
                (MouseButton::Right, ElementState::Pressed) => {
                    if self.session.game().is_some() && !self.session.has_dialog() {
                        self.drag = self.live_point().and(self.cursor);
                    }
                }
                (MouseButton::Right, ElementState::Released) => {
                    self.drag = None;
                    let pt = self.live_point();
                    let redraw = match (&mut self.session, pt) {
                        (Session::Standalone { game, .. }, Some(p)) => game.hover(p),
                        (Session::Standalone { game, .. }, None) => game.leave(),
                        (Session::Shell(shell), Some(p)) => shell.hover(p),
                        (Session::Shell(shell), None) => shell.leave(),
                    };
                    self.changed(redraw, true);
                }
                _ => {}
            },
            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                let redraw = match &mut self.session {
                    Session::Standalone { game, .. } => game.leave(),
                    Session::Shell(shell) => shell.leave(),
                };
                self.changed(redraw, true);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.last_cursor = Some(position);
                let (point, unclamped) = self
                    .window
                    .as_ref()
                    .map(|w| {
                        let size = w.inner_size();
                        if let Some(canvas) = canvas_size(size, self.scale) {
                            self.session.resize(canvas);
                        }
                        (
                            canvas_point(position, size, self.scale),
                            unclamped_canvas_point(position, size, self.scale),
                        )
                    })
                    .unwrap_or((None, None));
                if let Some(point) = point {
                    self.cursor = Some(position);
                    let redraw = self.drag.is_none()
                        && match &mut self.session {
                            Session::Standalone { game, .. } => game.hover(point),
                            Session::Shell(shell) => shell.hover(point),
                        };
                    self.changed(redraw, true);
                } else {
                    self.cursor = None;
                    let mut redraw = self.drag.is_none()
                        && match &mut self.session {
                            Session::Standalone { game, .. } => game.leave(),
                            Session::Shell(shell) => shell.leave(),
                        };
                    if self.drag.is_none()
                        && let Some(game) = self.session.game_mut()
                    {
                        if game.dragging_slider()
                            && let Some([x, _]) = unclamped
                        {
                            redraw |= game.drag_slider(x);
                        }
                        if game.dragging_dialog_entry()
                            && let Some(pt) = unclamped
                        {
                            redraw |= game.drag_dialog_entry(pt);
                        }
                    }
                    self.changed(redraw, true);
                }
                if self.session.has_dialog() || self.session.game().is_none() {
                    self.drag = None;
                }
                if let Some(start) = self.drag {
                    let scale = f64::from(self.scale.max(1));
                    let dx = ((position.x - start.x) / scale) as i32;
                    let dy = ((position.y - start.y) / scale) as i32;
                    if dx != 0 || dy != 0 {
                        self.drag = Some(PhysicalPosition::new(
                            start.x + f64::from(dx) * scale,
                            start.y + f64::from(dy) * scale,
                        ));
                        let redraw = self
                            .session
                            .game_mut()
                            .is_some_and(|g| g.apply(Move::Scroll([dx, dy])));
                        self.changed(redraw, true);
                    }
                }
            }
            _ => {}
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("sims-game: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let config = options()?;
    if let Some(house) = config.house {
        let game_options = Options {
            install: config.install.clone(),
            house: house.clone(),
            answers: config.answers,
            play: config.play,
            frames: sims_game::Frames::default(),
        };
        eprintln!("sims-game: booting {}", game_options.house.display());
        let started = Instant::now();
        let mut game = Game::open(&game_options)?;
        eprintln!(
            "sims-game: booted in {:.1} s",
            started.elapsed().as_secs_f32()
        );
        for problem in game.problems() {
            eprintln!("sims-game: {problem}");
        }
        if let Some(still) = config.still {
            let started = Instant::now();
            let ran = game.run(still.ticks);
            eprintln!(
                "sims-game: ran {ran} ticks in {:.1} s",
                started.elapsed().as_secs_f32()
            );
            for error in game.errors() {
                eprintln!("sims-game: error caught: {error}");
            }
            let sounds = game.take_sounds();
            if !sounds.is_empty() {
                eprintln!("sims-game: {} sound requests, not played", sounds.len());
            }
            if let Some(stop) = game.stopped() {
                eprintln!("sims-game: {stop}");
            }
            game.resize(still.size);
            let started = Instant::now();
            game.render();
            eprintln!(
                "sims-game: drew the frame in {} ms",
                started.elapsed().as_millis()
            );
            return std::fs::write(&still.file, png::encode(game.canvas()))
                .map_err(|e| format!("{}: {e}", still.file.display()));
        }
        let options_file = options_path(&config.saves);
        if options_file.is_file()
            && let Err(e) = game.load_options(&options_file)
        {
            eprintln!("sims-game: {}: {e}", options_file.display());
        }
        let saved_optn = game.options().to_optn();
        let sound = Sound::start(&config.install, config.mute);
        let mut app = App {
            session: Session::Standalone {
                game: Box::new(game),
                slot: slot(&house),
                options_file,
                saved_optn,
            },
            last_screen: ScreenKind::Lot,
            window: None,
            surface: None,
            surface_size: None,
            title_throttle: TitleThrottle::default(),
            redraw_pacer: RedrawPacer::new(None),
            scale: 1,
            cursor: None,
            last_cursor: None,
            drag: None,
            last_left_press: None,
            wake_clock: WakeClock::new(Instant::now()),
            dirty: true,
            photos: 0,
            reported: false,
            printed: 0,
            sound,
            sound_feed: sound::Feed::default(),
            focus_filter: FocusFilter::default(),
            saves: config.saves,
            ctrl_or_cmd_held: false,
            shift_held: false,
        };
        let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
        return event_loop.run_app(&mut app).map_err(|e| e.to_string());
    }

    eprintln!(
        "sims-game: opening Neighborhood shell ({})",
        config.install.display()
    );
    let started = Instant::now();
    let shell = NeighborhoodShell::open(
        config.install.clone(),
        config.saves.clone(),
        1,
        config.play,
        config.answers,
        sims_game::Frames::default(),
    )?;
    eprintln!(
        "sims-game: opened Neighborhood {} in {:.1} s",
        shell.neighborhood_slot(),
        started.elapsed().as_secs_f32()
    );
    let sound = Sound::start(&config.install, config.mute);
    let mut app = App {
        session: Session::Shell(Box::new(shell)),
        last_screen: ScreenKind::Neighborhood,
        window: None,
        surface: None,
        surface_size: None,
        title_throttle: TitleThrottle::default(),
        redraw_pacer: RedrawPacer::new(None),
        scale: 1,
        cursor: None,
        last_cursor: None,
        drag: None,
        last_left_press: None,
        wake_clock: WakeClock::new(Instant::now()),
        dirty: true,
        photos: 0,
        reported: false,
        printed: 0,
        sound,
        sound_feed: sound::Feed::default(),
        focus_filter: FocusFilter::default(),
        saves: config.saves,
        ctrl_or_cmd_held: false,
        shift_held: false,
    };
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.run_app(&mut app).map_err(|e| e.to_string())
}

/// The name of `house`'s save in the saves file: its full path, so houses
/// of the same number in two neighborhoods keep apart.
fn slot(house: &std::path::Path) -> String {
    let path = std::fs::canonicalize(house).unwrap_or_else(|_| house.to_path_buf());
    path.to_string_lossy().into_owned()
}

/// Whether a non-repeat Escape press should exit the desktop window.
/// Closing a pie menu, cancelling Buy Mode, or keeping an active Buy cursor
/// after a refused snapback all consume Escape instead of quitting.
fn escape_exits(closed_menu: bool, cancelled_buy: bool, has_buy_cursor: bool) -> bool {
    !closed_menu && !cancelled_buy && !has_buy_cursor
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use winit::dpi::PhysicalSize;

    #[test]
    fn canvas_point_matches_draw_clamp_on_odd_scaled_edges() {
        let size = PhysicalSize::new(801, 561);
        assert_eq!(canvas_size(size, 2), Some([400, 280]));
        assert_eq!(
            canvas_point(PhysicalPosition::new(800.0, 560.0), size, 2),
            Some([399, 279])
        );
        assert_eq!(
            canvas_point(PhysicalPosition::new(801.0, 560.0), size, 2),
            None
        );
        assert_eq!(
            canvas_point(PhysicalPosition::new(400.0, 561.0), size, 2),
            None
        );
        assert_eq!(
            canvas_point(PhysicalPosition::new(400.0, -1.0), size, 2),
            None
        );
    }

    #[test]
    fn canvas_point_rejects_cached_cursor_after_shrink() {
        let pos = PhysicalPosition::new(260.0, 204.0);
        assert_eq!(
            canvas_point(pos, PhysicalSize::new(800, 600), 1),
            Some([260, 204])
        );
        assert_eq!(canvas_point(pos, PhysicalSize::new(260, 600), 1), None);
        assert_eq!(canvas_point(pos, PhysicalSize::new(0, 600), 1), None);
    }

    #[test]
    fn unclamped_canvas_point_maps_out_of_window_slider_coordinates() {
        let size = PhysicalSize::new(800, 600);
        assert_eq!(
            unclamped_canvas_point(PhysicalPosition::new(-25.0, 520.0), size, 1),
            Some([-25, 520])
        );
        assert_eq!(
            unclamped_canvas_point(PhysicalPosition::new(1050.0, 520.0), size, 1),
            Some([1050, 520])
        );
        assert_eq!(
            unclamped_canvas_point(PhysicalPosition::new(-10.0, 520.0), size, 2),
            Some([-5, 260])
        );
    }

    #[test]
    fn focus_filter_drops_macos_startup_false_then_forwards_transitions() {
        let mut filter = FocusFilter::default();
        assert_eq!(filter.filter(false), None);
        assert_eq!(filter.filter(true), None);
        assert_eq!(filter.filter(false), Some(false));
        assert_eq!(filter.filter(true), Some(true));

        let mut direct = FocusFilter::default();
        assert_eq!(direct.filter(true), None);
        assert_eq!(direct.filter(false), Some(false));
    }

    fn blit_reference(
        canvas: &sims_render::Canvas,
        buffer: &mut [u32],
        bw: usize,
        bh: usize,
        scale: usize,
    ) {
        for y in 0..bh {
            let cy = (y / scale).min(canvas.height.saturating_sub(1));
            let row = &canvas.color[cy * canvas.width..(cy + 1) * canvas.width];
            for x in 0..bw {
                let cx = (x / scale).min(canvas.width.saturating_sub(1));
                let [r, g, b] = row.get(cx).copied().unwrap_or([0, 0, 0]);
                buffer[y * bw + x] = (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
            }
        }
    }

    #[test]
    fn blit_canvas_matches_reference_across_scales_and_odd_sizes() {
        let mut canvas = sims_render::Canvas::new(11, 7, [0, 0, 0]);
        for y in 0..canvas.height {
            for x in 0..canvas.width {
                canvas.color[y * canvas.width + x] = [
                    ((x * 23 + y * 7) & 0xff) as u8,
                    ((x * 11 + y * 31) & 0xff) as u8,
                    ((x * 5 + y * 19 + 42) & 0xff) as u8,
                ];
            }
        }
        let sizes = [
            (11, 7),
            (22, 14),
            (23, 15),
            (21, 13),
            (25, 17),
            (33, 21),
            (35, 22),
            (1, 1),
            (7, 5),
            (15, 10),
        ];
        for scale in [1usize, 2, 3, 4] {
            for (bw, bh) in sizes {
                let mut got = vec![0xDEAD_BEEF; bw * bh];
                let mut want = vec![0xDEAD_BEEF; bw * bh];
                blit_canvas(&canvas, &mut got, bw, bh, scale);
                blit_reference(&canvas, &mut want, bw, bh, scale);
                assert_eq!(got, want, "scale={scale} bw={bw} bh={bh}");
            }
        }
    }

    #[test]
    fn escape_does_not_exit_when_buy_snapback_is_refused() {
        assert!(!escape_exits(true, false, false));
        assert!(!escape_exits(false, true, false));
        assert!(!escape_exits(false, false, true));
        assert!(escape_exits(false, false, false));
    }

    #[test]
    fn within_step_budget_allows_first_tick_and_stops_at_cap() {
        let budget = Duration::from_millis(8);
        assert!(within_step_budget(0, Duration::ZERO, budget));
        assert!(within_step_budget(0, Duration::from_millis(20), budget));
        assert!(within_step_budget(1, Duration::from_millis(7), budget));
        assert!(within_step_budget(3, Duration::from_micros(7_999), budget));
        assert!(!within_step_budget(1, Duration::from_millis(8), budget));
        assert!(!within_step_budget(4, Duration::from_millis(12), budget));
    }

    #[test]
    fn wake_clock_preserves_presentation_time_across_capped_batches() {
        let t0 = Instant::now();
        let mut clock = WakeClock::new(t0);

        let w1_start = t0 + Duration::from_millis(16);
        let w1_end = w1_start + Duration::from_millis(2);
        assert_eq!(clock.elapsed_us(w1_start), (16_000, 16_000));
        clock.finish(w1_start, w1_end, false);

        let w2_start = w1_start + Duration::from_millis(16);
        let w2_end = w2_start + Duration::from_millis(9);
        assert_eq!(clock.elapsed_us(w2_start), (16_000, 16_000));
        clock.finish(w2_start, w2_end, true);

        let w3_start = w2_end + Duration::from_millis(5);
        assert_eq!(clock.elapsed_us(w3_start), (14_000, 5_000));
    }

    #[test]
    fn title_throttle_rate_limits_running_and_flushes_state_transitions() {
        let mut throttle = TitleThrottle::default();
        let interval = Duration::from_millis(250);
        let t0 = Instant::now();

        assert_eq!(
            throttle.update(0, TitleState::Running, t0, interval),
            Some("sims-game: tick 0")
        );
        assert_eq!(
            throttle.update(
                5,
                TitleState::Running,
                t0 + Duration::from_millis(100),
                interval
            ),
            None
        );
        assert_eq!(
            throttle.update(
                9,
                TitleState::Running,
                t0 + Duration::from_millis(249),
                interval
            ),
            None
        );
        assert_eq!(
            throttle.update(
                10,
                TitleState::Running,
                t0 + Duration::from_millis(250),
                interval
            ),
            Some("sims-game: tick 10")
        );
        assert_eq!(
            throttle.update(
                10,
                TitleState::Running,
                t0 + Duration::from_millis(600),
                interval
            ),
            None
        );
        assert_eq!(
            throttle.update(
                11,
                TitleState::Running,
                t0 + Duration::from_millis(610),
                interval
            ),
            Some("sims-game: tick 11")
        );
        assert_eq!(
            throttle.update(
                11,
                TitleState::Paused,
                t0 + Duration::from_millis(611),
                interval
            ),
            Some("sims-game: tick 11, paused")
        );
        assert_eq!(
            throttle.update(
                12,
                TitleState::Paused,
                t0 + Duration::from_millis(650),
                interval
            ),
            None
        );
        assert_eq!(
            throttle.update(
                12,
                TitleState::Running,
                t0 + Duration::from_millis(660),
                interval
            ),
            Some("sims-game: tick 12")
        );
        assert_eq!(
            throttle.update(
                12,
                TitleState::Stopped,
                t0 + Duration::from_millis(661),
                interval
            ),
            Some("sims-game: tick 12, stopped")
        );
        assert_eq!(
            throttle.update(
                12,
                TitleState::Stopped,
                t0 + Duration::from_millis(1000),
                interval
            ),
            None
        );
    }

    #[test]
    fn refresh_interval_handles_60hz_120hz_fallback_and_out_of_range() {
        assert_eq!(
            refresh_interval_from_millihertz(None),
            Duration::from_nanos(16_666_666)
        );
        assert_eq!(
            refresh_interval_from_millihertz(Some(60_000)),
            Duration::from_nanos(16_666_666)
        );
        assert_eq!(
            refresh_interval_from_millihertz(Some(120_000)),
            Duration::from_nanos(8_333_333)
        );
        // Zero / bogus OS values fall back safely to 60 Hz.
        assert_eq!(
            refresh_interval_from_millihertz(Some(0)),
            Duration::from_nanos(16_666_666)
        );
        assert_eq!(
            refresh_interval_from_millihertz(Some(5_000_000)),
            Duration::from_nanos(16_666_666)
        );
    }

    #[test]
    fn redraw_pacer_caps_passive_to_60fps_and_unlocks_display_refresh_on_input() {
        let t0 = Instant::now();
        // 120 Hz ProMotion monitor.
        let mut pacer = RedrawPacer::new(Some(120_000));
        assert_eq!(pacer.display_interval(), Duration::from_nanos(8_333_333));

        // First dirty frame always presents immediately.
        assert!(pacer.should_request_redraw(t0, true, false));
        // Duplicate request while redraw is already in flight is suppressed.
        assert!(!pacer.should_request_redraw(t0 + Duration::from_millis(1), true, false));
        pacer.on_present(t0);

        // Passive sim-only tick at +9 ms is suppressed (60 FPS passive cap = 16.67 ms).
        let t_9ms = t0 + Duration::from_millis(9);
        assert!(!pacer.should_request_redraw(t_9ms, true, false));
        assert_eq!(
            pacer.next_redraw_deadline(t_9ms, true, false),
            Some(t0 + PASSIVE_FPS_INTERVAL)
        );
        // Passive sim-only tick at +17 ms is allowed.
        let t_17ms = t0 + Duration::from_millis(17);
        assert!(pacer.should_request_redraw(t_17ms, true, false));
        pacer.on_present(t_17ms);

        // User interaction switches immediately to 120 Hz (8.33 ms interval).
        let t_interact = t_17ms + Duration::from_millis(9);
        pacer.note_interaction(t_interact);
        assert!(pacer.should_request_redraw(t_interact, true, false));
        pacer.on_present(t_interact);

        // Still inside 250 ms hold: 5 ms is throttled, 9 ms is allowed.
        assert!(!pacer.should_request_redraw(t_interact + Duration::from_millis(5), true, false));
        let t_next = t_interact + Duration::from_millis(9);
        assert!(pacer.should_request_redraw(t_next, true, false));
        pacer.on_present(t_next);

        // After 250 ms hold expires without continuous interaction, returns to 60 FPS cap.
        let t_quiet = t_interact + INTERACTIVE_HOLD + Duration::from_millis(1);
        pacer.on_present(t_quiet);
        assert!(!pacer.should_request_redraw(t_quiet + Duration::from_millis(10), true, false));
        // Continuous interaction (e.g. Buy/Build cursor or drag) keeps 120 Hz active.
        assert!(pacer.should_request_redraw(t_quiet + Duration::from_millis(10), true, true));
    }

    #[test]
    fn redraw_pacer_respects_low_refresh_display_and_monitor_changes() {
        let t0 = Instant::now();
        // 30 Hz display: even passive mode never exceeds 30 Hz (33.3 ms).
        let mut pacer = RedrawPacer::new(Some(30_000));
        assert!(pacer.should_request_redraw(t0, true, false));
        pacer.on_present(t0);
        assert!(!pacer.should_request_redraw(t0 + Duration::from_millis(20), true, false));
        assert!(pacer.should_request_redraw(t0 + Duration::from_millis(34), true, false));
        pacer.on_present(t0 + Duration::from_millis(34));

        // Moving window to a 120 Hz monitor updates display_interval cleanly.
        pacer.set_refresh_millihertz(Some(120_000));
        assert_eq!(pacer.display_interval(), Duration::from_nanos(8_333_333));
        pacer.request_immediate();
        assert!(pacer.should_request_redraw(t0 + Duration::from_millis(35), true, false));
    }

    #[test]
    fn redraw_pacer_recovers_after_skipped_or_failed_redraw() {
        let t0 = Instant::now();
        let mut pacer = RedrawPacer::new(Some(60_000));

        // Request initial redraw; duplicate request while in flight is suppressed.
        assert!(pacer.should_request_redraw(t0, true, false));
        assert!(!pacer.should_request_redraw(t0 + Duration::from_millis(1), true, false));
        assert_eq!(pacer.next_redraw_deadline(t0, true, false), None);

        // Redraw event is consumed, then skipped (e.g. minimized / zero size):
        // once resized back to drawable, a fresh request succeeds immediately.
        pacer.on_redraw_consumed();
        assert!(pacer.should_request_redraw(t0 + Duration::from_millis(2), true, false));
        pacer.on_present(t0 + Duration::from_millis(2));

        // A surface resize / buffer / present failure clears `redraw_requested`
        // AND paces retries separately so failed attempts cannot busy-spin.
        let t_fail = t0 + Duration::from_millis(20);
        assert!(pacer.should_request_redraw(t_fail, true, false));
        pacer.on_redraw_consumed();
        pacer.on_redraw_failed(t_fail);
        assert!(!pacer.should_request_redraw(t_fail + Duration::from_millis(1), true, false));
        assert_eq!(
            pacer.next_redraw_deadline(t_fail + Duration::from_millis(1), true, false),
            Some(t_fail + PASSIVE_FPS_INTERVAL)
        );
        // Once backoff elapses (or resize calls `request_immediate`), retry succeeds.
        assert!(pacer.should_request_redraw(t_fail + Duration::from_millis(17), true, false));
        pacer.on_present(t_fail + Duration::from_millis(17));
        // Even while in failure backoff, `request_immediate` (e.g. on Resize) wakes immediately.
        let t_fail2 = t_fail + Duration::from_millis(40);
        assert!(pacer.should_request_redraw(t_fail2, true, false));
        pacer.on_redraw_failed(t_fail2);
        pacer.request_immediate();
        assert!(pacer.should_request_redraw(t_fail2 + Duration::from_millis(1), true, false));
    }

    #[test]
    fn cli_defaults_to_neighborhood_while_keeping_house_and_png_backwards_compatible() {
        let env = Some(PathBuf::from("/sims"));

        // Default with no house arg -> Neighborhood shell.
        let cfg = parse_launch_args(Vec::<OsString>::new(), env.clone()).unwrap();
        assert_eq!(cfg.install, PathBuf::from("/sims"));
        assert!(cfg.house.is_none());
        assert!(cfg.still.is_none());

        // Explicit --neighborhood -> Neighborhood shell.
        let cfg = parse_launch_args([OsString::from("--neighborhood")], env.clone()).unwrap();
        assert!(cfg.house.is_none());

        // Explicit <install> <house> -> direct house launch.
        let cfg = parse_launch_args(
            [
                OsString::from("/sims"),
                OsString::from("/sims/UserData/Houses/House05.iff"),
                OsString::from("--play"),
            ],
            None,
        )
        .unwrap();
        assert_eq!(
            cfg.house,
            Some(PathBuf::from("/sims/UserData/Houses/House05.iff"))
        );
        assert!(cfg.play);

        // --png without house -> defaults to Goth house with answers = [3].
        let cfg = parse_launch_args(
            [OsString::from("--png"), OsString::from("out.png")],
            env.clone(),
        )
        .unwrap();
        assert_eq!(cfg.house, Some(goth_house(std::path::Path::new("/sims"))));
        assert_eq!(cfg.answers, vec![3]);
        assert!(cfg.still.is_some());
    }
}
