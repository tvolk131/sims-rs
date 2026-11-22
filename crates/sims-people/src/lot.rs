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
//! `(vp.dwX + width/2, vp.dwY + height/2)` with no half-pixel offset —
//! integer for even dimensions such as 800×600). Under Direct3D 6
//! rasterization rules, pixel `(x, y)` has its sample centre at integer
//! `(x, y)`, whereas [`crate::raster`] samples at `(x + 0.5, y + 0.5)`.
//! [`People::draw`] therefore adds `(+0.5, +0.5)` to each projected vertex
//! via `project_lot_vertex` so triangles sample at integer pixel centres.
//! Skin bitmaps are packed into 16-bit 5-6-5 textures on upload (0x5ca0d3 /
//! 0x5ca1f8 for indexed 8-bit skins, 0x5ca55f for 24-bit → 0x5ca635 →
//! 0x49f0ae, truncating `r >> 3`, `g >> 2`, `b >> 3`) with white materials
//! (0x5d452b, 0x5a1b5c), default point filtering (0x49b090, 0x5d3a65),
//! Gouraud shading (0x5ce935), `D3DTOP_MODULATE` (0x5d3a30), and dithering
//! off (0x5ce99e). [`People::draw`] wraps textured parts' writes in
//! `Packed565` to truncate texels to 5-6-5 and expand them back to 8-bit
//! unorm by MSB bit-replication (`(r5 << 3) | (r5 >> 2)`,
//! `(g6 << 2) | (g6 >> 4)`, `(b5 << 3) | (b5 >> 2)`) before modulation—a
//! standard unorm5/unorm6 expansion chosen for the Direct3D 6 rasterizer
//! (whose internal expansion is outside `Sims.exe`)—preserving the original
//! 8-bit bitmaps in the shared `Body` cache read by [`People::posed`] (such
//! as the pie-menu portrait renderer) and leaving untextured meshes (such as
//! the plumb bob's `white` material at 0x5d452b, texture handle 0 at
//! 0x5d48ce) at full 8-bit white—and `sims_render::Canvas`'s [`Target`]
//! implementation modulates with half-up 8-bit rounding (an approximation
//! that avoids truncating 1-ULP barycentric shortfalls below 1.0 by a full
//! 5-6-5 step).
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
//! light's side. Each lit lamp of the person's room adds a warm
//! `D3DLIGHT_PARALLELPOINT` light (`0x4673d0` → `0x5d009a` → `0x5d0d0b` →
//! `0x5d09a0(4)`): its strength follows the person's squared distance to the
//! lamp (`0x4b4ba6`..`0x4b4c37`), while Direct3D 6 transforms its
//! `dvPosition` into view space (`M_view * [dvPosition, 1]`) and takes the
//! parallel light direction from the view-space origin (the house camera's
//! eye, [`View::eye`], `0x49b0de`) towards `dvPosition` ([`lamps`]). After
//! dark the level is at most 0.8, as for everything drawn ([`level`]). The
//! level's fade over 250 ms and the person's flag that draws them at full
//! light are not ported (decisions entries "A person is lit by the
//! original's standard light, at their room's level", "The standard light
//! shines from the viewer's side, and lamps light people", and "Room lamps
//! are Direct3D 6 parallel point lights and 5-6-5 skin texels expand by MSB
//! bit-replication").

use crate::animation::{Clip, Library};
use crate::appearance::{Body, Outfit};
use crate::assets::Assets;
use crate::bob::{self, Bob};
use crate::house::Cast;
use crate::math::{Transform, Vec3};
use crate::person::{Player, UNITS_PER_TILE, base_clip, placement};
use crate::raster::{Lamp, Lighting, Shade, Target, draw_shaded_culled_scratch};
use crate::rig;
use crate::skin::{Skinned, skin_into};
use crate::suit::{AGE, Clothes, GENDER, JOB, JOB_LEVEL, SUIT, Wardrobe, dress};
use sims_core::F32;
use sims_formats::saved_objects::Person;
use sims_render::Camera;
use sims_render::depth::Depth;
use sims_sim::World;
use sims_sim::animator::{Animator, Base, Layer};
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

    /// The house camera's eye position in the drawing frame for a viewport
    /// of `size` `(width, height)` pixels, as `0x49b0de` passes to
    /// `0x5d0595`. Used to compute the view-space position of room lamps
    /// (`D3DLIGHT_PARALLELPOINT`, [`lamps`]).
    ///
    /// In `0x49b0de`, the orthographic house camera starts `eye_dist` units
    /// (`far - 12.5`, with `far = √3·(√2·3·64)/2 + (255·3/(2√6) + 68)/2`)
    /// behind the rotated lot corner along the 30°-down isometric view axis,
    /// then shifts along its screen right and up axes by
    /// `((width >> 1) - scroll_x)` and `((height >> 1) - scroll_y)` pixels
    /// scaled by `3√2 / (16 << zoom)`. `Camera::origin[0]` is `scroll_x - 1`
    /// (matching [`OBJECT_COLUMN`]).
    pub fn eye(&self, size: (u32, u32)) -> Vec3 {
        let span = sims_render::camera::ARRAY_SIZE as f32 * UNITS_PER_TILE;
        let h = (255.0 / (2.0 * 6.0f32.sqrt())) * UNITS_PER_TILE + 68.0;
        let far = 3.0f32.sqrt() * (2.0f32.sqrt() * span) * 0.5 + h * 0.5;
        let eye_dist = far - 12.5;
        let scale_px = 2.0f32.sqrt() * UNITS_PER_TILE / self.camera.zoom.tile_width() as f32;
        let s2 = 0.5f32.sqrt();
        let s6 = (3.0f32 / 8.0).sqrt();
        let s3 = 0.75f32.sqrt();
        let h2 = 0.5 * s2;
        // Basis in our drawing frame (half-turn about Y from Sims's world
        // frame: X_ours = -X_sims, Y_ours = Y_sims, Z_ours = -Z_sims).
        let (corner, eye_dir, right, up) = match self.camera.rotation & 3 {
            1 => (
                Vec3::new(-span, 0.0, 0.0),
                Vec3::new(s6, 0.5, -s6),
                Vec3::new(s2, 0.0, s2),
                Vec3::new(-h2, s3, h2),
            ),
            2 => (
                Vec3::new(-span, 0.0, -span),
                Vec3::new(s6, 0.5, s6),
                Vec3::new(-s2, 0.0, s2),
                Vec3::new(-h2, s3, -h2),
            ),
            3 => (
                Vec3::new(0.0, 0.0, -span),
                Vec3::new(-s6, 0.5, s6),
                Vec3::new(-s2, 0.0, -s2),
                Vec3::new(h2, s3, -h2),
            ),
            _ => (
                Vec3::ZERO,
                Vec3::new(-s6, 0.5, -s6),
                Vec3::new(s2, 0.0, -s2),
                Vec3::new(h2, s3, h2),
            ),
        };
        let scroll_x = i64::from(self.camera.origin[0]) + 1;
        let scroll_y = i64::from(self.camera.origin[1]);
        let dx = ((size.0 / 2) as i64 - scroll_x) as f32 * scale_px;
        let dy = ((size.1 / 2) as i64 - scroll_y) as f32 * scale_px;
        corner + eye_dir * eye_dist + right * dx - up * dy
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

const FULL_SIDE: usize = sims_sim::FULL_SIZE as usize;

/// Indexed view of every lit lamp (`sims_render::lot::gives_light`) in a
/// [`World`], reused across frames so [`lamps`] never rescans `world.objects`
/// for each tile in a room and avoids per-frame heap allocations.
#[derive(Debug)]
struct LitLamps {
    /// Flat `(tile, fine_position)` entries in `world.objects` ID order.
    entries: Vec<([i32; 3], [i32; 3])>,
    mask: [[u64; FULL_SIDE]; 2],
    has_other: bool,
    selected: Vec<[i32; 3]>,
}

impl Default for LitLamps {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            mask: [[0u64; FULL_SIDE]; 2],
            has_other: false,
            selected: Vec::with_capacity(ROOM_LAMPS),
        }
    }
}

impl LitLamps {
    fn from_world(world: &World) -> Self {
        let mut lit = Self::default();
        lit.populate(world);
        lit
    }

    fn populate(&mut self, world: &World) {
        self.entries.clear();
        self.mask = [[0u64; FULL_SIDE]; 2];
        self.has_other = false;
        let full = sims_sim::FULL_SIZE as i32;
        for o in world.objects.values() {
            if !sims_render::lot::gives_light(o) {
                continue;
            }
            let tile = World::tile(o.position).map(i32::from);
            if (1..=2).contains(&tile[2])
                && (0..full).contains(&tile[0])
                && (0..full).contains(&tile[1])
            {
                self.mask[(tile[2] - 1) as usize][tile[0] as usize] |= 1u64 << tile[1];
            } else {
                self.has_other = true;
            }
            self.entries.push((tile, o.position));
        }
    }

    fn lamps_for_into(
        &mut self,
        world: &World,
        level: i32,
        at: Vec3,
        eye: Vec3,
        out: &mut Vec<Lamp>,
    ) {
        out.clear();
        if self.entries.is_empty() {
            return;
        }
        let fine = [-at.z, -at.x].map(|u| (u * FINE_PER_UNIT).round() as i32);
        let [x, y] = fine.map(|f| f.div_euclid(16));
        let full = sims_sim::FULL_SIZE as i32;
        let inner = |n: i32| n >= 1 && n < full - 1;
        if !inner(x) || !inner(y) {
            return;
        }
        let room = usize::try_from(level - 1)
            .ok()
            .and_then(|index| world.rooms.map.get(index))
            .and_then(|map| map.get((x * full + y) as usize).copied())
            .filter(|&room| room >= 0);
        let Some(room) = room.and_then(|room| world.rooms.rooms.get(&room)) else {
            return;
        };
        self.selected.clear();
        'tiles: for &tile in &room.tiles {
            let tile = tile.map(i32::from);
            let present = if (1..=2).contains(&tile[2])
                && (0..full).contains(&tile[0])
                && (0..full).contains(&tile[1])
            {
                (self.mask[(tile[2] - 1) as usize][tile[0] as usize] >> tile[1]) & 1 != 0
            } else {
                self.has_other
            };
            if !present {
                continue;
            }
            for &(entry_tile, pos) in &self.entries {
                if entry_tile == tile {
                    self.selected.push(pos);
                    if self.selected.len() == ROOM_LAMPS {
                        break 'tiles;
                    }
                }
            }
        }
        for &[fx, fy, _] in &self.selected {
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
            if let Some(strength) = lamp_strength(from.dot(from)) {
                out.push(Lamp {
                    towards: (light - eye).normalize(),
                    strength,
                });
            }
        }
    }

    fn lamps_for(&mut self, world: &World, level: i32, at: Vec3, eye: Vec3) -> Vec<Lamp> {
        let mut out = Vec::new();
        self.lamps_for_into(world, level, at, eye, &mut out);
        out
    }
}

/// The lamps' lights on a person on `level` whose skeleton's root is at
/// `at`, viewed from house camera position `eye` ([`View::eye`]), both in
/// the drawing frame (`at` is the ground's height under the person, moved
/// by [`View::level_lift`] on a level above the first, as the original's
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
/// lamp on a tile listed twice lights twice.
///
/// A light is placed at the lamp's fine position, [`LAMP_HEIGHT`] above
/// the ground there, with [`lamp_strength`] computed from its squared
/// distance to `at` (0x4b4ba6..0x4b4c37). Because `Sims.exe` creates room
/// lamps as `D3DLIGHT_PARALLELPOINT` (`dltType = 4`, `0x4673d0` →
/// `0x5d009a` → `0x5d0d0b` → `0x5d09a0(4)`), Direct3D 6 ignores
/// `dvDirection`, transforms `dvPosition` into camera space
/// (`M_view * [dvPosition, 1]`), and lights with the unit vector from the
/// camera-space origin towards that transformed position—i.e.,
/// `(light - eye).normalize()` in the drawing frame.
///
/// The objects on a tile are taken in ID order. The original takes them
/// in the order the lot links them; the two differ only for a tile with
/// two lit lamps in a room past six lights.
pub fn lamps(world: &World, level: i32, at: Vec3, eye: Vec3) -> Vec<Lamp> {
    LitLamps::from_world(world).lamps_for(world, level, at, eye)
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
/// zero. Channels are modulated with half-up 8-bit rounding before
/// `Canvas::put` stores 8-bit RGB (quantized to 5-6-5 by `rgb565()`).
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
        // Half-up 8-bit rounding approximates Direct3D 6 modulation without
        // truncating a 1-ULP barycentric shortfall below 1.0 down by a full
        // 5-6-5 step. Textured lot parts are wrapped in `Packed565` by
        // `People::draw`, while untextured meshes (the plumb bob's `white`
        // material at 0x5d452b) stay full 8-bit white.
        let lit = |c: u8, s: f32| (f32::from(c) * s + 0.5).clamp(0.0, 255.0) as u8;
        let [r, g, b, a] = texel;
        self.put(
            x as i32,
            y as i32,
            [lit(r, shade[0]), lit(g, shade[1]), lit(b, shade[2]), a],
            z as i32,
        );
    }

    fn set_id(&mut self, id: Option<u16>) {
        sims_render::Canvas::set_id(self, id);
    }

    #[inline]
    fn row_index(&self, y: u32) -> usize {
        (y as usize).saturating_mul(self.width)
    }

    #[inline]
    fn passes_at(&self, row: usize, x: u32, y: u32, z: f32) -> bool {
        (y as usize) < self.height && self.depth_passes_at(row, x as usize, z as i32)
    }

    #[inline]
    fn write_passed_at(&mut self, row: usize, x: u32, y: u32, z: f32, texel: [u8; 4], shade: f32) {
        self.write_coloured_passed_at(row, x, y, z, texel, [shade; 3]);
    }

    #[inline]
    fn write_coloured_passed_at(
        &mut self,
        row: usize,
        x: u32,
        y: u32,
        z: f32,
        texel: [u8; 4],
        shade: [f32; 3],
    ) {
        if (x as usize) >= self.width || (y as usize) >= self.height {
            return;
        }
        let lit = |c: u8, s: f32| (f32::from(c) * s + 0.5).clamp(0.0, 255.0) as u8;
        let [r, g, b, a] = texel;
        self.put_passed_at(
            row.saturating_add(x as usize),
            [lit(r, shade[0]), lit(g, shade[1]), lit(b, shade[2]), a],
            z as i32,
        );
    }
}

/// Packs a texel's RGB channels to 16-bit 5-6-5 by truncation (verified in
/// `Sims.exe`'s texture surface upload at `0x5ca0d3` / `0x5ca1f8` for
/// indexed 8-bit skins and `0x5ca55f` for 24-bit BMPs → `0x5ca635` →
/// `0x49f0ae`) and expands them back to 8-bit unorm by MSB bit-replication
/// (`(r5 << 3) | (r5 >> 2)`, `(g6 << 2) | (g6 >> 4)`,
/// `(b5 << 3) | (b5 >> 2)`). Direct3D 6's internal 5-6-5-to-8-bit expansion
/// before `D3DTOP_MODULATE` lives inside Direct3D's rasterizer, not
/// `Sims.exe`; MSB bit-replication is a standard unorm5/unorm6 expansion
/// mapping maximum 5-6-5 white `(31, 63, 31)` to `[255, 255, 255]` (and
/// differing by at most 1 from rounded normalization on some low/mid codes).
fn expand_565([r, g, b, a]: [u8; 4]) -> [u8; 4] {
    let r5 = r >> 3;
    let g6 = g >> 2;
    let b5 = b >> 3;
    [
        (r5 << 3) | (r5 >> 2),
        (g6 << 2) | (g6 >> 4),
        (b5 << 3) | (b5 >> 2),
        a,
    ]
}

/// Target adapter used by [`People::draw`] for textured parts: quantizes
/// each sampled texel to 5-6-5 and expands it by MSB bit-replication on
/// the lot draw path without mutating the cached `Body` textures (which
/// [`People::posed`] shares with offscreen callers like the pie-menu
/// portrait renderer) and without affecting untextured meshes such as the
/// plumb bob.
pub(crate) struct Packed565<'a, T: ?Sized>(pub(crate) &'a mut T);

impl<T: Target + ?Sized> Target for Packed565<'_, T> {
    fn size(&self) -> (u32, u32) {
        self.0.size()
    }

    fn passes(&self, x: u32, y: u32, z: f32) -> bool {
        self.0.passes(x, y, z)
    }

    fn write(&mut self, x: u32, y: u32, z: f32, texel: [u8; 4], shade: f32) {
        self.0.write(x, y, z, expand_565(texel), shade);
    }

    fn write_coloured(&mut self, x: u32, y: u32, z: f32, texel: [u8; 4], shade: [f32; 3]) {
        self.0.write_coloured(x, y, z, expand_565(texel), shade);
    }

    fn set_id(&mut self, id: Option<u16>) {
        self.0.set_id(id);
    }

    #[inline]
    fn row_index(&self, y: u32) -> usize {
        self.0.row_index(y)
    }

    #[inline]
    fn passes_at(&self, row: usize, x: u32, y: u32, z: f32) -> bool {
        self.0.passes_at(row, x, y, z)
    }

    #[inline]
    fn write_passed_at(&mut self, row: usize, x: u32, y: u32, z: f32, texel: [u8; 4], shade: f32) {
        self.0
            .write_passed_at(row, x, y, z, expand_565(texel), shade);
    }

    #[inline]
    fn write_coloured_passed_at(
        &mut self,
        row: usize,
        x: u32,
        y: u32,
        z: f32,
        texel: [u8; 4],
        shade: [f32; 3],
    ) {
        self.0
            .write_coloured_passed_at(row, x, y, z, expand_565(texel), shade);
    }
}

/// Draws one skinned part of a person on the lot into `target`, with or
/// without room lamps, culling counter-clockwise screen-space back faces
/// as Direct3D 6's default `D3DRENDERSTATE_CULLMODE` (`D3DCULL_CCW`, never
/// overridden in `0x5ce911`–`0x5cea52`, `0x5d34c6`–`0x5d37f1`, or
/// `0x5d38d5`–`0x5d3c69`) does.
#[allow(clippy::too_many_arguments)]
fn draw_lot_part_scratch<T: Target + ?Sized>(
    target: &mut T,
    part: &crate::appearance::Part,
    skinned: &crate::skin::Skinned,
    lighting: Lighting,
    lamps: &[Lamp],
    project: impl Fn(Vec3) -> Vec3,
    shades: &mut Vec<Shade>,
    screen: &mut Vec<Vec3>,
) {
    let towards = lighting.towards.normalize();
    shades.clear();
    if lamps.is_empty() {
        shades.extend(
            skinned
                .normals
                .iter()
                .map(|&n| Shade::Grey(lighting.shade_unit(n, towards))),
        );
    } else {
        shades.extend(
            skinned
                .normals
                .iter()
                .map(|&n| lighting.shade_with_unit(n, towards, lamps)),
        );
    }
    draw_shaded_culled_scratch(target, part, skinned, shades, project, screen);
}

#[cfg(test)]
fn draw_lot_part<T: Target + ?Sized>(
    target: &mut T,
    part: &crate::appearance::Part,
    skinned: &crate::skin::Skinned,
    lighting: Lighting,
    lamps: &[Lamp],
    project: impl Fn(Vec3) -> Vec3,
) {
    let mut shades = Vec::new();
    let mut screen = Vec::new();
    draw_lot_part_scratch(
        target,
        part,
        skinned,
        lighting,
        lamps,
        project,
        &mut shades,
        &mut screen,
    );
}

/// Projects a world point for lot rasterization: [`View::place_on`] and
/// [`View::screen`], shifted by `(+0.5, +0.5)` so [`crate::raster`]'s
/// half-integer pixel-centre sampler matches Direct3D 6's integer pixel
/// centres (`0x49b0de`, `0x5cf263`, `0x5d3b44`).
fn project_lot_vertex(
    view: &View,
    p: Vec3,
    level: i32,
    depth: &impl Fn([f32; 2], f32, i32) -> f32,
) -> Vec3 {
    let (fine, elevation) = view.place_on(p, level);
    let [x, y] = view.screen(fine, elevation, level);
    Vec3::new(x + 0.5, y + 0.5, depth(fine, elevation, level))
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

    fn matches(&self, guid: u32, person: &sims_formats::saved_objects::Person) -> bool {
        if self.guid != guid {
            return false;
        }
        let suit =
            [SUIT, JOB, JOB_LEVEL, AGE, GENDER].map(|n| person.data.get(n).copied().unwrap_or(0));
        if self.suit != suit {
            return false;
        }
        if self
            .appearance
            .iter()
            .zip(&person.appearance)
            .any(|(a, b)| a != b)
        {
            return false;
        }
        self.accessories.len() == person.accessories.len()
            && self
                .accessories
                .iter()
                .zip(&person.accessories)
                .all(|((n, b), a)| n == &a.name && b == &a.binding)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LayerKey {
    clip: String,
    phase: F32,
    mode: i32,
    frames: u32,
    weight: F32,
}

impl LayerKey {
    fn from_layer(layer: &Layer) -> Self {
        Self {
            clip: layer.clip.clone(),
            phase: layer.phase,
            mode: layer.mode,
            frames: layer.frames,
            weight: layer.weight,
        }
    }

    fn matches(&self, layer: &Layer) -> bool {
        self.phase == layer.phase
            && self.mode == layer.mode
            && self.frames == layer.frames
            && self.weight == layer.weight
            && self.clip == layer.clip
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BasePoseKey {
    Empty,
    Layer(LayerKey),
    FallbackScript {
        clip: String,
        reverse: bool,
        start: i32,
        clock: i32,
    },
    FallbackPosture {
        saved_base: String,
        clock: i32,
    },
}

impl BasePoseKey {
    fn from_state(animator: &Animator, layer: Option<&Layer>, person: &Person) -> Self {
        match (&animator.base, layer) {
            (Base::Empty, _) => Self::Empty,
            (_, Some(layer)) => Self::Layer(LayerKey::from_layer(layer)),
            (
                Base::Script {
                    clip,
                    reverse,
                    start,
                },
                None,
            ) => Self::FallbackScript {
                clip: clip.clone(),
                reverse: *reverse,
                start: *start,
                clock: animator.clock,
            },
            (_, None) => Self::FallbackPosture {
                saved_base: person.animations[2].clone(),
                clock: animator.clock,
            },
        }
    }

    fn matches(&self, animator: &Animator, layer: Option<&Layer>, person: &Person) -> bool {
        match (self, &animator.base, layer) {
            (Self::Empty, Base::Empty, _) => true,
            (Self::Layer(k), base, Some(layer)) if !matches!(base, Base::Empty) => k.matches(layer),
            (
                Self::FallbackScript {
                    clip,
                    reverse,
                    start,
                    clock,
                },
                Base::Script {
                    clip: a_clip,
                    reverse: a_reverse,
                    start: a_start,
                },
                None,
            ) => {
                *reverse == *a_reverse
                    && *start == *a_start
                    && *clock == animator.clock
                    && clip == a_clip
            }
            (Self::FallbackPosture { saved_base, clock }, base, None)
                if !matches!(base, Base::Empty | Base::Script { .. }) =>
            {
                *clock == animator.clock && saved_base == &person.animations[2]
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PoseCacheKey {
    position: [F32; 3],
    ground_bits: u32,
    head_turn: F32,
    base: BasePoseKey,
    foreground: Option<LayerKey>,
    carry: Option<LayerKey>,
}

impl PoseCacheKey {
    fn from_state(
        animator: &Animator,
        ground_bits: u32,
        base_layer: Option<&Layer>,
        person: &Person,
    ) -> Self {
        Self {
            position: animator.position,
            ground_bits,
            head_turn: animator.head.turn,
            base: BasePoseKey::from_state(animator, base_layer, person),
            foreground: animator.foreground.as_ref().map(LayerKey::from_layer),
            carry: animator.carry.as_ref().map(LayerKey::from_layer),
        }
    }

    fn matches(
        &self,
        animator: &Animator,
        ground_bits: u32,
        base_layer: Option<&Layer>,
        person: &Person,
    ) -> bool {
        let opt_matches = |cached: &Option<LayerKey>, live: &Option<Layer>| match (cached, live) {
            (None, None) => true,
            (Some(k), Some(l)) => k.matches(l),
            _ => false,
        };
        self.position == animator.position
            && self.ground_bits == ground_bits
            && self.head_turn == animator.head.turn
            && opt_matches(&self.foreground, &animator.foreground)
            && opt_matches(&self.carry, &animator.carry)
            && self.base.matches(animator, base_layer, person)
    }
}

/// Cached posed bones and skinned parts for a single Sim so steady-state
/// frames between simulation ticks skip re-posing and re-skinning, and dynamic
/// frames reuse `Skinned` vertex/normal buffers in place.
#[derive(Debug)]
struct CachedPose {
    key: PoseCacheKey,
    root: Transform,
    bones: Vec<Transform>,
    skinned_parts: Vec<Skinned>,
    skinned_ready: bool,
}

/// Reusable per-frame scratch buffers for [`People::draw_in_viewport`].
#[derive(Debug, Default)]
struct PeopleScratch {
    shades: Vec<Shade>,
    screen: Vec<Vec3>,
    bob_skinned: Skinned,
    lamps: Vec<Lamp>,
    lit_lamps: LitLamps,
    ready: Vec<(u16, i32)>,
    draw_order: Vec<(u16, i32, f32)>,
}

/// One person as last loaded: the body, or why it could not be loaded
/// (not tried again until the key changes).
#[derive(Debug)]
struct Loaded {
    key: Key,
    body: Result<(Body, String), String>,
    cached_pose: Option<CachedPose>,
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
    /// Reusable per-frame scratch buffers for posing, lighting, and rasterization.
    scratch: PeopleScratch,
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
            scratch: PeopleScratch::default(),
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
            if self
                .people
                .get(&id)
                .is_some_and(|l| l.key.matches(object.guid, person))
            {
                continue;
            }
            let key = Key::of(object.guid, person);
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
            self.people.insert(
                id,
                Loaded {
                    key,
                    body,
                    cached_pose: None,
                },
            );
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

    /// Updates each eligible placed Sim's cached pose (and optionally skins
    /// uncensored parts) in place, populating `self.scratch.ready` and
    /// returning any posing problems.
    fn refresh_poses(&mut self, world: &World, top: i32, skin_parts: bool) -> Vec<String> {
        let mut player =
            Player::with_clips(&self.assets, &self.library, std::mem::take(&mut self.clips));
        self.scratch.ready.clear();
        let mut problems = Vec::new();
        let size = i32::from(world.size);
        for (&id, animator) in &world.animators {
            let (Some(loaded), Some(object)) = (self.people.get_mut(&id), world.objects.get(&id))
            else {
                continue;
            };
            let Ok((body, skeleton)) = &loaded.body else {
                continue;
            };
            let Some(person) = &object.person else {
                continue;
            };
            let [x, y, level] = World::tile(object.position).map(i32::from);
            if !(0..size).contains(&x) || !(0..size).contains(&y) || level > top {
                continue;
            }
            let Some(mut root) = placement(animator) else {
                continue;
            };
            // The terrain lifts a person on every level, as it lifts the
            // floors the renderer draws there (0x592132 adds the altitude
            // whatever the level).
            let at = [animator.position[0], animator.position[1]];
            let ground = sims_sim::head::ground_height(world, at)
                .map(F32::to_f32)
                .unwrap_or(0.0);
            root.translation.y = ground;
            let ground_bits = ground.to_bits();
            let base_layer = world.base_layers.get(&id);

            let pose_hit = loaded
                .cached_pose
                .as_ref()
                .is_some_and(|c| c.key.matches(animator, ground_bits, base_layer, person));

            if !pose_hit {
                let base = base_clip(person, skeleton);
                match player.pose(&body.skeleton, animator, base_layer, &base) {
                    Ok(pose) => {
                        let key =
                            PoseCacheKey::from_state(animator, ground_bits, base_layer, person);
                        let bones = pose.world(&body.skeleton, root);
                        if let Some(cached) = &mut loaded.cached_pose {
                            cached.key = key;
                            cached.root = root;
                            cached.bones = bones;
                            cached.skinned_ready = false;
                        } else {
                            loaded.cached_pose = Some(CachedPose {
                                key,
                                root,
                                bones,
                                skinned_parts: Vec::new(),
                                skinned_ready: false,
                            });
                        }
                    }
                    Err(e) => {
                        loaded.cached_pose = None;
                        problems.push(format!("person {id} is not posed: {e}"));
                        continue;
                    }
                }
            }

            if skin_parts
                && let Some(cached) = &mut loaded.cached_pose
                && !cached.skinned_ready
            {
                let uncensored = body.parts.iter().filter(|p| p.censor == 0).count();
                if cached.skinned_parts.len() < uncensored {
                    cached.skinned_parts.resize_with(uncensored, || Skinned {
                        positions: Vec::new(),
                        normals: Vec::new(),
                    });
                } else {
                    cached.skinned_parts.truncate(uncensored);
                }
                for (part, out) in body
                    .parts
                    .iter()
                    .filter(|p| p.censor == 0)
                    .zip(&mut cached.skinned_parts)
                {
                    skin_into(part, &cached.bones, out);
                }
                cached.skinned_ready = true;
            }
            self.scratch.ready.push((id, level));
        }
        self.clips = player.into_clips();
        problems
    }

    /// Each placed person whose body is loaded, posed as the world has
    /// them now, in object ID order. A person on a level above `top`, or
    /// whose object is off the lot, is left out. Call [`People::update`] first, so new people are loaded.
    pub fn posed(&mut self, world: &World, top: i32) -> (Vec<Posed<'_>>, Vec<String>) {
        let problems = self.refresh_poses(world, top, false);
        let mut posed = Vec::with_capacity(self.scratch.ready.len());
        for &(id, level) in &self.scratch.ready {
            if let Some(loaded) = self.people.get(&id)
                && let Ok((body, _)) = &loaded.body
                && let Some(cached) = &loaded.cached_pose
            {
                posed.push(Posed {
                    id,
                    level,
                    root: cached.root,
                    bones: cached.bones.clone(),
                    body,
                });
            }
        }
        (posed, problems)
    }

    /// Renders `person`'s pie-menu 3D head ([`crate::pie_head::draw_body`])
    /// into an RGBA [`crate::raster::Canvas`] of `size` pixels looking as
    /// `look` specifies. Call [`People::update`] first so the person's body
    /// is loaded.
    pub fn pie_head(
        &self,
        person: u16,
        size: [u32; 2],
        look: crate::pie_head::Look,
    ) -> Result<crate::raster::Canvas, String> {
        let loaded = self
            .people
            .get(&person)
            .ok_or_else(|| format!("person {person} is not loaded"))?;
        let (body, _) = loaded.body.as_ref().map_err(Clone::clone)?;
        crate::pie_head::draw_body(body, size, look).map_err(|e| format!("person {person}: {e}"))
    }

    /// Renders `person`'s pie-menu 3D head positioned at `center` inside the
    /// active house `viewport` ([`crate::pie_head::draw_body_in_viewport`]),
    /// clipped to the house viewport boundary and native near/far frustum for
    /// `zoom` rather than the 201×201 face bitmap (`0x462621` → `0x462985`).
    /// Call [`People::update`] first.
    pub fn pie_head_in_viewport(
        &self,
        person: u16,
        viewport: [u32; 2],
        center: [i32; 2],
        zoom: sims_render::Zoom,
        look: crate::pie_head::Look,
    ) -> Result<crate::pie_head::DrawnHead, String> {
        let loaded = self
            .people
            .get(&person)
            .ok_or_else(|| format!("person {person} is not loaded"))?;
        let (body, _) = loaded.body.as_ref().map_err(Clone::clone)?;
        crate::pie_head::draw_body_in_viewport(body, viewport, center, zoom, look)
            .map_err(|e| format!("person {person}: {e}"))
    }

    /// Draws every placed person on the camera's levels over a lot the lot
    /// renderer has drawn into `canvas` (`sims_render::draw_lot` first,
    /// and `sims_render::draw_held` after, as the original's frame does),
    /// each pixel tested against the lot's depth: walls, furniture and
    /// floors in front of a person hide them. Depth is the lot renderer's
    /// [`Depth::point`]; the pixels go in by its sprite rule
    /// ([`sims_render::Canvas::put`]). Uses the full canvas dimensions as
    /// the house viewport for [`View::eye`].
    pub fn draw_on_lot(
        &mut self,
        world: &World,
        camera: &Camera,
        canvas: &mut sims_render::Canvas,
    ) -> Drawn {
        let viewport = (canvas.width as u32, canvas.height as u32);
        self.draw_on_lot_in_viewport(world, camera, viewport, canvas)
    }

    /// Like [`People::draw_on_lot`], using `viewport` `(width, height)` as
    /// the active 3D house viewport passed to [`View::eye`] (`0x49ae5e`,
    /// `0x4f4b43`, `0x4d363b`, `0x53b2eb` → `0x4b136b` → `0x49b0bf` →
    /// `0x5cf1b6`: `(width, height)` with the bottom strip closed,
    /// `(width, height - 100)` with the strip open).
    pub fn draw_on_lot_in_viewport(
        &mut self,
        world: &World,
        camera: &Camera,
        viewport: (u32, u32),
        canvas: &mut sims_render::Canvas,
    ) -> Drawn {
        let depth = Depth::new(camera);
        self.draw_in_viewport(world, camera, viewport, canvas, |fine, elevation, level| {
            depth.point(fine, elevation, level)
        })
    }

    /// Draws every placed person on the camera's levels into `target`,
    /// far to near. `depth` gives the target's depth for a point: its fine
    /// view position, elevation above its level in near-zoom pixels, and
    /// the level (the lot renderer's [`Depth::point`]). Uses `target.size()`
    /// as the house viewport for [`View::eye`]. Call [`People::update`] first.
    pub fn draw<T: Target + ?Sized>(
        &mut self,
        world: &World,
        camera: &Camera,
        target: &mut T,
        depth: impl Fn([f32; 2], f32, i32) -> f32,
    ) -> Drawn {
        self.draw_in_viewport(world, camera, target.size(), target, depth)
    }

    /// Like [`People::draw`], taking the active 3D house `viewport` size
    /// `(width, height)` for [`View::eye`] (`0x49b0de`) independently of
    /// `target.size()` (e.g. `800×500` over an `800×600` framebuffer when
    /// the live-mode strip is open).
    pub fn draw_in_viewport<T: Target + ?Sized>(
        &mut self,
        world: &World,
        camera: &Camera,
        viewport: (u32, u32),
        target: &mut T,
        depth: impl Fn([f32; 2], f32, i32) -> f32,
    ) -> Drawn {
        let view = View::new(camera);
        let light = view.light();
        let eye = view.eye(viewport);
        // The plumb bob is loaded the first time the lot has one.
        let mut bob = self.bob.take();
        if bob.is_none() && world.objects.values().any(|o| o.guid == bob::STANDARD) {
            bob = Some(Bob::load(&self.assets));
        }
        let spin = bob::spin(self.clock);
        let bob_light = bob::light(camera.rotation);
        let mut problems = self.refresh_poses(world, camera.level, true);
        let People {
            people: loaded_people,
            scratch,
            ..
        } = self;
        // Far to near by the root's depth, then by ID, so nearer soft
        // edges blend over farther people.
        scratch.draw_order.clear();
        for &(id, person_level) in &scratch.ready {
            if let Some(loaded) = loaded_people.get(&id)
                && let Some(cached) = &loaded.cached_pose
            {
                let (fine, elevation) = view.place_on(cached.root.translation, person_level);
                let far = depth(fine, elevation, person_level);
                scratch.draw_order.push((id, person_level, far));
            }
        }
        scratch
            .draw_order
            .sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
        if !scratch.draw_order.is_empty() {
            scratch.lit_lamps.populate(world);
        }
        let drawn_count = scratch.draw_order.len();
        for i in 0..drawn_count {
            let (person_id, person_level, _) = scratch.draw_order[i];
            let Some(loaded) = loaded_people.get(&person_id) else {
                continue;
            };
            let Ok((body, _)) = &loaded.body else {
                continue;
            };
            let Some(cached) = &loaded.cached_pose else {
                continue;
            };
            let project = |p| project_lot_vertex(&view, p, person_level, &depth);
            // Each pixel the person draws names them, for a click.
            target.set_id(Some(person_id));
            let lighting = Lighting::standard(light, level(world, person_id));
            scratch.lit_lamps.lamps_for_into(
                world,
                person_level,
                cached.root.translation + view.level_lift(person_level),
                eye,
                &mut scratch.lamps,
            );
            for (part, skinned) in body
                .parts
                .iter()
                .filter(|p| p.censor == 0)
                .zip(&cached.skinned_parts)
            {
                if part.texture_name.is_some() {
                    let mut packed = Packed565(target);
                    draw_lot_part_scratch(
                        &mut packed,
                        part,
                        skinned,
                        lighting,
                        &scratch.lamps,
                        project,
                        &mut scratch.shades,
                        &mut scratch.screen,
                    );
                } else {
                    draw_lot_part_scratch(
                        target,
                        part,
                        skinned,
                        lighting,
                        &scratch.lamps,
                        project,
                        &mut scratch.shades,
                        &mut scratch.screen,
                    );
                }
            }
            // The plumb bob over their head, if they hold it. Its pixels
            // name the bob itself, so the pointer on it finds the bob, not
            // the person: the original's bob draw makes the bob the
            // graphic under the pointer when it changes the depth there
            // (0x586455), and the house view then lists the bob's pie
            // (0x48ca2c → 0x48c834).
            let Some(bob_id) = bob::held_by(world, person_id) else {
                continue;
            };
            let Some(head) =
                rig::bone(&body.skeleton, bob::HEAD_BONE).and_then(|i| cached.bones.get(i))
            else {
                continue;
            };
            let loaded_bob = match &bob {
                Some(Ok(loaded)) => loaded,
                Some(Err(e)) => {
                    problems.push(format!(
                        "person {}'s plumb bob is not drawn: {e}",
                        person_id
                    ));
                    continue;
                }
                None => continue,
            };
            // The mood, cut to an integer as the original's `_ftol` does.
            let mood = world
                .objects
                .get(&person_id)
                .and_then(|o| o.person.as_ref())
                .map_or(0, |p| p.motives[sims_sim::motives::MOOD] as i32);
            let colour = bob::colour(mood);
            target.set_id(Some(bob_id));
            let bones = [bob::root(*head, cached.root, spin)];
            for part in &loaded_bob.parts {
                skin_into(part, &bones, &mut scratch.bob_skinned);
                scratch.shades.clear();
                scratch.shades.extend(
                    scratch
                        .bob_skinned
                        .normals
                        .iter()
                        .map(|&n| Shade::Colour(bob::shade(colour, n, bob_light))),
                );
                draw_shaded_culled_scratch(
                    target,
                    part,
                    &scratch.bob_skinned,
                    &scratch.shades,
                    project,
                    &mut scratch.screen,
                );
            }
        }
        target.set_id(None);
        self.bob = bob;
        Drawn {
            people: drawn_count,
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
        assert_eq!(canvas.color, vec![[100, 50, 25], [0, 0, 0]]);
        assert_eq!(canvas.depth, vec![100, sims_render::depth::EMPTY]);
        // The pixel taken names the person; the one left out names nobody.
        assert_eq!([canvas.id_at(0, 0), canvas.id_at(1, 0)], [Some(5), None]);
    }

    /// `Packed565` truncates textured lot draws to 5-6-5 (`0x49f0ae`) and
    /// expands them back to 8-bit unorm by MSB bit-replication before
    /// modulation without modifying cached `Body` textures, while direct
    /// writes (untextured meshes such as the plumb bob's `white` material at
    /// `0x5d452b`) keep full 8-bit channels, and the lot canvas rounds
    /// modulation so a 1-ULP barycentric shortfall below 1.0 never loses a
    /// 5-6-5 step.
    #[test]
    fn the_lot_canvas_packs_textured_parts_to_565_and_rounds_modulation() {
        use sims_render::Canvas as LotCanvas;

        let mut canvas = LotCanvas::new(5, 1, [0, 0, 0]);
        // Textured grey write quantizes to 5-6-5 and expands via MSB bit-replication:
        // 207 -> r5=25 -> 206, 199 -> g6=49 -> 199, 55 -> b5=6 -> 49.
        {
            let mut packed = Packed565(&mut canvas);
            packed.write(0, 0, 0.0, [207, 199, 55, 255], 1.0);
            // Textured coloured (lamp-lit) write expands per channel and rounds:
            // [206*1.0, 199*0.5, 49*0.5] -> [206, 100, 25].
            packed.write_coloured(1, 0, 0.0, [207, 199, 55, 255], [1.0, 0.5, 0.5]);
            // Maximum 5-6-5 white [248, 252, 248] expands to full [255, 255, 255].
            packed.write(2, 0, 0.0, [248, 252, 248, 255], 1.0);
        }
        // 1 ULP below 1.0 (as barycentric weights w0 + w1 + w2 can give)
        // must round back to [40, 136, 40], not truncate to [39, 135, 39].
        let target: &mut dyn Target = &mut canvas;
        target.write(3, 0, 0.0, [40, 136, 40, 255], 0.999_999_94);
        // Untextured plumb bob modulated at full 255 without 5-6-5 packing.
        target.write_coloured(4, 0, 0.0, [255, 255, 255, 255], [0.5, 1.0, 0.25]);
        assert_eq!(
            canvas.color,
            vec![
                [206, 199, 49],
                [206, 100, 25],
                [255, 255, 255],
                [40, 136, 40],
                [128, 255, 64],
            ]
        );
    }

    /// `View::eye` (`0x49b0de`) places the orthographic camera at the exact
    /// depth and screen offset computed by `0x49b0de`: every world point
    /// projects onto the same screen coordinates as `View::screen`, the
    /// distance along the view axis matches `eye_dist = far - 12.5` at every
    /// rotation and zoom (including odd viewports and negative horizontal
    /// scroll), and shortening the viewport by the 100-pixel live strip shifts
    /// the eye by `50 * scale_px` along the camera's up axis.
    #[test]
    fn the_house_camera_eye_projects_world_points_onto_the_lot_view() {
        let span = sims_render::camera::ARRAY_SIZE as f32 * UNITS_PER_TILE;
        let h = (255.0 / (2.0 * 6.0f32.sqrt())) * UNITS_PER_TILE + 68.0;
        let far = 3.0f32.sqrt() * (2.0f32.sqrt() * span) * 0.5 + h * 0.5;
        let expected_eye_dist = far - 12.5;
        let s2 = 0.5f32.sqrt();
        let s6 = (3.0f32 / 8.0).sqrt();
        let s3 = 0.75f32.sqrt();
        let h2 = 0.5 * s2;
        for zoom in Zoom::ALL {
            let scale_px = 2.0f32.sqrt() * UNITS_PER_TILE / zoom.tile_width() as f32;
            for rotation in 0..4 {
                let (corner, eye_dir, right, up) = match rotation {
                    1 => (
                        Vec3::new(-span, 0.0, 0.0),
                        Vec3::new(s6, 0.5, -s6),
                        Vec3::new(s2, 0.0, s2),
                        Vec3::new(-h2, s3, h2),
                    ),
                    2 => (
                        Vec3::new(-span, 0.0, -span),
                        Vec3::new(s6, 0.5, s6),
                        Vec3::new(-s2, 0.0, s2),
                        Vec3::new(-h2, s3, -h2),
                    ),
                    3 => (
                        Vec3::new(0.0, 0.0, -span),
                        Vec3::new(-s6, 0.5, s6),
                        Vec3::new(-s2, 0.0, -s2),
                        Vec3::new(h2, s3, -h2),
                    ),
                    _ => (
                        Vec3::ZERO,
                        Vec3::new(-s6, 0.5, -s6),
                        Vec3::new(s2, 0.0, -s2),
                        Vec3::new(h2, s3, h2),
                    ),
                };
                for &(origin, size) in &[
                    ([279, -2004], (800, 600)),
                    ([335, -560], (1024, 768)),
                    ([-250, -560], (801, 599)),
                ] {
                    let c = Camera {
                        zoom,
                        rotation,
                        level: 1,
                        origin,
                        walls: Walls::Down,
                        focus: Default::default(),
                    };
                    let view = View::new(&c);
                    let eye = view.eye(size);
                    // Depth along the view axis must equal `eye_dist` from `0x49b0de`.
                    let depth_from_corner = (eye - corner).dot(eye_dir);
                    assert!(
                        (depth_from_corner - expected_eye_dist).abs() < 1e-3,
                        "{zoom:?} rot={rotation}: depth {depth_from_corner} != {expected_eye_dist}"
                    );
                    for p in [
                        Vec3::new(-88.5, 8.674, -79.5),
                        Vec3::new(-79.5, 3.674, -72.0),
                        Vec3::new(-12.0, 0.0, -150.0),
                    ] {
                        let (fine, elev) = view.place(p);
                        let want = view.screen(fine, elev, 1);
                        let rel = p - eye;
                        let sx = rel.dot(right) / scale_px + (size.0 / 2) as f32;
                        let sy = -rel.dot(up) / scale_px + (size.1 / 2) as f32;
                        assert!(
                            (sx - want[0]).abs() < 1e-3 && (sy - want[1]).abs() < 1e-3,
                            "{zoom:?} rot={rotation} {p:?}: got ({sx}, {sy}) != {want:?}"
                        );
                    }
                }
            }
        }
        // Pin the absolute `near-r2-0805` eye position against `0x49b0de`.
        let r2 = View::new(&Camera {
            zoom: Zoom::Near,
            rotation: 2,
            level: 1,
            origin: [279, -2004],
            walls: Walls::Down,
            focus: Default::default(),
        })
        .eye((800, 600));
        assert!(
            (r2.x - 37.166).abs() < 1e-3
                && (r2.y - 101.228).abs() < 1e-3
                && (r2.z - 42.791).abs() < 1e-3,
            "near-r2-0805 eye: {r2:?}"
        );
        // Shortening an 800×600 viewport by the 100-pixel live strip moves the
        // eye by 50 * scale_px along `up` (≈ 6.629 model units at far zoom).
        let far_view = View::new(&Camera {
            zoom: Zoom::Far,
            rotation: 0,
            level: 1,
            origin: [367, -130],
            walls: Walls::Cutaway,
            focus: Default::default(),
        });
        let full_eye = far_view.eye((800, 600));
        let strip_eye = far_view.eye((800, 500));
        let shift = (strip_eye - full_eye).length();
        let expected_shift = 50.0 * 2.0f32.sqrt() * UNITS_PER_TILE / Zoom::Far.tile_width() as f32;
        assert!(
            (shift - expected_shift).abs() < 1e-4,
            "far strip eye shift: {shift} != {expected_shift}"
        );
    }

    /// `project_lot_vertex` (used by `People::draw`) adds `(+0.5, +0.5)` to
    /// `View::screen` so `raster`'s half-integer pixel-centre sampler matches
    /// Direct3D 6's integer pixel centres (0x49b0de, 0x5cf263, 0x5d3b44).
    #[test]
    fn direct3d_integer_pixel_centers_shift_projected_triangles_half_a_pixel() {
        use crate::appearance::Part;
        use crate::raster::draw_part;
        use crate::skin::Skinned;
        use crate::texture::Texture;
        use sims_formats::avatar::{Mesh, MeshBinding, Vertex};
        use sims_render::Canvas as LotCanvas;

        let c = Camera {
            zoom: Zoom::Near,
            rotation: 0,
            level: 1,
            origin: [0, 0],
            walls: Walls::Up,
            focus: Default::default(),
        };
        let view = View::new(&c);
        // A world point whose `View::screen` lands on integer (sx, sy).
        let world_p = Vec3::new(0.0, 0.0, 0.0);
        let (fine, elev) = view.place_on(world_p, 1);
        let [sx, sy] = view.screen(fine, elev, 1);
        let projected = project_lot_vertex(&view, world_p, 1, &|_, _, _| 0.0);
        assert_eq!(projected.x, sx + 0.5);
        assert_eq!(projected.y, sy + 0.5);

        // Place a tiny world-space quad whose `View::screen` spans
        // [cx - 0.2, cx + 0.2] x [cy - 0.2, cy + 0.2] around an integer
        // pixel (cx, cy). Only `project_lot_vertex` hits pixel (cx, cy).
        let (cx, cy) = (sx.round() as i32, sy.round() as i32);
        let c = Camera {
            origin: [3 - cx, 3 - cy],
            ..c
        };
        let view = View::new(&c);
        let [sx, sy] = view.screen(fine, elev, 1);
        assert!((sx - 3.0).abs() < 1e-4 && (sy - 3.0).abs() < 1e-4);

        let vertex = |p: Vec3| Vertex {
            position: [p.x, p.y, p.z],
            normal: [0.0, 0.0, 1.0],
            uv: [0.5, 0.5],
            bone: 0,
        };
        // Along world (x - z), screen x moves; along world y, screen y moves.
        let dx = Vec3::new(0.005, 0.0, -0.005);
        let dy = Vec3::new(0.0, 0.008, 0.0);
        let corners = [
            world_p - dx - dy,
            world_p + dx - dy,
            world_p + dx + dy,
            world_p - dx + dy,
        ];
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
                vertices: corners.into_iter().map(vertex).collect(),
                blends: Vec::new(),
            },
            texture_name: None,
            texture: Texture::solid([248, 252, 248, 255]),
            censor: 0,
        };
        let skinned = Skinned {
            positions: corners.to_vec(),
            normals: vec![Vec3::new(0.0, 0.0, 1.0); 4],
        };
        let mut unshifted = LotCanvas::new(6, 6, [0, 0, 0]);
        draw_part(
            &mut unshifted,
            &part,
            &skinned,
            Lighting::standard(Vec3::new(0.0, 0.0, 1.0), 1.0),
            |p| {
                let (f, e) = view.place_on(p, 1);
                let [x, y] = view.screen(f, e, 1);
                Vec3::new(x, y, 0.0)
            },
        );
        let mut d3d6 = LotCanvas::new(6, 6, [0, 0, 0]);
        draw_part(
            &mut d3d6,
            &part,
            &skinned,
            Lighting::standard(Vec3::new(0.0, 0.0, 1.0), 1.0),
            |p| project_lot_vertex(&view, p, 1, &|_, _, _| 0.0),
        );
        assert_eq!(unshifted.color[3 * 6 + 3], [0, 0, 0]);
        assert_eq!(d3d6.color[3 * 6 + 3], [248, 252, 248]);
    }

    /// `draw_lot_part` culls counter-clockwise screen-space back faces so a
    /// nearer back-facing triangle (such as hair or limb interior geometry)
    /// never overwrites a front-facing triangle, matching Direct3D 6's
    /// default `D3DCULL_CCW`.
    #[test]
    fn draw_lot_part_culls_counter_clockwise_back_faces() {
        use crate::appearance::Part;
        use crate::skin::Skinned;
        use crate::texture::Texture;
        use sims_formats::avatar::{Mesh, MeshBinding, Vertex};
        use sims_render::Canvas as LotCanvas;

        let make_part = |triangles: Vec<[u32; 3]>, color: [u8; 4]| Part {
            appearance: "tri".into(),
            bone: 0,
            bones: vec![0],
            mesh: Mesh {
                name: "tri".into(),
                texture: String::new(),
                bones: vec!["ROOT".into()],
                bindings: vec![MeshBinding {
                    bone: 0,
                    first_vertex: 0,
                    vertex_count: 3,
                    first_blend: 0,
                    blend_count: 0,
                }],
                triangles,
                vertices: vec![
                    Vertex {
                        position: [0.0; 3],
                        normal: [0.0, 0.0, 1.0],
                        uv: [0.5, 0.5],
                        bone: 0,
                    };
                    3
                ],
                blends: Vec::new(),
            },
            texture_name: None,
            texture: Texture::solid(color),
            censor: 0,
        };
        let lighting = Lighting::standard(Vec3::new(0.0, 0.0, 1.0), 1.0);
        let front = make_part(vec![[0, 1, 2]], [200, 100, 50, 255]);
        let back = make_part(vec![[0, 2, 1]], [50, 200, 100, 255]);
        let skinned = |z: f32| Skinned {
            positions: vec![
                Vec3::new(1.0, 1.0, z),
                Vec3::new(7.0, 1.0, z),
                Vec3::new(4.0, 7.0, z),
            ],
            normals: vec![Vec3::new(0.0, 0.0, 1.0); 3],
        };
        let mut canvas = LotCanvas::new(8, 8, [0, 0, 0]);
        draw_lot_part(&mut canvas, &front, &skinned(100.0), lighting, &[], |p| p);
        draw_lot_part(&mut canvas, &back, &skinned(50.0), lighting, &[], |p| p);
        assert_eq!(canvas.color[3 * 8 + 4], [200, 100, 50]);
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

    /// `lamps` treats each lit room lamp as a Direct3D 6
    /// `D3DLIGHT_PARALLELPOINT` light (`0x4673d0` → `0x5d0d0b`): direction is
    /// `(light - eye).normalize()` (depending on camera `eye`, not the
    /// person's position `at`), while strength follows the squared distance
    /// between `at` and `light`. When `eye == light`, direction is zero
    /// rather than NaN.
    #[test]
    fn room_lamps_use_parallel_point_direction_from_camera_eye() {
        let mut world = World::empty();
        world.size = 64;
        let full = sims_sim::FULL_SIZE as usize;
        let mut map = vec![0i16; full * full];
        for x in 10..=20 {
            for y in 10..=20 {
                map[x * full + y] = 4;
            }
        }
        world.rooms.map = [map, Vec::new()];
        let room = sims_sim::rooms::Room {
            tiles: vec![[12, 12, 1]],
            ..Default::default()
        };
        world.rooms.rooms.insert(4, room);
        let mut data = vec![0i16; 64];
        (data[16], data[40], data[51]) = (1, 0x400, 25);
        world.objects.insert(
            71,
            sims_sim::Object {
                id: 71,
                guid: 0,
                footprint: [0; 4],
                position: [12 * 16 + 8, 12 * 16 + 8, 1],
                unknown_short: 0,
                attributes: Vec::new(),
                temps: Vec::new(),
                data,
                obsolete_word: 0,
                stack_capacity_bytes: 0,
                stack: Vec::new(),
                relationship_flag: 0,
                relationships: Vec::new(),
                slots: Vec::new(),
                dynamic_sprites: Vec::new(),
                multipart: None,
                portal: None,
                person: None,
                registry_word: None,
                last_result: false,
                removal_skips_resets: false,
                selected: false,
            },
        );

        let light_pos = Vec3::new(
            -(200.0) / FINE_PER_UNIT,
            LAMP_HEIGHT,
            -(200.0) / FINE_PER_UNIT,
        );
        let eye = Vec3::new(100.0, 100.0, 100.0);
        let expected_dir = (light_pos - eye).normalize();

        // Two people on opposite sides of the lamp receive the exact same
        // parallel-point direction, with different distance-based strengths
        // (near: d² = 43 < 225 → 1.0; far: d² = 245.5 ∈ (225, 400) → attenuated).
        let near_at = Vec3::new(-(184.0) / FINE_PER_UNIT, 0.0, -(184.0) / FINE_PER_UNIT);
        let far_at = Vec3::new(-(256.0) / FINE_PER_UNIT, 0.0, -(256.0) / FINE_PER_UNIT);
        let near_lamps = lamps(&world, 1, near_at, eye);
        let far_lamps = lamps(&world, 1, far_at, eye);
        assert_eq!(near_lamps.len(), 1);
        assert_eq!(far_lamps.len(), 1);
        assert_eq!(near_lamps[0].towards, expected_dir);
        assert_eq!(far_lamps[0].towards, expected_dir);
        assert_eq!(near_lamps[0].strength, 1.0);
        assert!(far_lamps[0].strength < 1.0 && far_lamps[0].strength > 0.0);

        // Degenerate eye == light produces zero direction, not NaN.
        let degenerate = lamps(&world, 1, near_at, light_pos);
        assert_eq!(degenerate[0].towards, Vec3::ZERO);

        // Shortening an 800×600 viewport to 800×500 (expanded live strip)
        // shifts the far-zoom eye by ≈ 6.629 units and changes the lamp
        // direction accordingly.
        let far_view = View::new(&Camera {
            zoom: Zoom::Far,
            rotation: 0,
            level: 1,
            origin: [367, -130],
            walls: Walls::Cutaway,
            focus: Default::default(),
        });
        let full_dir = lamps(&world, 1, near_at, far_view.eye((800, 600)))[0].towards;
        let strip_dir = lamps(&world, 1, near_at, far_view.eye((800, 500)))[0].towards;
        assert!(
            (full_dir - strip_dir).length() > 1e-2,
            "full={full_dir:?} strip={strip_dir:?}"
        );
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

    /// `LitLamps` preserves every detail of the reference room-tile scan:
    /// duplicate entries in `room.tiles`, multiple lit lamps on the same tile
    /// in ascending ID order, and enforcing the `ROOM_LAMPS` (6) cap *before*
    /// filtering by distance (`lamp_strength`).
    #[test]
    fn indexed_lit_lamps_match_reference_scan_with_duplicates_and_cap() {
        let mut world = World::empty();
        world.size = 64;
        let full = FULL_SIDE;
        let mut map = vec![0i16; full * full];
        map[12 * full + 12] = 7;
        world.rooms.map = [map, Vec::new()];
        world.rooms.rooms.insert(
            7,
            sims_sim::rooms::Room {
                tiles: vec![
                    [12, 12, 1],
                    [12, 12, 1],
                    [50, 50, 1],
                    [50, 50, 1],
                    [13, 12, 1],
                ],
                ..Default::default()
            },
        );
        let mk_lamp = |id: u16, fx: i32, fy: i32| {
            let mut data = vec![0i16; 64];
            (data[16], data[40], data[51]) = (1, 0x400, 25);
            sims_sim::Object {
                id,
                guid: 0,
                footprint: [0; 4],
                position: [fx, fy, 1],
                unknown_short: 0,
                attributes: Vec::new(),
                temps: Vec::new(),
                data,
                obsolete_word: 0,
                stack_capacity_bytes: 0,
                stack: Vec::new(),
                relationship_flag: 0,
                relationships: Vec::new(),
                slots: Vec::new(),
                dynamic_sprites: Vec::new(),
                multipart: None,
                portal: None,
                person: None,
                registry_word: None,
                last_result: false,
                removal_skips_resets: false,
                selected: false,
            }
        };
        // Two lit lamps on the same tile with distinct fine coordinates so
        // ID ordering produces distinct direction and strength bits.
        world
            .objects
            .insert(10, mk_lamp(10, 12 * 16 + 2, 12 * 16 + 3));
        world
            .objects
            .insert(20, mk_lamp(20, 12 * 16 + 14, 12 * 16 + 13));
        // Out-of-range lamp listed twice before tile (13, 12): must consume
        // slots 5 and 6 of ROOM_LAMPS before distance filtering.
        world
            .objects
            .insert(30, mk_lamp(30, 50 * 16 + 7, 50 * 16 + 9));
        // Nearby lamp on tile (13, 12) that would be #7 and must be dropped.
        world
            .objects
            .insert(40, mk_lamp(40, 13 * 16 + 6, 12 * 16 + 11));

        let at = Vec3::new(-(12.5 * 3.0), 0.0, -(12.5 * 3.0));
        let eye = Vec3::new(100.0, 100.0, 100.0);

        let reference_lamps = |world: &World, level: i32, at: Vec3, eye: Vec3| -> Vec<Lamp> {
            let fine = [-at.z, -at.x].map(|u| (u * FINE_PER_UNIT).round() as i32);
            let [x, y] = fine.map(|f| f.div_euclid(16));
            let full = sims_sim::FULL_SIZE as i32;
            let room = world.rooms.map[(level - 1) as usize][(x * full + y) as usize];
            let room = &world.rooms.rooms[&room];
            let mut selected = Vec::new();
            'tiles: for &tile in &room.tiles {
                let tile = tile.map(i32::from);
                for o in world.objects.values() {
                    if sims_render::lot::gives_light(o)
                        && World::tile(o.position).map(i32::from) == tile
                    {
                        selected.push(o);
                        if selected.len() == ROOM_LAMPS {
                            break 'tiles;
                        }
                    }
                }
            }
            selected
                .into_iter()
                .filter_map(|lamp| {
                    let [fx, fy, _] = lamp.position;
                    let ground = sims_sim::head::ground_height(
                        world,
                        [F32::from_f32(fx as f32), F32::from_f32(fy as f32)],
                    )
                    .map_or(0.0, F32::to_f32);
                    let light = Vec3::new(
                        -(fy as f32) / FINE_PER_UNIT,
                        ground + LAMP_HEIGHT,
                        -(fx as f32) / FINE_PER_UNIT,
                    );
                    let from = at - light;
                    Some(Lamp {
                        towards: (light - eye).normalize(),
                        strength: lamp_strength(from.dot(from))?,
                    })
                })
                .collect()
        };

        let bits = |ls: &[Lamp]| -> Vec<([u32; 3], u32)> {
            ls.iter()
                .map(|l| {
                    (
                        [l.towards.x, l.towards.y, l.towards.z].map(f32::to_bits),
                        l.strength.to_bits(),
                    )
                })
                .collect()
        };

        let got = lamps(&world, 1, at, eye);
        let want = reference_lamps(&world, 1, at, eye);
        assert_eq!(got.len(), 4);
        assert_ne!(
            bits(&got[0..1]),
            bits(&got[1..2]),
            "same-tile lamps must have distinct light vectors to pin ordering"
        );
        assert_eq!(bits(&got), bits(&want));
    }
}
