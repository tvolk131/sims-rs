//! A window that shows a lot drawn by `sims-render`.
//!
//! ```text
//! sims-viewer <install> <house file> [--snapshot <file.redb> [name]] [--save-dir <dir>] [--hud]
//! ```
//!
//! Without `--snapshot` the house file is read into a fresh world, or,
//! with `--hud`, loaded as the game loads it (`sims_vm::boot::Lot`, up to
//! the frame before the first tick), so the panel shows the family's
//! funds and its first member selected; a load that does not complete (a
//! house nobody lives in, or a load step not ported) is reported and the
//! house file shown as saved. With `--snapshot`, the world
//! saved by `sims boot --snapshot` (named `boot` unless another name is
//! given) is shown instead; the house file still names the neighborhood
//! the art is read with.
//!
//! Keys: `+`/`-` or the wheel zoom, `[`/`]` or `,`/`.` rotate, Page Up and
//! Page Down change the floor, `1`/`2`/`3` choose walls up, cutaway and
//! down, the arrow keys or a left-button drag scroll, `S` saves the frame
//! as a PNG, Escape quits. As in the original, the cutaway cuts the walls
//! in front of the last rooms the pointer passed over.
//!
//! `--hud` draws the original's live-mode control panel over the lot
//! (`sims-ui`), and the dialog the world waits on if there is one (a
//! snapshot taken while a script shows one); its camera buttons work,
//! and the commands it would send the simulation are printed instead.
//!
//! The window is drawn at the original's pixel size and scaled up by whole
//! pixels on high-DPI screens, so it keeps the original's look.

mod hud;
mod view;

use sims_render::{Canvas, Lot, Walls, draw, open_house, png};
use softbuffer::{Context, Surface};
use std::ffi::OsString;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use view::{Command, View};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

struct Options {
    install: PathBuf,
    house: PathBuf,
    snapshot: Option<(PathBuf, String)>,
    save_dir: PathBuf,
    hud: bool,
}

fn options() -> Result<Options, String> {
    let usage = "usage: sims-viewer <install> <house file> [--snapshot <file.redb> [name]] [--save-dir <dir>] [--hud]";
    // Paths need not be Unicode; flags and snapshot names must be.
    let mut args = std::env::args_os().skip(1);
    let install = args.next().ok_or(usage)?.into();
    let house = args.next().ok_or(usage)?.into();
    let mut snapshot = None;
    let mut save_dir = PathBuf::from(".");
    let mut hud = false;
    let mut rest: Vec<OsString> = args.collect();
    rest.reverse();
    while let Some(arg) = rest.pop() {
        match arg.to_str() {
            Some("--snapshot") => {
                let path = rest.pop().ok_or(usage)?;
                let name = match rest.last().map(|n| n.to_str()) {
                    Some(Some(n)) if !n.starts_with("--") => {
                        let name = n.to_string();
                        rest.pop();
                        name
                    }
                    Some(None) => return Err("a snapshot name must be Unicode".to_string()),
                    _ => "boot".to_string(),
                };
                snapshot = Some((PathBuf::from(path), name));
            }
            Some("--save-dir") => save_dir = rest.pop().ok_or(usage)?.into(),
            Some("--hud") => hud = true,
            _ => return Err(usage.to_string()),
        }
    }
    Ok(Options {
        install,
        house,
        snapshot,
        save_dir,
        hud,
    })
}

struct Viewer {
    world: sims_sim::World,
    lot: Lot,
    assets: sims_render::Assets,
    view: View,
    canvas: Canvas,
    stats: sims_render::Stats,
    dirty: bool,
    save_dir: PathBuf,
    saved: usize,
    window: Option<Rc<Window>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    scale: u32,
    drag: Option<PhysicalPosition<f64>>,
    cursor: PhysicalPosition<f64>,
    hud: Option<hud::Hud>,
}

impl Viewer {
    fn command(&mut self, command: Command) {
        if command == Command::Save {
            self.save();
        } else if self.view.apply(command) {
            self.dirty = true;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }

    fn save(&mut self) {
        // Keys can arrive before the redraw: draw what the name describes.
        self.render();
        let c = &self.view.camera;
        let name = format!(
            "view-z{}-r{}-f{}-{}.png",
            c.zoom.level(),
            c.rotation,
            c.level,
            self.saved
        );
        let path = self.save_dir.join(name);
        match std::fs::write(&path, png::encode(&self.canvas)) {
            Ok(()) => eprintln!("saved {}", path.display()),
            Err(e) => eprintln!("could not save {}: {e}", path.display()),
        }
        self.saved += 1;
    }

    fn key(&mut self, key: &Key) {
        let step = 64;
        let command = match key {
            Key::Named(NamedKey::Escape) => return,
            Key::Named(NamedKey::ArrowLeft) => Command::Scroll([step, 0]),
            Key::Named(NamedKey::ArrowRight) => Command::Scroll([-step, 0]),
            Key::Named(NamedKey::ArrowUp) => Command::Scroll([0, step]),
            Key::Named(NamedKey::ArrowDown) => Command::Scroll([0, -step]),
            Key::Named(NamedKey::PageUp) => Command::LevelUp,
            Key::Named(NamedKey::PageDown) => Command::LevelDown,
            Key::Character(c) => match c.as_str() {
                "+" | "=" => Command::ZoomIn,
                "-" | "_" => Command::ZoomOut,
                "[" | "," => Command::Rotate(false),
                "]" | "." => Command::Rotate(true),
                "1" => Command::Walls(Walls::Up),
                "2" => Command::Walls(Walls::Cutaway),
                "3" => Command::Walls(Walls::Down),
                "s" | "S" => Command::Save,
                _ => return,
            },
            _ => return,
        };
        self.command(command);
    }

    /// Draws the lot into the canvas if the view changed since the last
    /// drawing.
    fn render(&mut self) {
        if !self.dirty {
            return;
        }
        let [cw, ch] = self.view.size.map(|n| n.max(1) as usize);
        if (self.canvas.width, self.canvas.height) != (cw, ch) {
            self.canvas = Canvas::new(cw, ch, [0, 0, 0]);
        }
        self.stats = draw(
            &self.world,
            &self.lot,
            &mut self.assets,
            &self.view.camera,
            &mut self.canvas,
        );
        if let Some(hud) = &mut self.hud {
            hud.draw(&self.world, &self.view, &self.assets, &mut self.canvas);
        }
        self.dirty = false;
        if let Some(window) = &self.window {
            window.set_title(&format!(
                "sims-viewer: {} ({} floors, {} walls, {} objects)",
                self.view.describe(),
                self.stats.floors,
                self.stats.walls,
                self.stats.objects
            ));
        }
    }

    /// The cursor in canvas pixels.
    fn canvas_point(&self, position: PhysicalPosition<f64>) -> [i32; 2] {
        let scale = f64::from(self.scale.max(1));
        [(position.x / scale) as i32, (position.y / scale) as i32]
    }

    /// A left press: the panel's if it covers the point, else a drag.
    fn left_press(&mut self) {
        let point = self.canvas_point(self.cursor);
        if let Some(hud) = &mut self.hud
            && hud.covers(point)
        {
            let (command, changed) = hud.press(point);
            if changed {
                self.dirty = true;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            if let Some(command) = command {
                self.command(command);
            }
            return;
        }
        self.drag = Some(self.cursor);
    }

    fn redraw(&mut self) {
        let Some(size) = self.window.as_ref().map(|w| w.inner_size()) else {
            return;
        };
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return;
        };
        let scale = self.scale.max(1);
        let canvas_size = [size.width / scale, size.height / scale]
            .map(|n| i32::try_from(n).unwrap_or(i32::MAX).max(1));
        if canvas_size != self.view.size {
            self.view.resize(canvas_size);
            self.dirty = true;
        }
        self.render();
        let Some(surface) = &mut self.surface else {
            return;
        };
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        let (bw, bh) = (size.width as usize, size.height as usize);
        let scale = scale as usize;
        for y in 0..bh {
            let cy = (y / scale).min(self.canvas.height.saturating_sub(1));
            for x in 0..bw {
                let cx = (x / scale).min(self.canvas.width.saturating_sub(1));
                let [r, g, b] = self
                    .canvas
                    .color
                    .get(cy * self.canvas.width + cx)
                    .copied()
                    .unwrap_or([0, 0, 0]);
                buffer[y * bw + x] = (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
            }
        }
        let _ = buffer.present();
    }
}

impl ApplicationHandler for Viewer {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("sims-viewer")
            .with_inner_size(LogicalSize::new(1024.0, 768.0));
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Rc::new(w),
            Err(e) => {
                eprintln!("could not open a window: {e}");
                event_loop.exit();
                return;
            }
        };
        let surface =
            Context::new(window.clone()).and_then(|context| Surface::new(&context, window.clone()));
        match surface {
            Ok(s) => self.surface = Some(s),
            Err(e) => {
                eprintln!("could not draw into the window: {e}");
                event_loop.exit();
                return;
            }
        }
        self.scale = window.scale_factor().round().max(1.0) as u32;
        self.window = Some(window.clone());
        self.dirty = true;
        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(_) => {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor.round().max(1.0) as u32;
                self.dirty = true;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
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
                // Sideways and empty wheel events leave the zoom alone.
                if y > 0.0 {
                    self.command(Command::ZoomIn);
                } else if y < 0.0 {
                    self.command(Command::ZoomOut);
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.drag = None;
                if state == ElementState::Pressed {
                    self.left_press();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                let point = self.canvas_point(position);
                let mut redraw = false;
                if let Some(hud) = &mut self.hud
                    && hud.hover(point)
                {
                    redraw = true;
                }
                // The cutaway follows the rooms the pointer passes over;
                // the original looks only while the cutaway is on, and
                // only over the lot's view, not its panel.
                let over_panel = self.hud.as_ref().is_some_and(|h| h.covers(point));
                if self.drag.is_none() && !over_panel && self.view.camera.walls == Walls::Cutaway {
                    let camera = self.view.camera;
                    self.view.camera.focus.point(&self.lot, &camera, point);
                    redraw |= self.view.camera.focus != camera.focus;
                }
                if redraw {
                    self.dirty = true;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
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
                        self.command(Command::Scroll([dx, dy]));
                    }
                }
            }
            _ => {}
        }
    }
}

/// The house loaded as the game loads it, up to the frame before the
/// first tick, or None, with the reason printed, when the load does not
/// complete: a house nobody lives in, or a step-13 visit that is not
/// ported. The viewer then shows the house file as it was saved.
fn booted(options: &Options) -> Option<sims_sim::World> {
    let frames = sims_vm::boot::Frames {
        after: Vec::new(),
        every: Vec::new(),
    };
    let fallback = "showing the house file as it was saved";
    match sims_vm::boot::Lot::load(&options.install, &options.house, &[], frames) {
        Err(error) => {
            eprintln!("the house was not loaded as the game loads it ({error}); {fallback}");
            None
        }
        Ok(lot) if lot.load_report().finished.is_none() => {
            let stop = lot
                .load_report()
                .stopped
                .as_deref()
                .unwrap_or("not reported");
            eprintln!("the load stopped at step 13 ({stop}); {fallback}");
            None
        }
        Ok(lot) => Some(lot.world().clone()),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = options()?;
    let opened = open_house(&options.install, &options.house)?;
    // Whether the world's floors and walls are known to be this install's
    // catalogs' keys, which the load gives them (load step 5): not so for
    // the house file as saved, whose patterns keep the house's own
    // numbers, nor for a snapshot, which does not record the catalogs it
    // was keyed by.
    let mut renumbered = options.snapshot.is_none();
    let world = match &options.snapshot {
        // Opening a snapshot store creates it if it is missing, so check
        // first rather than leave an empty file behind a mistyped path.
        Some((path, _)) if !path.is_file() => {
            return Err(format!("no snapshot file at {}", path.display()).into());
        }
        Some((path, name)) => sims_sim::Snapshots::open(path)?
            .load(name)?
            .ok_or_else(|| format!("{} holds no world named {name}", path.display()))?,
        // The panel shows what the load's steps 17 and 18 set: the
        // family's funds and its first member selected.
        None if options.hud => booted(&options).unwrap_or_else(|| {
            renumbered = false;
            opened.world
        }),
        None => opened.world,
    };
    let lot = Lot::from_world(&world)?;
    let hud = if options.hud {
        let mut hud = hud::Hud::open(&options.install, &options.house, &opened.assets, &world)?;
        hud.priced = renumbered;
        Some(hud)
    } else {
        None
    };
    let view = View::new([1024, 768], lot.built_center(), opened.rotation);
    let mut viewer = Viewer {
        world,
        lot,
        assets: opened.assets,
        view,
        canvas: Canvas::new(1, 1, [0, 0, 0]),
        stats: sims_render::Stats::default(),
        dirty: true,
        save_dir: options.save_dir,
        saved: 0,
        window: None,
        surface: None,
        scale: 1,
        drag: None,
        cursor: PhysicalPosition::new(0.0, 0.0),
        hud,
    };
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut viewer)?;
    Ok(())
}
