//! The people of a running lot, drawn in the lot renderer's view.
//!
//! [`People`] follows a [`World`] as it changes: it loads a person's body
//! when the person appears or changes clothes, and forgets people who are
//! gone. Every frame it poses each placed person from the simulation's
//! animator and draws them with `sims_render`'s [`Camera`], at any zoom and
//! rotation, into any [`Target`]. It only reads the world.
//!
//! **Where a person is.** A person's points are in the units the
//! original's model matrix uses (0x592132, [`crate::placement`]): three to
//! a tile, the height up. [`View::place`] turns a point into the lot
//! renderer's terms: its fine position in the view (sixteenths of a tile,
//! after the camera's rotation, [`Camera::rotate_fine`]) and its elevation
//! in near-zoom pixels. A unit of height is 32√6/3 ≈ 26.13 near-zoom
//! pixels. That follows from the original's house camera (0x49b0de, read
//! by the old prototype in `legacy/crates/sims-sim/src/pose/portrait/
//! view.rs`): orthographic, tilted down 30°, 3√2/(16 << zoom) units to a
//! pixel. It agrees with the terrain: a step of the lot's altitude lifts a
//! person 3/(2√6) units (0x59226f), and lifts the lot renderer's ground 16
//! near-zoom pixels. [`View::screen`] then projects as the lot renderer's
//! `project_fine` does, with a level's floor height per level above the
//! first, and one column right as the lot renderer draws objects (its
//! `OBJECT_COLUMN`; unchecked for people).
//!
//! A person stands on the terrain under it ([`ground_height`]: the tile's
//! altitude where its position falls, as the original finds it, 0x478dfe).
//! On a level above the ground the original adds an offset from the
//! view's table, which is not ported: such a person stands at the level's
//! floor.
//!
//! **Depth** is the target's. The caller gives a function from a point's
//! fine view position, elevation and level to its depth, the signature of
//! the lot renderer's planned `Depth::point`, and the rasterizer
//! interpolates it linearly across each triangle. How the original gives a
//! person's mesh its depth is **unverified**.
//!
//! **Light** is our own ([`crate::raster::LIGHT`]), held fixed relative
//! to the viewer as the view turns, as the original's standard light is
//! (see the decisions entry "How the original lights a person"). Room
//! light and lamps are not applied.

use crate::animation::{Clip, Library};
use crate::appearance::Body;
use crate::assets::Assets;
use crate::house::Cast;
use crate::math::{Transform, Vec3};
use crate::person::{Player, UNITS_PER_TILE, base_clip, placement};
use crate::raster::{LIGHT, Target, draw_part};
use crate::skin::skin;
use sims_core::F32;
use sims_render::Camera;
use sims_sim::World;
use std::collections::BTreeMap;
use std::path::Path;

/// Near-zoom pixels per unit of height: 32√6/3.
pub const NEAR_PIXELS_PER_UNIT: f32 = 26.127_89;

/// Fine units (sixteenths of a tile) per unit.
const FINE_PER_UNIT: f32 = 16.0 / UNITS_PER_TILE;

/// The side of the lot's arrays in fine units, on which the rotation is
/// defined (the lot renderer's `ARRAY_SIZE` tiles).
const ARRAY_FINE: f32 = sims_render::camera::ARRAY_SIZE as f32 * 16.0;

/// The lot renderer draws objects one column right of the projected point
/// (its `OBJECT_COLUMN`); people are drawn the same way, unchecked.
const OBJECT_COLUMN: f32 = 1.0;

/// The lot's altitude array: one byte per tile corner, four to a tile.
const ALTITUDE: u16 = 0;

/// The side of the lot's arrays in tiles, whatever the lot's size.
const FULL: usize = sims_render::lot::FULL;

/// How the lot renderer's camera sees the people's world.
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub camera: Camera,
}

impl View {
    pub fn new(camera: &Camera) -> Self {
        Self { camera: *camera }
    }

    /// A world point's fine position in the view (sixteenths of a tile,
    /// after the camera's rotation) and its elevation in near-zoom pixels.
    /// [`crate::placement`] puts fine position (x, y) at world
    /// (−3y/16, 0, −3x/16).
    pub fn place(&self, p: Vec3) -> ([f32; 2], f32) {
        let fine = [-p.z * FINE_PER_UNIT, -p.x * FINE_PER_UNIT];
        (
            rotate(self.camera.rotation, fine, ARRAY_FINE),
            p.y * NEAR_PIXELS_PER_UNIT,
        )
    }

    /// The screen position of a fine view position on `level`, raised
    /// `elevation` near-zoom pixels: the lot renderer's `project_fine`
    /// without its rounding, one column right as objects are drawn. The
    /// pixel a point falls in is the floor of each coordinate.
    pub fn screen(&self, [x, y]: [f32; 2], elevation: f32, level: i32) -> [f32; 2] {
        let camera = &self.camera;
        let shift = camera.zoom.level();
        let scale = camera.zoom.near_scale() as f32;
        let floor = camera.zoom.floor_height() as f32;
        [
            (x - y) * (8 << shift) as f32 / 16.0 + camera.origin[0] as f32 + OBJECT_COLUMN,
            (x + y) * (4 << shift) as f32 / 16.0 + camera.origin[1] as f32
                - floor * (level - 1) as f32
                - elevation / scale,
        ]
    }

    /// The direction towards the light: [`LIGHT`] as it is at rotation 0,
    /// turned with the view so it stays where it is relative to the
    /// viewer.
    pub fn light(&self) -> Vec3 {
        // LIGHT's direction in the view at rotation 0, then back out
        // through this rotation.
        let fine = [-LIGHT.z, -LIGHT.x];
        let back = rotate((4 - (self.camera.rotation & 3)) & 3, fine, 0.0);
        Vec3::new(-back[1], LIGHT.y, -back[0])
    }
}

/// Turns a fine position (or, with `size` 0, a direction) by `rotation`
/// quarter turns, as the lot renderer's `rotate_fine` does.
fn rotate(rotation: u8, [x, y]: [f32; 2], size: f32) -> [f32; 2] {
    match rotation & 3 {
        1 => [size - y, x],
        2 => [size - x, size - y],
        3 => [y, size - x],
        _ => [x, y],
    }
}

/// The terrain's height, in units, under fine position `position` on the
/// ground level, as the original places a person (0x478dfe, 0x59226f, as
/// `sims_vm`'s head model does it): the position rounded to the nearest
/// sixteenth, then the altitude of its tile's two triangles there. `None`
/// off the lot, or when the lot has no altitude array.
pub fn ground_height(world: &World, position: [f32; 2]) -> Option<f32> {
    // Rounded as the original does: add a half, round to single
    // precision, truncate.
    let point = position.map(|v| (v + 0.5).trunc());
    if point.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return None;
    }
    let point = point.map(|v| v as i32);
    // The arrays are always FULL tiles on a side, whatever the lot's size.
    let size = i32::from(world.size).min(FULL as i32);
    let [first, second] = point.map(|p| p >> 4);
    if !(0..size).contains(&first) || !(0..size).contains(&second) {
        return None;
    }
    let full = FULL;
    let altitude = world
        .arrays
        .get(&ALTITUDE)
        .filter(|a| a.stride == 4 && a.data.len() == full * full * 4)?;
    let at = (first as usize * full + second as usize) * 4;
    let corners = [0, 1, 2, 3].map(|i| altitude.data[at + i]);
    // The first offset the altitude takes is the second coordinate's.
    let altitude = sims_sim::head::altitude_at(corners, point[1] & 15, point[0] & 15);
    Some(sims_sim::head::terrain_height(altitude).to_f32())
}

/// What makes a person's body: their type, and the appearances and
/// accessories their record names. When any of it changes, the body is
/// loaded again.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    guid: u32,
    appearance: [String; 10],
    accessories: Vec<(String, String)>,
}

impl Key {
    fn of(guid: u32, person: &sims_formats::saved_objects::Person) -> Self {
        Self {
            guid,
            appearance: person.appearance.clone(),
            accessories: person
                .accessories
                .iter()
                .map(|a| (a.name.clone(), a.binding.clone()))
                .collect(),
        }
    }
}

/// One person as last loaded: the body, or why it could not be loaded
/// (not tried again until the key changes).
#[derive(Debug)]
struct Loaded {
    key: Key,
    body: Result<(Body, String), String>,
}

/// A person posed for this frame.
#[derive(Debug)]
pub struct Posed<'a> {
    /// The person's object ID.
    pub id: u16,
    /// The level the person is on (1 the ground floor).
    pub level: i32,
    /// Where the simulation has the person: the skeleton root's transform,
    /// on the terrain.
    pub root: Transform,
    /// Every bone's world transform, in the skeleton's order.
    pub bones: Vec<Transform>,
    pub body: &'a Body,
}

/// What a frame drew.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Drawn {
    /// People drawn.
    pub people: usize,
    /// One line for each placed person not drawn, and why.
    pub problems: Vec<String>,
}

/// The people of a running lot, and what drawing them needs.
#[derive(Debug)]
pub struct People {
    assets: Assets,
    library: Library,
    cast: Cast,
    /// Clips loaded so far, kept between frames.
    clips: BTreeMap<String, Clip>,
    people: BTreeMap<u16, Loaded>,
}

impl People {
    /// Everything needed to draw the people of `house` (a house file in a
    /// neighborhood's `Houses` directory) from the install at `root`.
    pub fn open(root: &Path, house: &Path) -> Result<Self, String> {
        let assets = Assets::scan(root)?;
        let library = Library::scan(&assets)?;
        let cast = Cast::of_house(root, house)?;
        Ok(Self::new(assets, library, cast))
    }

    pub fn new(assets: Assets, library: Library, cast: Cast) -> Self {
        Self {
            assets,
            library,
            cast,
            clips: BTreeMap::new(),
            people: BTreeMap::new(),
        }
    }

    /// Follows `world`: loads the body of each person who appeared or
    /// changed clothes, and forgets people who are gone. Returns one line
    /// for each person whose body could not be loaded now (each is said
    /// once, until the person changes again).
    pub fn update(&mut self, world: &World) -> Vec<String> {
        let mut problems = Vec::new();
        self.people
            .retain(|id, _| world.objects.get(id).is_some_and(|o| o.person.is_some()));
        for (&id, object) in &world.objects {
            let Some(person) = &object.person else {
                continue;
            };
            let key = Key::of(object.guid, person);
            if self.people.get(&id).is_some_and(|l| l.key == key) {
                continue;
            }
            let body = self
                .cast
                .residents([(id, object.guid, person)])
                .and_then(|found| {
                    let resident = found
                        .into_iter()
                        .next()
                        .ok_or_else(|| format!("person {id} is not a person type"))?;
                    let body = Body::load(&self.assets, &resident.skeleton, &resident.outfit)?;
                    Ok((body, resident.skeleton))
                })
                .map_err(|e| format!("person {id} is not drawn: {e}"));
            if let Err(e) = &body {
                problems.push(e.clone());
            }
            self.people.insert(id, Loaded { key, body });
        }
        problems
    }

    /// Each placed person whose body is loaded, posed as the world has
    /// them now, in object ID order. A person on a level above `top`, or
    /// whose object is off the lot, is left out. Call [`People::update`] first, so new people are loaded.
    pub fn posed(&mut self, world: &World, top: i32) -> (Vec<Posed<'_>>, Vec<String>) {
        let mut player =
            Player::with_clips(&self.assets, &self.library, std::mem::take(&mut self.clips));
        let mut posed = Vec::new();
        let mut problems = Vec::new();
        for (&id, animator) in &world.animators {
            let (
                Some(Loaded {
                    body: Ok((body, skeleton)),
                    ..
                }),
                Some(object),
            ) = (self.people.get(&id), world.objects.get(&id))
            else {
                continue;
            };
            let Some(person) = &object.person else {
                continue;
            };
            let Some(mut root) = placement(animator) else {
                continue;
            };
            // An object taken off the lot is at tile (-1, -1); a person
            // there is not drawn, wherever their animator was left.
            let [x, y, level] = World::tile(object.position).map(i32::from);
            let size = i32::from(world.size);
            if !(0..size).contains(&x) || !(0..size).contains(&y) || level > top {
                continue;
            }
            if level <= 1 {
                let at = [animator.position[0], animator.position[1]].map(F32::to_f32);
                root.translation.y = ground_height(world, at).unwrap_or(0.0);
            }
            let base = base_clip(person, skeleton);
            match player.pose(&body.skeleton, animator, &base) {
                Ok(pose) => posed.push(Posed {
                    id,
                    level,
                    root,
                    bones: pose.world(&body.skeleton, root),
                    body,
                }),
                Err(e) => problems.push(format!("person {id} is not posed: {e}")),
            }
        }
        self.clips = player.into_clips();
        (posed, problems)
    }

    /// Draws every placed person on the camera's levels into `target`,
    /// far to near. `depth` gives the target's depth for a point: its fine
    /// view position, its elevation above its level in near-zoom pixels,
    /// and the level (the lot renderer's `Depth::point`). Call
    /// [`People::update`] first.
    pub fn draw<T: Target + ?Sized>(
        &mut self,
        world: &World,
        camera: &Camera,
        target: &mut T,
        depth: impl Fn([f32; 2], f32, i32) -> f32,
    ) -> Drawn {
        let view = View::new(camera);
        let light = view.light();
        let (mut posed, problems) = self.posed(world, camera.level);
        // Far to near by the root's depth, then by ID, so nearer soft
        // edges blend over farther people.
        let far = |p: &Posed| {
            let (fine, elevation) = view.place(p.root.translation);
            depth(fine, elevation, p.level)
        };
        posed.sort_by(|a, b| far(b).total_cmp(&far(a)).then(a.id.cmp(&b.id)));
        for person in &posed {
            for part in person.body.parts.iter().filter(|p| p.censor == 0) {
                let skinned = skin(part, &person.bones);
                draw_part(target, part, &skinned, light, |p| {
                    let (fine, elevation) = view.place(p);
                    let [x, y] = view.screen(fine, elevation, person.level);
                    Vec3::new(x, y, depth(fine, elevation, person.level))
                });
            }
        }
        Drawn {
            people: posed.len(),
            problems,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_render::{Walls, Zoom};
    use sims_sim::animator::Animator;

    fn camera(zoom: Zoom, rotation: u8) -> Camera {
        Camera {
            zoom,
            rotation,
            level: 2,
            origin: [400, -300],
            walls: Walls::Up,
            focus: Default::default(),
        }
    }

    /// A person's root lands on the lot where the lot renderer projects
    /// the person's position, at every zoom and rotation and on both
    /// levels, raised as the renderer raises a point.
    #[test]
    fn a_person_stands_where_the_lot_renderer_projects_their_position() {
        for zoom in Zoom::ALL {
            for rotation in 0..4 {
                let c = camera(zoom, rotation);
                let view = View::new(&c);
                for [x, y] in [[0, 0], [37, 5], [360, 376], [1000, 17]] {
                    let mut animator = Animator::new(0);
                    animator.position = [x as f32, y as f32, 3.0].map(F32::from_f32);
                    let root = placement(&animator).unwrap();
                    let (fine, elevation) = view.place(root.translation);
                    let expected = c.rotate_fine([x, y]);
                    assert!(
                        (fine[0] - expected[0] as f32).abs() < 1e-3
                            && (fine[1] - expected[1] as f32).abs() < 1e-3,
                        "{zoom:?} {rotation}: {fine:?} != {expected:?}"
                    );
                    assert_eq!(elevation, 0.0);
                    for level in [1, 2] {
                        // Elevations a whole number of this zoom's pixels.
                        for up in [0, 16, 64] {
                            let e = up * zoom.near_scale();
                            let [sx, sy] = view.screen(expected.map(|v| v as f32), e as f32, level);
                            let [px, py] = c.project_fine(expected, level, e);
                            assert_eq!(
                                [sx.floor() as i32, sy.floor() as i32],
                                [px + 1, py],
                                "{zoom:?} {rotation} {level} {up}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A step of the lot's altitude lifts a person as far on screen as it
    /// lifts the lot renderer's ground: 16 near-zoom pixels.
    #[test]
    fn an_altitude_step_lifts_a_person_as_it_lifts_the_ground() {
        let view = View::new(&camera(Zoom::Near, 0));
        let step = sims_sim::head::terrain_height(F32::from_f32(1.0)).to_f32();
        let (_, elevation) = view.place(Vec3::new(0.0, step, 0.0));
        assert!((elevation - 16.0).abs() < 1e-3, "{elevation}");
    }

    /// The light keeps its place relative to the viewer as the view
    /// turns.
    #[test]
    fn the_light_turns_with_the_view() {
        let at = |rotation| {
            let view = View::new(&camera(Zoom::Medium, rotation));
            let light = view.light();
            // The light's direction in the view's fine terms.
            let (a, _) = view.place(light);
            let (o, _) = view.place(Vec3::ZERO);
            ([a[0] - o[0], a[1] - o[1]], light.y)
        };
        let first = at(0);
        for rotation in 1..4 {
            let (d, y) = at(rotation);
            assert!((d[0] - first.0[0]).abs() < 1e-4 && (d[1] - first.0[1]).abs() < 1e-4);
            assert_eq!(y, first.1);
        }
        assert_eq!(View::new(&camera(Zoom::Far, 0)).light(), LIGHT);
    }

    fn lot_with_altitude(corners: [u8; 4]) -> World {
        let mut world = World::empty();
        let full = FULL;
        let data = corners.repeat(full * full);
        world.arrays.insert(
            ALTITUDE,
            sims_formats::lot::Array {
                width: full,
                height: full,
                stride: 4,
                encoding: 0,
                data,
            },
        );
        world
    }

    #[test]
    fn a_person_stands_on_the_terrain() {
        let world = lot_with_altitude([3, 3, 3, 3]);
        let three = sims_sim::head::terrain_height(F32::from_f32(3.0)).to_f32();
        assert_eq!(ground_height(&world, [40.0, 70.4]), Some(three));
        // Rounded as the original rounds: half a sixteenth below the lot
        // is still on it.
        assert_eq!(ground_height(&world, [-0.5, 0.0]), Some(three));
        // Off the lot, or with no altitude, there is no ground.
        assert_eq!(ground_height(&world, [-20.0, 0.0]), None);
        assert_eq!(ground_height(&world, [64.0 * 16.0, 0.0]), None);
        assert_eq!(ground_height(&world, [f32::NAN, 0.0]), None);
        assert_eq!(ground_height(&World::empty(), [40.0, 40.0]), None);
        // A size beyond the arrays does not reach past them.
        let mut wide = world.clone();
        wide.size = 65;
        assert_eq!(ground_height(&wide, [64.0 * 16.0, 0.0]), None);
    }
}
