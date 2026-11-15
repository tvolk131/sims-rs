//! The game's window.
//!
//! ```text
//! sims-game [<install>] [<house file>] [--answers A,B,...]
//!           [--png <file> [--ticks N] [--size WxH]]
//! ```
//!
//! The install defaults to `SIMS_INSTALL`, and the house to the Goths'
//! (`UserData/Houses/House05.iff`). `--answers` answers the dialogs that
//! offer a choice, in order, as `sims boot` does; for the Goths' house it
//! defaults to `3` (No to the Help System).
//!
//! The lot runs at normal speed, a tick every 50 milliseconds. The
//! panel's speed, pause and camera buttons work. Keys: `+`/`-` or the
//! wheel zoom, `[`/`]` or `,`/`.` rotate, Page Up and Page Down change the
//! floor, `1`/`2`/`3` walls up, cutaway and down, the arrows or a
//! right-button drag scroll, space pauses, Escape quits.
//!
//! `--png` opens no window: it runs `--ticks` ticks (default 0) as fast as
//! they go, draws one frame of `--size` (default 800x600) and saves it.
//!
//! The window is drawn at the original's pixel size and scaled up by
//! whole pixels on high-DPI screens.

// A window shell: it reads the wall clock to pace the lot.
#![allow(clippy::disallowed_methods)]

use sims_game::game::goth_house;
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

/// A frame drawn without a window.
struct Still {
    file: PathBuf,
    ticks: u32,
    size: [i32; 2],
}

fn options() -> Result<(Options, Option<Still>), String> {
    let usage = "usage: sims-game [<install>] [<house file>] [--answers A,B,...] \
                 [--png <file> [--ticks N] [--size WxH]]";
    let mut paths = Vec::new();
    let mut answers = None;
    let mut png = None;
    let mut ticks = 0;
    let mut size = [800, 600];
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let mut text = |name: &str| -> Result<String, String> {
            args.next()
                .and_then(|a| a.into_string().ok())
                .ok_or_else(|| format!("{name} needs a value"))
        };
        if arg == "--png" {
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
    let answers = answers.unwrap_or_else(|| if house == goths { vec![3] } else { Vec::new() });
    let still = png.map(|file| Still { file, ticks, size });
    Ok((
        Options {
            install,
            house,
            answers,
        },
        still,
    ))
}

struct App {
    game: Game,
    window: Option<Rc<Window>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    scale: u32,
    cursor: PhysicalPosition<f64>,
    drag: Option<PhysicalPosition<f64>>,
    last: Instant,
    dirty: bool,
    photos: usize,
    /// The stop already printed, so it is printed once.
    reported: bool,
}

impl App {
    fn point(&self, position: PhysicalPosition<f64>) -> [i32; 2] {
        let scale = f64::from(self.scale.max(1));
        [(position.x / scale) as i32, (position.y / scale) as i32]
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
                self.game.pace.toggle_pause();
                self.changed(true);
                return;
            }
            Key::Character(c) => match c.as_str() {
                "+" | "=" => Move::ZoomIn,
                "-" | "_" => Move::ZoomOut,
                "[" | "," => Move::Rotate(false),
                "]" | "." => Move::Rotate(true),
                "1" => Move::Walls(Walls::Up),
                "2" => Move::Walls(Walls::Cutaway),
                "3" => Move::Walls(Walls::Down),
                _ => return,
            },
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
        let ran = self.game.advance(micros);
        let mut redraw = ran > 0;
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
        let canvas_size = [size.width / scale, size.height / scale]
            .map(|n| i32::try_from(n).unwrap_or(i32::MAX).max(1));
        if self.game.resize(canvas_size) {
            self.dirty = true;
        }
        if self.dirty {
            self.game.render();
            self.dirty = false;
            let lot = self.game.lot();
            window.set_title(&format!(
                "sims-game: tick {}{}",
                lot.ticks(),
                if self.game.stopped().is_some() {
                    ", stopped"
                } else if self.game.pace.paused {
                    ", paused"
                } else {
                    ""
                }
            ));
        }
        let Some(surface) = &mut self.surface else {
            return;
        };
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        let canvas = self.game.canvas();
        let (bw, bh) = (size.width as usize, size.height as usize);
        let scale = scale as usize;
        for y in 0..bh {
            let cy = (y / scale).min(canvas.height.saturating_sub(1));
            let row = &canvas.color[cy * canvas.width..(cy + 1) * canvas.width];
            for x in 0..bw {
                let cx = (x / scale).min(canvas.width.saturating_sub(1));
                let [r, g, b] = row.get(cx).copied().unwrap_or([0, 0, 0]);
                buffer[y * bw + x] = (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
            }
        }
        let _ = buffer.present();
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
            Ok(s) => self.surface = Some(s),
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
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(_) => self.changed(true),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor.round().max(1.0) as u32;
                self.changed(true);
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if event.logical_key == Key::Named(NamedKey::Escape) {
                    event_loop.exit();
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
                    let outcome = self.game.press(self.point(self.cursor));
                    self.changed(outcome.redraw);
                    if outcome.photo {
                        self.photo();
                    }
                }
                (MouseButton::Right, ElementState::Pressed) => self.drag = Some(self.cursor),
                (MouseButton::Right, ElementState::Released) => self.drag = None,
                _ => {}
            },
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                let point = self.point(position);
                let redraw = self.drag.is_none() && self.game.hover(point);
                self.changed(redraw);
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
    let (options, still) = options()?;
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
        scale: 1,
        cursor: PhysicalPosition::new(0.0, 0.0),
        drag: None,
        last: Instant::now(),
        dirty: true,
        photos: 0,
        reported: false,
    };
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.run_app(&mut app).map_err(|e| e.to_string())
}
