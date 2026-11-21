//! The Sim's 3D head drawn in the middle of the pie menu.
//!
//! Ported from `Sims.exe`'s pie-head controller (`0x462078`–`0x462a50`,
//! called through `0x587cf1` at `0x4a953e` and `0x4ab77d` inside the house
//! viewport's Direct3D 6 scene):
//!
//! - **Parts & pose** (`0x462437`–`0x462575`): draws every non-censored part
//!   (`censor == 0`) bound to the `"HEAD"` bone (`0x6c4528`), skinned in bind
//!   pose with identity bone transforms (`0x599bf2(1)` → `0x59a020`, using
//!   zero translation `0x6f7620` and identity rotation `0x6722b8`), not the
//!   Sim's live lot animation pose.
//! - **Position & scale** (`0x4625ae`–`0x462685`): in steady state (after
//!   the 400 ms pop-in), the head's bone origin sits `Y_OFFSET` (14 px,
//!   `0x4625ae`) below the main pie menu's center on screen, unprojected into
//!   the house camera's world space at mid-depth and scaled by
//!   `(12.0 / (1 << zoom)) * 1.75` (`0x4625d9`, `[0x6c470c] = 1.75`). Because
//!   the house camera (`0x49b0de`) has `3√2 / (16 << zoom)` world units per
//!   pixel, the `(1 << zoom)` factor cancels identically across all three
//!   zooms, yielding a constant orthographic screen scale of
//!   `56√2 ≈ 79.196` pixels per HEAD-bone unit.
//! - **Orientation & pointer tracking** (`0x46234d`–`0x462428`,
//!   `0x462688`–`0x462778`, `0x462822`–`0x46292d`): the head tilts continuously
//!   towards the pointer by `sqrt(distance * 0.0005)` (`0x4623cb`) along
//!   `angle = atan2(dx, -dy)` (`0x5eb859`, zeroed inside the 35 px dead zone
//!   when no label box is under the pointer at `0x5eb8a7`), adds the constant
//!   `0.3 / 60 = 0.005` rad offset (`0x462688`, `[0x6c4710] = 0.3`), and adds
//!   an 8-way compass offset (`±0.6` cardinals, `±0.4` diagonals, jump table
//!   `0x462a53`) for the highlighted option. The model transform applies
//!   `RotY(0.05π + pitch) * RotZ(π/2) * RotY((rot - 0.5)*(-π/2) + yaw)`.
//!   Composed with the house camera's view rotation
//!   `RotY(-(45° - 90°·rot) + π) * RotX(-π/6)` (`0x49b0de` → `0x5d0595` →
//!   `0x5d0672`), the lot rotation `rot` cancels identically, leaving
//!   `RotY(0.05π + pitch) * RotZ(π/2) * RotY(yaw + π) * RotX(-π/6)` at all
//!   four lot rotations.
//! - **Lighting & 5-6-5 textures** (`0x462576`–`0x4625a4`, `0x4a953e`): all
//!   lot people remove their directional and room lights in `0x4b4cf5` before
//!   the pie head draws, so the pie head renders with ambient-only lighting:
//!   `1.0` when an option is highlighted, and `0.8` (`[0x667454]`) otherwise.
//!   Textured meshes use the same 16-bit 5-6-5 texel expansion (`Packed565`)
//!   and Direct3D 6 integer pixel-centre offset `(+0.5, +0.5)` as lot people.

use crate::appearance::Body;
use crate::lot::Packed565;
use crate::math::{Transform, Vec3};
use crate::raster::{Canvas, Shade, Target, draw_shaded};
use crate::rig;
use crate::skin::skin;
use std::f32::consts::{FRAC_PI_2, FRAC_PI_6, PI, SQRT_2};

/// The skeleton bone whose attached parts are drawn in the pie menu
/// (`0x6c4528` = `"HEAD"`).
pub const HEAD_BONE: &str = "HEAD";

/// Screen pixels per HEAD-bone unit, invariant across all three lot zooms:
/// `(12.0 / (1 << zoom)) * 1.75 / (3√2 / (16 << zoom)) = 56√2 ≈ 79.196`
/// (`0x4625d9`, `0x46260d` `[0x6c470c] = 1.75`, `0x49b0de`).
pub const SCALE_PIXELS: f32 = 56.0 * SQRT_2;

/// Steady-state vertical offset of the HEAD bone origin below the main pie
/// menu's center, in pixels (`0x4625ae`: `0xe` = 14).
pub const Y_OFFSET: f32 = 14.0;

/// Base upward pitch added before `RotZ(π/2)` (`0x462845`: `0.05 * π`).
pub const BASE_PITCH: f32 = 0.05 * PI;

/// Small constant angle added to both pitch and yaw (`0x462688`:
/// `[0x6c4710] = 0.3` times `[0x667448] = 1/60`).
pub const IDLE_BIAS: f32 = 0.3 / 60.0;

/// Multiplier inside `sqrt(distance * CURSOR_DIST_SCALE)` for continuous
/// pointer tracking (`0x4623cb`: `[0x667460] = 0.0005`).
pub const CURSOR_DIST_SCALE: f32 = 0.0005;

/// Cardinal and diagonal look-at angle offsets for the 8 compass directions
/// (`0x4626e6`–`0x462778`, `[0x666a40] = 0.6`, `[0x666a44] = 0.4`).
pub const CARDINAL_TURN: f32 = 0.6;
pub const DIAGONAL_TURN: f32 = 0.4;

/// Ambient light level when an option is highlighted (`1.0`) vs idle (`0.8`,
/// `[0x667454]`, `0x462576`–`0x462586`).
pub const AMBIENT_HIGHLIGHTED: f32 = 1.0;
pub const AMBIENT_IDLE: f32 = 0.8;

/// Dead-zone radius around the pie center inside which `cTSPieMenu` clears
/// its pointer angle when no label box is under the pointer (`0x51882d`,
/// `0x5eb8a7`).
pub const DEAD_ZONE: f32 = 35.0;

/// Where the pie-menu head is looking and how brightly it is lit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// Pointer distance from the main menu's center in pixels
    /// (`cTSPieMenu::+0x164`, `0x5eb82d`).
    pub distance: f32,
    /// Pointer angle clockwise from straight up in radians (`atan2(dx, -dy)`,
    /// `cTSPieMenu::+0x168`, `0x5eb85e`), or `0.0` inside the dead zone with
    /// no label box hit.
    pub angle: f32,
    /// Compass direction (`0..8`, clockwise from straight up) of the
    /// highlighted menu entry (`0x462372`–`0x4623b4`), or `None` when nothing
    /// is highlighted.
    pub direction: Option<u8>,
}

impl Look {
    /// Neutral look: pointer at the center and nothing highlighted.
    pub const fn neutral() -> Self {
        Self {
            distance: 0.0,
            angle: 0.0,
            direction: None,
        }
    }

    /// Computes the pie head's [`Look`] from the main menu's `center`,
    /// `pointer` position, `highlighted` entry index (if any), and total
    /// `item_count` (`0x5eb7f0`–`0x5eb8c9`, `0x46234d`–`0x4623b4`).
    pub fn from_menu(
        center: [i32; 2],
        pointer: Option<[i32; 2]>,
        highlighted: Option<usize>,
        item_count: usize,
    ) -> Self {
        let direction = highlighted.and_then(|index| compass_direction(index, item_count));
        let Some(point) = pointer else {
            return Self {
                distance: 0.0,
                angle: 0.0,
                direction,
            };
        };
        let dx = f64::from(point[0]) - f64::from(center[0]);
        let dy = f64::from(point[1]) - f64::from(center[1]);
        let distance = (dx * dx + dy * dy).sqrt() as f32;
        // 0x5eb833: distance < 1 returns before updating angle (leaves 0).
        // 0x5eb8a7..0x5eb8c7: inside the dead zone, angle is cleared to 0
        // unless the pointer hit a label's own box (highlighted.is_some()).
        let angle = if distance < 1.0 || (highlighted.is_none() && distance <= DEAD_ZONE) {
            0.0
        } else {
            dx.atan2(-dy) as f32
        };
        Self {
            distance,
            angle,
            direction,
        }
    }

    /// The steady-state `[pitch, yaw]` angles in radians (`0x4623c5`–`0x4623f3`,
    /// `0x462688`–`0x462778`).
    pub fn angles(&self) -> [f32; 2] {
        let dist = if self.distance.is_finite() {
            self.distance.max(0.0)
        } else {
            0.0
        };
        let angle = if self.angle.is_finite() {
            self.angle
        } else {
            0.0
        };
        let r = (dist * CURSOR_DIST_SCALE).sqrt();
        let (sin_a, cos_a) = angle.sin_cos();
        let mut pitch = cos_a * r + IDLE_BIAS;
        let mut yaw = -sin_a * r + IDLE_BIAS;
        if let Some(dir) = self.direction {
            match dir {
                0 => pitch += CARDINAL_TURN,
                1 => {
                    pitch += DIAGONAL_TURN;
                    yaw -= DIAGONAL_TURN;
                }
                2 => yaw -= CARDINAL_TURN,
                3 => {
                    pitch -= DIAGONAL_TURN;
                    yaw -= DIAGONAL_TURN;
                }
                4 => pitch -= CARDINAL_TURN,
                5 => {
                    pitch -= DIAGONAL_TURN;
                    yaw += DIAGONAL_TURN;
                }
                6 => yaw += CARDINAL_TURN,
                7 => {
                    pitch += DIAGONAL_TURN;
                    yaw += DIAGONAL_TURN;
                }
                _ => {}
            }
        }
        [pitch, yaw]
    }

    /// The Direct3D 6 ambient light factor (`0x462576`–`0x462586`).
    pub const fn ambient(&self) -> f32 {
        if self.direction.is_some() {
            AMBIENT_HIGHLIGHTED
        } else {
            AMBIENT_IDLE
        }
    }
}

/// Maps a highlighted menu item `index` out of `item_count` entries to one of
/// the 8 compass directions (`0..8`, clockwise from straight up), matching
/// `0x462372`–`0x4623b4`. Returns `None` if an out-of-range `index` maps
/// past 7 (`0x4626e6`).
pub fn compass_direction(index: usize, item_count: usize) -> Option<u8> {
    let dir = match item_count {
        0 | 1 => 0,
        2 => index.checked_shl(2)?,
        3 | 4 => index.checked_mul(2)?,
        5..=7 => index,
        _ => {
            if index <= 7 {
                index
            } else {
                (index & 1) << 2
            }
        }
    };
    (dir <= 7).then_some(dir as u8)
}

/// Right-handed rotation about the X axis (`0x5c3a20` axis 0).
fn rot_x(v: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    Vec3::new(v.x, v.y * c - v.z * s, v.y * s + v.z * c)
}

/// Right-handed rotation about the Y axis (`0x5c3a20` axis 1).
fn rot_y(v: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    Vec3::new(v.x * c + v.z * s, v.y, -v.x * s + v.z * c)
}

/// Right-handed rotation about the Z axis (`0x5c3a20` axis 2).
fn rot_z(v: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    Vec3::new(v.x * c - v.y * s, v.x * s + v.y * c, v.z)
}

/// Projects a HEAD-bone-local vertex `point` into screen pixels and depth,
/// with `origin` the screen position of the HEAD bone origin (shifted by
/// `(+0.5, +0.5)` for Direct3D 6 integer pixel-centre sampling).
///
/// Matches the exact composition of `0x462845`–`0x46292d` and the house
/// camera `0x49b0de` → `0x5d0595` → `0x5d0672`:
/// `RotY(0.05π + pitch) * RotZ(π/2) * RotY(yaw + π) * RotX(-π/6)`.
pub fn project(point: Vec3, origin: [f32; 2], pitch: f32, yaw: f32) -> Vec3 {
    let v = rot_y(point, BASE_PITCH + pitch);
    let v = rot_z(v, FRAC_PI_2);
    let v = rot_y(v, yaw + PI);
    let v = rot_x(v, -FRAC_PI_6);
    Vec3::new(
        origin[0] + v.x * SCALE_PIXELS,
        origin[1] - v.y * SCALE_PIXELS,
        v.z,
    )
}

/// Target wrapper matching Direct3D 6's half-up 8-bit modulation rounding
/// (same rule as `sims_render::Canvas`'s `Target` impl in [`crate::lot`])
/// when writing opaque pie-head pixels into a [`Canvas`].
struct PieTarget<'a>(&'a mut Canvas);

impl Target for PieTarget<'_> {
    fn size(&self) -> (u32, u32) {
        self.0.size()
    }

    fn passes(&self, x: u32, y: u32, z: f32) -> bool {
        self.0.passes(x, y, z)
    }

    fn write(&mut self, x: u32, y: u32, z: f32, texel: [u8; 4], s: f32) {
        let lit = |c: u8| (f32::from(c) * s + 0.5).clamp(0.0, 255.0) as u8;
        let [r, g, b, a] = texel;
        self.0.write(x, y, z, [lit(r), lit(g), lit(b), a], 1.0);
    }
}

/// Draws `body`'s pie-menu 3D head into an RGBA [`Canvas`] of `size` pixels
/// (typically `[201, 201]`), clear (`[0, 0, 0, 0]`) outside the head.
///
/// Returns an error if `body`'s skeleton has no `HEAD` bone or no visible
/// parts are bound to it.
pub fn draw_body(body: &Body, size: [u32; 2], look: Look) -> Result<Canvas, String> {
    let head_bone = rig::bone(&body.skeleton, HEAD_BONE)
        .ok_or_else(|| "skeleton has no HEAD bone".to_owned())?;
    let identity = vec![Transform::IDENTITY; body.skeleton.bones.len()];
    let parts: Vec<_> = body
        .parts
        .iter()
        .filter(|p| p.censor == 0 && p.bone == head_bone)
        .map(|p| (p, skin(p, &identity)))
        .collect();
    let count: usize = parts.iter().map(|(_, s)| s.positions.len()).sum();
    if count == 0 {
        return Err("body has no head to draw".to_owned());
    }
    let [width, height] = size;
    let mut canvas = Canvas::new(width, height, [0; 4]);
    let [pitch, yaw] = look.angles();
    // 0x4625ae + 0x462621: bone origin sits 14 px below the face center
    // (width/2, height/2), with (+0.5, +0.5) for D3D6 integer pixel centres.
    let origin = [
        (width / 2) as f32 + 0.5,
        (height / 2) as f32 + Y_OFFSET + 0.5,
    ];
    let ambient = Shade::Grey(look.ambient());
    let mut target = PieTarget(&mut canvas);
    for (part, skinned) in &parts {
        let shade = vec![ambient; skinned.positions.len()];
        let proj = |p| project(p, origin, pitch, yaw);
        if part.texture_name.is_some() {
            let mut packed = Packed565(&mut target);
            draw_shaded(&mut packed, part, skinned, &shade, proj);
        } else {
            draw_shaded(&mut target, part, skinned, &shade, proj);
        }
    }
    Ok(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compass_directions_match_native_mapping() {
        assert_eq!(compass_direction(0, 1), Some(0));
        assert_eq!(compass_direction(0, 2), Some(0));
        assert_eq!(compass_direction(1, 2), Some(4));
        assert_eq!(compass_direction(2, 2), None);
        assert_eq!(compass_direction(0, 3), Some(0));
        assert_eq!(compass_direction(1, 3), Some(2));
        assert_eq!(compass_direction(2, 3), Some(4));
        assert_eq!(compass_direction(3, 4), Some(6));
        assert_eq!(compass_direction(4, 4), None);
        for i in 0..7 {
            assert_eq!(compass_direction(i, 7), Some(i as u8));
        }
        assert_eq!(compass_direction(7, 10), Some(7));
        assert_eq!(compass_direction(8, 10), Some(0));
        assert_eq!(compass_direction(9, 10), Some(4));
    }

    #[test]
    fn all_eight_compass_directions_apply_expected_turn_offsets() {
        let expected = [
            (CARDINAL_TURN, 0.0),
            (DIAGONAL_TURN, -DIAGONAL_TURN),
            (0.0, -CARDINAL_TURN),
            (-DIAGONAL_TURN, -DIAGONAL_TURN),
            (-CARDINAL_TURN, 0.0),
            (-DIAGONAL_TURN, DIAGONAL_TURN),
            (0.0, CARDINAL_TURN),
            (DIAGONAL_TURN, DIAGONAL_TURN),
        ];
        for (dir, (dp, dy)) in expected.into_iter().enumerate() {
            let look = Look {
                distance: 0.0,
                angle: 0.0,
                direction: Some(dir as u8),
            };
            let [pitch, yaw] = look.angles();
            assert!((pitch - (IDLE_BIAS + dp)).abs() < 1e-6, "dir {dir}");
            assert!((yaw - (IDLE_BIAS + dy)).abs() < 1e-6, "dir {dir}");
        }
        // Non-finite inputs fall back cleanly to finite idle bias.
        let weird = Look {
            distance: f32::NAN,
            angle: f32::INFINITY,
            direction: None,
        };
        assert_eq!(weird.angles(), [IDLE_BIAS, IDLE_BIAS]);
    }

    #[test]
    fn look_dead_zone_and_ambient_match_native() {
        // Inside dead zone with no highlight: angle zeroed, ambient 0.8.
        let idle = Look::from_menu([200, 200], Some([210, 220]), None, 4);
        assert_eq!(idle.angle, 0.0);
        assert_eq!(idle.direction, None);
        assert_eq!(idle.ambient(), AMBIENT_IDLE);

        // Highlighted top option: pitch increases sharply, ambient 1.0.
        let up = Look::from_menu([200, 198], Some([200, 123]), Some(0), 4);
        assert_eq!(up.direction, Some(0));
        assert_eq!(up.ambient(), AMBIENT_HIGHLIGHTED);
        let [up_pitch, up_yaw] = up.angles();
        assert!(
            up_pitch > 0.75,
            "expected sharp upward pitch, got {up_pitch}"
        );
        assert!((up_yaw - IDLE_BIAS).abs() < 1e-5);

        // Highlighted bottom option: pitch decreases sharply.
        let down = Look::from_menu([400, 274], Some([400, 348]), Some(4), 6);
        assert_eq!(down.direction, Some(4));
        let [down_pitch, _] = down.angles();
        assert!(
            down_pitch < -0.75,
            "expected sharp downward pitch, got {down_pitch}"
        );
    }

    #[test]
    fn projection_orientates_nose_toward_viewer_and_tracks_pitch_yaw() {
        let origin = [100.5, 114.5];
        // In HEAD-bone local space: +X is crown/up, +Z is nose/forward.
        let crown = project(Vec3::new(1.0, 0.0, 0.0), origin, 0.0, 0.0);
        let nose = project(Vec3::new(0.0, 0.0, 1.0), origin, 0.0, 0.0);
        assert!(crown.y < origin[1], "crown should project above origin");
        assert!(nose.z < 0.0, "nose should project nearer than origin");

        // Positive pitch raises the nose on screen; negative pitch lowers it.
        let nose_up = project(Vec3::new(0.0, 0.0, 1.0), origin, 0.6, 0.0);
        let nose_down = project(Vec3::new(0.0, 0.0, 1.0), origin, -0.6, 0.0);
        assert!(nose_up.y < nose.y && nose_down.y > nose.y);

        // Positive yaw (left option) turns nose left (-x); negative yaw right (+x).
        let nose_left = project(Vec3::new(0.0, 0.0, 1.0), origin, 0.0, 0.6);
        let nose_right = project(Vec3::new(0.0, 0.0, 1.0), origin, 0.0, -0.6);
        assert!(nose_left.x < nose.x && nose_right.x > nose.x);
    }
}
