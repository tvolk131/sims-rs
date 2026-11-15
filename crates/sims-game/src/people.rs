//! The people, drawn by `sims-people` over the lot `sims-render` drew.
//!
//! The people's renderer draws 3D bodies with an orthographic camera; the
//! lot is drawn with the original's 2:1 projection. [`camera`] builds the
//! 3D camera that puts a point on the ground where the lot's camera puts
//! it, at every zoom and rotation, so a person stands on the tile the
//! simulation says. Heights are drawn at the scale that projection gives
//! a horizontal length; that the original draws people at that scale is
//! **unverified**.
//!
//! The people are drawn into their own image and laid over the lot
//! whole: a wall or a counter in front of a person does not hide them yet.
//! Hiding people behind what is in front of them belongs to the render
//! and people tracks.

use sims_people::math::{Transform, Vec3};
use sims_people::person::UNITS_PER_TILE;
use sims_people::raster::{self, Camera as PeopleCamera};
use sims_people::{Assets, Body, Cast, Library, Player, Resident, base_clip, placement};
use sims_render::{Camera, Canvas};
use sims_sim::World;
use std::collections::BTreeSet;
use std::path::Path;

/// Model units per sixteenth of a tile.
const UNITS_PER_FINE: f32 = UNITS_PER_TILE / 16.0;

/// The side of the lot's arrays in sixteenths of a tile, which the
/// lot camera's rotation turns about.
const ARRAY_FINE: f32 = (sims_render::camera::ARRAY_SIZE * 16) as f32;

/// The 3D camera that draws a ground point where `camera` draws it, on a
/// canvas of `size` pixels.
pub fn camera(camera: &Camera, size: [usize; 2]) -> PeopleCamera {
    let zoom = camera.zoom.level();
    // Screen pixels across per sixteenth of a tile, along the view's x
    // minus y: the lot camera's `(x - y) * (8 << zoom) / 16`.
    let across = (8u32 << zoom) as f32 / 16.0;
    // A sixteenth of a tile along x is UNITS_PER_FINE / sqrt 2 across the
    // 3D view, which turns an eighth of a turn.
    let scale = across * std::f32::consts::SQRT_2 / UNITS_PER_FINE;
    let half = std::f32::consts::FRAC_1_SQRT_2;
    // The lot camera's rotation turns the view a quarter turn each; the
    // 3D view turns the other way about its vertical axis.
    let yaw = match camera.rotation & 3 {
        0 => [half, half],
        1 => [-half, half],
        2 => [-half, -half],
        _ => [half, -half],
    };
    // The fine view position (after rotation) under the canvas's middle,
    // inverting the lot camera's projection without rounding.
    let [w, h] = size.map(|n| n as f32);
    let d = (w / 2.0 - camera.origin[0] as f32) / across;
    let s = (h / 2.0 - camera.origin[1] as f32) / (across / 2.0);
    let (vx, vy) = ((s + d) / 2.0, (s - d) / 2.0);
    let m = ARRAY_FINE;
    let [x, y] = match camera.rotation & 3 {
        1 => [vy, m - vx],
        2 => [m - vx, m - vy],
        3 => [m - vy, vx],
        _ => [vx, vy],
    };
    PeopleCamera {
        yaw,
        pitch: [0.5, 0.75f32.sqrt()],
        scale,
        center: ground(x, y),
    }
}

/// The 3D point of the fine ground position `x`, `y`, as
/// [`sims_people::placement`] places a person.
fn ground(x: f32, y: f32) -> Vec3 {
    Vec3::new(-y * UNITS_PER_FINE, 0.0, -x * UNITS_PER_FINE)
}

/// Model units a person is raised to stand on level `level`, so its feet
/// are drawn where the lot draws that floor.
fn level_height(camera: &PeopleCamera, lot: &Camera, level: i32) -> f32 {
    let pixels = (lot.zoom.floor_height() * (level - 1)) as f32;
    // A unit up is drawn cos 30° * scale pixels up.
    pixels / (camera.pitch[1] * camera.scale)
}

struct Drawn {
    resident: Resident,
    body: Body,
    base: String,
}

/// The people of a lot: their bodies, loaded as they appear.
pub struct People {
    player: Player<'static>,
    assets: &'static Assets,
    cast: Cast,
    drawn: Vec<Drawn>,
    /// Person IDs already loaded, or that failed, so each is tried once.
    tried: BTreeSet<u16>,
    /// What went wrong, one line each, for the host to show.
    pub problems: Vec<String>,
}

impl People {
    /// Reads the people's files from the install at `root`, and the cast
    /// of the neighborhood `house` is in. The files and the clips live as
    /// long as the program, as the people's player needs.
    pub fn open(root: &Path, house: &Path) -> Result<Self, String> {
        let assets: &'static Assets = Box::leak(Box::new(Assets::scan(root)?));
        let library: &'static Library = Box::leak(Box::new(Library::scan(assets)?));
        Ok(Self {
            player: Player::new(assets, library),
            assets,
            cast: Cast::of_house(root, house)?,
            drawn: Vec::new(),
            tried: BTreeSet::new(),
            problems: Vec::new(),
        })
    }

    /// Loads the people `world` holds that were not tried yet: at the
    /// start, and those a script makes while the lot runs.
    pub fn refresh(&mut self, world: &World) {
        for (&id, object) in &world.objects {
            let Some(person) = &object.person else {
                continue;
            };
            if !self.tried.insert(id) {
                continue;
            }
            let resident = match self.cast.residents([(id, object.guid, person)]) {
                Ok(mut found) if !found.is_empty() => found.remove(0),
                Ok(_) => continue,
                Err(e) => {
                    self.problems.push(format!("person {id} is not read: {e}"));
                    continue;
                }
            };
            match Body::load(self.assets, &resident.skeleton, &resident.outfit) {
                Ok(body) => {
                    let base = base_clip(&resident.person, &resident.skeleton);
                    self.drawn.push(Drawn {
                        resident,
                        body,
                        base,
                    });
                }
                Err(e) => self.problems.push(format!("person {id} is not drawn: {e}")),
            }
        }
    }

    /// Draws every placed person on the levels `camera` shows over the
    /// lot in `canvas`.
    pub fn draw(&mut self, world: &World, camera: &Camera, canvas: &mut Canvas) {
        let size = [canvas.width, canvas.height];
        let view = self::camera(camera, size);
        let mut image = raster::Canvas::new(
            u32::try_from(size[0]).unwrap_or(0),
            u32::try_from(size[1]).unwrap_or(0),
            [0, 0, 0, 0],
        );
        let mut any = false;
        for drawn in &self.drawn {
            let id = drawn.resident.id;
            let (Some(animator), Some(object)) = (world.animators.get(&id), world.objects.get(&id))
            else {
                continue;
            };
            let level = object.position[2].max(1);
            if level > camera.level {
                continue;
            }
            let Some(mut root) = placement(animator) else {
                continue;
            };
            root.translation.y += level_height(&view, camera, level);
            let pose = match self
                .player
                .pose(&drawn.body.skeleton, animator, &drawn.base)
            {
                Ok(pose) => pose,
                Err(e) => {
                    let line = format!("person {id} is not posed: {e}");
                    if !self.problems.contains(&line) {
                        self.problems.push(line);
                    }
                    continue;
                }
            };
            let bones: Vec<Transform> = pose.world(&drawn.body.skeleton, root);
            image.draw_body(&view, &drawn.body, &bones);
            any = true;
        }
        if any {
            overlay(&image, canvas);
        }
    }
}

/// Lays the people's image over the lot: opaque pixels replace the lot's,
/// partly transparent ones blend over it.
fn overlay(image: &raster::Canvas, canvas: &mut Canvas) {
    for (out, rgba) in canvas.color.iter_mut().zip(image.rgba.as_chunks::<4>().0) {
        match rgba[3] {
            0 => {}
            255 => *out = [rgba[0], rgba[1], rgba[2]],
            a => {
                let a = u16::from(a);
                for (o, &c) in out.iter_mut().zip(&rgba[..3]) {
                    *o = ((u16::from(c) * a + u16::from(*o) * (255 - a)) / 255) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_render::{Focus, Walls, Zoom};

    /// Where the 3D camera draws a 3D point, as the people's rasterizer
    /// places it.
    fn screen(camera: &PeopleCamera, size: [usize; 2], p: Vec3) -> [f32; 2] {
        let v = camera.view(p);
        [
            size[0] as f32 * 0.5 + v.x * camera.scale,
            size[1] as f32 * 0.5 - v.y * camera.scale,
        ]
    }

    #[test]
    fn the_ground_lands_where_the_lot_draws_it() {
        let size = [801, 600];
        for zoom in Zoom::ALL {
            for rotation in 0..4 {
                let lot = Camera {
                    zoom,
                    rotation,
                    level: 1,
                    origin: [137, -211],
                    walls: Walls::Cutaway,
                    focus: Focus::default(),
                };
                let people = camera(&lot, size);
                for fine in [[0, 0], [16, 0], [0, 16], [400, 250], [1000, 37]] {
                    let expected = lot.project_fine(lot.rotate_fine(fine), 1, 0);
                    let [x, y] = screen(&people, size, ground(fine[0] as f32, fine[1] as f32));
                    // The lot camera rounds down to whole pixels.
                    assert!(
                        (x - expected[0] as f32).abs() <= 1.01
                            && (y - expected[1] as f32).abs() <= 1.01,
                        "zoom {zoom:?} rotation {rotation} fine {fine:?}: \
                         {x},{y} for {expected:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn upstairs_people_stand_on_the_second_floor() {
        let lot = Camera {
            zoom: Zoom::Medium,
            rotation: 0,
            level: 2,
            origin: [0, 0],
            walls: Walls::Cutaway,
            focus: Focus::default(),
        };
        let size = [640, 480];
        let people = camera(&lot, size);
        let raised = ground(300.0, 200.0) + Vec3::new(0.0, level_height(&people, &lot, 2), 0.0);
        let expected = lot.project_fine(lot.rotate_fine([300, 200]), 2, 0);
        let [x, y] = screen(&people, size, raised);
        assert!((x - expected[0] as f32).abs() <= 1.01, "{x} {expected:?}");
        assert!((y - expected[1] as f32).abs() <= 1.01, "{y} {expected:?}");
    }
}
