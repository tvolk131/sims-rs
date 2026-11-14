//! A small window that plays the people of a saved house.
//!
//! ```text
//! people-viewer <install> [house file] [--snapshot <file>] [--walk]
//!               [--png <file> [--at <milliseconds>]]
//! ```
//!
//! Without a snapshot, the house's people stand side by side, facing the
//! viewer, and play the standing loop; `W` makes them walk on the spot.
//! With a snapshot (a `sims_sim::Snapshots` file, such as the one
//! `sims boot --snapshot` writes, read under the name `boot`),
//! each person the simulation has placed is drawn where it is, playing the
//! clips its animator holds at that moment. The people come from the
//! snapshot's own person objects, so people a script made after the load
//! are drawn too (in their type's default outfit while their record names
//! none). The house file then only names the neighborhood whose characters
//! are registered.
//!
//! `--png` draws one 800 by 600 frame, `--at` milliseconds in, to a PNG
//! file instead of opening a window.
//!
//! Keys: `W` walk or stand, left and right arrows turn the view by an
//! eighth of a turn, `+` and `-` zoom, `Esc` quits.
//!
//! Built only with the `viewer` feature:
//! `cargo run -p sims-people --features viewer --bin people-viewer -- <install>`.

// A window shell: it reads the wall clock to play clips in real time, and
// nothing it computes reaches the simulation.
#![allow(clippy::disallowed_methods)]

use sims_core::F32;
use sims_people::math::Vec3;
use sims_people::raster::{Camera, Canvas};
use sims_people::{
    Assets, Body, Cast, Library, Player, Resident, base_clip, placement, residents, residents_of,
    walk_clip,
};
use sims_sim::World;
use sims_sim::animator::{Animator, Layer};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const BACKGROUND: [u8; 4] = [38, 40, 56, 255];

struct Person {
    resident: Resident,
    body: Body,
    base: String,
}

struct Viewer {
    /// Loads each clip once. The assets and the clip library live as long
    /// as the program.
    player: Player<'static>,
    people: Vec<Person>,
    world: Option<World>,
    start: Instant,
    walking: bool,
    /// Eighths of a turn the view is turned by.
    turn: i32,
    scale: f32,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("people-viewer: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut install = None;
    let mut house = None;
    let mut snapshot = None;
    let mut png = None;
    let mut at = 0;
    let mut walking = false;
    while let Some(arg) = args.next() {
        if arg == "--snapshot" {
            snapshot = Some(args.next().ok_or("--snapshot needs a file")?);
        } else if arg == "--png" {
            png = Some(args.next().ok_or("--png needs a file")?);
        } else if arg == "--at" {
            let ms = args.next().ok_or("--at needs milliseconds")?;
            at = ms.parse().map_err(|e| format!("--at {ms}: {e}"))?;
        } else if arg == "--walk" {
            walking = true;
        } else if install.is_none() {
            install = Some(PathBuf::from(arg));
        } else if house.is_none() {
            house = Some(PathBuf::from(arg));
        } else {
            return Err(format!("unexpected argument {arg:?}"));
        }
    }
    let install = install.ok_or("usage: people-viewer <install> [house] [--snapshot <file>]")?;
    let house = house.unwrap_or_else(|| install.join("UserData/Houses/House05.iff"));
    let assets: &'static Assets = Box::leak(Box::new(Assets::scan(&install)?));
    let library: &'static Library = Box::leak(Box::new(Library::scan(assets)?));
    let world = match snapshot {
        Some(file) => Some(
            sims_sim::Snapshots::open(Path::new(&file))
                .and_then(|s| s.load("boot"))
                .map_err(|e| format!("{file}: {e}"))?
                .ok_or_else(|| format!("{file} holds no snapshot named boot"))?,
        ),
        None => None,
    };
    // With a snapshot, the people it holds now, including those a script
    // made after the load; otherwise the saved house's.
    let found = match &world {
        Some(world) => residents_of(&Cast::of_house(&install, &house)?, world)?,
        None => residents(&install, &house)?,
    };
    let mut people = Vec::new();
    for resident in found {
        // One person whose files are missing does not stop the others.
        let body = match Body::load(assets, &resident.skeleton, &resident.outfit) {
            Ok(body) => body,
            Err(e) => {
                eprintln!("people-viewer: person {} is not drawn: {e}", resident.id);
                continue;
            }
        };
        let base = base_clip(&resident.person, &resident.skeleton);
        people.push(Person {
            resident,
            body,
            base,
        });
    }
    let mut viewer = Viewer {
        player: Player::new(assets, library),
        people,
        world,
        start: Instant::now(),
        walking,
        turn: 0,
        scale: 40.0,
        window: None,
        surface: None,
    };
    if let Some(file) = png {
        let canvas = viewer.render(800, 600, at)?;
        return std::fs::write(&file, canvas.png()).map_err(|e| format!("{file}: {e}"));
    }
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.run_app(&mut viewer).map_err(|e| e.to_string())
}

/// An animator for the demonstration: at `x` along the line, facing the
/// viewer of the house view, standing or walking on the spot.
fn demonstration(x: f32, clock: i32, walk: Option<(&str, f32, u32)>) -> Animator {
    let mut animator = Animator::new(clock);
    animator.position = [x, -x, 7.0].map(F32::from_f32);
    animator.base = true;
    if let Some((clip, duration, frames)) = walk {
        let mut layer = Layer::start(clip, F32::from_f32(duration), frames, true, 0, false, clock);
        layer.mode = 1;
        let phase = (f64::from(clock) / f64::from(duration)).fract();
        layer.phase = F32::from_f32(phase as f32);
        animator.foreground = Some(layer);
    }
    animator
}

impl Viewer {
    /// Draws the scene `clock` milliseconds in.
    fn render(&mut self, width: u32, height: u32, clock: i32) -> Result<Canvas, String> {
        let mut canvas = Canvas::new(width, height, BACKGROUND);
        let player = &mut self.player;
        // Each person's animator and the body it is drawn with.
        let mut scene: Vec<(&Person, Animator)> = Vec::new();
        match &self.world {
            Some(world) => {
                for (id, animator) in &world.animators {
                    if let Some(person) = self.people.iter().find(|p| p.resident.id == *id) {
                        scene.push((person, animator.clone()));
                    }
                }
            }
            None => {
                let n = self.people.len() as f32;
                for (i, person) in self.people.iter().enumerate() {
                    // Side by side across the view, two thirds of a tile
                    // apart, in sixteenths of a tile.
                    let x = (i as f32 - (n - 1.0) / 2.0) * 16.0 / 1.5 / 2f32.sqrt();
                    let walk = if self.walking {
                        let name = walk_clip(&person.resident.skeleton);
                        let clip = player.clip(&name)?;
                        Some((name, clip.animation.duration_ms, clip.animation.frames))
                    } else {
                        None
                    };
                    let walk = walk.as_ref().map(|(n, d, f)| (n.as_str(), *d, *f));
                    scene.push((person, demonstration(x, clock, walk)));
                }
            }
        }
        let mut posed = Vec::new();
        for (person, animator) in &scene {
            let Some(root) = placement(animator) else {
                continue;
            };
            let pose = player.pose(&person.body.skeleton, animator, &person.base)?;
            posed.push((*person, pose.world(&person.body.skeleton, root), root));
        }
        let center = if posed.is_empty() {
            Vec3::ZERO
        } else {
            let sum = posed
                .iter()
                .fold(Vec3::ZERO, |a, (_, _, r)| a + r.translation);
            sum * (1.0 / posed.len() as f32) + Vec3::new(0.0, 2.6, 0.0)
        };
        let camera = Camera::house_view(self.scale, center)
            .turned(self.turn as f32 * std::f32::consts::FRAC_PI_4);
        for (person, bones, _) in &posed {
            canvas.draw_body(&camera, &person.body, bones);
        }
        Ok(canvas)
    }

    fn redraw(&mut self) -> Result<(), String> {
        let (Some(window), Some(_)) = (&self.window, &self.surface) else {
            return Ok(());
        };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return Ok(());
        };
        // Draw at the window's logical size and repeat each pixel, so a
        // high-density screen does not quadruple the work.
        let factor = (window.scale_factor().round() as u32).max(1);
        let (cw, ch) = ((w.get() / factor).max(1), (h.get() / factor).max(1));
        let clock = self.start.elapsed().as_millis().min(i32::MAX as u128) as i32;
        let canvas = self.render(cw, ch, clock)?;
        let surface = self.surface.as_mut().ok_or("no surface")?;
        surface.resize(w, h).map_err(|e| e.to_string())?;
        let mut buffer = surface.buffer_mut().map_err(|e| e.to_string())?;
        for y in 0..h.get() {
            let cy = (y / factor).min(ch - 1);
            for x in 0..w.get() {
                let cx = (x / factor).min(cw - 1);
                let i = ((cy * cw + cx) * 4) as usize;
                let [r, g, b] = [canvas.rgba[i], canvas.rgba[i + 1], canvas.rgba[i + 2]];
                buffer[(y * w.get() + x) as usize] =
                    (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
            }
        }
        buffer.present().map_err(|e| e.to_string())
    }

    fn key(&mut self, event: &KeyEvent, event_loop: &ActiveEventLoop) {
        if event.state != ElementState::Pressed {
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => event_loop.exit(),
            Key::Named(NamedKey::ArrowLeft) => self.turn -= 1,
            Key::Named(NamedKey::ArrowRight) => self.turn += 1,
            Key::Character(c) if c.eq_ignore_ascii_case("w") => self.walking = !self.walking,
            Key::Character(c) if c == "+" || c == "=" => self.scale *= 1.25,
            Key::Character(c) if c == "-" => self.scale /= 1.25,
            _ => {}
        }
    }
}

impl ApplicationHandler for Viewer {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("people-viewer")
            .with_inner_size(LogicalSize::new(800.0, 600.0));
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Rc::new(w),
            Err(e) => {
                eprintln!("people-viewer: {e}");
                event_loop.exit();
                return;
            }
        };
        let surface = softbuffer::Context::new(window.clone())
            .and_then(|context| softbuffer::Surface::new(&context, window.clone()));
        match surface {
            Ok(surface) => self.surface = Some(surface),
            Err(e) => {
                eprintln!("people-viewer: {e}");
                event_loop.exit();
                return;
            }
        }
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => self.key(&event, event_loop),
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.redraw() {
                    eprintln!("people-viewer: {e}");
                    event_loop.exit();
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}
