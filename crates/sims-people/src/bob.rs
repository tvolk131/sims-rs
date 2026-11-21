//! The plumb bob: the diamond over the selected person's head, coloured
//! by their mood.
//!
//! **What it is.** The plumb bob is an ordinary object (`PlumbBob.iff`,
//! "PlumbBob - Standard", GUID 0x2F726FFF) that the game puts in the
//! selected person's head slot: container slot 2, the slot the person's
//! tick redraws (0x58dae5; decisions entry "the head slot's redraw").
//! It has no sprites. Its object string 200 names a skeleton, `root`, and
//! a suit, `head-arrow,ROOT=white`; the suit binds the mesh
//! `xskin-head-arrow-ROOT-ARROW` (in `Animation.far`) to the bone `ROOT`.
//! No `root-skeleton.cmx.bcf` exists: the graphic poses that one bone
//! itself, so [`Bob`] makes a skeleton of it. No `white.bmp` exists
//! either, so the mesh is drawn white and the light gives it its colour.
//!
//! **Where it is.** Each tick the head slot's redraw gives the graphic
//! the position of the person's `HEAD` bone (bone +0x4c to +0x54, found
//! by name, 0x5990e0) times the person's placement (animator +0xb0),
//! with no rotation of the bone's own (0x58dbd8 to 0x58dc03). Drawing it
//! (0x585dc5) sets the root bone at (0, 3, 0) in that frame, turned by
//! the original's Euler-to-quaternion routine (0x5c4bb0) with a quarter
//! turn about x and the spin about z: so the mesh, 2.1 units long on its
//! z axis, stands upright from 3 units above the head and spins about
//! its own length (0x5861ce to 0x586246). The spin is the game's clock in
//! milliseconds (0x6e54e0) times 0.001 × π: half a turn a second.
//!
//! **Its colour.** The suit's name picks a mode (0x5856db to 0x58579f):
//! `head-arrow` 1, `head-arrow-date` 2, `head-star` 3. In mode 1, which
//! [`colour`] ports, the mood (the person's motive 3, cut to an integer,
//! 0x58dc29 to 0x58dc46) times 0.012, held to −1..1, gives a colour from
//! red through pale yellow to green (0x58605d to 0x586165). That colour
//! is the colour of a light of intensity 1 (0x58616a), and a quarter of
//! it plus 0.05 is the ambient light (0x586184 to 0x5861c9); the mesh is
//! white, so a vertex takes `ambient + colour × max(0, n · l)`.
//!
//! **Its light's direction** ([`light`]) is the house camera's unit
//! vector from its eye to the centre (the view object's +0x184, set by
//! 0x49b0de), turned by 80° about y and then 30° about x (the matrix at
//! 0x6f6b38, built at 0x585d46), given to the light as its direction
//! (0x585fe6, 0x5d0b7c). Which way the light shines along it is
//! **taken from a screenshot**, not read: the original's bob at medium
//! zoom (7:38 AM, Mortimer on the porch bench) is brightest on the
//! viewer's left and lit on its lower half, as it is with `l` that
//! vector, and dark on the left with `l` its opposite. With it, ours
//! spans the same box on screen (horizontal edges within a pixel and
//! vertical edges within two pixels, as the needle-sharp tips depend on
//! pixel-column alignment; the pixels inside it are not compared one by
//! one). The turn is in
//! the lot's terms, not the view's, so the light falls from another side
//! at each rotation of the view; only rotation 0 was compared.
//!
//! **Not ported.** Modes 2 and 3 (the date and star bobs), whose GUIDs
//! are not drawn. Which bone the head slot names (the slot's +0x20) is
//! not read; `HEAD` is assumed, and puts the bob where the screenshot
//! has it.

use crate::appearance::{Part, load_appearance, load_mesh};
use crate::assets::Assets;
use crate::math::{Transform, Vec3};
use crate::rig;
use crate::texture::Texture;
use sims_formats::avatar::{Bone, Skeleton};

/// The standard plumb bob's GUID.
pub const STANDARD: u32 = 0x2F72_6FFF;

/// The suit the standard plumb bob wears (its object string 200).
pub const SUIT: &str = "head-arrow";

/// The person's container slot that holds the plumb bob: the head slot
/// (the container entry at begin + 0x48, 0x58dae5).
pub const HEAD_SLOT: usize = 2;

/// The bone whose position the head slot follows.
pub const HEAD_BONE: &str = "HEAD";

/// How far above the head bone the root bone sits, in units (0x586231).
pub const RAISE: f32 = 3.0;

/// The plumb bob's mesh on its one-bone skeleton.
#[derive(Debug, Clone)]
pub struct Bob {
    pub skeleton: Skeleton,
    pub parts: Vec<Part>,
}

impl Bob {
    /// Loads the standard plumb bob's suit, [`SUIT`], on a skeleton of
    /// the one bone `ROOT`, drawn white.
    pub fn load(assets: &Assets) -> Result<Self, String> {
        let skeleton = Skeleton {
            name: "root".to_owned(),
            bones: vec![Bone {
                name: "ROOT".to_owned(),
                parent: None,
                translation: [0.0; 3],
                rotation: [0.0, 0.0, 0.0, 1.0],
                properties: Vec::new(),
                flags: [0; 3],
                wiggle: Default::default(),
            }],
        };
        let appearance = load_appearance(assets, SUIT)?;
        let mut parts = Vec::new();
        for binding in &appearance.bindings {
            let mesh = load_mesh(assets, &binding.mesh)?;
            let bones = mesh
                .bones
                .iter()
                .map(|b| {
                    rig::bone(&skeleton, b)
                        .ok_or_else(|| format!("the plumb bob's mesh names the bone {b:?}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            parts.push(Part {
                appearance: SUIT.to_owned(),
                bone: 0,
                bones,
                mesh,
                texture_name: None,
                texture: Texture::solid([255, 255, 255, 255]),
                censor: binding.censor,
            });
        }
        Ok(Self { skeleton, parts })
    }
}

/// The plumb bob `person` holds in their head slot: its object ID, if
/// the slot holds an object with the standard plumb bob's GUID.
pub fn held_by(world: &sims_sim::World, person: u16) -> Option<u16> {
    let slot = world.objects.get(&person)?.slots.get(HEAD_SLOT)?;
    let id = u16::try_from(slot.object).ok().filter(|&id| id != 0)?;
    (world.objects.get(&id)?.guid == STANDARD).then_some(id)
}

/// The colour of the plumb bob's light for a mood (motive 3 cut to an
/// integer), in mode 1 (0x58605d to 0x586165): red at −1, pale yellow
/// (1, 1, 0.6) at 0, green at 1, with `m` the mood × 0.012 held to −1..1.
pub fn colour(mood: i32) -> [f32; 3] {
    let m = (mood as f32 * 0.012).clamp(-1.0, 1.0);
    if m >= 0.0 {
        let r = 1.0 - m;
        let b = if m < 0.4 { r - (0.4 - m) } else { r };
        [r, 1.0, b]
    } else {
        let g = 1.0 - m * m;
        let b = if m > -0.4 { g - (m + 0.4) } else { g };
        [1.0, g, b]
    }
}

/// How far the plumb bob has spun at `clock` milliseconds, in radians:
/// the clock × 0.001 × π (0x5861ce), taken over whole turns (one every
/// 2000 ms) so a large clock keeps its precision.
pub fn spin(clock: u32) -> f32 {
    (clock % 2000) as f32 * 0.001 * std::f32::consts::PI
}

/// The original's Euler-to-quaternion routine (0x5c4bb0) for angles `x`
/// and `z` about those axes (its middle angle 0), in its own convention
/// `[x, y, z, w]`: the turn about x, then the turn about z.
fn euler_xz(x: f32, z: f32) -> [f32; 4] {
    let (sx, cx) = (x * 0.5).sin_cos();
    let (sz, cz) = (z * 0.5).sin_cos();
    [cz * sx, sz * sx, sz * cx, cz * cx]
}

/// Where the plumb bob's root bone is: `head` is the head bone's place
/// (only its position counts) and `placement` the person's, and the bob
/// has spun `spin` radians.
pub fn root(head: Transform, placement: Transform, spin: f32) -> Transform {
    let graphic = Transform {
        rotation: placement.rotation,
        translation: head.translation,
    };
    let local = rig::from_file(
        [0.0, RAISE, 0.0],
        euler_xz(std::f32::consts::FRAC_PI_2, spin),
    );
    graphic * local
}

/// A turn of `degrees` about the x axis (`axis` 0) or the y axis (1),
/// as the original's 0x5c3a20 makes it, for row vectors.
fn turn(axis: usize, degrees: f32) -> [[f32; 3]; 3] {
    let (s, c) = degrees.to_radians().sin_cos();
    if axis == 0 {
        [[1.0, 0.0, 0.0], [0.0, c, s], [0.0, -s, c]]
    } else {
        [[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]]
    }
}

/// `a` then `b`, for row vectors (the original's 0x49b87c).
fn then(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// The row vector `v` times `m` (the original's 0x5c39a0).
fn times(v: Vec3, m: [[f32; 3]; 3]) -> Vec3 {
    let v = [v.x, v.y, v.z];
    let c = |j: usize| (0..3).map(|k| v[k] * m[k][j]).sum();
    Vec3::new(c(0), c(1), c(2))
}

/// The plumb bob's light vector in the world at the view's `rotation`
/// (0 to 3). The house camera (0x49b0de) turns by 45° − 90° × rotation
/// about y and tilts 30° down about x, puts its eye along its z axis
/// and keeps the unit vector from the eye to the centre (+0x184); the
/// bob's light is that vector turned by 80° about y and then 30° about x
/// (0x6f6b38, built at 0x585d46; 0x585fe6).
pub fn light(rotation: u8) -> Vec3 {
    let yaw = 45.0 - 90.0 * f32::from(rotation & 3);
    let camera = then(turn(0, -30.0), turn(1, yaw));
    let eye = times(Vec3::new(0.0, 0.0, 1.0), camera);
    let turned = then(turn(1, 80.0), turn(0, 30.0));
    times(Vec3::ZERO - eye, turned).normalize()
}

/// The light a vertex with unit normal `normal` takes from the plumb
/// bob's light of `colour`, with `light` its unit direction
/// ([`light`]): the ambient light (a quarter of the colour plus 0.05)
/// and the colour by the angle.
pub fn shade(colour: [f32; 3], normal: Vec3, light: Vec3) -> [f32; 3] {
    let d = normal.dot(light).max(0.0);
    colour.map(|c| c * 0.25 + 0.05 + c * d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Quat;

    #[test]
    fn the_mood_colours_the_bob_from_red_through_pale_yellow_to_green() {
        assert_eq!(colour(100), [0.0, 1.0, 0.0]);
        assert_eq!(colour(-100), [1.0, 0.0, 0.0]);
        let [r, g, b] = colour(0);
        assert_eq!([r, g], [1.0, 1.0]);
        assert!((b - 0.6).abs() < 1e-6);
        // Past 83 the colour is held.
        assert_eq!(colour(90), colour(200));
        // Halfway up: r and b are 1 − m.
        let [r, g, b] = colour(50);
        assert!((r - 0.4).abs() < 1e-6 && g == 1.0 && (b - 0.4).abs() < 1e-6);
        // A little below zero, blue drops faster than green.
        let [r, g, b] = colour(-20);
        assert_eq!(r, 1.0);
        assert!(g > b);
    }

    #[test]
    fn the_euler_routine_turns_about_x_then_z() {
        // As the original's routine gives for (π/2, 0, 0.3), read by
        // running it.
        let q = euler_xz(std::f32::consts::FRAC_PI_2, 0.3);
        for (a, b) in q.iter().zip([0.6992, 0.1057, 0.1057, 0.6992]) {
            assert!((a - b).abs() < 1e-4, "{q:?}");
        }
    }

    #[test]
    fn the_bob_stands_upright_three_units_over_the_head_and_spins_about_its_length() {
        let head = Transform {
            rotation: Quat::IDENTITY,
            translation: Vec3::new(1.0, 5.0, 2.0),
        };
        let placement = Transform {
            rotation: Quat::IDENTITY,
            translation: Vec3::new(1.0, 0.0, 2.0),
        };
        for spin in [0.0, 0.7, 2.0] {
            let at = root(head, placement, spin);
            let base = at.point(Vec3::ZERO);
            assert!((base - Vec3::new(1.0, 8.0, 2.0)).length() < 1e-5);
            // The mesh's length (its z) points up, whatever the spin.
            let tip = at.point(Vec3::new(0.0, 0.0, 2.1));
            assert!((tip - Vec3::new(1.0, 10.1, 2.0)).length() < 1e-4, "{tip:?}");
        }
        // The spin turns the mesh's sides about the vertical.
        let side = |spin| root(head, placement, spin).vector(Vec3::new(1.0, 0.0, 0.0));
        assert!((side(0.0) - side(1.0)).length() > 0.5);
        assert!(side(1.0).y.abs() < 1e-5);
    }

    /// A world direction as the view sees it: its fine x and y in the
    /// view and its height.
    fn seen(rotation: u8, d: Vec3) -> [f32; 3] {
        let camera = sims_render::Camera {
            zoom: sims_render::Zoom::Near,
            rotation,
            level: 1,
            origin: [0, 0],
            walls: sims_render::Walls::Up,
            focus: Default::default(),
        };
        let view = crate::View::new(&camera);
        let (a, ha) = view.place(Vec3::ZERO);
        let (b, hb) = view.place(d);
        [b[0] - a[0], b[1] - a[1], hb - ha]
    }

    #[test]
    fn the_camera_keeps_its_place_in_the_view_and_the_light_turns_with_the_lot() {
        // The eye is up and towards the low corner of the view.
        let eye = |r: u8| {
            let yaw = 45.0 - 90.0 * f32::from(r);
            times(Vec3::new(0.0, 0.0, 1.0), then(turn(0, -30.0), turn(1, yaw)))
        };
        let [x, y, h] = seen(0, eye(0));
        assert!(x < 0.0 && (x - y).abs() < 1e-3 && h > 0.0, "{x} {y} {h}");
        for r in 1..4 {
            let (a, b) = (seen(0, eye(0)), seen(r, eye(r)));
            for (a, b) in a.iter().zip(b) {
                assert!((a - b).abs() < 1e-3, "rotation {r}: {a} {b}");
            }
        }
        // The light is the camera's vector turned in the lot's terms, so
        // the view sees it from another side at each rotation. At
        // rotation 0 it is on the viewer's left (fine x − y falls to the
        // left) and below.
        let [x, y, h] = seen(0, light(0));
        assert!(x - y < 0.0 && h < 0.0, "{x} {y} {h}");
        let [x1, y1, _] = seen(1, light(1));
        assert!((x1 - x).abs() + (y1 - y).abs() > 1.0);
    }

    #[test]
    fn the_spin_is_half_a_turn_a_second() {
        assert_eq!(spin(0), 0.0);
        assert!((spin(1000) - std::f32::consts::PI).abs() < 1e-5);
        assert_eq!(spin(2500), spin(500));
    }
}
