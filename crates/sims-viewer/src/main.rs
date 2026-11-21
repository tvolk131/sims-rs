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
use std::path::{Path, PathBuf};
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
            let (command, changed) = hud.press(point, &self.world);
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
fn booted(options: &Options) -> Option<(sims_sim::World, u8)> {
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
        Ok(lot) => Some((lot.world().clone(), lot.floor_rotation())),
    }
}

/// Loads a named world snapshot and its saved `HOUS 0` floor rotation (falling
/// back to `fallback_rotation` for snapshots written before floor rotations
/// were stored alongside worlds).
fn load_snapshot(
    path: &Path,
    name: &str,
    fallback_rotation: u8,
) -> Result<(sims_sim::World, u8), Box<dyn std::error::Error>> {
    // Opening a snapshot store creates it if it is missing, so check
    // first rather than leave an empty file behind a mistyped path.
    if !path.is_file() {
        return Err(format!("no snapshot file at {}", path.display()).into());
    }
    let store = sims_sim::Snapshots::open(path)?;
    let world = store
        .load(name)?
        .ok_or_else(|| format!("{} holds no world named {name}", path.display()))?;
    let floor_rotation = store
        .load_floor_rotation(name)?
        .unwrap_or(fallback_rotation);
    Ok((world, floor_rotation))
}

/// Builds the viewer's render [`Lot`] with the house file's saved `HOUS 0`
/// camera rotation so level-1 road dashes (`10` and `11`) keep their saved
/// orientation at opening (`0x4a861f`) and swap on odd-parity turns
/// (`0x4a8042`–`0x4a80b1`).
fn viewer_lot(world: &sims_sim::World, rotation: u8) -> sims_render::Result<Lot> {
    Ok(Lot::from_world(world)?.with_floor_rotation(rotation))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = options()?;
    let opened = open_house(&options.install, &options.house)?;
    let (world, floor_rotation) = match &options.snapshot {
        Some((path, name)) => load_snapshot(path, name, opened.rotation)?,
        // The panel shows what the load's steps 17 and 18 set: the
        // family's funds and its first member selected.
        None if options.hud => booted(&options).unwrap_or((opened.world, opened.rotation)),
        None => (opened.world, opened.rotation),
    };
    let lot = viewer_lot(&world, floor_rotation)?;
    let hud = if options.hud {
        Some(hud::Hud::open(
            &options.install,
            &options.house,
            &opened.assets,
            &world,
        )?)
    } else {
        None
    };
    let view = View::new([1024, 768], lot.built_center(), floor_rotation);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_keeps_odd_saved_house_road_dashes_unswapped_at_opening() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            eprintln!("skipping: set SIMS_INSTALL to check House24 in sims-viewer");
            return;
        };
        let house24 = root.join("UserData/Houses/House24.iff");
        let mut opened = open_house(&root, &house24).unwrap();
        assert_eq!(opened.rotation, 1);
        let lot = viewer_lot(&opened.world, opened.rotation).unwrap();
        assert_eq!(lot.floor_rotation, 1);
        let view = View::new([1024, 768], lot.built_center(), opened.rotation);
        assert_eq!(view.camera.rotation, 1);

        // Tile (39, 6) holds stored floor 10 on House24's Y-road. At the
        // viewer's opening camera (rotation 1), it must draw sprite 10, not
        // swap to sprite 11 (0x4a861f).
        let cell = Lot::cell(39, 6);
        assert_eq!(lot.levels[0].floors[cell], 10);
        let mut camera = view.camera;
        camera.zoom = sims_render::Zoom::Near;
        camera.level = 1;
        camera.walls = Walls::Down;
        camera.origin = [0, 0];
        let elev = i32::from(lot.heights[cell][0]) * 16;
        let p = camera.project_fine(camera.rotate_fine([39 * 16 + 8, 6 * 16 + 8]), 1, elev);
        camera.origin = [80 - p[0], 50 - p[1]];

        let mut actual = Canvas::new(160, 100, [0, 0, 0]);
        draw(
            &opened.world,
            &lot,
            &mut opened.assets,
            &camera,
            &mut actual,
        );

        let wrong_baseline = lot.clone().with_floor_rotation(0);
        let mut wrong = Canvas::new(160, 100, [0, 0, 0]);
        draw(
            &opened.world,
            &wrong_baseline,
            &mut opened.assets,
            &camera,
            &mut wrong,
        );
        assert_ne!(actual, wrong);

        let mut expected_lot = wrong_baseline;
        for id in &mut expected_lot.levels[0].floors {
            if matches!(*id, 10 | 11) {
                *id ^= 1;
            }
        }
        let mut expected = Canvas::new(160, 100, [0, 0, 0]);
        draw(
            &opened.world,
            &expected_lot,
            &mut opened.assets,
            &camera,
            &mut expected,
        );
        assert_eq!(actual, expected);
    }

    #[test]
    fn viewer_snapshot_uses_snapshots_floor_rotation_not_art_houses() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            eprintln!("skipping: set SIMS_INSTALL to check --snapshot floor rotation");
            return;
        };
        let house05 = root.join("UserData/Houses/House05.iff");
        let house24 = root.join("UserData/Houses/House24.iff");
        let booted_h05 = sims_vm::boot::Lot::load(
            &root,
            &house05,
            &[3],
            sims_vm::boot::Frames {
                after: Vec::new(),
                every: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(booted_h05.floor_rotation(), 0);

        let snap_path = std::env::temp_dir().join(format!(
            "sims-viewer-snapshot-floor-rot-{}.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&snap_path);
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = Cleanup(snap_path.clone());

        {
            let store = sims_sim::Snapshots::open(&snap_path).unwrap();
            store
                .save_with_floor_rotation("boot", booted_h05.world(), booted_h05.floor_rotation())
                .unwrap();
            store.save("legacy", booted_h05.world()).unwrap();
        }

        // Open House24 (saved at rotation 1) as the art house and draw House05's
        // boot snapshot (saved at rotation 0) across all four camera rotations.
        let mut h24 = open_house(&root, &house24).unwrap();
        assert_eq!(h24.rotation, 1);

        let (world, floor_rotation) = load_snapshot(&snap_path, "boot", h24.rotation).unwrap();
        assert_eq!(floor_rotation, 0);
        let lot = viewer_lot(&world, floor_rotation).unwrap();
        assert_eq!(lot.floor_rotation, 0);

        // Legacy snapshots without a recorded rotation fall back to the art house.
        let (_, legacy_rotation) = load_snapshot(&snap_path, "legacy", h24.rotation).unwrap();
        assert_eq!(legacy_rotation, h24.rotation);

        // Tile (46, 5) holds stored floor 11 on House05's Y-road. With House24's
        // rotation (1) erroneously used as the baseline, all four camera rotations
        // draw the perpendicular dash.
        let cell = Lot::cell(46, 5);
        assert_eq!(lot.levels[0].floors[cell], 11);
        let wrong_lot = lot.clone().with_floor_rotation(h24.rotation);
        let elev = i32::from(lot.heights[cell][0]) * 16;

        for rotation in 0u8..4 {
            let mut camera = View::new([160, 100], lot.built_center(), rotation).camera;
            camera.zoom = sims_render::Zoom::Near;
            camera.level = 1;
            camera.walls = Walls::Down;
            camera.origin = [0, 0];
            let p = camera.project_fine(camera.rotate_fine([46 * 16 + 8, 5 * 16 + 8]), 1, elev);
            camera.origin = [80 - p[0], 50 - p[1]];

            let mut actual = Canvas::new(160, 100, [0, 0, 0]);
            draw(&world, &lot, &mut h24.assets, &camera, &mut actual);

            let mut wrong = Canvas::new(160, 100, [0, 0, 0]);
            draw(&world, &wrong_lot, &mut h24.assets, &camera, &mut wrong);
            assert_ne!(
                actual, wrong,
                "rotation {rotation} should differ from wrong art-house baseline"
            );

            let mut expected_lot = lot.clone().with_floor_rotation(0);
            if rotation & 1 == 1 {
                for id in &mut expected_lot.levels[0].floors {
                    if matches!(*id, 10 | 11) {
                        *id ^= 1;
                    }
                }
            }
            // Bypass swap on expected_lot by matching floor_rotation to camera.rotation.
            expected_lot.floor_rotation = rotation;
            let mut expected = Canvas::new(160, 100, [0, 0, 0]);
            draw(
                &world,
                &expected_lot,
                &mut h24.assets,
                &camera,
                &mut expected,
            );
            assert_eq!(actual, expected, "rotation {rotation} road dashes mismatch");
        }
    }
}
