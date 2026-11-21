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
//! `OBJECT_COLUMN`).
//!
//! **Direct3D 6 rasterization.** The original submits person and plumb-bob
//! meshes to Direct3D 6 (`IDirect3DDevice3::DrawIndexedPrimitive` at
//! 0x5d3b44 and `DrawIndexedPrimitiveVB` at 0x5d36c6) through the house
//! camera (0x49b0de, viewport matrix at 0x5cf263 mapping NDC (0, 0) to
//! integer `(vp.dwX + width/2, vp.dwY + height/2)` with no half-pixel
//! offset). Under Direct3D 6 rasterization rules, pixel `(x, y)` has its
//! sample centre at integer `(x, y)`, whereas [`crate::raster`] samples at
//! `(x + 0.5, y + 0.5)`. [`People::draw`] therefore adds `(+0.5, +0.5)` to
//! each projected vertex so triangles sample at integer pixel centres. Skin
//! bitmaps are uploaded as 16-bit 5-6-5 textures (0x5ca55f → 0x5ca635 →
//! 0x49f0ae, truncating `r >> 3`, `g >> 2`, `b >> 3`) with white materials
//! (0x5d452b, 0x5a1b5c), point filtering (0x5d3a65), Gouraud shading
//! (0x5ce935), `D3DTOP_MODULATE` (0x5d3a30), and dithering off (0x5ce99e);
//! `sims_render::Canvas`'s [`Target`] implementation matches that 5-6-5
//! texel masking and rounded 8-bit modulation.
//!
//! A person stands on the terrain under it (`sims_sim::head::ground_height`:
//! the tile's altitude where its position falls, as the original finds it,
//! 0x478dfe), on every level: the original's model matrix (0x592132) adds
//! the altitude whatever the level, and the renderer lifts an upper
//! level's floor by the terrain under it too. On a level above the ground
//! the original then adds its view's level offset ([`LEVEL_OFFSET`]) once
//! per level above the first: up, and a sixth of a tile towards the viewer
//! along both of the view's axes, whatever the rotation
//! ([`View::place_on`]).
//!
//! **Depth.** [`People::draw_on_lot`] draws over a lot `sims_render` has
//! drawn, as the original does: the lot first (`sims_render::draw_lot`),
//! then the people, each pixel tested against the lot's depth but never
//! part of what the lot drew, then the objects that hold a person
//! (`sims_render::draw_held`), tested against the people's depth. Each vertex takes the lot renderer's [`Depth::point`] (its fine
//! view position, elevation and level, in the original's depth space),
//! which the rasterizer interpolates linearly across each triangle.
//! [`People::draw`] takes any depth function of that shape and any
//! [`Target`]. How the original gives a person's mesh its depth is
//! **unverified**.
//!
//! **The plumb bob** ([`crate::bob`]) is drawn over the person who holds
//! it in their head slot, right after them, by the original's own light
//! and colour; [`People::set_clock`] spins it.
//!
//! **Light** is the original's standard light ([`STANDARD_LIGHT`]) at the
//! person's level ([`level`]), lit as [`Lighting::standard`] says. The
//! light keeps its place relative to the viewer as the view turns: the
//! original turns it by the view's matrix (+0x130 of the view object, a
//! turn about the vertical by a quarter turn per rotation, built at
//! 0x49b60e) before it sets it (0x4b49c7). A person in a room at full
//! light is drawn in full light all over; one in a dark room shows the
//! light's side. Each lit lamp of the person's room adds a warm light from
//! the lamp to the person ([`lamps`]). After dark the level is at most
//! 0.8, as for everything drawn ([`level`]). The level's fade over
//! 250 ms and the person's flag that draws them at full light are not
//! ported (decisions entries "A person is lit
//! by the original's standard light, at their room's level" and "The
//! standard light shines from the viewer's side, and lamps light
//! people").

use crate::animation::{Clip, Library};
use crate::appearance::{Body, Outfit};
use crate::assets::Assets;
use crate::bob::{self, Bob};
use crate::house::Cast;
use crate::math::{Transform, Vec3};
use crate::person::{Player, UNITS_PER_TILE, base_clip, placement};
use crate::raster::{Lamp, Lighting, Shade, Target, draw_part, draw_shaded};
use crate::rig;
use crate::skin::skin;
use crate::suit::{AGE, Clothes, GENDER, JOB, JOB_LEVEL, SUIT, Wardrobe, dress};
use sims_core::F32;
use sims_formats::saved_objects::Person;
use sims_render::Camera;
use sims_render::depth::Depth;
use sims_sim::World;
use std::collections::BTreeMap;
use std::path::Path;

/// Near-zoom pixels per unit of height: 32√6/3.
pub const NEAR_PIXELS_PER_UNIT: f32 = 26.127_89;

/// Fine units (sixteenths of a tile) per unit.
const FINE_PER_UNIT: f32 = 16.0 / UNITS_PER_TILE;

/// What the original adds to a person's position for each level above the
/// first, in model units in the view (x and z along the view's two axes
/// towards the viewer, y up): the three floats at the house view's
/// +0x170 − 12·zoom (0x5922d2, through the view's virtual +0x24, which is
/// +0xb8). The table is filled at run time; these values are read from the
/// running original's memory, and are the same at all three zooms. The
/// original turns the vector by the view's quarter turns back into the
/// lot (0x5c3a20 by −rotation × π/2, the turn it builds the view's own
/// matrix from at 0x49b60e), so it stays the same in the view at every
/// rotation. On screen it nets out to 232.33 near-zoom pixels up, a
/// third of a pixel more than the renderer's floor height, and it puts the
/// person a sixth of a tile nearer.
pub const LEVEL_OFFSET: [f32; 3] = [0.5, 9.300_511, 0.5];

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

    /// A world point's fine position in the view and its elevation, as
    /// [`View::place`] has them, for a person on `level`: moved by the
    /// original's [`LEVEL_OFFSET`] once per level above the first, less
    /// the floor height the lot renderer already raises `level` by. The
    /// offset's sideways parts go along the view's own axes, so they are
    /// the same at every rotation.
    pub fn place_on(&self, p: Vec3, level: i32) -> ([f32; 2], f32) {
        let (fine, elevation) = self.place(p);
        let above = (level - 1).max(0) as f32;
        // Fine x is the original's model z, fine y its model x.
        let toward = |unit: f32| unit * FINE_PER_UNIT * above;
        let floor = sims_render::Zoom::Near.floor_height() as f32;
        let lift = (LEVEL_OFFSET[1] * NEAR_PIXELS_PER_UNIT - floor) * above;
        (
            [
                fine[0] + toward(LEVEL_OFFSET[2]),
                fine[1] + toward(LEVEL_OFFSET[0]),
            ],
            elevation + lift,
        )
    }

    /// How far the original's model root is from the floor's on `level`,
    /// in the drawing frame: its [`LEVEL_OFFSET`] once per level above the
    /// first, whole (the floor's height included). The offset is fixed in
    /// the view, so its sideways part turns back with the camera's
    /// rotation: [`View::place`] of a point moved by it is
    /// [`View::place_on`] of the point, but for the floor height.
    pub fn level_lift(&self, level: i32) -> Vec3 {
        let above = (level - 1).max(0) as f32;
        // The view's fine axes, in units; undo the rotation's turn.
        let [u, v] = [LEVEL_OFFSET[2] * above, LEVEL_OFFSET[0] * above];
        let [x, y] = match self.camera.rotation & 3 {
            1 => [v, -u],
            2 => [-u, -v],
            3 => [-v, u],
            _ => [u, v],
        };
        // Fine (x, y) is the drawing frame's (−y, ·, −x).
        Vec3::new(-y, LEVEL_OFFSET[1] * above, -x)
    }

    /// The screen position of a fine view position on `level`, raised
    /// `elevation` near-zoom pixels: the lot renderer's
    /// [`Camera::project_fine_f32`], one column right as objects are
    /// drawn. The pixel a point falls in is the floor of each coordinate.
    pub fn screen(&self, fine: [f32; 2], elevation: f32, level: i32) -> [f32; 2] {
        let [x, y] = self.camera.project_fine_f32(fine, level, elevation);
        [x + OBJECT_COLUMN, y]
    }

    /// The direction towards the standard light in the drawing frame:
    /// [`STANDARD_LIGHT`], which is in the original's model frame, turned
    /// half a turn about the vertical into ours ([`View::place`] has
    /// fine (x, y) at (−3y/16, 0, −3x/16), the original at (3y/16, 0,
    /// 3x/16)), then turned with the view so it stays where it is
    /// relative to the viewer.
    pub fn light(&self) -> Vec3 {
        // The light's direction in the view at rotation 0, then back out
        // through this rotation.
        let light = Vec3::new(-STANDARD_LIGHT.x, STANDARD_LIGHT.y, -STANDARD_LIGHT.z);
        let fine = [-light.z, -light.x];
        let back = rotate((4 - (self.camera.rotation & 3)) & 3, fine, 0.0);
        Vec3::new(-back[1], light.y, -back[0])
    }
}

/// The direction towards the original's standard light at rotation 0, in
/// the original's model frame, where fine position (x, y) is at (3y/16,
/// height, 3x/16): read from its running memory, where a lamp at fine
/// (424, 472) has its light at (88.5, 8.674, 79.5). [`View::light`] turns
/// it into the drawing frame. The light is
/// `GameData/Lights/stdlight.lgt`: one white directional light of
/// intensity 1 whose direction, the way it shines, is (−0.644, −0.418,
/// −0.641). Towards it is the opposite: up, and back towards the house
/// camera at rotation 0, whose eye is along (0.61, 0.5, 0.61) (0x49b0de),
/// towards larger fine positions, which the lot renderer draws lower and
/// nearer. That direction is also the way to the position the file gives
/// the light, (6.79, 9.55, 6.79): a light above and behind the viewer. A
/// person with their back to the viewer in a dim room is lit on the back,
/// as the original's screenshot shows (decisions entry "The standard
/// light shines from the viewer's side, and lamps light people").
pub const STANDARD_LIGHT: Vec3 = Vec3::new(0.644_362_6, 0.417_676, 0.640_580_7);

/// The level a person is lit at, 0.5 to 1: the light byte of the room
/// at their position as the lot renderer draws it
/// ([`sims_render::lot::room_light`]; full light off the lot and outside
/// every room), over 255, held between 0.5 and the brightest a lit tile
/// may be, 1 by day and 0.8 otherwise ([`sims_render::light::brightest`]).
/// The original reads the level from the drawing's lighting record
/// (0x4b4499), whose only setter holds it so (0x4b4525); the lot's objects set it from the tile's light byte over
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
    let brightest = sims_render::light::brightest(world.globals.get(4).copied().unwrap_or(0));
    (f32::from(byte) / 255.0).clamp(0.5, brightest)
}

/// The furthest a lamp's light reaches a person, squared, in model units
/// (0x6687b8), and how far it stays at full strength (0x6687b4).
const LAMP_REACH: f32 = 400.0;
const LAMP_FULL: f32 = 225.0;

/// How far above the ground under a lamp the original puts its light, in
/// model units (0x467f56, 0x667678).
const LAMP_HEIGHT: f32 = 5.0;

/// The most lamp lights the original keeps for a room (0x467eff).
const ROOM_LAMPS: usize = 6;

/// The lamps' lights on a person on `level` whose skeleton's root is at
/// `at` (the drawing frame: the ground's height under them, moved by
/// [`View::level_lift`] on a level above the first, as the original's
/// model root is). The original sets them with the standard light
/// (0x4b4b04 to 0x4b4c95), from its lighting record of the person: their
/// level and their root, rounded back to fine position (0x4b4987 through
/// 0x454fb9, half away from zero).
///
/// The tile of that fine position (a sixteenth of it, rounded down) must
/// be inside the lot's outer ring of tiles. The room is the word the
/// level's room map holds there; a negative word (a tile a diagonal wall
/// splits) takes no lamps. The room's lights are those its count leaves
/// (0x467ed7): it visits the room's listed tiles in order, repeats
/// included, and each lit lamp ([`sims_render::lot::gives_light`])
/// standing on a tile it visits adds a light, at most [`ROOM_LAMPS`]. So a
/// lamp on a tile listed twice lights twice. A light is at the lamp's fine
/// position, [`LAMP_HEIGHT`] above the ground there, and shines from there
/// to the root at [`lamp_strength`].
///
/// The objects on a tile are taken in ID order. The original takes them
/// in the order the lot links them; the two differ only for a tile with
/// two lit lamps in a room past six lights.
pub fn lamps(world: &World, level: i32, at: Vec3) -> Vec<Lamp> {
    let fine = [-at.z, -at.x].map(|u| (u * FINE_PER_UNIT).round() as i32);
    let [x, y] = fine.map(|f| f.div_euclid(16));
    let full = sims_sim::FULL_SIZE as i32;
    let inner = |n: i32| n >= 1 && n < full - 1;
    if !inner(x) || !inner(y) {
        return Vec::new();
    }
    let room = usize::try_from(level - 1)
        .ok()
        .and_then(|index| world.rooms.map.get(index))
        .and_then(|map| map.get((x * full + y) as usize).copied())
        .filter(|&room| room >= 0);
    let Some(room) = room.and_then(|room| world.rooms.rooms.get(&room)) else {
        return Vec::new();
    };
    room.tiles
        .iter()
        .flat_map(|&tile| {
            world.objects.values().filter(move |o| {
                World::tile(o.position).map(i32::from) == tile.map(i32::from)
                    && sims_render::lot::gives_light(o)
            })
        })
        .take(ROOM_LAMPS)
        .filter_map(|lamp| {
            let [fx, fy, _] = lamp.position;
            let ground = sims_sim::head::ground_height(
                world,
                [F32::from_f32(fx as f32), F32::from_f32(fy as f32)],
            )
            .map_or(0.0, F32::to_f32);
            // The drawing frame: fine (x, y) at (−3y/16, height, −3x/16).
            let light = Vec3::new(
                -(fy as f32) / FINE_PER_UNIT,
                ground + LAMP_HEIGHT,
                -(fx as f32) / FINE_PER_UNIT,
            );
            let from = at - light;
            Some(Lamp {
                towards: (light - at).normalize(),
                strength: lamp_strength(from.dot(from))?,
            })
        })
        .collect()
}

/// A lamp's strength on a person at squared distance `squared` (model
/// units) from its light: 1 below [`LAMP_FULL`], `1 / ((d² − 225) / 2 +
/// 1)` from there to [`LAMP_REACH`], and none past it (0x6687b4, 0x6687b8;
/// the half at 0x666578).
pub fn lamp_strength(squared: f32) -> Option<f32> {
    if squared > LAMP_REACH {
        None
    } else if squared < LAMP_FULL {
        Some(1.0)
    } else {
        Some(1.0 / ((squared - LAMP_FULL) * 0.5 + 1.0))
    }
}

/// The lot renderer's canvas takes people pixel by pixel, by its sprite
/// rule: a depth no farther than what is there wins (ties too), a pixel
/// fainter than its `FAINT` is left out, a soft one blends, and every
/// pixel drawn writes its depth. Depths are cut to whole units toward
/// zero. Texels are masked to the original's 16-bit 5-6-5 texture format
/// (0x49f0ae) and modulated with half-up 8-bit rounding before `Canvas`
/// packs them to 5-6-5 with dithering off (0x5ce99e).
impl Target for sims_render::Canvas {
    fn size(&self) -> (u32, u32) {
        (self.width as u32, self.height as u32)
    }

    fn passes(&self, x: u32, y: u32, z: f32) -> bool {
        sims_render::Canvas::passes(self, x as i32, y as i32, z as i32)
    }

    fn write(&mut self, x: u32, y: u32, z: f32, texel: [u8; 4], shade: f32) {
        self.write_coloured(x, y, z, texel, [shade; 3]);
    }

    fn write_coloured(&mut self, x: u32, y: u32, z: f32, texel: [u8; 4], shade: [f32; 3]) {
        // The original packs skin BMPs into 16-bit 5-6-5 texture surfaces
        // (0x5ca55f -> 0x5ca635 -> 0x49f0ae: r>>3, g>>2, b>>3) before
        // Direct3D 6 modulates them by the Gouraud diffuse colour
        // (D3DTOP_MODULATE at 0x5d3a30) into a 5-6-5 frame buffer with
        // dithering off (D3DRENDERSTATE_DITHERENABLE = 0 at 0x5ce99e).
        let lit = |c: u8, mask: u8, s: f32| (f32::from(c & mask) * s + 0.5).clamp(0.0, 255.0) as u8;
        let [r, g, b, a] = texel;
        self.put(
            x as i32,
            y as i32,
            [
                lit(r, 0xf8, shade[0]),
                lit(g, 0xfc, shade[1]),
                lit(b, 0xf8, shade[2]),
                a,
            ],
            z as i32,
        );
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

/// What makes a person's body: their type, the appearances and
/// accessories their record names, and the data words their suit's
/// clothes are chosen by ([`crate::suit`]). When any of it changes, the
/// body is loaded again.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    guid: u32,
    appearance: [String; 10],
    accessories: Vec<(String, String)>,
    suit: [i16; 5],
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
            suit: [SUIT, JOB, JOB_LEVEL, AGE, GENDER]
                .map(|n| person.data.get(n).copied().unwrap_or(0)),
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
    /// The names a suit's templates are matched against, gathered the
    /// first time a person wears a suit.
    wardrobe: Option<Wardrobe>,
    /// The plumb bob, once a person holds one: loaded the first time.
    bob: Option<Result<Bob, String>>,
    /// The game's clock in milliseconds, which spins the plumb bob.
    clock: u32,
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
            wardrobe: None,
            bob: None,
            clock: 0,
        }
    }

    /// Sets the clock, in milliseconds, that spins the plumb bob (the
    /// original's 0x6e54e0). It starts at 0, where the bob has not spun.
    pub fn set_clock(&mut self, milliseconds: u32) {
        self.clock = milliseconds;
    }

    /// Follows `world`: loads the body of each person who appeared or
    /// changed clothes, and forgets people who are gone. A person wears
    /// what their suit picks ([`crate::suit::dress`]). Returns one line
    /// for each person whose body could not be loaded now, or whose suit
    /// could not be followed and who is drawn in their record's clothes
    /// (each is said once, until the person changes again).
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
                    let outfit = match self.dress(object.guid, person) {
                        Ok(Some(outfit)) => outfit,
                        Ok(None) => resident.outfit,
                        Err(e) => {
                            problems.push(format!(
                                "person {id} is drawn in their record's clothes: {e}"
                            ));
                            resident.outfit
                        }
                    };
                    let body = Body::load(&self.assets, &resident.skeleton, &outfit)?;
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

    /// What `person` (of the type `guid`) wears for their suit, or None
    /// for suit 0, whose clothes are the record's (or, for a record that
    /// names no body, the type's default outfit).
    fn dress(&mut self, guid: u32, person: &Person) -> Result<Option<Outfit>, String> {
        if person.data.get(SUIT).copied().unwrap_or(0) == 0 {
            return Ok(None);
        }
        let assets = &self.assets;
        let wardrobe = self.wardrobe.get_or_insert_with(|| Wardrobe::of(assets));
        let rows = self.cast.body_strings(guid)?;
        let clothes = dress(
            &Clothes::of(person, &rows),
            &person.data,
            &rows,
            self.cast.careers(),
            wardrobe,
        )?;
        Ok(Some(clothes.wear(person, &rows)))
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
            // The terrain lifts a person on every level, as it lifts the
            // floors the renderer draws there (0x592132 adds the altitude
            // whatever the level).
            let at = [animator.position[0], animator.position[1]];
            root.translation.y = sims_sim::head::ground_height(world, at)
                .map(F32::to_f32)
                .unwrap_or(0.0);
            let base = base_clip(person, skeleton);
            match player.pose(&body.skeleton, animator, world.base_layers.get(&id), &base) {
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
    /// renderer has drawn into `canvas` (`sims_render::draw_lot` first,
    /// and `sims_render::draw_held` after, as the original's frame does),
    /// each pixel tested against the lot's depth: walls, furniture and
    /// floors in front of a person hide them. Depth is the lot renderer's
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
        // The plumb bob is loaded the first time the lot has one.
        let mut bob = self.bob.take();
        if bob.is_none() && world.objects.values().any(|o| o.guid == bob::STANDARD) {
            bob = Some(Bob::load(&self.assets));
        }
        let spin = bob::spin(self.clock);
        let bob_light = bob::light(camera.rotation);
        let (mut posed, mut problems) = self.posed(world, camera.level);
        // Far to near by the root's depth, then by ID, so nearer soft
        // edges blend over farther people.
        let far = |p: &Posed| {
            let (fine, elevation) = view.place_on(p.root.translation, p.level);
            depth(fine, elevation, p.level)
        };
        posed.sort_by(|a, b| far(b).total_cmp(&far(a)).then(a.id.cmp(&b.id)));
        for person in &posed {
            let project = |p| {
                let (fine, elevation) = view.place_on(p, person.level);
                let [x, y] = view.screen(fine, elevation, person.level);
                // Direct3D 6 samples pixel (x, y) at integer (x, y);
                // raster::triangle samples at (x + 0.5, y + 0.5).
                Vec3::new(x + 0.5, y + 0.5, depth(fine, elevation, person.level))
            };
            // Each pixel the person draws names them, for a click.
            target.set_id(Some(person.id));
            let lighting = Lighting::standard(light, level(world, person.id));
            let lamps = lamps(
                world,
                person.level,
                person.root.translation + view.level_lift(person.level),
            );
            for part in person.body.parts.iter().filter(|p| p.censor == 0) {
                let skinned = skin(part, &person.bones);
                if lamps.is_empty() {
                    draw_part(target, part, &skinned, lighting, project);
                    continue;
                }
                let shade: Vec<Shade> = skinned
                    .normals
                    .iter()
                    .map(|&n| lighting.shade_with(n, &lamps))
                    .collect();
                draw_shaded(target, part, &skinned, &shade, project);
            }
            // The plumb bob over their head, if they hold it. Its pixels
            // name the bob itself, so the pointer on it finds the bob, not
            // the person: the original's bob draw makes the bob the
            // graphic under the pointer when it changes the depth there
            // (0x586455), and the house view then lists the bob's pie
            // (0x48ca2c → 0x48c834).
            let Some(bob_id) = bob::held_by(world, person.id) else {
                continue;
            };
            let Some(head) =
                rig::bone(&person.body.skeleton, bob::HEAD_BONE).and_then(|i| person.bones.get(i))
            else {
                continue;
            };
            let loaded = match &bob {
                Some(Ok(loaded)) => loaded,
                Some(Err(e)) => {
                    problems.push(format!(
                        "person {}'s plumb bob is not drawn: {e}",
                        person.id
                    ));
                    continue;
                }
                None => continue,
            };
            // The mood, cut to an integer as the original's `_ftol` does.
            let mood = world
                .objects
                .get(&person.id)
                .and_then(|o| o.person.as_ref())
                .map_or(0, |p| p.motives[sims_sim::motives::MOOD] as i32);
            let colour = bob::colour(mood);
            target.set_id(Some(bob_id));
            let bones = [bob::root(*head, person.root, spin)];
            for part in &loaded.parts {
                let skinned = skin(part, &bones);
                let shade: Vec<Shade> = skinned
                    .normals
                    .iter()
                    .map(|&n| Shade::Colour(bob::shade(colour, n, bob_light)))
                    .collect();
                draw_shaded(target, part, &skinned, &shade, project);
            }
        }
        target.set_id(None);
        let people = posed.len();
        drop(posed);
        self.bob = bob;
        Drawn { people, problems }
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

    /// A person upstairs stands where the original's level offset puts
    /// them: 232.33 near-zoom pixels above where they would stand on the
    /// ground floor, straight up, and a sixth of a tile nearer along both
    /// of the view's axes, at every zoom and rotation.
    #[test]
    fn a_person_upstairs_takes_the_originals_level_offset() {
        for zoom in Zoom::ALL {
            for rotation in 0..4 {
                let view = View::new(&camera(zoom, rotation));
                let p = Vec3::new(-7.0, 0.25, -11.0);
                let (low, low_up) = view.place_on(p, 1);
                assert_eq!((low, low_up), view.place(p));
                let (high, high_up) = view.place_on(p, 2);
                for axis in 0..2 {
                    assert!(
                        (high[axis] - low[axis] - 8.0 / 3.0).abs() < 1e-3,
                        "{zoom:?} {rotation}"
                    );
                }
                let a = view.screen(low, low_up, 1);
                let b = view.screen(high, high_up, 2);
                let scale = zoom.near_scale() as f32;
                assert!(
                    (b[0] - a[0]).abs() < 1e-3,
                    "{zoom:?} {rotation}: {a:?} {b:?}"
                );
                assert!(
                    (a[1] - b[1] - 232.33 / scale).abs() < 0.01,
                    "{zoom:?} {rotation}: {} px up",
                    a[1] - b[1]
                );
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
        assert_eq!(canvas.color, vec![[100, 50, 24], [0, 0, 0]]);
        assert_eq!(canvas.depth, vec![100, sims_render::depth::EMPTY]);
        // The pixel taken names the person; the one left out names nobody.
        assert_eq!([canvas.id_at(0, 0), canvas.id_at(1, 0)], [Some(5), None]);
    }

    /// The lot canvas masks texel channels to the original's 5-6-5 texture
    /// surface precision (0x49f0ae) and rounds the modulated 8-bit result
    /// so a 1-ULP barycentric shortfall below 1.0 never loses a 5-6-5 step.
    #[test]
    fn the_lot_canvas_packs_texels_to_565_and_rounds_modulation() {
        use sims_render::Canvas as LotCanvas;
        let mut canvas = LotCanvas::new(3, 1, [0, 0, 0]);
        let target: &mut dyn Target = &mut canvas;
        // 5-6-5 masking drops the low 3/2/3 bits before modulation.
        target.write(0, 0, 0.0, [207, 199, 55, 255], 1.0);
        // 1 ULP below 1.0 (as barycentric weights w0 + w1 + w2 can give)
        // must round back to [40, 136, 40], not truncate to [39, 135, 39].
        target.write(1, 0, 0.0, [40, 136, 40, 255], 0.999_999_94);
        // Coloured (lamp-lit) shading follows the exact same rule per channel.
        target.write_coloured(2, 0, 0.0, [207, 199, 55, 255], [1.0, 0.5, 0.5]);
        assert_eq!(
            canvas.color,
            vec![[200, 196, 48], [40, 136, 40], [200, 98, 24]]
        );
    }

    /// Direct3D 6 samples pixel (x, y) at integer (x, y), so a triangle
    /// spanning [2.0, 4.0] x [3.0, 5.0] in screen coordinates covers pixels
    /// x = 2..=3 and y = 3..=4 (top-left rule) once shifted by (+0.5, +0.5)
    /// into `raster`'s half-integer pixel-centre grid.
    #[test]
    fn direct3d_integer_pixel_centers_shift_projected_triangles_half_a_pixel() {
        use crate::appearance::Part;
        use crate::skin::Skinned;
        use crate::texture::Texture;
        use sims_formats::avatar::{Mesh, MeshBinding, Vertex};
        use sims_render::Canvas as LotCanvas;

        let vertex = |x: f32, y: f32| Vertex {
            position: [x, y, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.5, 0.5],
            bone: 0,
        };
        let part = Part {
            appearance: "quad".into(),
            bone: 0,
            bones: vec![0],
            mesh: Mesh {
                name: "quad".into(),
                texture: String::new(),
                bones: vec!["ROOT".into()],
                bindings: vec![MeshBinding {
                    bone: 0,
                    first_vertex: 0,
                    vertex_count: 4,
                    first_blend: 0,
                    blend_count: 0,
                }],
                triangles: vec![[0, 1, 2], [0, 2, 3]],
                vertices: vec![
                    vertex(2.0, 3.0),
                    vertex(4.0, 3.0),
                    vertex(4.0, 5.0),
                    vertex(2.0, 5.0),
                ],
                blends: Vec::new(),
            },
            texture_name: None,
            texture: Texture::solid([248, 252, 248, 255]),
            censor: 0,
        };
        let skinned = Skinned {
            positions: vec![
                Vec3::new(2.0, 3.0, 0.0),
                Vec3::new(4.0, 3.0, 0.0),
                Vec3::new(4.0, 5.0, 0.0),
                Vec3::new(2.0, 5.0, 0.0),
            ],
            normals: vec![Vec3::new(0.0, 0.0, 1.0); 4],
        };
        // Shift a thin box spanning [2.0, 2.2] x [3.0, 3.2]: integer sample
        // point (2, 3) is inside it, while half-integer (2.5, 3.5) is not.
        let tiny = Skinned {
            positions: vec![
                Vec3::new(1.8, 2.8, 0.0),
                Vec3::new(2.2, 2.8, 0.0),
                Vec3::new(2.2, 3.2, 0.0),
                Vec3::new(1.8, 3.2, 0.0),
            ],
            normals: vec![Vec3::new(0.0, 0.0, 1.0); 4],
        };
        let mut unshifted = LotCanvas::new(6, 6, [0, 0, 0]);
        draw_part(
            &mut unshifted,
            &part,
            &tiny,
            Lighting::standard(Vec3::new(0.0, 0.0, 1.0), 1.0),
            |p| p,
        );
        let mut d3d6 = LotCanvas::new(6, 6, [0, 0, 0]);
        draw_part(
            &mut d3d6,
            &part,
            &tiny,
            Lighting::standard(Vec3::new(0.0, 0.0, 1.0), 1.0),
            |p| Vec3::new(p.x + 0.5, p.y + 0.5, p.z),
        );
        // Without the D3D6 (+0.5, +0.5) shift, [1.8, 2.2] x [2.8, 3.2] hits
        // no half-integer centre; with it, integer pixel (2, 3) is hit.
        assert_eq!(unshifted.color[3 * 6 + 2], [0, 0, 0]);
        assert_eq!(d3d6.color[3 * 6 + 2], [248, 252, 248]);
        // And the 2x2 quad [2, 4] x [3, 5] covers exactly x=2..=3, y=3..=4.
        let mut quad = LotCanvas::new(6, 6, [0, 0, 0]);
        draw_part(
            &mut quad,
            &part,
            &skinned,
            Lighting::standard(Vec3::new(0.0, 0.0, 1.0), 1.0),
            |p| Vec3::new(p.x + 0.5, p.y + 0.5, p.z),
        );
        for y in 0..6 {
            for x in 0..6 {
                let want = if (2..=3).contains(&x) && (3..=4).contains(&y) {
                    [248, 252, 248]
                } else {
                    [0, 0, 0]
                };
                assert_eq!(quad.color[y * 6 + x], want, "({x}, {y})");
            }
        }
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
        let turned = Vec3::new(-STANDARD_LIGHT.x, STANDARD_LIGHT.y, -STANDARD_LIGHT.z);
        assert_eq!(View::new(&camera(Zoom::Far, 0)).light(), turned);
    }

    /// At every rotation the standard light is above and behind the
    /// viewer, straight back along the view: its fine x and y in the view
    /// are equal and grow, towards the viewer, as the house camera's eye
    /// does. The lot renderer draws a larger fine position lower on the
    /// screen, nearer.
    #[test]
    fn the_standard_light_is_above_and_behind_the_viewer() {
        for rotation in 0..4 {
            let view = View::new(&camera(Zoom::Medium, rotation));
            let light = view.light();
            let (a, _) = view.place(light);
            let (o, _) = view.place(Vec3::ZERO);
            let [x, y] = [a[0] - o[0], a[1] - o[1]];
            assert!(
                x > 0.0 && y > 0.0 && (x - y).abs() < 0.1 * x.abs(),
                "{x} {y}"
            );
            assert!(light.y > 0.0);
            // Towards the viewer is down the screen.
            let [_, near] = view.screen(a, 0.0, 1);
            let [_, middle] = view.screen(o, 0.0, 1);
            assert!(near > middle, "{near} {middle}, rotation {rotation}");
        }
    }

    /// A lamp adds its warm light channel by channel on the side facing
    /// it, each channel held to 1; with no lamps the shade is the grey
    /// one, and a side away from the lamp takes nothing from it.
    #[test]
    fn a_lamps_strength_falls_off_between_15_and_20_units() {
        assert_eq!(lamp_strength(0.0), Some(1.0));
        assert_eq!(lamp_strength(224.9), Some(1.0));
        assert_eq!(lamp_strength(225.0), Some(1.0));
        assert_eq!(lamp_strength(227.0), Some(0.5));
        assert_eq!(lamp_strength(400.0), Some(1.0 / 88.5));
        assert_eq!(lamp_strength(400.01), None);
    }

    /// The level lift is the offset `place_on` adds, at every rotation:
    /// a point moved by it is placed where `place_on` puts the point,
    /// but for the floor height the lot renderer raises a level by.
    #[test]
    fn the_level_lift_is_the_offset_place_on_adds() {
        let floor = sims_render::Zoom::Near.floor_height() as f32;
        let p = Vec3::new(-20.0, 3.5, -30.0);
        for rotation in 0..4 {
            let view = View::new(&camera(Zoom::Medium, rotation));
            assert_eq!(view.level_lift(1), Vec3::new(0.0, 0.0, 0.0));
            let (fine, elevation) = view.place(p + view.level_lift(2));
            let (want, want_elevation) = view.place_on(p, 2);
            for axis in 0..2 {
                assert!(
                    (fine[axis] - want[axis]).abs() < 1e-3,
                    "{rotation}: {fine:?} {want:?}"
                );
            }
            assert!(
                (elevation - floor - want_elevation).abs() < 1e-3,
                "{rotation}"
            );
        }
    }

    #[test]
    fn a_lamp_adds_its_warm_light_facing_it() {
        let half = Lighting::standard(STANDARD_LIGHT, 0.5);
        let away = -STANDARD_LIGHT.normalize();
        assert_eq!(half.shade_with(away, &[]), Shade::Grey(half.shade(away)));
        let lamp = Lamp {
            towards: away,
            strength: 0.25,
        };
        let Shade::Colour(lit) = half.shade_with(away, &[lamp]) else {
            panic!("a lamp gives a coloured shade");
        };
        let grey = 128.0 / 255.0;
        for (channel, colour) in lit.into_iter().zip(crate::raster::LAMP_COLOUR) {
            assert!((channel - (grey + 0.25 * colour)).abs() < 1e-6, "{lit:?}");
        }
        assert_eq!(
            half.shade_with(-away, &[lamp]),
            Shade::Colour([1.0; 3]),
            "the side facing the standard light is already full"
        );
        let full = Lamp {
            towards: away,
            strength: 1.0,
        };
        assert_eq!(half.shade_with(away, &[full]), Shade::Colour([1.0; 3]));
    }

    /// At full light a person is at full light all over; at half light
    /// the side away from the light is at the ambient grey, 128/255, and
    /// the side facing it at full light. A person the world does not know
    /// is at full light.
    #[test]
    fn the_level_sets_the_ambient_grey_and_the_lights_strength() {
        assert_eq!(level(&World::empty(), 7), 1.0);
        // After dark nothing is lit above 0.8.
        let mut night = World::empty();
        night.globals = vec![0; 32];
        night.globals[4] = 2;
        assert_eq!(level(&night, 7), f32::from_bits(0x3f4c_cccd));
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
