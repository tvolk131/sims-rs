//! The game's window.
//!
//! ```text
//! sims-game [<install>] [<house file>] [--answers A,B,...] [--mute]
//!           [--play] [--saves <file>] [--png <file> [--ticks N] [--size WxH]]
//! ```
//!
//! The install defaults to `SIMS_INSTALL`, and the house to the Goths'
//! (`UserData/Houses/House05.iff`). `--answers` answers the dialogs that
//! offer a choice, in order, as `sims boot` does. Once they run out, a
//! dialog waits for the player, as the original's does, and a press on
//! one of its buttons answers it. `--png` has nobody to answer, so there
//! the Goths' house defaults to `3` (No to the Help System).
//!
//! The lot runs at the original's pace: a tick every 42 milliseconds at
//! normal speed, 14 at fast and 2 at ultra. The panel's speed, pause and
//! camera buttons work; a paused lot goes on ticking with its clock
//! standing, as the original's does. Keys: `+`/`-` or the
//! wheel zoom, `[`/`]` or `,`/`.` rotate, Page Up and Page Down change the
//! floor, `1`/`2`/`3` walls up, cutaway and down, the arrows or a
//! right-button drag scroll, space pauses, Escape quits.
//!
//! F5 saves the lot and F9 goes back to the save (`sims_vm::boot::save`),
//! one save for each house, kept in the REDB file `--saves` (default
//! `sims-game-saves.redb` in the current directory). This is this
//! reimplementation's save, not the original's house file: the original
//! cannot open it. A save made while a dialog waits shows the dialog
//! again when restored.
//!
//! A press on a portrait selects that Sim.
//!
//! A left click on a person or an object opens its pie menu for the
//! selected Sim (`sims_ui::pie`, around the click); a click on an entry
//! queues it, one on a submenu's entry opens the submenu, and a click
//! anywhere else, or Escape, closes the menu. The pointer moves to the
//! middle of a submenu that opens, and of a menu that had to move to
//! stay in the window, as the original moves it. Escape quits only when no menu is open, and a
//! held Escape's repeats do nothing.
//!
//! The lot runs in strict mode: the first error stops it, and the error
//! is written across the top. `--play` runs it in play mode instead: an
//! object whose tick fails is reset and the lot goes on, and each new
//! error is printed and shown across the top for a while.
//!
//! Sound plays on the default output device (on macOS, or built with the
//! `sound` feature) unless `--mute` is given.
//!
//! `--png` opens no window: it runs `--ticks` ticks (default 0) as fast as
//! they go, draws one frame of `--size` (default 800x600) and saves it.
//!
//! The window is drawn at the original's pixel size and scaled up by
//! whole pixels on high-DPI screens.

// A window shell: it reads the wall clock to pace the lot.
#![allow(clippy::disallowed_methods)]

use sims_game::game::goth_house;
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

fn options() -> Result<(Options, Option<Still>, bool, PathBuf), String> {
    let usage = "usage: sims-game [<install>] [<house file>] [--answers A,B,...] [--mute] \
                 [--play] [--saves <file>] [--png <file> [--ticks N] [--size WxH]]";
    let mut paths = Vec::new();
    let mut answers = None;
    let mut png = None;
    let mut ticks = 0;
    let mut size = [800, 600];
    let mut mute = false;
    let mut play = false;
    let mut saves = PathBuf::from("sims-game-saves.redb");
    let mut args = std::env::args_os().skip(1);
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
        None => std::env::var_os("SIMS_INSTALL")
            .map(PathBuf::from)
            .ok_or("give the install, or set SIMS_INSTALL")?,
    };
    let goths = goth_house(&install);
    let house = paths.next().unwrap_or_else(|| goths.clone());
    if paths.next().is_some() {
        return Err(usage.into());
    }
    // In the window the player answers; a still has nobody to.
    let answers = answers.unwrap_or_else(|| {
        if house == goths && png.is_some() {
            vec![3]
        } else {
            Vec::new()
        }
    });
    let still = png.map(|file| Still { file, ticks, size });
    Ok((
        Options {
            install,
            house,
            answers,
            play,
            frames: sims_game::Frames::default(),
        },
        still,
        mute,
        saves,
    ))
}

struct App {
    game: Game,
    window: Option<Rc<Window>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    surface_size: Option<(NonZeroU32, NonZeroU32)>,
    last_title: String,
    scale: u32,
    cursor: Option<PhysicalPosition<f64>>,
    drag: Option<PhysicalPosition<f64>>,
    last: Instant,
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
    /// The REDB file of saves, and this house's name in it.
    saves: PathBuf,
    slot: String,
    /// Modifier keys tracked from `WindowEvent::ModifiersChanged` for Buy Mode
    /// shortcuts (`Ctrl/Cmd+Z` Undo, `Ctrl+Y`/`Ctrl/Cmd+Shift+Z` Redo).
    ctrl_or_cmd_held: bool,
    shift_held: bool,
}

/// Filters startup `WindowEvent::Focused` notifications so ordinary window
/// launch sends no kind 25 (matching `0x4eee14` + `0x4f7610`). Winit 0.30 on
/// macOS queues a synthetic `Focused(false)` when creating the window delegate
/// before `makeKeyAndOrderFront` emits `Focused(true)`; dropping pre-activation
/// events until the window first becomes active prevents a spurious
/// off/on pair that would pause ambience and arm the 2-second nudge countdown.
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

impl App {
    fn sync_size(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        if let Some(canvas) = canvas_size(size, self.scale) {
            self.game.resize(canvas);
        }
        if self
            .cursor
            .is_some_and(|c| canvas_point(c, size, self.scale).is_none())
        {
            self.cursor = None;
            let _ = self.game.leave();
        }
    }

    fn live_point(&mut self) -> Option<[i32; 2]> {
        let size = self.window.as_ref()?.inner_size();
        if let Some(canvas) = canvas_size(size, self.scale) {
            self.game.resize(canvas);
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
        let Some([x, y]) = self.game.take_pointer() else {
            return;
        };
        // The middle of that canvas pixel, so it reads back as the same.
        let scale = f64::from(self.scale.max(1));
        let position =
            PhysicalPosition::new((f64::from(x) + 0.5) * scale, (f64::from(y) + 0.5) * scale);
        if let Some(w) = &self.window {
            // Not every platform lets a program move the pointer; the
            // menu still works where it is.
            if w.set_cursor_position(position).is_ok() {
                self.cursor = Some(position);
            }
        }
        if let Some(point) = self.live_point() {
            let redraw = self.game.hover(point);
            self.changed(redraw);
        }
    }

    fn changed(&mut self, redraw: bool) {
        if redraw {
            self.dirty = true;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }

    fn photo(&mut self) {
        let path = PathBuf::from(format!("sims-game-{}.png", self.photos));
        self.photos += 1;
        match std::fs::write(&path, png::encode(self.game.canvas())) {
            Ok(()) => eprintln!("saved {}", path.display()),
            Err(e) => eprintln!("could not save {}: {e}", path.display()),
        }
    }

    /// Saves the lot (F5) or goes back to its save (F9).
    fn save(&mut self, restore: bool) {
        let store = match sims_sim::Snapshots::open(&self.saves) {
            Ok(store) => store,
            Err(e) => {
                eprintln!("sims-game: {}: {e}", self.saves.display());
                return;
            }
        };
        if !restore {
            self.game.save_to(&store, &self.slot);
        } else if self.game.restore_from(&store, &self.slot) {
            // The errors and the stop are the restored lot's.
            self.printed = 0;
            self.reported = false;
            self.last = Instant::now();
        }
        self.changed(true);
    }

    fn key(&mut self, key: &Key) {
        let step = 64;
        let command = match key {
            Key::Named(NamedKey::ArrowLeft) => Move::Scroll([step, 0]),
            Key::Named(NamedKey::ArrowRight) => Move::Scroll([-step, 0]),
            Key::Named(NamedKey::ArrowUp) => Move::Scroll([0, step]),
            Key::Named(NamedKey::ArrowDown) => Move::Scroll([0, -step]),
            Key::Named(NamedKey::PageUp) => Move::LevelUp,
            Key::Named(NamedKey::PageDown) => Move::LevelDown,
            Key::Named(NamedKey::Space) => {
                self.game.toggle_pause();
                self.changed(true);
                return;
            }
            Key::Named(NamedKey::F5) => return self.save(false),
            Key::Named(NamedKey::F9) => return self.save(true),
            Key::Character(c) => {
                if self.ctrl_or_cmd_held {
                    match c.as_str() {
                        "z" | "Z" if !self.shift_held => {
                            if self.game.undo_buy() {
                                self.changed(true);
                            }
                            return;
                        }
                        "z" | "Z" | "y" | "Y" => {
                            if self.game.redo_buy() {
                                self.changed(true);
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
                        if self.game.rotate_held(false) {
                            self.changed(true);
                            return;
                        }
                        Move::Rotate(false)
                    }
                    "." | ">" => {
                        if self.game.rotate_held(true) {
                            self.changed(true);
                            return;
                        }
                        Move::Rotate(true)
                    }
                    "1" => Move::Walls(Walls::Up),
                    "2" => Move::Walls(Walls::Cutaway),
                    "3" => Move::Walls(Walls::Down),
                    _ => return,
                }
            }
            _ => return,
        };
        let redraw = self.game.apply(command);
        self.changed(redraw);
    }

    /// Steps the lot for the time since the last call.
    fn tick(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last);
        self.last = now;
        let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        let waited = self.game.asking().is_some();
        let ran = self.game.advance(micros);
        // A dialog shown at the first tick due is drawn though none ran.
        let mut redraw = ran > 0 || (!waited && self.game.asking().is_some());
        // The plumb bob spins and the Live-mode pause button blinks by the
        // time passed, ticks or not.
        redraw |= self.game.spin_due() || self.game.blink_due();
        self.sound_feed
            .update(&mut self.game, |input| self.sound.send(input));
        for error in &self.game.errors()[self.printed..] {
            eprintln!("sims-game: error caught: {error}");
        }
        self.printed = self.game.errors().len();
        if !self.reported
            && let Some(stop) = self.game.stopped()
        {
            eprintln!("sims-game: {stop}");
            self.reported = true;
            // The failed step may have changed the lot, and the banner
            // shows now.
            redraw = true;
        }
        self.changed(redraw);
    }

    fn redraw(&mut self) {
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
        if self.game.resize(canvas_size) {
            self.dirty = true;
        }
        if self.dirty {
            self.game.render();
            self.dirty = false;
            let lot = self.game.lot();
            let title = format!(
                "sims-game: tick {}{}",
                lot.ticks(),
                if self.game.stopped().is_some() {
                    ", stopped"
                } else if self.game.paused() {
                    ", paused"
                } else {
                    ""
                }
            );
            if title != self.last_title {
                window.set_title(&title);
                self.last_title = title;
            }
        }
        let Some(surface) = &mut self.surface else {
            return;
        };
        if self.surface_size != Some((w, h)) {
            if surface.resize(w, h).is_err() {
                return;
            }
            self.surface_size = Some((w, h));
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            self.surface_size = None;
            return;
        };
        let canvas = self.game.canvas();
        let (bw, bh) = (size.width as usize, size.height as usize);
        blit_canvas(canvas, &mut buffer, bw, bh, scale as usize);
        let _ = buffer.present();
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
        self.last = Instant::now();
        self.dirty = true;
        window.request_redraw();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.tick();
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + WAKE));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(focused) => {
                if let Some(focused) = self.focus_filter.filter(focused)
                    && self
                        .sound_feed
                        .set_focused(focused, &mut self.game, |input| self.sound.send(input))
                {
                    // Drop any wall-clock time spent paused or stalled while
                    // unfocused so the first tick after resume does not catch up.
                    self.last = Instant::now();
                    self.changed(true);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(size) => {
                self.sync_size(size);
                self.changed(true);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor.round().max(1.0) as u32;
                self.surface_size = None;
                if let Some(size) = self.window.as_ref().map(|w| w.inner_size()) {
                    self.sync_size(size);
                }
                self.changed(true);
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                self.shift_held = state.shift_key();
                self.ctrl_or_cmd_held = state.control_key() || state.super_key();
                self.game.set_shift(self.shift_held);
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if event.logical_key == Key::Named(NamedKey::Escape) {
                    // Escape closes the pie menu, else quits. A repeat
                    // does neither, so holding Escape to close a menu does
                    // not quit.
                    if event.repeat {
                    } else if self.game.close_menu() || self.game.cancel_buy() {
                        self.changed(true);
                    } else {
                        event_loop.exit();
                    }
                } else {
                    self.key(&event.logical_key);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => f64::from(y),
                    MouseScrollDelta::PixelDelta(p) => p.y,
                };
                let command = if y > 0.0 {
                    Move::ZoomIn
                } else if y < 0.0 {
                    Move::ZoomOut
                } else {
                    return;
                };
                let redraw = self.game.apply(command);
                self.changed(redraw);
            }
            WindowEvent::MouseInput { state, button, .. } => match (button, state) {
                (MouseButton::Left, ElementState::Pressed) => {
                    if let Some(point) = self.live_point() {
                        let outcome = self.game.press(point);
                        self.changed(outcome.redraw);
                        self.move_pointer();
                        if outcome.photo {
                            self.photo();
                        }
                    }
                }
                (MouseButton::Left, ElementState::Released) => {
                    let outcome = match self.live_point() {
                        Some(point) => self.game.release(point),
                        None => self.game.release_outside(),
                    };
                    self.changed(outcome.redraw);
                    self.move_pointer();
                }
                (MouseButton::Right, ElementState::Pressed) => {
                    self.drag = self.live_point().and(self.cursor);
                }
                (MouseButton::Right, ElementState::Released) => self.drag = None,
                _ => {}
            },
            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                let redraw = self.game.leave();
                self.changed(redraw);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let point = self.window.as_ref().and_then(|w| {
                    let size = w.inner_size();
                    if let Some(canvas) = canvas_size(size, self.scale) {
                        self.game.resize(canvas);
                    }
                    canvas_point(position, size, self.scale)
                });
                if let Some(point) = point {
                    self.cursor = Some(position);
                    let redraw = self.drag.is_none() && self.game.hover(point);
                    self.changed(redraw);
                } else {
                    self.cursor = None;
                    let redraw = self.drag.is_none() && self.game.leave();
                    self.changed(redraw);
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
                        let redraw = self.game.apply(Move::Scroll([dx, dy]));
                        self.changed(redraw);
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
    let (options, still, mute, saves) = options()?;
    eprintln!("sims-game: booting {}", options.house.display());
    let started = Instant::now();
    let mut game = Game::open(&options)?;
    eprintln!(
        "sims-game: booted in {:.1} s",
        started.elapsed().as_secs_f32()
    );
    for problem in game.problems() {
        eprintln!("sims-game: {problem}");
    }
    if let Some(still) = still {
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
    let mut app = App {
        game,
        window: None,
        surface: None,
        surface_size: None,
        last_title: String::new(),
        scale: 1,
        cursor: None,
        drag: None,
        last: Instant::now(),
        dirty: true,
        photos: 0,
        reported: false,
        printed: 0,
        sound: Sound::start(&options.install, mute),
        sound_feed: sound::Feed::default(),
        focus_filter: FocusFilter::default(),
        saves,
        slot: slot(&options.house),
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

#[cfg(test)]
mod tests {
    use super::*;
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
    fn focus_filter_drops_macos_startup_false_then_forwards_transitions() {
        // Winit 0.30 on macOS queues synthetic Focused(false) during window
        // creation before makeKeyAndOrderFront emits Focused(true).
        let mut filter = FocusFilter::default();
        assert_eq!(filter.filter(false), None);
        assert_eq!(filter.filter(true), None);
        assert_eq!(filter.filter(false), Some(false));
        assert_eq!(filter.filter(true), Some(true));

        // Platforms starting directly with Focused(true) also send no startup kind 25.
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
}
