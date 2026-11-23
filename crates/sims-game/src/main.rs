//! The game's window.
//!
//! ```text
//! sims-game [<install>] [<house file>] [--neighborhood] [--answers A,B,...] [--mute]
//!           [--play | --strict] [--debug] [--saves <file>] [--png <file> [--ticks N] [--size WxH]]
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

use sims_content::Install;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_game::game::goth_house;
use sims_game::hud::DialogKey;
use sims_game::neighborhood::{LoadingPresenter, NeighborhoodShell, ScreenKind, ShellOutcome};
use sims_game::sound::{self, Input};
use sims_game::view::Move;
use sims_game::{Game, Options};
use sims_render::{Canvas, png};
use sims_ui::art::Art;
use sims_ui::command::{Mode, Speed};
use sims_ui::frame::Frame;
use sims_ui::loading::{LoadingKind, LoadingScreen, LoadingState, LoadingStrings};
use softbuffer::{Context, Surface};
use std::cell::{Cell, RefCell};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use winit::window::{Window, WindowId};

/// Splash animation and worker-completion poll interval while `StartupLoading`
/// is on screen (~60 FPS). Once startup completes, `about_to_wait` uses exact
/// next-event deadlines (`ControlFlow::Wait` / `ControlFlow::WaitUntil`).
const STARTUP_POLL: Duration = Duration::from_millis(16);

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
    fn empty() -> Self {
        Self(None)
    }

    fn from_mixer(mixer: Result<sims_game::sound::Mixer, String>) -> Self {
        let started = mixer.and_then(sims_game::output::Output::start);
        match started {
            Ok(output) => Self(Some(output)),
            Err(e) => {
                eprintln!("sims-game: playing without sound: {e}");
                Self(None)
            }
        }
    }

    fn start(install: &std::path::Path, mute: bool) -> Self {
        if mute {
            return Self(None);
        }
        Self::from_mixer(sims_game::sound::Mixer::open(install))
    }

    fn start_open_pack(
        wavs: Option<&std::collections::BTreeMap<String, Vec<u8>>>,
        mute: bool,
    ) -> Self {
        if mute {
            return Self(None);
        }
        let empty = std::collections::BTreeMap::new();
        Self::from_mixer(Ok(sims_game::sound::Mixer::open_pack(
            wavs.unwrap_or(&empty),
        )))
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
    fn empty() -> Self {
        Self
    }

    fn start(_: &std::path::Path, mute: bool) -> Self {
        if !mute {
            eprintln!("sims-game: built without sound (the `sound` feature)");
        }
        Self
    }

    fn start_open_pack(
        _: Option<&std::collections::BTreeMap<String, Vec<u8>>>,
        mute: bool,
    ) -> Self {
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

/// Pending CLI co-op action applied once startup completes.
#[derive(Debug, Clone, Default)]
struct CoopCliConfig {
    host: bool,
    join: Option<String>,
    player_name: Option<String>,
    loopback: bool,
}

/// Parsed CLI launch configuration.
struct LaunchConfig {
    install: PathBuf,
    /// `Some(pack_dir)` when launching in zero-commercial-asset standalone
    /// `open-pack/` mode (via `--open-pack [dir]` or automatic fallback when
    /// neither `<install>` nor `SIMS_INSTALL` is provided).
    open_pack: Option<PathBuf>,
    /// `Some(house)` for direct house launch or `--png`; `None` for the
    /// Neighborhood screen shell.
    house: Option<PathBuf>,
    answers: Vec<u8>,
    play: bool,
    debug: bool,
    still: Option<Still>,
    mute: bool,
    saves: PathBuf,
    coop: CoopCliConfig,
}

fn parse_launch_args<I>(args: I, env_install: Option<PathBuf>) -> Result<LaunchConfig, String>
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let usage = "usage: sims-game [--open-pack [<dir>]] [<install>] [<house file>] [--neighborhood] \
                 [--answers A,B,...] [--mute] [--play | --strict] [--debug] [--saves <file>] \
                 [--host | --join <ticket>] [--player-name <name>] [--coop-loopback] \
                 [--png <file> [--ticks N] [--size WxH]]";
    let mut paths = Vec::new();
    let mut answers = None;
    let mut png = None;
    let mut ticks = 0;
    let mut size = [800, 600];
    let mut mute = false;
    let mut play_flag = false;
    let mut strict_flag = false;
    let mut debug = false;
    let mut neighborhood_flag = false;
    let mut open_pack_flag = false;
    let mut open_pack_explicit: Option<PathBuf> = None;
    let mut saves = PathBuf::from("sims-game-saves.redb");
    let mut coop = CoopCliConfig::default();
    let mut args = args.into_iter().peekable();
    while let Some(arg) = args.next() {
        let mut text = |name: &str| -> Result<String, String> {
            args.next()
                .and_then(|a| a.into_string().ok())
                .ok_or_else(|| format!("{name} needs a value"))
        };
        if arg == "--mute" {
            mute = true;
        } else if arg == "--play" {
            play_flag = true;
        } else if arg == "--strict" {
            strict_flag = true;
        } else if arg == "--debug" {
            debug = true;
        } else if arg == "--neighborhood" {
            neighborhood_flag = true;
        } else if arg == "--open-pack" {
            open_pack_flag = true;
            if let Some(next) = args.peek()
                && !next.to_str().is_some_and(|s| s.starts_with("--"))
            {
                open_pack_explicit = Some(PathBuf::from(args.next().expect("peeked")));
            }
        } else if let Some(val) = arg.to_str().and_then(|s| s.strip_prefix("--open-pack=")) {
            if val.trim().is_empty() {
                return Err("--open-pack= needs a directory path".into());
            }
            open_pack_flag = true;
            open_pack_explicit = Some(PathBuf::from(val));
        } else if arg == "--host" {
            coop.host = true;
        } else if arg == "--join" {
            let ticket = text("--join")?;
            let trimmed = ticket.trim().to_owned();
            sims_net::InviteTicket::decode_string(&trimmed)
                .map_err(|e| format!("--join invalid invite ticket: {e}"))?;
            coop.join = Some(trimmed);
        } else if arg == "--player-name" {
            let name = text("--player-name")?;
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err("--player-name cannot be empty".into());
            }
            coop.player_name = Some(trimmed.to_owned());
        } else if arg == "--coop-loopback" {
            coop.loopback = true;
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
    if play_flag && strict_flag {
        return Err("--play and --strict cannot be combined".into());
    }
    if coop.host && coop.join.is_some() {
        return Err("--host and --join cannot be combined".into());
    }
    if png.is_some() && (coop.host || coop.join.is_some()) {
        return Err("--png cannot be combined with --host or --join".into());
    }
    // Interactive desktop play defaults to graceful play mode (`play: true`)
    // unless `--strict` is explicitly requested; headless `--png` captures keep
    // strict mode by default unless `--play` is explicitly given.
    let play = if strict_flag {
        false
    } else if play_flag {
        true
    } else {
        png.is_none()
    };

    if open_pack_flag {
        if !paths.is_empty() {
            return Err("--open-pack cannot be combined with an install or house path".into());
        }
        if neighborhood_flag {
            return Err("--open-pack cannot be combined with --neighborhood".into());
        }
        if coop.host || coop.join.is_some() {
            return Err("--open-pack cannot be combined with --host or --join".into());
        }
        let pack_dir = match open_pack_explicit {
            Some(dir) => dir,
            None => sims_game::open_pack::discover_open_pack_dir().ok_or_else(|| {
                "could not locate standalone open-pack/ directory (pass --open-pack <dir>)"
                    .to_owned()
            })?,
        };
        let still = png.map(|file| Still { file, ticks, size });
        return Ok(LaunchConfig {
            install: pack_dir.clone(),
            open_pack: Some(pack_dir),
            house: None,
            answers: answers.unwrap_or_default(),
            play,
            debug,
            still,
            mute,
            saves,
            coop,
        });
    }

    let mut paths = paths.into_iter();
    let install = match paths.next() {
        Some(path) => path,
        None => match env_install {
            Some(env_path) => env_path,
            None => {
                // Automatic zero-commercial-asset standalone fallback when neither
                // `<install>` nor `SIMS_INSTALL` is provided.
                if neighborhood_flag {
                    return Err(
                        "--neighborhood requires a Complete Collection install or SIMS_INSTALL"
                            .into(),
                    );
                }
                if coop.host || coop.join.is_some() {
                    return Err(
                        "--host/--join requires a Complete Collection install or SIMS_INSTALL"
                            .into(),
                    );
                }
                let pack_dir = sims_game::open_pack::discover_open_pack_dir().ok_or_else(|| {
                    "give the install, set SIMS_INSTALL, or run with --open-pack <dir>".to_owned()
                })?;
                let still = png.map(|file| Still { file, ticks, size });
                return Ok(LaunchConfig {
                    install: pack_dir.clone(),
                    open_pack: Some(pack_dir),
                    house: None,
                    answers: answers.unwrap_or_default(),
                    play,
                    debug,
                    still,
                    mute,
                    saves,
                    coop,
                });
            }
        },
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
        open_pack: None,
        house,
        answers,
        play,
        debug,
        still,
        mute,
        saves,
        coop,
    })
}

fn options() -> Result<LaunchConfig, String> {
    parse_launch_args(
        std::env::args_os().skip(1),
        std::env::var_os("SIMS_INSTALL").map(PathBuf::from),
    )
}

type StartupResult = Result<
    (
        NeighborhoodShell,
        Option<Result<sims_game::sound::Mixer, String>>,
    ),
    String,
>;

/// Native startup loading screen state shown while `NeighborhoodShell::open`
/// and `Mixer::open` run in the background.
struct StartupLoading {
    loading_screen: LoadingScreen,
    loading_art: Art,
    loading_strings: LoadingStrings,
    canvas: Canvas,
    size: [i32; 2],
    started: Instant,
    last_tick: Instant,
    rx: mpsc::Receiver<StartupResult>,
    install: PathBuf,
    mute: bool,
}

impl StartupLoading {
    fn new(install: PathBuf, mute: bool, rx: mpsc::Receiver<StartupResult>) -> Self {
        let mut loading_screen = LoadingScreen::new();
        let _ = loading_screen.begin(1);
        let (loading_art, loading_strings) = match Install::detect(&install) {
            Ok(inst) => {
                let art = UiGraphics::open(&inst)
                    .ok()
                    .and_then(|ui| sims_ui::loading::load_art(&ui, US_ENGLISH).ok())
                    .unwrap_or_default();
                let strings = LoadingStrings::load(&inst, US_ENGLISH).unwrap_or_default();
                (art, strings)
            }
            Err(_) => (Art::default(), LoadingStrings::default()),
        };
        let now = Instant::now();
        Self {
            loading_screen,
            loading_art,
            loading_strings,
            canvas: Canvas::new(800, 600, [0, 0, 0]),
            size: [800, 600],
            started: now,
            last_tick: now,
            rx,
            install,
            mute,
        }
    }

    fn resize(&mut self, size: [i32; 2]) -> bool {
        let size = [size[0].max(1), size[1].max(1)];
        if self.size == size {
            return false;
        }
        self.size = size;
        self.canvas = Canvas::new(size[0] as usize, size[1] as usize, [0, 0, 0]);
        true
    }

    fn tick(&mut self, now: Instant) {
        let dt_ms = now.saturating_duration_since(self.last_tick).as_millis() as u64;
        self.last_tick = now;
        self.loading_screen.tick(dt_ms);
    }

    fn render(&mut self) {
        let w = self.size[0].max(1) as u32;
        let h = self.size[1].max(1) as u32;
        let elapsed_s = self.started.elapsed().as_secs_f32();
        let progress = (0.1 + (elapsed_s / 6.0) * 0.85).clamp(0.1, 0.95);
        let state = LoadingState {
            kind: LoadingKind::Startup,
            progress,
            destination_label: None,
            caption_override: None,
        };
        let mut frame = match Frame::from_packed(w, h, std::mem::take(&mut self.canvas.color)) {
            Ok(mut frame) => {
                frame.fill(
                    sims_ui::frame::Rect::new(0, 0, w as i32, h as i32),
                    [0, 0, 0],
                );
                frame
            }
            Err(_) => Frame::new(w, h, [0, 0, 0]),
        };
        self.loading_screen
            .draw(&mut frame, &state, &self.loading_art, &self.loading_strings);
        self.canvas.width = w as usize;
        self.canvas.height = h as usize;
        self.canvas.color = frame.into_packed();
    }
}

type LocalLoadingPresenter = Box<dyn FnMut(&[u32], [i32; 2], Vec<Input>)>;

thread_local! {
    static MAIN_THREAD_LOADING_PRESENTER: RefCell<Option<LocalLoadingPresenter>> =
        RefCell::new(None);
}

/// Active desktop session: startup loading splash, standalone explicit house
/// launch, or the full Neighborhood ↔ Family Bin ↔ Lot shell.
enum Session {
    Starting(Box<StartupLoading>),
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
            Self::Starting(_) => None,
            Self::Standalone { game, .. } => Some(game),
            Self::Shell(shell) => shell.game(),
        }
    }

    fn game_mut(&mut self) -> Option<&mut Game> {
        match self {
            Self::Starting(_) => None,
            Self::Standalone { game, .. } => Some(game),
            Self::Shell(shell) => shell.game_mut(),
        }
    }

    fn screen_kind(&self) -> ScreenKind {
        match self {
            Self::Starting(_) => ScreenKind::Loading,
            Self::Standalone { .. } => ScreenKind::Lot,
            Self::Shell(shell) => shell.screen(),
        }
    }

    fn has_dialog(&self) -> bool {
        match self {
            Self::Starting(_) => false,
            Self::Standalone { game, .. } => game.has_dialog(),
            Self::Shell(shell) => shell.has_dialog(),
        }
    }

    fn coop_lobby_open(&self) -> bool {
        match self {
            Self::Starting(_) => false,
            Self::Standalone { game, .. } => game.coop_lobby_open(),
            Self::Shell(shell) => shell.coop_lobby_open(),
        }
    }

    fn is_actively_interacting(&self) -> bool {
        match self {
            Self::Starting(_) => false,
            Self::Standalone { game, .. } => game.is_actively_interacting(),
            Self::Shell(shell) => shell.is_actively_interacting(),
        }
    }

    fn resize(&mut self, size: [i32; 2]) -> bool {
        match self {
            Self::Starting(start) => start.resize(size),
            Self::Standalone { game, .. } => game.resize(size),
            Self::Shell(shell) => shell.resize(size),
        }
    }

    fn render(&mut self) {
        match self {
            Self::Starting(start) => start.render(),
            Self::Standalone { game, .. } => {
                let _ = game.render();
            }
            Self::Shell(shell) => shell.render(),
        }
    }

    fn canvas(&self) -> &sims_render::Canvas {
        match self {
            Self::Starting(start) => &start.canvas,
            Self::Standalone { game, .. } => game.canvas(),
            Self::Shell(shell) => shell.canvas(),
        }
    }

    fn peek_present_damage(&self) -> Option<&[sims_render::canvas::Rect]> {
        match self {
            Self::Starting(_) => None,
            Self::Standalone { game, .. } => game.peek_present_damage(),
            Self::Shell(shell) => shell.peek_present_damage(),
        }
    }

    fn on_presented(&mut self) {
        match self {
            Self::Starting(_) => {}
            Self::Standalone { game, .. } => game.on_presented(),
            Self::Shell(shell) => shell.on_presented(),
        }
    }

    fn persist_options(&mut self) {
        match self {
            Self::Starting(_) => {}
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

type SharedSurface = Rc<RefCell<Option<Surface<Rc<Window>, Rc<Window>>>>>;
type SharedSurfaceSize = Rc<Cell<Option<(NonZeroU32, NonZeroU32)>>>;
type PresentSig = (usize, usize, usize, usize, usize, ScreenKind);
type SharedPresentSig = Rc<Cell<Option<PresentSig>>>;

struct App {
    session: Session,
    last_screen: ScreenKind,
    last_lot_generation: (u64, u64),
    window: Option<Rc<Window>>,
    surface: SharedSurface,
    surface_size: SharedSurfaceSize,
    last_present_sig: SharedPresentSig,
    title_throttle: TitleThrottle,
    redraw_pacer: RedrawPacer,
    scale: Rc<Cell<u32>>,
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
    sound: Rc<RefCell<Sound>>,
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
    /// Fatal background startup error propagated as a nonzero exit status from `run()`.
    startup_error: Option<String>,
    /// Deferred CLI co-op action applied once the Neighborhood shell finishes loading.
    pending_coop: CoopCliConfig,
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

/// Default initial logical window size (`1024×768`).
const INITIAL_WINDOW_SIZE: [u32; 2] = [1024, 768];

/// Minimum logical window size (`800×600`), matching The Sims 1's smallest
/// native resolution so the neighborhood map and top bar never crush below
/// native dimensions.
const MIN_WINDOW_SIZE: [u32; 2] = [800, 600];

/// Builds the desktop window attributes with initial `1024×768` size and a
/// `800×600` minimum logical inner size.
fn window_attributes() -> winit::window::WindowAttributes {
    Window::default_attributes()
        .with_title("sims-game")
        .with_inner_size(LogicalSize::new(
            f64::from(INITIAL_WINDOW_SIZE[0]),
            f64::from(INITIAL_WINDOW_SIZE[1]),
        ))
        .with_min_inner_size(LogicalSize::new(
            f64::from(MIN_WINDOW_SIZE[0]),
            f64::from(MIN_WINDOW_SIZE[1]),
        ))
}

/// Converts a display `scale_factor` into an integer pixel-doubling scale.
///
/// Uses floor (with a tiny epsilon for floating-point noise like `1.99999`)
/// rather than round so fractional HiDPI scales (e.g. `1.5×` or `1.75×`)
/// never divide an `800×600` logical minimum window down below `800×600`
/// canvas pixels (`1200 / 2 = 600 < 800`), while keeping integer Retina
/// scales (`1.0 → 1`, `2.0 → 2`, `3.0 → 3`) exact.
fn window_scale(scale_factor: f64) -> u32 {
    if !scale_factor.is_finite() || scale_factor <= 1.0 {
        return 1;
    }
    ((scale_factor + 1e-4).floor().clamp(1.0, 16.0)) as u32
}

/// The canvas dimensions for a physical client area `size` at `scale`,
/// matching [`App::redraw`].
fn canvas_size(size: winit::dpi::PhysicalSize<u32>, scale: u32) -> Option<[i32; 2]> {
    if size.width == 0 || size.height == 0 {
        return None;
    }
    let scale = scale.max(1);
    let snap_min = |phys: u32, min_logical: u32| -> i32 {
        let floored = phys / scale;
        let dim = if scale > 1 && floored < min_logical && phys.div_ceil(scale) == min_logical {
            min_logical
        } else {
            floored
        };
        i32::try_from(dim).unwrap_or(i32::MAX).max(1)
    };
    Some([
        snap_min(size.width, MIN_WINDOW_SIZE[0]),
        snap_min(size.height, MIN_WINDOW_SIZE[1]),
    ])
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
            .is_some_and(|w| canvas_size(w.inner_size(), self.scale.get().max(1)).is_some())
    }

    fn send_sound(&self, input: Input) {
        self.sound.borrow().send(input);
    }

    fn build_loading_presenter(&self) -> Option<LoadingPresenter> {
        let window = self.window.clone()?;
        let surface = Rc::clone(&self.surface);
        let surface_size = Rc::clone(&self.surface_size);
        let last_present_sig = Rc::clone(&self.last_present_sig);
        let scale_cell = Rc::clone(&self.scale);
        let sound = Rc::clone(&self.sound);
        MAIN_THREAD_LOADING_PRESENTER.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(
                move |pixels: &[u32], size: [i32; 2], audio: Vec<Input>| {
                    last_present_sig.set(None);
                    {
                        let snd = sound.borrow();
                        for input in audio {
                            snd.send(input);
                        }
                    }
                    let win_size = window.inner_size();
                    let (Some(w), Some(h)) = (
                        NonZeroU32::new(win_size.width),
                        NonZeroU32::new(win_size.height),
                    ) else {
                        return;
                    };
                    let mut surf_guard = surface.borrow_mut();
                    let Some(surf) = surf_guard.as_mut() else {
                        return;
                    };
                    if surface_size.get() != Some((w, h)) {
                        if surf.resize(w, h).is_err() {
                            surface_size.set(None);
                            return;
                        }
                        surface_size.set(Some((w, h)));
                    }
                    let Ok(mut buffer) = surf.buffer_mut() else {
                        surface_size.set(None);
                        return;
                    };
                    let cw = usize::try_from(size[0]).unwrap_or(0);
                    let ch = usize::try_from(size[1]).unwrap_or(0);
                    let mut tmp_canvas = Canvas::new(cw, ch, [0, 0, 0]);
                    tmp_canvas.color = pixels.to_vec();
                    let (bw, bh) = (win_size.width as usize, win_size.height as usize);
                    let scale = scale_cell.get().max(1) as usize;
                    blit_canvas(&tmp_canvas, &mut buffer, bw, bh, scale);
                    if buffer.present().is_err() {
                        surface_size.set(None);
                    }
                },
            ));
        });
        Some(Box::new(|pixels, size, audio| {
            MAIN_THREAD_LOADING_PRESENTER.with(|slot| {
                if let Some(cb) = slot.borrow_mut().as_mut() {
                    cb(pixels, size, audio);
                }
            });
        }))
    }

    fn attach_loading_presenter(&mut self) {
        let presenter = self.build_loading_presenter();
        if let Session::Shell(shell) = &mut self.session {
            shell.set_loading_presenter(presenter);
        }
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

    fn current_lot_generation(&self) -> (u64, u64) {
        match &self.session {
            Session::Starting(_) => (0, 0),
            Session::Standalone { game, .. } => (0, game.restore_generation()),
            Session::Shell(shell) => (
                shell.lot_generation(),
                shell.game().map_or(0, Game::restore_generation),
            ),
        }
    }

    fn drain_shell_audio(&mut self) {
        let screen_now = self.session.screen_kind();
        let lot_gen = self.current_lot_generation();
        if screen_now != self.last_screen || lot_gen != self.last_lot_generation {
            let was_lot = self.last_screen == ScreenKind::Lot;
            self.last_screen = screen_now;
            self.last_lot_generation = lot_gen;
            if was_lot && screen_now == ScreenKind::Lot {
                self.send_sound(sound::Input::Events(vec![sims_audio::SoundEvent::StopAll]));
            }
            self.sound_feed.reset_for_lot();
            self.drag = None;
            self.printed = 0;
            self.reported = false;
            self.wake_clock.reset(Instant::now());
            self.redraw_pacer.request_immediate();
        }
        if let Session::Shell(shell) = &mut self.session {
            let inputs = shell.take_audio_inputs();
            for input in inputs {
                self.send_sound(input);
            }
        }
    }

    fn persist_options(&mut self) {
        self.session.persist_options();
    }

    fn shutdown_session(&mut self) {
        self.persist_options();
        match &mut self.session {
            Session::Starting(_) => {}
            Session::Standalone { game, .. } => {
                if game.coop_active() {
                    game.leave_coop();
                }
            }
            Session::Shell(shell) => {
                shell.leave_coop_if_active();
                shell.flush_background_saves();
            }
        }
    }

    fn slider_release_point(&mut self) -> Option<[i32; 2]> {
        let scale = self.scale.get();
        #[cfg(target_os = "macos")]
        if let Some(window) = self.window.as_ref()
            && let Some(point) = live_macos_cursor_point(window, scale)
        {
            return Some(point);
        }
        if let Some(point) = self.live_point() {
            return Some(point);
        }
        self.window
            .as_ref()
            .and_then(|w| unclamped_canvas_point(self.last_cursor?, w.inner_size(), scale))
    }

    fn sync_size(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        let scale = self.scale.get();
        let resized = canvas_size(size, scale).is_some_and(|canvas| self.session.resize(canvas));
        if let Some(cursor) = self.cursor {
            match canvas_point(cursor, size, scale) {
                None => {
                    self.cursor = None;
                    let _ = match &mut self.session {
                        Session::Starting(_) => false,
                        Session::Standalone { game, .. } => game.leave(),
                        Session::Shell(shell) => shell.leave(),
                    };
                }
                Some(point) if resized && self.drag.is_none() => {
                    let _ = match &mut self.session {
                        Session::Starting(_) => false,
                        Session::Standalone { game, .. } => game.hover(point),
                        Session::Shell(shell) => shell.hover(point),
                    };
                }
                Some(_) => {}
            }
        }
    }

    fn live_point(&mut self) -> Option<[i32; 2]> {
        let scale = self.scale.get();
        let size = self.window.as_ref()?.inner_size();
        if let Some(canvas) = canvas_size(size, scale) {
            self.session.resize(canvas);
        }
        let cursor = self.cursor?;
        let point = canvas_point(cursor, size, scale);
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
        let scale = f64::from(self.scale.get().max(1));
        let position =
            PhysicalPosition::new((f64::from(x) + 0.5) * scale, (f64::from(y) + 0.5) * scale);
        if let Some(w) = &self.window
            && w.set_cursor_position(position).is_ok()
        {
            self.cursor = Some(position);
        }
        if let Some(point) = self.live_point() {
            let redraw = match &mut self.session {
                Session::Starting(_) => false,
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
        self.session.render();
        let path = PathBuf::from(format!("sims-game-{}.png", self.photos));
        self.photos += 1;
        let bytes = if let Some(game) = self.session.game() {
            let cropped = game.capture_snapshot_canvas();
            png::encode(&cropped)
        } else {
            png::encode(self.session.canvas())
        };
        match std::fs::write(&path, bytes) {
            Ok(()) => eprintln!("saved {}", path.display()),
            Err(e) => eprintln!("could not save {}: {e}", path.display()),
        }
    }

    /// Saves the lot (F5) or goes back to its save (F9). Returns `true` when
    /// the operation succeeded (`0x4f26cf`, checked by `0x4f29cc` before quit).
    fn save(&mut self, restore: bool) -> bool {
        let ok = match &mut self.session {
            Session::Starting(_) => false,
            Session::Standalone { game, slot, .. } => {
                let store = match sims_sim::Snapshots::open(&self.saves) {
                    Ok(store) => store,
                    Err(e) => {
                        eprintln!("sims-game: {}: {e}", self.saves.display());
                        let (title, msg) = if restore {
                            ("Restore Failed", format!("The save was not restored: {e}"))
                        } else {
                            ("Save Failed", format!("The lot was not saved: {e}"))
                        };
                        if game.has_ui_dialog() {
                            game.set_note(msg);
                        } else {
                            game.show_notice(title, msg);
                        }
                        self.changed(true, true);
                        return false;
                    }
                };
                if !restore {
                    let saved = game.save_to(&store, slot);
                    if !saved
                        && !game.has_ui_dialog()
                        && let Some(msg) = game.note().map(str::to_owned)
                    {
                        game.show_notice("Save Failed", msg);
                    }
                    saved
                } else if game.restore_from(&store, slot) {
                    self.printed = 0;
                    self.reported = false;
                    self.wake_clock.reset(Instant::now());
                    true
                } else {
                    if !game.has_ui_dialog()
                        && let Some(msg) = game.note().map(str::to_owned)
                    {
                        game.show_notice("Restore Failed", msg);
                    }
                    false
                }
            }
            Session::Shell(shell) => {
                if !restore {
                    match shell.save_current_lot() {
                        Ok(_) => true,
                        Err(e) => {
                            shell.show_notice("Save Failed", format!("The lot was not saved: {e}"));
                            false
                        }
                    }
                } else {
                    match shell.restore_current_lot() {
                        Ok(_) => {
                            self.printed = 0;
                            self.reported = false;
                            self.wake_clock.reset(Instant::now());
                            true
                        }
                        Err(e) => {
                            shell.show_notice(
                                "Restore Failed",
                                format!("The save was not restored: {e}"),
                            );
                            false
                        }
                    }
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
        if outcome.open_coop_lobby
            && let Some(game) = self.session.game_mut()
        {
            game.open_coop_lobby(None);
            self.changed(true, true);
        }
        if outcome.open_save_manager
            && let Some(game) = self.session.game_mut()
        {
            game.show_notice(
                "Save Manager",
                "Save Manager requires launching in Neighborhood / REDB mode (without a house file).",
            );
            self.changed(true, true);
        }
        let saved_ok = if outcome.save { self.save(false) } else { true };
        if outcome.save
            && !saved_ok
            && let Some(game) = self.session.game_mut()
        {
            let msg = game
                .note()
                .map(str::to_owned)
                .unwrap_or_else(|| "The lot was not saved.".into());
            game.after_save_failed_with_notice("Save Failed", msg);
            self.changed(true, true);
        }
        if outcome.neighborhood
            && saved_ok
            && let Some(game) = self.session.game_mut()
        {
            game.show_notice(
                "Neighborhood",
                "Neighborhood view requires launching without a house file (or with --neighborhood).",
            );
            self.changed(true, true);
        }
        if outcome.quit && saved_ok {
            self.shutdown_session();
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
            self.shutdown_session();
            event_loop.exit();
        }
    }

    fn key(&mut self, logical_key: &Key, physical_key: PhysicalKey, repeat: bool) {
        let Some(action) = classify_lot_hotkey(
            logical_key,
            physical_key,
            self.ctrl_or_cmd_held,
            self.shift_held,
            repeat,
        ) else {
            return;
        };
        let Some(game) = self.session.game_mut() else {
            return;
        };
        match action {
            LotHotkey::TogglePause => {
                game.toggle_pause();
                self.changed(true, true);
            }
            LotHotkey::SetSpeed(speed) => {
                game.set_speed(speed);
                self.changed(true, true);
            }
            LotHotkey::Mode(mode) => {
                game.hotkey_mode(mode);
                self.changed(true, true);
            }
            LotHotkey::ToggleCameraStrip => {
                game.toggle_camera_strip();
                self.changed(true, true);
            }
            LotHotkey::TogglePanel => {
                game.toggle_panel();
                self.changed(true, true);
            }
            LotHotkey::NextSim => {
                if game.select_next_sim() {
                    self.changed(true, true);
                }
            }
            LotHotkey::CentreSelectedSim => {
                if game.centre_on_selected() {
                    self.changed(true, true);
                }
            }
            LotHotkey::ToggleFloor => {
                let redraw = game.toggle_floor();
                self.changed(redraw, true);
            }
            LotHotkey::StepWalls(up) => {
                let _ = game.step_walls(up);
                self.changed(true, true);
            }
            LotHotkey::RotateHeldOrCamera(cw) => {
                if game.rotate_held(cw) {
                    self.changed(true, true);
                } else {
                    let redraw = game.apply(Move::Rotate(cw));
                    self.changed(redraw, true);
                }
            }
            LotHotkey::Move(mv) => {
                let redraw = game.apply(mv);
                self.changed(redraw, true);
            }
            LotHotkey::Photo => {
                self.photo();
            }
            LotHotkey::CatalogPage(delta) => {
                if game.page_catalog(delta) {
                    self.changed(true, true);
                }
            }
            LotHotkey::SellHeld => {
                if game.sell_held() {
                    self.changed(true, true);
                }
            }
            LotHotkey::Undo => {
                if game.undo_buy() {
                    self.changed(true, true);
                }
            }
            LotHotkey::Redo => {
                if game.redo_buy() {
                    self.changed(true, true);
                }
            }
            LotHotkey::QuickSave => {
                let _ = self.save(false);
            }
            LotHotkey::RestoreSave => {
                let _ = self.save(true);
            }
            LotHotkey::OpenSaveManager => {
                if let Session::Shell(shell) = &mut self.session
                    && shell.open_save_manager(None)
                {
                    self.changed(true, true);
                }
            }
            LotHotkey::OpenTravelDialog => {
                if let Session::Shell(shell) = &mut self.session {
                    if let Err(err) = shell.open_default_travel_dialog() {
                        shell.show_notice("Travel", format!("Travel unavailable: {err}"));
                    }
                    self.changed(true, true);
                }
            }
            LotHotkey::ReturnHome => {
                if let Session::Shell(shell) = &mut self.session
                    && shell.active_trip().is_some()
                {
                    if let Err(err) = shell.return_from_destination()
                        && (shell.screen() == ScreenKind::Lot || shell.note().is_none())
                    {
                        shell.show_notice("Travel", format!("Could not return home: {err}"));
                    }
                    self.drain_shell_audio();
                    self.changed(true, true);
                }
            }
            LotHotkey::OpenShop => {
                if let Some(kind) = game.shop_kind_for_lot()
                    && game.open_shop(kind)
                {
                    self.changed(true, true);
                }
            }
        }
    }

    /// Steps the active lot (or neighborhood animation timer) for the time
    /// since the last call.
    fn tick(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if let Session::Starting(start) = &mut self.session {
            start.tick(now);
            match start.rx.try_recv() {
                Ok(Ok((mut shell, preloaded_mixer))) => {
                    eprintln!(
                        "sims-game: opened Neighborhood {} in {:.1} s",
                        shell.neighborhood_slot(),
                        start.started.elapsed().as_secs_f32()
                    );
                    let size = start.size;
                    let install = start.install.clone();
                    let mute = start.mute;
                    let _ = start.loading_screen.finish();
                    shell.resize(size);
                    #[cfg(any(target_os = "macos", feature = "sound"))]
                    let next_sound = match preloaded_mixer {
                        Some(mixer) => Sound::from_mixer(mixer),
                        None => Sound::start(&install, mute),
                    };
                    #[cfg(not(any(target_os = "macos", feature = "sound")))]
                    let next_sound = {
                        let _ = preloaded_mixer;
                        Sound::start(&install, mute)
                    };
                    *self.sound.borrow_mut() = next_sound;
                    let pending_coop = std::mem::take(&mut self.pending_coop);
                    if let Some(name) = pending_coop.player_name {
                        shell.coop_set_player_name(name);
                    }
                    if pending_coop.loopback {
                        shell.coop_set_iroh_config(sims_net::IrohConfig::Loopback);
                    }
                    if pending_coop.host {
                        if let Err(err) = shell.coop_host_current_or_default() {
                            eprintln!("sims-game: co-op host failed: {err}");
                            shell.show_notice("Co-op", format!("Co-op host failed: {err}"));
                        }
                    } else if let Some(ticket) = pending_coop.join
                        && let Err(err) = shell.coop_join_from_shell(ticket)
                    {
                        eprintln!("sims-game: co-op join failed: {err}");
                        shell.show_notice("Co-op", format!("Co-op join failed: {err}"));
                    }
                    self.session = Session::Shell(Box::new(shell));
                    self.attach_loading_presenter();
                    self.drain_shell_audio();
                    self.wake_clock.reset(Instant::now());
                    self.changed(true, true);
                }
                Ok(Err(err)) => {
                    eprintln!("sims-game: startup failed: {err}");
                    self.startup_error = Some(err);
                    event_loop.exit();
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    let err = "startup worker disconnected".to_owned();
                    eprintln!("sims-game: {err}");
                    self.startup_error = Some(err);
                    event_loop.exit();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.changed(true, false);
                }
            }
            return;
        }
        let (clock_us, step_us) = self.wake_clock.elapsed_us(now);
        self.drain_shell_audio();
        let shell_modal_open = matches!(
            &self.session,
            Session::Shell(shell)
                if shell.save_manager_open()
                    || shell.album_open()
                    || shell.travel_dialog_open()
        );
        if shell_modal_open {
            let mut redraw = match &mut self.session {
                Session::Shell(shell) => shell.tick_shell(clock_us),
                Session::Standalone { .. } | Session::Starting(_) => false,
            };
            // Keep co-op lockstep & peer ACKs running beneath presentation
            // modals (Save Manager / Photo Album) so neither side stalls.
            if let Some(game) = self.session.game_mut()
                && game.coop_active()
            {
                let (ran, capped) = game.advance_while(clock_us, step_us, |ran| {
                    within_step_budget(ran, now.elapsed(), SIM_BUDGET)
                });
                self.wake_clock.finish(now, Instant::now(), capped);
                redraw |= ran > 0 || game.blink_due();
            } else {
                self.wake_clock.finish(now, Instant::now(), false);
            }
            if let Session::Shell(shell) = &mut self.session {
                redraw |= shell.pump_coop_restage();
            }
            self.changed(redraw, false);
            return;
        }
        let Some(game) = self.session.game_mut() else {
            self.wake_clock.finish(now, Instant::now(), false);
            let (redraw, interactive) = match &mut self.session {
                Session::Shell(shell) => {
                    let r = shell.tick_shell(clock_us);
                    (r, shell.has_active_fades())
                }
                Session::Standalone { .. } | Session::Starting(_) => (false, false),
            };
            self.changed(redraw, interactive);
            return;
        };

        let waited = game.asking().is_some() || game.asking_special_panel().is_some();
        let (ran, capped) = game.advance_while(clock_us, step_us, |ran| {
            within_step_budget(ran, now.elapsed(), SIM_BUDGET)
        });
        self.wake_clock.finish(now, Instant::now(), capped);
        let asking_now = game.asking().is_some() || game.asking_special_panel().is_some();
        let mut redraw = ran > 0 || (!waited && asking_now);
        let scrolled = self.drag.is_none() && game.edge_scroll(clock_us);
        let pie_anim = game.pie_animating();
        let ui_anim = game.ui_animating();
        let panning = game.take_camera_pan_moved() || game.camera_panning();
        redraw |= scrolled || pie_anim || ui_anim || panning || game.spin_due() || game.blink_due();
        let sound_rc = Rc::clone(&self.sound);
        self.sound_feed
            .update(game, |input| sound_rc.borrow().send(input));
        let start = self.printed.min(game.errors().len());
        for error in &game.errors()[start..] {
            eprintln!("sims-game: error caught: {error}");
        }
        self.printed = game.errors().len();
        if !self.reported
            && let Some(stop) = game.stopped().map(str::to_owned)
        {
            eprintln!("sims-game: {stop}");
            if !game.debug() {
                game.show_notice("Lot Stopped", stop);
            }
            self.reported = true;
            redraw = true;
        }
        if let Session::Shell(shell) = &mut self.session {
            redraw |= shell.pump_coop_restage();
            redraw |= shell.poll_background_saves();
        } else if let Session::Standalone { game, .. } = &mut self.session
            && let Some((host_house, host_family)) = game.pending_coop_restage()
        {
            let msg = format!(
                "Host is playing Lot {host_house} (family {host_family}), which does not match \
                 this standalone house file. Relaunch with House{host_house:02}.iff or in \
                 Neighborhood mode and rejoin."
            );
            game.fail_pending_coop_restage(msg.clone());
            game.show_notice("Co-op", msg);
            redraw = true;
        }
        // Edge-scrolling, smooth camera pan, and active UI / 400 ms pie animations
        // use display-refresh cadence, while pure simulation ticks / plumb-bob spin /
        // pause blink are passive (60 FPS cap).
        self.changed(redraw, scrolled || pie_anim || ui_anim || panning);
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
        let scale = self.scale.get().max(1);
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
                Session::Starting(_) => {
                    if let Some(title) = self
                        .title_throttle
                        .update_custom("sims-game: Loading...".into(), now)
                    {
                        window.set_title(title);
                    }
                }
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
        let mut surf_guard = self.surface.borrow_mut();
        let Some(surface) = surf_guard.as_mut() else {
            self.dirty = true;
            self.redraw_pacer.on_redraw_failed(Instant::now());
            return;
        };
        if self.surface_size.get() != Some((w, h)) {
            self.last_present_sig.set(None);
            if surface.resize(w, h).is_err() {
                self.surface_size.set(None);
                self.dirty = true;
                self.redraw_pacer.on_redraw_failed(Instant::now());
                return;
            }
            self.surface_size.set(Some((w, h)));
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            self.surface_size.set(None);
            self.last_present_sig.set(None);
            self.dirty = true;
            self.redraw_pacer.on_redraw_failed(Instant::now());
            return;
        };
        let canvas = self.session.canvas();
        let (bw, bh) = (size.width as usize, size.height as usize);
        let scale = scale as usize;
        let cur_sig: PresentSig = (
            bw,
            bh,
            scale,
            canvas.width,
            canvas.height,
            self.session.screen_kind(),
        );
        let presented = if buffer.age() == 1
            && self.last_present_sig.get() == Some(cur_sig)
            && let Some(dirty_rects) = self.session.peek_present_damage()
            && !dirty_rects.is_empty()
        {
            let mut damage = Vec::with_capacity(dirty_rects.len());
            for &rect in dirty_rects {
                if let Some(sb_rect) = blit_canvas_rect(canvas, &mut buffer, bw, bh, scale, rect) {
                    damage.push(sb_rect);
                }
            }
            if damage.is_empty() {
                blit_canvas(canvas, &mut buffer, bw, bh, scale);
                buffer.present()
            } else if window_supports_exposure_safe_partial_present(&window) {
                buffer.present_with_damage(&damage)
            } else {
                // On X11/Win32, winit coalesces OS exposure events into
                // `RedrawRequested`; `blit_canvas_rect` already avoided
                // rewriting unchanged pixels in the retained backbuffer
                // (`age() == 1`), and full `present()` guarantees any exposed
                // window region outside `damage` is repainted.
                buffer.present()
            }
        } else {
            blit_canvas(canvas, &mut buffer, bw, bh, scale);
            buffer.present()
        };
        if presented.is_err() {
            self.surface_size.set(None);
            self.last_present_sig.set(None);
            self.dirty = true;
            self.redraw_pacer.on_redraw_failed(Instant::now());
            return;
        }
        self.last_present_sig.set(Some(cur_sig));
        self.session.on_presented();
        self.redraw_pacer.on_present(Instant::now());
    }

    /// Exact next wakeup instant for the OS event loop, or `None` when the
    /// active screen is completely idle (allowing `ControlFlow::Wait` with zero
    /// periodic timer wakeups until an input, co-op, or autosave event arrives).
    fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let continuous = self.continuous_interaction();
        let anim_interval_us = u64::try_from(
            self.redraw_pacer
                .target_interval(now, continuous)
                .as_micros(),
        )
        .unwrap_or(16_667)
        .max(1_000);
        let can_draw = self.window_drawable();
        let dragging = self.drag.is_some();
        let session_deadline = match &self.session {
            Session::Starting(_) => Some(now + STARTUP_POLL),
            Session::Standalone { game, .. } => game
                .next_wake_us_with_window(dragging, anim_interval_us, can_draw, self.dirty)
                .map(|us| now + Duration::from_micros(us)),
            Session::Shell(shell) => shell
                .next_wake_us_with_window(dragging, anim_interval_us, can_draw, self.dirty)
                .map(|us| now + Duration::from_micros(us)),
        };
        let redraw_deadline = if can_draw {
            self.redraw_pacer
                .next_redraw_deadline(now, self.dirty, continuous)
        } else {
            None
        };
        match (session_deadline, redraw_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }
}

/// Whether `softbuffer::Buffer::present_with_damage` can safely copy only the
/// dirty rectangles without leaving OS-exposed window regions unpainted.
/// Composited surfaces (Wayland, Web) retain their window backing store across
/// exposures, whereas X11/Win32 coalesce OS exposure events into winit's
/// `RedrawRequested` and need a full `present()` after the partial CPU blit.
fn window_supports_exposure_safe_partial_present(window: &Window) -> bool {
    window.display_handle().is_ok_and(|handle| {
        matches!(
            handle.as_raw(),
            RawDisplayHandle::Wayland(_) | RawDisplayHandle::Web(_)
        )
    })
}

#[inline]
fn blit_row_1x(src_row: &[u32], dst_row: &mut [u32]) {
    let copy_w = dst_row.len().min(src_row.len());
    dst_row[..copy_w].copy_from_slice(&src_row[..copy_w]);
    if dst_row.len() > copy_w {
        dst_row[copy_w..].fill(src_row[copy_w - 1]);
    }
}

#[inline]
fn blit_row_2x(src_row: &[u32], dst_row: &mut [u32]) {
    let pairs = (dst_row.len() / 2).min(src_row.len());
    let (head, tail) = dst_row.split_at_mut(pairs * 2);
    for (pair, &px) in head
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .zip(&src_row[..pairs])
    {
        *pair = [px, px];
    }
    if !tail.is_empty() {
        let cx = pairs.min(src_row.len() - 1);
        tail.fill(src_row[cx]);
    }
}

/// Blits `canvas` into the `bw × bh` `softbuffer` pixel buffer scaled up by
/// integer `scale`, matching `(y / scale).min(h - 1)` / `(x / scale).min(w - 1)`
/// bit-for-bit, with specialized `memcpy` (`scale == 1`) and pure-`u32`
/// auto-vectorizable (`scale == 2`) paths.
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
            if bw == canvas.width && bh <= canvas.height {
                buffer[..total].copy_from_slice(&canvas.color[..total]);
                return;
            }
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
                        *dst = src_row[cx];
                    }
                }
            }
        }
    }
}

/// Blits only the canvas-space half-open rectangle `rect = [rx0, ry0, rx1, ry1)`
/// into `buffer` at integer `scale`, returning the exact `softbuffer::Rect`
/// updated (including any trailing edge-clamped columns/rows when `rect`
/// touches the right or bottom edge of `canvas`).
fn blit_canvas_rect(
    canvas: &sims_render::Canvas,
    buffer: &mut [u32],
    bw: usize,
    bh: usize,
    scale: usize,
    rect: sims_render::canvas::Rect,
) -> Option<softbuffer::Rect> {
    let scale = scale.max(1);
    let total = bw.checked_mul(bh)?;
    if total == 0
        || buffer.len() < total
        || canvas.width == 0
        || canvas.height == 0
        || canvas.color.len() < canvas.width.saturating_mul(canvas.height)
    {
        return None;
    }
    let cw = canvas.width;
    let ch = canvas.height;
    let cw_i = i32::try_from(cw).unwrap_or(i32::MAX);
    let ch_i = i32::try_from(ch).unwrap_or(i32::MAX);
    let cx0 = rect[0].clamp(0, cw_i) as usize;
    let cy0 = rect[1].clamp(0, ch_i) as usize;
    let cx1 = rect[2].clamp(cx0 as i32, cw_i) as usize;
    let cy1 = rect[3].clamp(cy0 as i32, ch_i) as usize;
    if cx0 >= cx1 || cy0 >= cy1 {
        return None;
    }
    let bx0 = cx0.saturating_mul(scale).min(bw);
    let bx1 = if cx1 >= cw {
        bw
    } else {
        cx1.saturating_mul(scale).min(bw)
    };
    let by0 = cy0.saturating_mul(scale).min(bh);
    let by1 = if cy1 >= ch {
        bh
    } else {
        cy1.saturating_mul(scale).min(bh)
    };
    if bx0 >= bx1 || by0 >= by1 {
        return None;
    }
    let row_w = bx1 - bx0;
    let max_cy = ch - 1;
    let max_cx = cw - 1;
    match scale {
        1 => {
            for by in by0..by1 {
                let cy = by.min(max_cy);
                if by > by0 && cy == (by - 1).min(max_cy) {
                    let prev = (by - 1) * bw + bx0;
                    buffer.copy_within(prev..prev + row_w, by * bw + bx0);
                } else {
                    let src_row = &canvas.color[cy * cw + cx0..cy * cw + cx1];
                    blit_row_1x(src_row, &mut buffer[by * bw + bx0..by * bw + bx1]);
                }
            }
        }
        2 => {
            for by in by0..by1 {
                let cy = (by >> 1).min(max_cy);
                if by > by0 && cy == ((by - 1) >> 1).min(max_cy) {
                    let prev = (by - 1) * bw + bx0;
                    buffer.copy_within(prev..prev + row_w, by * bw + bx0);
                } else {
                    let src_row = &canvas.color[cy * cw + cx0..cy * cw + cx1];
                    blit_row_2x(src_row, &mut buffer[by * bw + bx0..by * bw + bx1]);
                }
            }
        }
        _ => {
            for by in by0..by1 {
                let cy = (by / scale).min(max_cy);
                if by > by0 && cy == ((by - 1) / scale).min(max_cy) {
                    let prev = (by - 1) * bw + bx0;
                    buffer.copy_within(prev..prev + row_w, by * bw + bx0);
                } else {
                    let src_row = &canvas.color[cy * cw..(cy + 1) * cw];
                    let dst_row = &mut buffer[by * bw + bx0..by * bw + bx1];
                    for (i, dst) in dst_row.iter_mut().enumerate() {
                        let cx = ((bx0 + i) / scale).min(max_cx);
                        *dst = src_row[cx];
                    }
                }
            }
        }
    }
    Some(softbuffer::Rect {
        x: u32::try_from(bx0).ok()?,
        y: u32::try_from(by0).ok()?,
        width: NonZeroU32::new(u32::try_from(row_w).ok()?)?,
        height: NonZeroU32::new(u32::try_from(by1 - by0).ok()?)?,
    })
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = window_attributes();
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
                *self.surface.borrow_mut() = Some(s);
                self.surface_size.set(None);
            }
            Err(e) => {
                eprintln!("sims-game: could not draw into the window: {e}");
                event_loop.exit();
                return;
            }
        }
        self.scale.set(window_scale(window.scale_factor()));
        self.window = Some(window.clone());
        self.attach_loading_presenter();
        self.refresh_monitor_rate();
        self.drain_shell_audio();
        self.wake_clock.reset(Instant::now());
        self.dirty = true;
        self.redraw_pacer.request_immediate();
        self.maybe_request_redraw(Instant::now());
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.tick(event_loop);
        let now = Instant::now();
        match self.next_deadline(now) {
            Some(deadline) => event_loop.set_control_flow(ControlFlow::WaitUntil(deadline)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        // Account for any time spent idle under the previous screen/modal state
        // before dispatching interactive input so closing a modal never dumps
        // modal idle time into live simulation ticks and newly hovered map lots
        // start their debounce timer at the current event time.
        if !matches!(self.session, Session::Starting(_))
            && matches!(
                event,
                WindowEvent::KeyboardInput { .. }
                    | WindowEvent::CursorMoved { .. }
                    | WindowEvent::CursorLeft { .. }
                    | WindowEvent::MouseInput { .. }
                    | WindowEvent::MouseWheel { .. }
            )
        {
            self.tick(event_loop);
        }
        match event {
            WindowEvent::CloseRequested => {
                self.shutdown_session();
                event_loop.exit();
            }
            WindowEvent::Focused(focused) => {
                if focused {
                    self.refresh_monitor_rate();
                    if let Session::Shell(shell) = &mut self.session
                        && shell.refresh_importable_families()
                    {
                        self.changed(true, true);
                    }
                }
                if let Some(focused) = self.focus_filter.filter(focused) {
                    let sound_rc = Rc::clone(&self.sound);
                    if let Some(game) = self.session.game_mut() {
                        if self
                            .sound_feed
                            .set_focused(focused, game, |input| sound_rc.borrow().send(input))
                        {
                            self.wake_clock.reset(Instant::now());
                            self.changed(true, true);
                        }
                    } else {
                        self.sound_feed
                            .set_shell_focused(focused, |input| sound_rc.borrow().send(input));
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
                self.scale.set(window_scale(scale_factor));
                self.surface_size.set(None);
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
                let is_cheat_toggle = !event.repeat
                    && self.ctrl_or_cmd_held
                    && self.shift_held
                    && matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("c"));
                let is_coop_toggle = !event.repeat
                    && (event.logical_key == Key::Named(NamedKey::F6)
                        || (self.ctrl_or_cmd_held
                            && !self.shift_held
                            && matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("m"))));
                let save_manager_open = matches!(
                    &self.session,
                    Session::Shell(shell) if shell.save_manager_open()
                );
                let album_open = matches!(
                    &self.session,
                    Session::Shell(shell) if shell.album_open()
                );
                let travel_dialog_open = matches!(
                    &self.session,
                    Session::Shell(shell) if shell.travel_dialog_open()
                );
                let shop_open = self.session.game().is_some_and(Game::shop_open)
                    || matches!(&self.session, Session::Shell(shell) if shell.import_picker_open());
                let book_open = self.session.game().is_some_and(Game::book_open)
                    || matches!(&self.session, Session::Shell(shell) if shell.credits_open());
                let coop_lobby_open = self.session.coop_lobby_open();
                let cheat_console_open = self.session.game().is_some_and(Game::cheat_console_open);
                if is_coop_toggle
                    && !save_manager_open
                    && !album_open
                    && !travel_dialog_open
                    && !shop_open
                    && !book_open
                    && !cheat_console_open
                    && self.session.screen_kind() != ScreenKind::CreateFamily
                    && (coop_lobby_open || !self.session.has_dialog())
                {
                    let toggled = match &mut self.session {
                        Session::Starting(_) => false,
                        Session::Standalone { game, .. } => game.toggle_coop_lobby(),
                        Session::Shell(shell) => shell.toggle_coop_lobby(),
                    };
                    self.changed(toggled, true);
                    return;
                }
                if coop_lobby_open {
                    let is_paste = !event.repeat
                        && self.ctrl_or_cmd_held
                        && !self.shift_held
                        && matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("v"));
                    match &mut self.session {
                        Session::Starting(_) => {}
                        Session::Standalone { game, .. } => {
                            let out = if is_paste {
                                sims_game::game::Outcome {
                                    redraw: game.coop_lobby_paste(None),
                                    ..sims_game::game::Outcome::default()
                                }
                            } else {
                                match &event.logical_key {
                                    Key::Named(NamedKey::Escape) if !event.repeat => {
                                        game.coop_lobby_escape()
                                    }
                                    Key::Named(NamedKey::Enter) if !event.repeat => {
                                        game.coop_lobby_submit()
                                    }
                                    Key::Named(NamedKey::Backspace) => sims_game::game::Outcome {
                                        redraw: game.coop_lobby_backspace(),
                                        ..sims_game::game::Outcome::default()
                                    },
                                    Key::Named(NamedKey::Tab) => sims_game::game::Outcome {
                                        redraw: game.coop_lobby_toggle_tab(),
                                        ..sims_game::game::Outcome::default()
                                    },
                                    _ => {
                                        let mut changed = false;
                                        if !self.ctrl_or_cmd_held
                                            && let Some(text) = &event.text
                                        {
                                            for ch in text.chars() {
                                                changed |= game.coop_lobby_char(ch);
                                            }
                                        }
                                        sims_game::game::Outcome {
                                            redraw: changed,
                                            ..sims_game::game::Outcome::default()
                                        }
                                    }
                                }
                            };
                            self.handle_standalone_outcome(event_loop, out);
                        }
                        Session::Shell(shell) => {
                            let out = if is_paste {
                                ShellOutcome {
                                    redraw: shell.coop_lobby_paste(None),
                                    ..ShellOutcome::default()
                                }
                            } else {
                                match &event.logical_key {
                                    Key::Named(NamedKey::Escape) if !event.repeat => shell.escape(),
                                    Key::Named(NamedKey::Enter) if !event.repeat => {
                                        shell.key_enter()
                                    }
                                    Key::Named(NamedKey::Backspace) => shell.key_backspace(),
                                    Key::Named(NamedKey::Tab) => shell.key_tab(),
                                    _ => {
                                        let mut combined = ShellOutcome::default();
                                        if !self.ctrl_or_cmd_held
                                            && let Some(text) = &event.text
                                        {
                                            for ch in text.chars() {
                                                let o = shell.key_char(ch);
                                                combined.redraw |= o.redraw;
                                            }
                                        }
                                        combined
                                    }
                                }
                            };
                            self.handle_shell_outcome(event_loop, out);
                        }
                    }
                    return;
                }
                if is_cheat_toggle
                    && !save_manager_open
                    && !album_open
                    && !travel_dialog_open
                    && !shop_open
                    && !book_open
                    && !self.session.has_dialog()
                    && let Some(game) = self.session.game_mut()
                {
                    game.toggle_cheat_console();
                    self.changed(true, true);
                    return;
                }
                if save_manager_open && let Session::Shell(shell) = &mut self.session {
                    let out = match &event.logical_key {
                        Key::Named(NamedKey::ArrowUp) => shell.key_up(),
                        Key::Named(NamedKey::ArrowDown) => shell.key_down(),
                        Key::Named(NamedKey::Enter) if !event.repeat => shell.key_enter(),
                        Key::Named(NamedKey::Escape) if !event.repeat => shell.escape(),
                        _ => ShellOutcome::default(),
                    };
                    self.handle_shell_outcome(event_loop, out);
                    return;
                }
                if travel_dialog_open && let Session::Shell(shell) = &mut self.session {
                    let out = match &event.logical_key {
                        Key::Named(NamedKey::ArrowLeft) => shell.key_left(),
                        Key::Named(NamedKey::ArrowRight) => shell.key_right(),
                        Key::Named(NamedKey::Tab) => shell.key_tab(),
                        Key::Named(NamedKey::Enter) if !event.repeat => shell.key_enter(),
                        Key::Named(NamedKey::Escape) if !event.repeat => shell.escape(),
                        _ => ShellOutcome::default(),
                    };
                    self.handle_shell_outcome(event_loop, out);
                    return;
                }
                if album_open && let Session::Shell(shell) = &mut self.session {
                    let out = match &event.logical_key {
                        Key::Named(NamedKey::Backspace) => shell.key_backspace(),
                        Key::Named(NamedKey::Delete) => shell.key_delete(),
                        Key::Named(NamedKey::ArrowLeft) => shell.key_left(),
                        Key::Named(NamedKey::ArrowRight) => shell.key_right(),
                        Key::Named(NamedKey::ArrowUp) => shell.key_up(),
                        Key::Named(NamedKey::ArrowDown) => shell.key_down(),
                        Key::Named(NamedKey::Home) => shell.key_home(),
                        Key::Named(NamedKey::End) => shell.key_end(),
                        Key::Named(NamedKey::PageUp) => shell.key_page_up(),
                        Key::Named(NamedKey::PageDown) => shell.key_page_down(),
                        Key::Named(NamedKey::Tab) => shell.key_tab(),
                        Key::Named(NamedKey::Enter) if !event.repeat => shell.key_enter(),
                        Key::Named(NamedKey::Escape) if !event.repeat => shell.escape(),
                        Key::Named(NamedKey::Space) => shell.key_char(' '),
                        Key::Character(c)
                            if self.ctrl_or_cmd_held
                                && !event.repeat
                                && c.eq_ignore_ascii_case("s") =>
                        {
                            shell.key_save_shortcut()
                        }
                        _ => {
                            let mut combined = ShellOutcome::default();
                            if !self.ctrl_or_cmd_held
                                && let Some(text) = &event.text
                            {
                                for ch in text.chars() {
                                    let o = shell.key_char(ch);
                                    combined.redraw |= o.redraw;
                                }
                            }
                            combined
                        }
                    };
                    self.handle_shell_outcome(event_loop, out);
                    return;
                }
                if book_open {
                    match &mut self.session {
                        Session::Starting(_) => {}
                        Session::Standalone { game, .. } => {
                            let out = match &event.logical_key {
                                Key::Named(NamedKey::ArrowUp) => sims_game::Outcome {
                                    redraw: game.book_scroll_lines(-3),
                                    ..sims_game::Outcome::default()
                                },
                                Key::Named(NamedKey::ArrowDown) => sims_game::Outcome {
                                    redraw: game.book_scroll_lines(3),
                                    ..sims_game::Outcome::default()
                                },
                                Key::Named(NamedKey::ArrowLeft | NamedKey::PageUp) => {
                                    sims_game::Outcome {
                                        redraw: game.book_prev_page(),
                                        ..sims_game::Outcome::default()
                                    }
                                }
                                Key::Named(NamedKey::ArrowRight | NamedKey::PageDown) => {
                                    sims_game::Outcome {
                                        redraw: game.book_next_page(),
                                        ..sims_game::Outcome::default()
                                    }
                                }
                                Key::Named(NamedKey::Enter | NamedKey::Escape) if !event.repeat => {
                                    game.book_close()
                                }
                                _ => sims_game::Outcome::default(),
                            };
                            self.handle_standalone_outcome(event_loop, out);
                        }
                        Session::Shell(shell) => {
                            let out = match &event.logical_key {
                                Key::Named(NamedKey::ArrowUp) => shell.key_up(),
                                Key::Named(NamedKey::ArrowDown) => shell.key_down(),
                                Key::Named(NamedKey::ArrowLeft) => shell.key_left(),
                                Key::Named(NamedKey::ArrowRight) => shell.key_right(),
                                Key::Named(NamedKey::PageUp) => shell.key_page_up(),
                                Key::Named(NamedKey::PageDown) => shell.key_page_down(),
                                Key::Named(NamedKey::Enter) if !event.repeat => shell.key_enter(),
                                Key::Named(NamedKey::Escape) if !event.repeat => shell.escape(),
                                _ => ShellOutcome::default(),
                            };
                            self.handle_shell_outcome(event_loop, out);
                        }
                    }
                    return;
                }
                if shop_open {
                    match &mut self.session {
                        Session::Starting(_) => {}
                        Session::Standalone { game, .. } => {
                            let out = match &event.logical_key {
                                Key::Named(NamedKey::ArrowUp) => {
                                    let r = game.shop_select_prev();
                                    sims_game::Outcome {
                                        redraw: r,
                                        ..sims_game::Outcome::default()
                                    }
                                }
                                Key::Named(NamedKey::ArrowDown) => {
                                    let r = game.shop_select_next();
                                    sims_game::Outcome {
                                        redraw: r,
                                        ..sims_game::Outcome::default()
                                    }
                                }
                                Key::Named(NamedKey::ArrowLeft) => game.shop_rotate_preview(-45),
                                Key::Named(NamedKey::ArrowRight) => game.shop_rotate_preview(45),
                                Key::Named(NamedKey::Enter) if !event.repeat => game.shop_submit(),
                                Key::Named(NamedKey::Escape) if !event.repeat => game.shop_escape(),
                                _ => sims_game::Outcome::default(),
                            };
                            self.handle_standalone_outcome(event_loop, out);
                        }
                        Session::Shell(shell) => {
                            let out = match &event.logical_key {
                                Key::Named(NamedKey::ArrowUp) => shell.key_up(),
                                Key::Named(NamedKey::ArrowDown) => shell.key_down(),
                                Key::Named(NamedKey::ArrowLeft) => shell.key_left(),
                                Key::Named(NamedKey::ArrowRight) => shell.key_right(),
                                Key::Named(NamedKey::Enter) if !event.repeat => shell.key_enter(),
                                Key::Named(NamedKey::Escape) if !event.repeat => shell.escape(),
                                _ => ShellOutcome::default(),
                            };
                            self.handle_shell_outcome(event_loop, out);
                        }
                    }
                    return;
                }
                if !event.repeat
                    && event.logical_key == Key::Named(NamedKey::F10)
                    && !self.session.has_dialog()
                    && self.session.screen_kind() == ScreenKind::Neighborhood
                    && let Session::Shell(shell) = &mut self.session
                {
                    let _ = shell.open_photo_album();
                    self.changed(true, true);
                    return;
                }
                if self.session.screen_kind() == ScreenKind::CreateFamily
                    && let Session::Shell(shell) = &mut self.session
                {
                    let out = match &event.logical_key {
                        Key::Named(NamedKey::Backspace) => shell.key_backspace(),
                        Key::Named(NamedKey::Delete) => shell.key_delete(),
                        Key::Named(NamedKey::ArrowLeft) => shell.key_left(),
                        Key::Named(NamedKey::ArrowRight) => shell.key_right(),
                        Key::Named(NamedKey::Home) => shell.key_home(),
                        Key::Named(NamedKey::End) => shell.key_end(),
                        Key::Named(NamedKey::Tab) => shell.key_tab(),
                        Key::Named(NamedKey::Enter) if !event.repeat => shell.key_enter(),
                        Key::Named(NamedKey::Escape) if !event.repeat => shell.escape(),
                        Key::Named(NamedKey::Space) => shell.key_char(' '),
                        _ => {
                            let mut combined = ShellOutcome::default();
                            if !self.ctrl_or_cmd_held
                                && let Some(text) = &event.text
                            {
                                for ch in text.chars() {
                                    let o = shell.key_char(ch);
                                    combined.redraw |= o.redraw;
                                }
                            }
                            combined
                        }
                    };
                    self.handle_shell_outcome(event_loop, out);
                    return;
                }
                if !self.session.has_dialog()
                    && self.session.game().is_some_and(Game::cheat_console_open)
                    && let Some(game) = self.session.game_mut()
                {
                    let mut changed = false;
                    match &event.logical_key {
                        Key::Named(NamedKey::Escape) if !event.repeat => {
                            changed = game.close_cheat_console();
                        }
                        Key::Named(NamedKey::Enter) if !event.repeat => {
                            let _ = game.cheat_console_submit();
                            changed = true;
                        }
                        Key::Named(NamedKey::Backspace) => {
                            changed = game.cheat_console_backspace();
                        }
                        Key::Named(NamedKey::Delete) => {
                            changed = game.cheat_console_delete();
                        }
                        Key::Named(NamedKey::ArrowLeft) => {
                            changed = game.cheat_console_left();
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            changed = game.cheat_console_right();
                        }
                        Key::Named(NamedKey::ArrowUp) => {
                            changed = game.cheat_console_history_up();
                        }
                        Key::Named(NamedKey::ArrowDown) => {
                            changed = game.cheat_console_history_down();
                        }
                        Key::Named(NamedKey::Home) => {
                            changed = game.cheat_console_home();
                        }
                        Key::Named(NamedKey::End) => {
                            changed = game.cheat_console_end();
                        }
                        Key::Named(NamedKey::Space) => {
                            changed = game.cheat_console_char(' ');
                        }
                        _ => {
                            if !self.ctrl_or_cmd_held
                                && let Some(text) = &event.text
                            {
                                for ch in text.chars() {
                                    changed |= game.cheat_console_char(ch);
                                }
                            }
                        }
                    }
                    self.changed(changed, true);
                    return;
                }
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
                                Session::Starting(_) => {}
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
                                    Session::Starting(_) => {}
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
                                Session::Starting(_) => {}
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
                            Session::Starting(_) => {}
                            Session::Standalone { game, .. } => {
                                let closed = game.close_menu() || game.close_camera_strip();
                                let cancelled = !closed && game.cancel_buy();
                                let escaped_options =
                                    !closed && !cancelled && game.escape_options();
                                if !escaped_options
                                    && escape_exits(closed, cancelled, game.buy_cursor().is_some())
                                {
                                    self.shutdown_session();
                                    event_loop.exit();
                                } else {
                                    self.changed(true, true);
                                }
                            }
                            Session::Shell(shell) => {
                                if let Some(game) = shell.game_mut() {
                                    let closed = game.close_menu() || game.close_camera_strip();
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
                                        self.shutdown_session();
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
                } else if !lot_hotkeys_blocked(LotInputFocus {
                    cheat_console_open,
                    has_dialog: self.session.has_dialog(),
                    album_open,
                    coop_lobby_open,
                    create_family: self.session.screen_kind() == ScreenKind::CreateFamily,
                }) {
                    self.key(&event.logical_key, event.physical_key, event.repeat);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => f64::from(y),
                    MouseScrollDelta::PixelDelta(p) => p.y,
                };
                if let Session::Shell(shell) = &mut self.session
                    && (shell.save_manager_open()
                        || shell.album_open()
                        || shell.import_picker_open()
                        || shell.credits_open())
                {
                    let redraw = shell.wheel(y);
                    self.changed(redraw, true);
                    return;
                }
                if let Some(game) = self.session.game_mut()
                    && game.book_open()
                {
                    let redraw = game.book_wheel(y);
                    self.changed(redraw, true);
                    return;
                }
                if let Some(game) = self.session.game_mut()
                    && game.shop_open()
                {
                    let redraw = game.shop_wheel(y);
                    self.changed(redraw, true);
                    return;
                }
                if self.session.has_dialog() || self.session.coop_lobby_open() {
                    return;
                }
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
                            Session::Starting(_) => {}
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
                        Session::Starting(_) => {}
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
                    if self.session.game().is_some()
                        && !self.session.has_dialog()
                        && !self.session.coop_lobby_open()
                    {
                        self.drag = self.live_point().and(self.cursor);
                    }
                }
                (MouseButton::Right, ElementState::Released) => {
                    self.drag = None;
                    let pt = self.live_point();
                    let redraw = match (&mut self.session, pt) {
                        (Session::Starting(_), _) => false,
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
                    Session::Starting(_) => false,
                    Session::Standalone { game, .. } => game.leave(),
                    Session::Shell(shell) => shell.leave(),
                };
                self.changed(redraw, true);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.last_cursor = Some(position);
                let scale = self.scale.get();
                let (point, unclamped) = self
                    .window
                    .as_ref()
                    .map(|w| {
                        let size = w.inner_size();
                        if let Some(canvas) = canvas_size(size, scale) {
                            self.session.resize(canvas);
                        }
                        (
                            canvas_point(position, size, scale),
                            unclamped_canvas_point(position, size, scale),
                        )
                    })
                    .unwrap_or((None, None));
                if let Some(point) = point {
                    self.cursor = Some(position);
                    let redraw = self.drag.is_none()
                        && match &mut self.session {
                            Session::Starting(_) => false,
                            Session::Standalone { game, .. } => game.hover(point),
                            Session::Shell(shell) => shell.hover(point),
                        };
                    self.changed(redraw, true);
                } else {
                    self.cursor = None;
                    let mut redraw = self.drag.is_none()
                        && match &mut self.session {
                            Session::Starting(_) => false,
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
                if self.session.has_dialog()
                    || self.session.coop_lobby_open()
                    || self.session.game().is_none()
                {
                    self.drag = None;
                }
                if let Some(start) = self.drag {
                    let scale = f64::from(self.scale.get().max(1));
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
    if let Some(pack_dir) = config.open_pack {
        eprintln!(
            "sims-game: booting clean-room open-pack ({})",
            pack_dir.display()
        );
        let started = Instant::now();
        let mut game = Game::open_open_pack(
            &pack_dir,
            &config.answers,
            config.play,
            sims_game::Frames::default(),
        )?;
        game.set_debug(config.debug);
        eprintln!(
            "sims-game: booted open-pack in {:.2} s",
            started.elapsed().as_secs_f32()
        );
        for problem in game.problems() {
            eprintln!("sims-game: {problem}");
        }
        if let Some(still) = config.still {
            let started = Instant::now();
            let ran = game.run(still.ticks);
            eprintln!(
                "sims-game: ran {ran} ticks in {:.2} s",
                started.elapsed().as_secs_f32()
            );
            for error in game.errors() {
                eprintln!("sims-game: error caught: {error}");
            }
            if let Some(stop) = game.stopped() {
                eprintln!("sims-game: {stop}");
                return Err(format!(
                    "open-pack headless run stopped after {ran}/{} ticks: {stop}",
                    still.ticks
                ));
            }
            if !config.play && !game.errors().is_empty() {
                return Err(format!(
                    "open-pack headless strict run encountered {} VM error(s) after {ran}/{} ticks",
                    game.errors().len(),
                    still.ticks
                ));
            }
            if ran < still.ticks {
                if let Some(dialog) = game.asking() {
                    return Err(format!(
                        "open-pack headless run paused on dialog (object {}, answers {:?}) after {ran}/{} ticks (supply --answers)",
                        dialog.shown.object, dialog.shown.answers, still.ticks
                    ));
                }
                return Err(format!(
                    "open-pack headless run completed only {ran}/{} ticks",
                    still.ticks
                ));
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
        let sound = Sound::start_open_pack(game.open_pack_sounds(), config.mute);
        let save_slot = game.open_pack_save_slot().unwrap_or("open-pack").to_owned();
        let mut app = App {
            session: Session::Standalone {
                game: Box::new(game),
                slot: save_slot,
                options_file,
                saved_optn,
            },
            last_screen: ScreenKind::Lot,
            last_lot_generation: (0, 0),
            window: None,
            surface: Rc::new(RefCell::new(None)),
            surface_size: Rc::new(Cell::new(None)),
            last_present_sig: Rc::new(Cell::new(None)),
            title_throttle: TitleThrottle::default(),
            redraw_pacer: RedrawPacer::new(None),
            scale: Rc::new(Cell::new(1)),
            cursor: None,
            last_cursor: None,
            drag: None,
            last_left_press: None,
            wake_clock: WakeClock::new(Instant::now()),
            dirty: true,
            photos: 0,
            reported: false,
            printed: 0,
            sound: Rc::new(RefCell::new(sound)),
            sound_feed: sound::Feed::default(),
            focus_filter: FocusFilter::default(),
            saves: config.saves,
            ctrl_or_cmd_held: false,
            shift_held: false,
            startup_error: None,
            pending_coop: CoopCliConfig::default(),
        };
        let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
        return event_loop.run_app(&mut app).map_err(|e| e.to_string());
    }
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
        #[cfg(any(target_os = "macos", feature = "sound"))]
        let (mut game, preloaded_mixer) = if config.still.is_none() && !config.mute {
            let (game_res, mixer_res) = std::thread::scope(|scope| {
                let mixer_handle =
                    scope.spawn(|| sims_game::sound::Mixer::open(&game_options.install));
                let game_res = Game::open(&game_options);
                (
                    game_res,
                    mixer_handle
                        .join()
                        .unwrap_or_else(|_| Err("sound mixer worker panicked".to_owned())),
                )
            });
            (game_res?, Some(mixer_res))
        } else {
            (Game::open(&game_options)?, None)
        };
        #[cfg(not(any(target_os = "macos", feature = "sound")))]
        let mut game = Game::open(&game_options)?;
        game.set_debug(config.debug);
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
        if let Some(name) = config.coop.player_name.clone() {
            game.coop_set_player_name(name);
        }
        if config.coop.loopback {
            game.coop_set_iroh_config(sims_net::IrohConfig::Loopback);
        }
        if config.coop.host {
            if let Err(err) = game.coop_start_host_async(None) {
                eprintln!("sims-game: co-op host failed: {err}");
                game.show_notice("Co-op", format!("Co-op host failed: {err}"));
            } else {
                game.open_coop_lobby(Some(sims_ui::coop::LobbyTab::Host));
            }
        } else if let Some(ticket) = config.coop.join.clone() {
            if let Err(err) = game.coop_start_join_async(ticket, None) {
                eprintln!("sims-game: co-op join failed: {err}");
                game.show_notice("Co-op", format!("Co-op join failed: {err}"));
            } else {
                game.open_coop_lobby(Some(sims_ui::coop::LobbyTab::Join));
            }
        }
        #[cfg(any(target_os = "macos", feature = "sound"))]
        let sound = match preloaded_mixer {
            Some(mixer) => Sound::from_mixer(mixer),
            None => Sound::start(&config.install, config.mute),
        };
        #[cfg(not(any(target_os = "macos", feature = "sound")))]
        let sound = Sound::start(&config.install, config.mute);
        let mut app = App {
            session: Session::Standalone {
                game: Box::new(game),
                slot: slot(&house),
                options_file,
                saved_optn,
            },
            last_screen: ScreenKind::Lot,
            last_lot_generation: (0, 0),
            window: None,
            surface: Rc::new(RefCell::new(None)),
            surface_size: Rc::new(Cell::new(None)),
            last_present_sig: Rc::new(Cell::new(None)),
            title_throttle: TitleThrottle::default(),
            redraw_pacer: RedrawPacer::new(None),
            scale: Rc::new(Cell::new(1)),
            cursor: None,
            last_cursor: None,
            drag: None,
            last_left_press: None,
            wake_clock: WakeClock::new(Instant::now()),
            dirty: true,
            photos: 0,
            reported: false,
            printed: 0,
            sound: Rc::new(RefCell::new(sound)),
            sound_feed: sound::Feed::default(),
            focus_filter: FocusFilter::default(),
            saves: config.saves,
            ctrl_or_cmd_held: false,
            shift_held: false,
            startup_error: None,
            pending_coop: CoopCliConfig::default(),
        };
        let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
        return event_loop.run_app(&mut app).map_err(|e| e.to_string());
    }

    eprintln!(
        "sims-game: opening Neighborhood shell ({})",
        config.install.display()
    );
    let (tx, rx) = mpsc::channel();
    let install_bg = config.install.clone();
    let saves_bg = config.saves.clone();
    let play_bg = config.play;
    let debug_bg = config.debug;
    let answers_bg = config.answers;
    let mute_bg = config.mute;
    std::thread::spawn(move || {
        #[cfg(any(target_os = "macos", feature = "sound"))]
        let result: StartupResult = if !mute_bg {
            let install_for_mixer = install_bg.clone();
            let (shell_res, mixer_res) = std::thread::scope(|scope| {
                let mixer_handle =
                    scope.spawn(|| sims_game::sound::Mixer::open(&install_for_mixer));
                let shell_res = NeighborhoodShell::open(
                    install_bg.clone(),
                    saves_bg.clone(),
                    1,
                    play_bg,
                    answers_bg,
                    sims_game::Frames::default(),
                )
                .map(|mut shell| {
                    shell.set_debug(debug_bg);
                    shell
                });
                (
                    shell_res,
                    mixer_handle
                        .join()
                        .unwrap_or_else(|_| Err("sound mixer worker panicked".to_owned())),
                )
            });
            shell_res.map(|shell| (shell, Some(mixer_res)))
        } else {
            NeighborhoodShell::open(
                install_bg,
                saves_bg,
                1,
                play_bg,
                answers_bg,
                sims_game::Frames::default(),
            )
            .map(|mut shell| {
                shell.set_debug(debug_bg);
                (shell, None)
            })
        };
        #[cfg(not(any(target_os = "macos", feature = "sound")))]
        let result: StartupResult = {
            let _ = mute_bg;
            NeighborhoodShell::open(
                install_bg,
                saves_bg,
                1,
                play_bg,
                answers_bg,
                sims_game::Frames::default(),
            )
            .map(|mut shell| {
                shell.set_debug(debug_bg);
                (shell, None)
            })
        };
        let _ = tx.send(result);
    });
    let startup = StartupLoading::new(config.install.clone(), config.mute, rx);
    let mut app = App {
        session: Session::Starting(Box::new(startup)),
        last_screen: ScreenKind::Loading,
        last_lot_generation: (0, 0),
        window: None,
        surface: Rc::new(RefCell::new(None)),
        surface_size: Rc::new(Cell::new(None)),
        last_present_sig: Rc::new(Cell::new(None)),
        title_throttle: TitleThrottle::default(),
        redraw_pacer: RedrawPacer::new(None),
        scale: Rc::new(Cell::new(1)),
        cursor: None,
        last_cursor: None,
        drag: None,
        last_left_press: None,
        wake_clock: WakeClock::new(Instant::now()),
        dirty: true,
        photos: 0,
        reported: false,
        printed: 0,
        sound: Rc::new(RefCell::new(Sound::empty())),
        sound_feed: sound::Feed::default(),
        focus_filter: FocusFilter::default(),
        saves: config.saves,
        ctrl_or_cmd_held: false,
        shift_held: false,
        startup_error: None,
        pending_coop: config.coop,
    };
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    if let Some(err) = app.startup_error {
        return Err(err);
    }
    Ok(())
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

/// Active UI focus flags that suppress bare Lot-mode keyboard hotkeys so
/// typing in any text-input surface never triggers pause/speed/mode/camera
/// commands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LotInputFocus {
    cheat_console_open: bool,
    has_dialog: bool,
    album_open: bool,
    coop_lobby_open: bool,
    create_family: bool,
}

/// Returns `true` when any text-input or modal surface has keyboard focus and
/// Lot-mode hotkeys must be suppressed.
fn lot_hotkeys_blocked(focus: LotInputFocus) -> bool {
    focus.cheat_console_open
        || focus.has_dialog
        || focus.album_open
        || focus.coop_lobby_open
        || focus.create_family
}

/// Authentic `Sims.exe` 1.91 Lot-mode keyboard action (`0x4d0ef0` `OnChar`,
/// `0x4d12b1` `OnKeyDown`, `0x51ba1c` `cWinLive::OnKeyDown`, and `UIText.iff`
/// `STR# 161`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LotHotkey {
    TogglePause,
    SetSpeed(Speed),
    Mode(Mode),
    ToggleCameraStrip,
    TogglePanel,
    NextSim,
    CentreSelectedSim,
    ToggleFloor,
    StepWalls(bool),
    RotateHeldOrCamera(bool),
    Move(Move),
    Photo,
    CatalogPage(i32),
    SellHeld,
    Undo,
    Redo,
    QuickSave,
    RestoreSave,
    OpenSaveManager,
    OpenTravelDialog,
    ReturnHome,
    OpenShop,
}

/// Maps a pressed `winit` key event in Lot mode to its authentic `Sims.exe`
/// 1.91 hotkey action, honoring Ctrl/Cmd modifiers, physical backquote /
/// numpad keys, and key-repeat suppression on one-shot toggles.
fn classify_lot_hotkey(
    logical_key: &Key,
    physical_key: PhysicalKey,
    ctrl_or_cmd: bool,
    shift: bool,
    repeat: bool,
) -> Option<LotHotkey> {
    const SCROLL_STEP: i32 = 64;
    if ctrl_or_cmd {
        if let Key::Character(c) = logical_key {
            return match c.as_str() {
                "z" | "Z" if !shift => Some(LotHotkey::Undo),
                "z" | "Z" | "y" | "Y" => Some(LotHotkey::Redo),
                "s" | "S" if !shift && !repeat => Some(LotHotkey::QuickSave),
                "l" | "L" if !shift && !repeat => Some(LotHotkey::OpenSaveManager),
                "t" | "T" if !shift && !repeat => Some(LotHotkey::OpenTravelDialog),
                "h" | "H" if !shift && !repeat => Some(LotHotkey::ReturnHome),
                _ => None,
            };
        }
        return None;
    }

    match physical_key {
        PhysicalKey::Code(KeyCode::Backquote) => {
            return (!repeat).then_some(LotHotkey::TogglePause);
        }
        PhysicalKey::Code(KeyCode::NumpadAdd) => {
            return Some(LotHotkey::Move(Move::ZoomIn));
        }
        PhysicalKey::Code(KeyCode::NumpadSubtract) => {
            return Some(LotHotkey::Move(Move::ZoomOut));
        }
        _ => {}
    }

    match logical_key {
        Key::Named(NamedKey::ArrowLeft) => Some(LotHotkey::Move(Move::Scroll([SCROLL_STEP, 0]))),
        Key::Named(NamedKey::ArrowRight) => Some(LotHotkey::Move(Move::Scroll([-SCROLL_STEP, 0]))),
        Key::Named(NamedKey::ArrowUp) => Some(LotHotkey::Move(Move::Scroll([0, SCROLL_STEP]))),
        Key::Named(NamedKey::ArrowDown) => Some(LotHotkey::Move(Move::Scroll([0, -SCROLL_STEP]))),
        Key::Named(NamedKey::PageUp) => Some(LotHotkey::Move(Move::LevelUp)),
        Key::Named(NamedKey::PageDown) => Some(LotHotkey::Move(Move::LevelDown)),
        Key::Named(NamedKey::Home) => Some(LotHotkey::StepWalls(true)),
        Key::Named(NamedKey::End) => Some(LotHotkey::StepWalls(false)),
        Key::Named(NamedKey::Space) => (!repeat).then_some(LotHotkey::NextSim),
        Key::Named(NamedKey::Enter) => (!repeat).then_some(LotHotkey::CentreSelectedSim),
        Key::Named(NamedKey::Tab) => (!repeat).then_some(LotHotkey::ToggleFloor),
        Key::Named(NamedKey::F1) => (!repeat).then_some(LotHotkey::Mode(Mode::Live)),
        Key::Named(NamedKey::F2) => (!repeat).then_some(LotHotkey::Mode(Mode::Buy)),
        Key::Named(NamedKey::F3) => (!repeat).then_some(LotHotkey::Mode(Mode::Build)),
        Key::Named(NamedKey::F4) => (!repeat).then_some(LotHotkey::ToggleCameraStrip),
        Key::Named(NamedKey::F5) => (!repeat).then_some(LotHotkey::Mode(Mode::Options)),
        Key::Named(NamedKey::F7) => (!repeat).then_some(LotHotkey::OpenTravelDialog),
        Key::Named(NamedKey::F8) => (!repeat).then_some(LotHotkey::OpenShop),
        Key::Named(NamedKey::F9) => (!repeat).then_some(LotHotkey::RestoreSave),
        Key::Named(NamedKey::F10) => (!repeat).then_some(LotHotkey::TogglePanel),
        Key::Named(NamedKey::Delete | NamedKey::Backspace) => Some(LotHotkey::SellHeld),
        Key::Character(c) => match c.as_str() {
            "`" | "~" | "p" | "P" => (!repeat).then_some(LotHotkey::TogglePause),
            "1" => Some(LotHotkey::SetSpeed(Speed::Normal)),
            "2" => Some(LotHotkey::SetSpeed(Speed::Fast)),
            "3" => Some(LotHotkey::SetSpeed(Speed::Ultra)),
            " " => (!repeat).then_some(LotHotkey::NextSim),
            "+" | "=" => Some(LotHotkey::Move(Move::ZoomIn)),
            "-" | "_" => Some(LotHotkey::Move(Move::ZoomOut)),
            "[" => Some(LotHotkey::Move(Move::Rotate(false))),
            "]" => Some(LotHotkey::Move(Move::Rotate(true))),
            "," | "<" => Some(LotHotkey::RotateHeldOrCamera(false)),
            "." | ">" => Some(LotHotkey::RotateHeldOrCamera(true)),
            "c" | "C" => (!repeat).then_some(LotHotkey::Photo),
            "z" | "Z" => Some(LotHotkey::CatalogPage(-1)),
            "x" | "X" => Some(LotHotkey::CatalogPage(1)),
            _ => None,
        },
        _ => None,
    }
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
                buffer[y * bw + x] = row.get(cx).copied().unwrap_or(0);
            }
        }
    }

    #[test]
    fn blit_canvas_matches_reference_across_scales_and_odd_sizes() {
        let mut canvas = sims_render::Canvas::new(11, 7, [0, 0, 0]);
        for y in 0..canvas.height {
            for x in 0..canvas.width {
                canvas.color[y * canvas.width + x] = sims_render::pack_rgb([
                    ((x * 23 + y * 7) & 0xff) as u8,
                    ((x * 11 + y * 31) & 0xff) as u8,
                    ((x * 5 + y * 19 + 42) & 0xff) as u8,
                ]);
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
    fn blit_canvas_rect_matches_full_blit_across_scales_and_edges() {
        let mut base = sims_render::Canvas::new(11, 7, [0, 0, 0]);
        let mut updated = sims_render::Canvas::new(11, 7, [0, 0, 0]);
        for y in 0..base.height {
            for x in 0..base.width {
                let px = sims_render::pack_rgb([
                    ((x * 17 + y * 9) & 0xff) as u8,
                    ((x * 5 + y * 29) & 0xff) as u8,
                    ((x * 13 + y * 3 + 7) & 0xff) as u8,
                ]);
                base.color[y * base.width + x] = px;
                updated.color[y * updated.width + x] = px;
            }
        }
        let rects: &[[i32; 4]] = &[
            [0, 0, 3, 2],   // top-left corner
            [3, 2, 7, 5],   // interior
            [8, 4, 11, 7],  // right + bottom edge (exercises trailing clamp)
            [-2, 1, 2, 10], // partially out-of-bounds
        ];
        for &[x0, y0, x1, y1] in rects {
            for y in y0.clamp(0, 7)..y1.clamp(0, 7) {
                for x in x0.clamp(0, 11)..x1.clamp(0, 11) {
                    updated.color[y as usize * 11 + x as usize] ^= 0x005A_A53C;
                }
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
            (7, 5),
            (15, 10),
        ];
        for scale in [1usize, 2, 3, 4] {
            for (bw, bh) in sizes {
                let mut partial = vec![0u32; bw * bh];
                let mut full = vec![0u32; bw * bh];
                blit_canvas(&base, &mut partial, bw, bh, scale);
                blit_canvas(&updated, &mut full, bw, bh, scale);
                for &rect in rects {
                    let _ = blit_canvas_rect(&updated, &mut partial, bw, bh, scale, rect);
                }
                assert_eq!(
                    partial, full,
                    "partial rect mismatch scale={scale} bw={bw} bh={bh}"
                );
            }
        }
    }

    #[test]
    #[ignore]
    #[allow(clippy::disallowed_methods)]
    fn bench_blit_canvas() {
        let (cw, ch) = (1024usize, 768usize);
        let mut canvas = sims_render::Canvas::new(cw, ch, [0, 0, 0]);
        for y in 0..ch {
            for x in 0..cw {
                canvas.color[y * cw + x] = sims_render::pack_rgb([
                    ((x * 23 + y * 7) & 0xff) as u8,
                    ((x * 11 + y * 31) & 0xff) as u8,
                    ((x * 5 + y * 19 + 42) & 0xff) as u8,
                ]);
            }
        }
        let mut buf_1x = vec![0u32; cw * ch];
        let mut buf_2x = vec![0u32; (cw * 2) * (ch * 2)];
        // Warmup
        blit_canvas(&canvas, &mut buf_1x, cw, ch, 1);
        blit_canvas(&canvas, &mut buf_2x, cw * 2, ch * 2, 2);

        let iters = 200usize;
        let t0 = Instant::now();
        for _ in 0..iters {
            blit_canvas(&canvas, &mut buf_1x, cw, ch, 1);
            std::hint::black_box(&buf_1x);
        }
        let ms_1x = t0.elapsed().as_secs_f64() * 1000.0 / iters as f64;

        let t1 = Instant::now();
        for _ in 0..iters {
            blit_canvas(&canvas, &mut buf_2x, cw * 2, ch * 2, 2);
            std::hint::black_box(&buf_2x);
        }
        let ms_2x = t1.elapsed().as_secs_f64() * 1000.0 / iters as f64;

        println!(
            "BENCH_BLIT 1024x768 1x: {ms_1x:.4} ms/frame, 2x (2048x1536): {ms_2x:.4} ms/frame ({iters} iters, checksum={:08x}/{:08x})",
            buf_1x[0] ^ buf_1x[cw * ch - 1],
            buf_2x[0] ^ buf_2x[(cw * 2) * (ch * 2) - 1],
        );
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

        // Default with no house arg -> Neighborhood shell in graceful play mode, debug off.
        let cfg = parse_launch_args(Vec::<OsString>::new(), env.clone()).unwrap();
        assert_eq!(cfg.install, PathBuf::from("/sims"));
        assert!(cfg.house.is_none());
        assert!(cfg.still.is_none());
        assert!(
            cfg.play,
            "interactive Neighborhood mode defaults to play mode"
        );
        assert!(!cfg.debug, "--debug defaults to false");

        // Explicit --neighborhood --strict --debug -> strict halt mode + debug banners.
        let cfg = parse_launch_args(
            [
                OsString::from("--neighborhood"),
                OsString::from("--strict"),
                OsString::from("--debug"),
            ],
            env.clone(),
        )
        .unwrap();
        assert!(cfg.house.is_none());
        assert!(!cfg.play, "--strict disables play mode");
        assert!(cfg.debug, "--debug enables diagnostic banner mode");

        // Explicit <install> <house> -> direct interactive house launch defaults to play mode.
        let cfg = parse_launch_args(
            [
                OsString::from("/sims"),
                OsString::from("/sims/UserData/Houses/House05.iff"),
            ],
            None,
        )
        .unwrap();
        assert_eq!(
            cfg.house,
            Some(PathBuf::from("/sims/UserData/Houses/House05.iff"))
        );
        assert!(
            cfg.play,
            "interactive direct house launch defaults to play mode"
        );
        assert!(!cfg.debug);

        // Explicit --play remains accepted for backwards compatibility.
        let cfg = parse_launch_args(
            [
                OsString::from("/sims"),
                OsString::from("/sims/UserData/Houses/House05.iff"),
                OsString::from("--play"),
            ],
            None,
        )
        .unwrap();
        assert!(cfg.play);

        // --play and --strict together are rejected.
        assert!(
            parse_launch_args(
                [OsString::from("--play"), OsString::from("--strict")],
                env.clone(),
            )
            .is_err()
        );

        // --png without house -> defaults to Goth house with answers = [3] and strict mode.
        let cfg = parse_launch_args(
            [OsString::from("--png"), OsString::from("out.png")],
            env.clone(),
        )
        .unwrap();
        assert_eq!(cfg.house, Some(goth_house(std::path::Path::new("/sims"))));
        assert_eq!(cfg.answers, vec![3]);
        assert!(cfg.still.is_some());
        assert!(!cfg.play, "headless --png defaults to strict mode");
        assert!(!cfg.debug);

        // --png --play explicitly opts into play mode for still capture.
        let cfg = parse_launch_args(
            [
                OsString::from("--png"),
                OsString::from("out.png"),
                OsString::from("--play"),
            ],
            env.clone(),
        )
        .unwrap();
        assert!(cfg.play);
    }

    #[test]
    fn cli_parses_coop_host_join_and_rejects_invalid_combinations() {
        let env = Some(PathBuf::from("/sims"));

        let host_cfg = parse_launch_args(
            [
                OsString::from("--host"),
                OsString::from("--player-name"),
                OsString::from("Alice"),
                OsString::from("--coop-loopback"),
            ],
            env.clone(),
        )
        .unwrap();
        assert!(host_cfg.coop.host);
        assert!(host_cfg.coop.join.is_none());
        assert_eq!(host_cfg.coop.player_name.as_deref(), Some("Alice"));
        assert!(host_cfg.coop.loopback);

        // Generate a real canonical simscoop1... invite ticket via loopback IrohBridge.
        let mut bridge =
            sims_game::coop::IrohBridge::bind_blocking(sims_net::IrohConfig::Loopback).unwrap();
        let ticket = bridge.invite_ticket_string().to_owned();
        bridge.shutdown();
        let join_cfg = parse_launch_args(
            [
                OsString::from("--join"),
                OsString::from(&ticket),
                OsString::from("--player-name"),
                OsString::from("Bob"),
            ],
            env.clone(),
        )
        .unwrap();
        assert!(!join_cfg.coop.host);
        assert_eq!(join_cfg.coop.join.as_deref(), Some(ticket.as_str()));
        assert_eq!(join_cfg.coop.player_name.as_deref(), Some("Bob"));

        // Invalid ticket fails immediately.
        assert!(
            parse_launch_args(
                [OsString::from("--join"), OsString::from("not-a-ticket")],
                env.clone(),
            )
            .is_err()
        );
        // --host + --join rejected.
        assert!(
            parse_launch_args(
                [
                    OsString::from("--host"),
                    OsString::from("--join"),
                    OsString::from(&ticket),
                ],
                env.clone(),
            )
            .is_err()
        );
        // --png + --host rejected.
        assert!(
            parse_launch_args(
                [
                    OsString::from("--png"),
                    OsString::from("out.png"),
                    OsString::from("--host"),
                ],
                env,
            )
            .is_err()
        );
    }

    #[test]
    fn cli_supports_open_pack_flag_and_automatic_standalone_fallback() {
        // Explicit `--open-pack` discovers workspace open-pack/ even when SIMS_INSTALL is set.
        let cfg = parse_launch_args(
            [OsString::from("--open-pack")],
            Some(PathBuf::from("/sims")),
        )
        .unwrap();
        assert!(cfg.open_pack.is_some());
        assert!(cfg.house.is_none());
        assert!(cfg.play);

        // Explicit `--open-pack <dir> --png out.png --ticks 5`.
        let cfg = parse_launch_args(
            [
                OsString::from("--open-pack"),
                OsString::from("custom-pack"),
                OsString::from("--png"),
                OsString::from("out.png"),
                OsString::from("--ticks"),
                OsString::from("5"),
            ],
            None,
        )
        .unwrap();
        assert_eq!(cfg.open_pack, Some(PathBuf::from("custom-pack")));
        assert_eq!(cfg.still.as_ref().map(|s| s.ticks), Some(5));
        assert!(
            !cfg.play,
            "headless --open-pack --png defaults to strict mode"
        );

        // Automatic fallback when neither <install> nor SIMS_INSTALL is given.
        let cfg = parse_launch_args(Vec::<OsString>::new(), None).unwrap();
        assert!(cfg.open_pack.is_some());
        assert!(cfg.play);

        // Automatic fallback with `--png` and no SIMS_INSTALL.
        let cfg =
            parse_launch_args([OsString::from("--png"), OsString::from("shot.png")], None).unwrap();
        assert!(cfg.open_pack.is_some());
        assert!(cfg.still.is_some());

        // --open-pack + --neighborhood or --host is rejected.
        assert!(
            parse_launch_args(
                [
                    OsString::from("--open-pack"),
                    OsString::from("--neighborhood"),
                ],
                None,
            )
            .is_err()
        );
        assert!(
            parse_launch_args(
                [OsString::from("--open-pack"), OsString::from("--host")],
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn window_attributes_enforces_800x600_min_size_and_hidpi_scale_floor() {
        assert_eq!(MIN_WINDOW_SIZE, [800, 600]);
        assert_eq!(INITIAL_WINDOW_SIZE, [1024, 768]);

        let attrs = window_attributes();
        assert_eq!(
            attrs.inner_size,
            Some(winit::dpi::Size::Logical(LogicalSize::new(1024.0, 768.0)))
        );
        assert_eq!(
            attrs.min_inner_size,
            Some(winit::dpi::Size::Logical(LogicalSize::new(800.0, 600.0)))
        );

        assert_eq!(window_scale(1.0), 1);
        assert_eq!(window_scale(1.25), 1);
        assert_eq!(window_scale(1.5), 1);
        assert_eq!(window_scale(1.75), 1);
        assert_eq!(window_scale(1.99999), 2);
        assert_eq!(window_scale(2.0), 2);
        assert_eq!(window_scale(2.5), 2);
        assert_eq!(window_scale(3.0), 3);
        assert_eq!(window_scale(0.0), 1);
        assert_eq!(window_scale(-2.0), 1);
        assert_eq!(window_scale(f64::NAN), 1);

        // Odd physical rounding right at the 800×600 logical minimum on Retina/HiDPI
        // never drops the canvas below 800×600, while non-boundary odd sizes keep
        // their existing floor behavior.
        assert_eq!(
            canvas_size(PhysicalSize::new(1600, 1200), 2),
            Some([800, 600])
        );
        assert_eq!(
            canvas_size(PhysicalSize::new(1599, 1199), 2),
            Some([800, 600])
        );
        assert_eq!(
            canvas_size(PhysicalSize::new(1598, 1198), 2),
            Some([799, 599])
        );
        assert_eq!(
            canvas_size(PhysicalSize::new(2399, 1799), 3),
            Some([800, 600])
        );
        assert_eq!(
            canvas_size(PhysicalSize::new(1200, 900), window_scale(1.5)),
            Some([1200, 900])
        );
    }

    #[test]
    fn native_lot_hotkey_table_matches_sims_1_91() {
        let unid = PhysicalKey::Code(KeyCode::KeyA);
        let ch = |s: &str| Key::Character(s.into());

        // Pause: backquote / tilde / physical Backquote / P (non-repeat only).
        assert_eq!(
            classify_lot_hotkey(&ch("`"), unid, false, false, false),
            Some(LotHotkey::TogglePause)
        );
        assert_eq!(
            classify_lot_hotkey(&ch("~"), unid, false, true, false),
            Some(LotHotkey::TogglePause)
        );
        assert_eq!(
            classify_lot_hotkey(
                &ch("§"),
                PhysicalKey::Code(KeyCode::Backquote),
                false,
                false,
                false
            ),
            Some(LotHotkey::TogglePause)
        );
        assert_eq!(
            classify_lot_hotkey(&ch("p"), unid, false, false, false),
            Some(LotHotkey::TogglePause)
        );
        assert_eq!(
            classify_lot_hotkey(&ch("P"), unid, false, true, false),
            Some(LotHotkey::TogglePause)
        );
        assert_eq!(
            classify_lot_hotkey(&ch("p"), unid, false, false, true),
            None
        );

        // Speed 1 / 2 / 3 (NOT walls!).
        assert_eq!(
            classify_lot_hotkey(&ch("1"), unid, false, false, false),
            Some(LotHotkey::SetSpeed(Speed::Normal))
        );
        assert_eq!(
            classify_lot_hotkey(&ch("2"), unid, false, false, false),
            Some(LotHotkey::SetSpeed(Speed::Fast))
        );
        assert_eq!(
            classify_lot_hotkey(&ch("3"), unid, false, false, false),
            Some(LotHotkey::SetSpeed(Speed::Ultra))
        );

        // Modes & panels: F1-F5 + F10, plus Ctrl+S save.
        assert_eq!(
            classify_lot_hotkey(&Key::Named(NamedKey::F1), unid, false, false, false),
            Some(LotHotkey::Mode(Mode::Live))
        );
        assert_eq!(
            classify_lot_hotkey(&Key::Named(NamedKey::F2), unid, false, false, false),
            Some(LotHotkey::Mode(Mode::Buy))
        );
        assert_eq!(
            classify_lot_hotkey(&Key::Named(NamedKey::F3), unid, false, false, false),
            Some(LotHotkey::Mode(Mode::Build))
        );
        assert_eq!(
            classify_lot_hotkey(&Key::Named(NamedKey::F4), unid, false, false, false),
            Some(LotHotkey::ToggleCameraStrip)
        );
        assert_eq!(
            classify_lot_hotkey(&Key::Named(NamedKey::F5), unid, false, false, false),
            Some(LotHotkey::Mode(Mode::Options))
        );
        assert_eq!(
            classify_lot_hotkey(&Key::Named(NamedKey::F10), unid, false, false, false),
            Some(LotHotkey::TogglePanel)
        );
        assert_eq!(
            classify_lot_hotkey(&ch("s"), unid, true, false, false),
            Some(LotHotkey::QuickSave)
        );

        // Sim selection, camera, walls, floor, zoom, rotate, snapshot, catalog paging.
        assert_eq!(
            classify_lot_hotkey(&Key::Named(NamedKey::Space), unid, false, false, false),
            Some(LotHotkey::NextSim)
        );
        assert_eq!(
            classify_lot_hotkey(&Key::Named(NamedKey::Enter), unid, false, false, false),
            Some(LotHotkey::CentreSelectedSim)
        );
        assert_eq!(
            classify_lot_hotkey(&Key::Named(NamedKey::Tab), unid, false, false, false),
            Some(LotHotkey::ToggleFloor)
        );
        assert_eq!(
            classify_lot_hotkey(&Key::Named(NamedKey::Home), unid, false, false, false),
            Some(LotHotkey::StepWalls(true))
        );
        assert_eq!(
            classify_lot_hotkey(&Key::Named(NamedKey::End), unid, false, false, false),
            Some(LotHotkey::StepWalls(false))
        );
        assert_eq!(
            classify_lot_hotkey(&Key::Named(NamedKey::PageUp), unid, false, false, false),
            Some(LotHotkey::Move(Move::LevelUp))
        );
        assert_eq!(
            classify_lot_hotkey(&Key::Named(NamedKey::PageDown), unid, false, false, false),
            Some(LotHotkey::Move(Move::LevelDown))
        );
        assert_eq!(
            classify_lot_hotkey(&ch("+"), unid, false, true, false),
            Some(LotHotkey::Move(Move::ZoomIn))
        );
        assert_eq!(
            classify_lot_hotkey(&ch("="), unid, false, false, false),
            Some(LotHotkey::Move(Move::ZoomIn))
        );
        assert_eq!(
            classify_lot_hotkey(
                &ch("+"),
                PhysicalKey::Code(KeyCode::NumpadAdd),
                false,
                false,
                false
            ),
            Some(LotHotkey::Move(Move::ZoomIn))
        );
        assert_eq!(
            classify_lot_hotkey(&ch("-"), unid, false, false, false),
            Some(LotHotkey::Move(Move::ZoomOut))
        );
        assert_eq!(
            classify_lot_hotkey(
                &ch("-"),
                PhysicalKey::Code(KeyCode::NumpadSubtract),
                false,
                false,
                false
            ),
            Some(LotHotkey::Move(Move::ZoomOut))
        );
        assert_eq!(
            classify_lot_hotkey(&ch(","), unid, false, false, false),
            Some(LotHotkey::RotateHeldOrCamera(false))
        );
        assert_eq!(
            classify_lot_hotkey(&ch("."), unid, false, false, false),
            Some(LotHotkey::RotateHeldOrCamera(true))
        );
        assert_eq!(
            classify_lot_hotkey(&ch("c"), unid, false, false, false),
            Some(LotHotkey::Photo)
        );
        assert_eq!(
            classify_lot_hotkey(&ch("z"), unid, false, false, false),
            Some(LotHotkey::CatalogPage(-1))
        );
        assert_eq!(
            classify_lot_hotkey(&ch("x"), unid, false, false, false),
            Some(LotHotkey::CatalogPage(1))
        );
        assert_eq!(
            classify_lot_hotkey(&Key::Named(NamedKey::Delete), unid, false, false, false),
            Some(LotHotkey::SellHeld)
        );
        assert_eq!(
            classify_lot_hotkey(&ch("z"), unid, true, false, false),
            Some(LotHotkey::Undo)
        );
        assert_eq!(
            classify_lot_hotkey(&ch("z"), unid, true, true, false),
            Some(LotHotkey::Redo)
        );
        assert_eq!(
            classify_lot_hotkey(&ch("y"), unid, true, false, false),
            Some(LotHotkey::Redo)
        );
        // Ctrl+P / Ctrl+1 / Ctrl+C never fire bare Lot hotkeys.
        assert_eq!(
            classify_lot_hotkey(&ch("p"), unid, true, false, false),
            None
        );
        assert_eq!(
            classify_lot_hotkey(&ch("1"), unid, true, false, false),
            None
        );
        assert_eq!(
            classify_lot_hotkey(&ch("c"), unid, true, false, false),
            None
        );
    }

    #[test]
    fn text_input_guard_suppresses_hotkeys_in_all_five_text_surfaces() {
        assert!(!lot_hotkeys_blocked(LotInputFocus::default()));
        assert!(lot_hotkeys_blocked(LotInputFocus {
            cheat_console_open: true,
            ..LotInputFocus::default()
        }));
        assert!(lot_hotkeys_blocked(LotInputFocus {
            has_dialog: true,
            ..LotInputFocus::default()
        }));
        assert!(lot_hotkeys_blocked(LotInputFocus {
            album_open: true,
            ..LotInputFocus::default()
        }));
        assert!(lot_hotkeys_blocked(LotInputFocus {
            coop_lobby_open: true,
            ..LotInputFocus::default()
        }));
        assert!(lot_hotkeys_blocked(LotInputFocus {
            create_family: true,
            ..LotInputFocus::default()
        }));
    }
}
