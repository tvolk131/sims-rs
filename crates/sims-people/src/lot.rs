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
//! A person stands on the terrain under it (`sims_sim::head::ground_height`:
//! the tile's altitude where its position falls, as the original finds it,
//! 0x478dfe).
//! On a level above the ground the original adds an offset from the
//! view's table, which is not ported: such a person stands at the level's
//! floor.
//!
//! **Depth.** [`People::draw_on_lot`] draws over a lot `sims_render` has
//! drawn, as the original does: the lot first, then the people, each
//! pixel tested against the lot's depth but never part of what the lot
//! drew. Each vertex takes the lot renderer's [`Depth::point`] (its fine
//! view position, elevation and level, in the original's depth space),
//! which the rasterizer interpolates linearly across each triangle.
//! [`People::draw`] takes any depth function of that shape and any
//! [`Target`]. How the original gives a person's mesh its depth is
//! **unverified**.
//!
//! **Light** is the original's standard light ([`STANDARD_LIGHT`]) at the
//! person's level ([`level`]), lit as [`Lighting::standard`] says. The
//! light keeps its place relative to the viewer as the view turns: the
//! original turns it by the view's matrix (+0x130 of the view object, a
//! turn about the vertical by a quarter turn per rotation, built at
//! 0x49b60e) before it sets it (0x4b49c7). A person in a room at full
//! light is drawn in full light all over; one in a dark room shows the
//! light's side. The room's lamps (one light each, from the lamp to the
//! person), the level's fade over 250 ms, the dimmer daylight cap after
//! dark (0.8) and the person's flag that draws them at full light are
//! not ported (decisions entry "A person is lit by the original's
//! standard light, at their room's level").

use crate::animation::{Clip, Library};
use crate::appearance::Body;
use crate::assets::Assets;
use crate::house::Cast;
use crate::math::{Transform, Vec3};
use crate::person::{Player, UNITS_PER_TILE, base_clip, placement};
use crate::raster::{Lighting, Target, draw_part};
use crate::skin::skin;
use sims_core::F32;
use sims_render::Camera;
use sims_render::depth::Depth;
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
    /// `elevation` near-zoom pixels: the lot renderer's
    /// [`Camera::project_fine_f32`], one column right as objects are
    /// drawn. The pixel a point falls in is the floor of each coordinate.
    pub fn screen(&self, fine: [f32; 2], elevation: f32, level: i32) -> [f32; 2] {
        let [x, y] = self.camera.project_fine_f32(fine, level, elevation);
        [x + OBJECT_COLUMN, y]
    }

    /// The direction towards the standard light: [`STANDARD_LIGHT`] as it
    /// is at rotation 0, turned with the view so it stays where it is
    /// relative to the viewer.
    pub fn light(&self) -> Vec3 {
        // The light's direction in the view at rotation 0, then back out
        // through this rotation.
        let light = STANDARD_LIGHT;
        let fine = [-light.z, -light.x];
        let back = rotate((4 - (self.camera.rotation & 3)) & 3, fine, 0.0);
        Vec3::new(-back[1], light.y, -back[0])
    }
}

/// The direction towards the original's standard light at rotation 0, in
/// the drawing frame (the files' frame, [`crate::rig`]). The light is
/// `GameData/Lights/stdlight.lgt`: one white directional light of
/// intensity 1 whose direction, the way it shines, is (−0.644, −0.418,
/// −0.641). Towards it is the opposite: up, and back towards the house
/// camera at rotation 0, whose eye is along (0.61, 0.5, 0.61) (0x49b0de).
/// That direction is also the way to the position the file gives the
/// light, (6.79, 9.55, 6.79): a light above and behind the viewer. That
/// the direction is the way the light shines, as Direct3D takes a
/// directional light's, is read from the file's own position, not from
/// the running game; a person at full light, the only kind compared,
/// shows no side.
pub const STANDARD_LIGHT: Vec3 = Vec3::new(0.644_362_6, 0.417_676, 0.640_580_7);

/// The level a person is lit at, 0.5 to 1: the light byte of the room
/// at their position as the lot renderer draws it
/// ([`sims_render::lot::room_light`]; full light off the lot and outside
/// every room), over 255, held between 0.5 and 1. The original reads the
/// level from the drawing's lighting record (0x4b4499), whose only setter
/// holds it between 0.5 and the brightest a lit tile may be, 1 by day
/// (0x4b4525); the lot's objects set it from the tile's light byte over
/// 255 (0x467482). That a person's record is set the same way, from the
/// tile they stand on, is **unverified**.
pub fn level(world: &World, person: u16) -> f32 {
    let byte = world
        .objects
        .get(&person)
        .and_then(|o| world.room_at(o.position).ok())
        .filter(|&room| room >= 0)
        .and_then(|room| world.rooms.rooms.get(&room))
        .map_or(255, sims_render::lot::room_light);
    (f32::from(byte) / 255.0).clamp(0.5, 1.0)
}

/// The lot renderer's canvas takes people pixel by pixel, by its sprite
/// rule: a depth no farther than what is there wins (ties too), a pixel
/// fainter than its `FAINT` is left out, a soft one blends, and every
/// pixel drawn writes its depth. Depths are cut to whole units toward
/// zero.
impl Target for sims_render::Canvas {
    fn size(&self) -> (u32, u32) {
        (self.width as u32, self.height as u32)
    }

    fn passes(&self, x: u32, y: u32, z: f32) -> bool {
        sims_render::Canvas::passes(self, x as i32, y as i32, z as i32)
    }

    fn write(&mut self, x: u32, y: u32, z: f32, texel: [u8; 4], shade: f32) {
        let lit = |c: u8| (f32::from(c) * shade).min(255.0) as u8;
        let [r, g, b, a] = texel;
        self.put(x as i32, y as i32, [lit(r), lit(g), lit(b), a], z as i32);
    }

    fn set_id(&mut self, id: Option<u16>) {
        sims_render::Canvas::set_id(self, id);
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
                let at = [animator.position[0], animator.position[1]];
                root.translation.y = sims_sim::head::ground_height(world, at)
                    .map(F32::to_f32)
                    .unwrap_or(0.0);
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

    /// Draws every placed person on the camera's levels over a lot the lot
    /// renderer has drawn into `canvas` (`sims_render::draw` first), each
    /// pixel tested against the lot's depth: walls, furniture and floors
    /// in front of a person hide them. Depth is the lot renderer's
    /// [`Depth::point`]; the pixels go in by its sprite rule
    /// ([`sims_render::Canvas::put`]).
    pub fn draw_on_lot(
        &mut self,
        world: &World,
        camera: &Camera,
        canvas: &mut sims_render::Canvas,
    ) -> Drawn {
        let depth = Depth::new(camera);
        self.draw(world, camera, canvas, |fine, elevation, level| {
            depth.point(fine, elevation, level)
        })
    }

    /// Draws every placed person on the camera's levels into `target`,
    /// far to near. `depth` gives the target's depth for a point: its fine
    /// view position, its elevation above its level in near-zoom pixels,
    /// and the level (the lot renderer's [`Depth::point`]). Call
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
            // Each pixel the person draws names them, for a click.
            target.set_id(Some(person.id));
            let lighting = Lighting::standard(light, level(world, person.id));
            for part in person.body.parts.iter().filter(|p| p.censor == 0) {
                let skinned = skin(part, &person.bones);
                draw_part(target, part, &skinned, lighting, |p| {
                    let (fine, elevation) = view.place(p);
                    let [x, y] = view.screen(fine, elevation, person.level);
                    Vec3::new(x, y, depth(fine, elevation, person.level))
                });
            }
        }
        target.set_id(None);
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

    /// A person's pixel goes into the lot renderer's canvas by its sprite
    /// rule: a tie or nearer wins and writes its depth, farther loses, a
    /// faint texel is left out, and the shade darkens the colour.
    #[test]
    fn the_lot_canvas_takes_a_persons_pixels_by_the_sprite_rule() {
        use sims_render::Canvas as LotCanvas;
        let mut canvas = LotCanvas::new(2, 1, [0, 0, 0]);
        canvas.plot(0, 0, 100, [9, 9, 9]);
        let target: &mut dyn Target = &mut canvas;
        assert_eq!(target.size(), (2, 1));
        target.set_id(Some(5));
        assert!(!target.passes(0, 0, 101.0));
        // Cut toward zero: 100.9 is 100, a tie.
        assert!(target.passes(0, 0, 100.9));
        target.write(0, 0, 100.9, [200, 100, 50, 255], 0.5);
        target.write(
            1,
            0,
            -3.5,
            [200, 100, 50, sims_render::canvas::FAINT - 1],
            1.0,
        );
        assert_eq!(canvas.color, vec![[100, 50, 25], [0, 0, 0]]);
        assert_eq!(canvas.depth, vec![100, sims_render::depth::EMPTY]);
        // The pixel taken names the person; the one left out names nobody.
        assert_eq!([canvas.id_at(0, 0), canvas.id_at(1, 0)], [Some(5), None]);
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
        assert_eq!(View::new(&camera(Zoom::Far, 0)).light(), STANDARD_LIGHT);
    }

    /// At every rotation the standard light is above and behind the
    /// viewer, straight back along the view: its fine x and y in the view
    /// are equal and fall towards the viewer, as the house camera's eye
    /// does.
    #[test]
    fn the_standard_light_is_above_and_behind_the_viewer() {
        for rotation in 0..4 {
            let view = View::new(&camera(Zoom::Medium, rotation));
            let light = view.light();
            let (a, _) = view.place(light);
            let (o, _) = view.place(Vec3::ZERO);
            let [x, y] = [a[0] - o[0], a[1] - o[1]];
            assert!(
                x < 0.0 && y < 0.0 && (x - y).abs() < 0.1 * x.abs(),
                "{x} {y}"
            );
            assert!(light.y > 0.0);
        }
    }

    /// At full light a person is at full light all over; at half light
    /// the side away from the light is at the ambient grey, 128/255, and
    /// the side facing it at full light. A person the world does not know
    /// is at full light.
    #[test]
    fn the_level_sets_the_ambient_grey_and_the_lights_strength() {
        assert_eq!(level(&World::empty(), 7), 1.0);
        let towards = STANDARD_LIGHT.normalize();
        let full = Lighting::standard(STANDARD_LIGHT, 1.0);
        assert_eq!(full.shade(-towards), 1.0);
        let half = Lighting::standard(STANDARD_LIGHT, 0.5);
        assert_eq!(half.shade(-towards), 128.0 / 255.0);
        assert_eq!(half.shade(towards), 1.0);
        // At an angle, the light adds its strength times the cosine: at
        // 60° from the light, half its strength.
        let side = Vec3::new(towards.z, 0.0, -towards.x).normalize();
        let tilted = (towards * 0.5 + side * (3.0f32.sqrt() / 2.0)).normalize();
        assert!((tilted.dot(towards) - 0.5).abs() < 1e-6);
        assert!((half.shade(tilted) - (128.0 / 255.0 + 0.25)).abs() < 1e-6);
    }
}
