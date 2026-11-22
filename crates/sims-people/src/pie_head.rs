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
use crate::raster::{Canvas, Shade, Target, draw_shaded_culled};
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

/// Duration of the pie-menu backdrop pop-in, submenu center/radius glide, and
/// head scale/`Y_OFFSET` ramp, in milliseconds (`[0x6c4700] = 400`,
/// `0x4621e3`–`0x462273`, `0x4625e1`–`0x46260d`).
pub const POP_IN_MS: u32 = 400;

/// Duration of the pie head's direction-change ease-out curve, in milliseconds
/// (`[0x6c4704] = 400`, `0x46277a`–`0x4627d2`).
pub const DIRECTION_EASE_MS: u32 = 400;

/// Steady-state radius of the dark radial pie-menu backdrop in pixels
/// (`[0x6c4708] = 100`, `0x4621db`).
pub const BACKDROP_RADIUS: i32 = 100;

/// Ease-out curve coefficients (`0x462799`–`0x4627b0`):
/// `u(t) = sqrt(DIRECTION_EASE_BASE + DIRECTION_EASE_SPAN * t)` where
/// `[0x666a38] = 0.1` and `[0x667444] = 0.9`.
pub const DIRECTION_EASE_BASE: f32 = 0.1;
pub const DIRECTION_EASE_SPAN: f32 = 0.9;

/// Largest face side accepted by [`draw_body`], matching `sims_ui::pie`'s
/// face-bitmap cap and preventing `u32` area overflow in [`Canvas::new`].
pub const MAX_SIDE: u32 = 4096;

/// Animated circular backdrop state evaluated by [`PieAnimState::backdrop`]
/// (`0x46216e`–`0x46228d`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackdropState {
    pub center: [i32; 2],
    pub radius: i32,
}

/// Animated pie-head pose evaluated by [`PieAnimState::head_pose`]
/// (`0x4625ae`–`0x462822`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeadPose {
    pub pitch: f32,
    pub yaw: f32,
    /// Head scale multiplier (`0.0..=1.0`, ramping linearly over [`POP_IN_MS`]).
    pub scale: f32,
    /// Vertical offset below the main pie center (`0.0..=Y_OFFSET`, ramping
    /// linearly over [`POP_IN_MS`]).
    pub y_offset: f32,
    /// Ambient light factor ([`AMBIENT_HIGHLIGHTED`] or [`AMBIENT_IDLE`]).
    pub ambient: f32,
}

impl HeadPose {
    /// Steady-state pose matching [`draw_body`] and [`draw_body_in_viewport`]
    /// bit-for-bit once all 400 ms transitions finish.
    pub fn steady(look: Look) -> Self {
        let [pitch, yaw] = look.angles();
        Self {
            pitch,
            yaw,
            scale: 1.0,
            y_offset: Y_OFFSET,
            ambient: look.ambient(),
        }
    }
}

/// Pure `Sims.exe` 400 ms pie-menu transition curve helpers
/// (`0x4621e3`–`0x462273`, `0x4625e1`–`0x46260d`, `0x462799`–`0x4627d2`).
pub struct PieTransition;

impl PieTransition {
    /// Linear progress `t ∈ [0.0, 1.0]` over [`POP_IN_MS`] (400 ms).
    pub fn pop_in_progress(elapsed_ms: u32) -> f32 {
        if elapsed_ms >= POP_IN_MS {
            1.0
        } else {
            elapsed_ms as f32 / POP_IN_MS as f32
        }
    }

    /// Head scale multiplier during the 400 ms pop-in (`0x4625e1`–`0x46260d`).
    pub fn head_scale(elapsed_ms: u32) -> f32 {
        Self::pop_in_progress(elapsed_ms)
    }

    /// Head vertical offset below the main pie center during the 400 ms pop-in
    /// (`0x4625ff`–`0x46260a`: `_ftol(t * 14.0)` while `elapsed_ms < 400`,
    /// `Y_OFFSET` once settled).
    pub fn head_y_offset(elapsed_ms: u32) -> f32 {
        if elapsed_ms >= POP_IN_MS {
            Y_OFFSET
        } else {
            (Self::pop_in_progress(elapsed_ms) * Y_OFFSET).trunc()
        }
    }

    /// Native direction-change ease-out factor (`0x462799`–`0x4627b0`):
    /// `sqrt(0.1 + 0.9 * (elapsed_ms / 400.0))` for `elapsed_ms < 400`, and
    /// `1.0` once `elapsed_ms >= 400`.
    pub fn direction_ease(elapsed_ms: u32) -> f32 {
        if elapsed_ms >= DIRECTION_EASE_MS {
            1.0
        } else {
            let t = elapsed_ms as f32 / DIRECTION_EASE_MS as f32;
            (DIRECTION_EASE_BASE + DIRECTION_EASE_SPAN * t).sqrt()
        }
    }

    /// Linear integer interpolation matching `0x46222f`–`0x462267`
    /// (`_ftol((1.0 - t) * from + t * to)` for `t < 1.0`, exact `to` at `1.0`).
    pub fn lerp_i32(from: i32, to: i32, t: f32) -> i32 {
        if !t.is_finite() || t >= 1.0 {
            return to;
        }
        if t <= 0.0 {
            return from;
        }
        let tf = f64::from(t);
        ((1.0 - tf) * f64::from(from) + tf * f64::from(to)).trunc() as i32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastDirection {
    /// Initial sentinel `[esi + 0x50] = -2` (`0x4620ca`) before the first frame.
    Unset,
    /// Nothing highlighted (`-1`).
    None,
    /// Highlighted compass direction (`0..=7`).
    Dir(u8),
}

impl LastDirection {
    fn from_opt(dir: Option<u8>) -> Self {
        match dir {
            Some(d) => Self::Dir(d),
            None => Self::None,
        }
    }
}

/// Tracks `Sims.exe`'s three 400 ms pie-menu transition animations
/// (`0x462078`–`0x462822`):
///
/// 1. Backdrop pop-in from radius `0` to `100` (`[0x6c4708]`) and submenu /
///    parent center-and-radius linear interpolation (`0x462106`, `0x46213e`,
///    `0x46216e`–`0x46228d`).
/// 2. Head pop-in scale (`0.0 -> 1.0`) and `Y_OFFSET` (`0 -> 14 px`) linear
///    ramp (`0x4625c1`–`0x46260d`).
/// 3. Head direction-change ease-out (`0x46269b`–`0x46281e`,
///    `sqrt(0.1 + 0.9 * t)` over `400 ms`).
#[derive(Debug, Clone, PartialEq)]
pub struct PieAnimState {
    main_center: [i32; 2],
    /// `[esi + 0x10]`: true during initial open pop-in until `400 ms` elapses
    /// or a submenu opens (`0x4620d1`, `0x462117`, `0x462216`).
    is_pop_in: bool,
    /// `[esi + 0x48]`: timestamp (ms) when the current backdrop/pop-in started.
    backdrop_start_ms: u32,
    backdrop_from: BackdropState,
    backdrop_to: BackdropState,
    backdrop_cur: BackdropState,
    /// `[esi + 0x50]`: previously observed highlight direction.
    last_direction: LastDirection,
    /// `[esi + 0x4c]`: timestamp (ms) when the active direction ease started.
    dir_start_ms: Option<u32>,
    /// `[esi + 0x70]` (pitch) and `[esi + 0x6c]` (yaw) snapshotted on direction
    /// change (`0x4626cf`–`0x4626d8`).
    dir_from_angles: [f32; 2],
}

impl PieAnimState {
    /// Starts a new pie-menu open transition centered at `main_center` at
    /// `now_ms` (`0x462078`, `0x462175`–`0x4621e0`).
    pub fn open(main_center: [i32; 2], now_ms: u32) -> Self {
        let from = BackdropState {
            center: main_center,
            radius: 0,
        };
        let to = BackdropState {
            center: main_center,
            radius: BACKDROP_RADIUS,
        };
        Self {
            main_center,
            is_pop_in: true,
            backdrop_start_ms: now_ms,
            backdrop_from: from,
            backdrop_to: to,
            backdrop_cur: from,
            last_direction: LastDirection::Unset,
            dir_start_ms: None,
            dir_from_angles: [0.0, 0.0],
        }
    }

    /// Starts a 400 ms submenu backdrop transition from the current
    /// interpolated backdrop to `(submenu_center, BACKDROP_RADIUS)` and
    /// finishes the initial head pop-in (`0x462106`–`0x46213b`).
    pub fn open_submenu(&mut self, submenu_center: [i32; 2], now_ms: u32) {
        let cur = self.backdrop(now_ms);
        self.is_pop_in = false;
        self.backdrop_start_ms = now_ms;
        self.backdrop_from = cur;
        self.backdrop_to = BackdropState {
            center: submenu_center,
            radius: BACKDROP_RADIUS,
        };
    }

    /// Starts a 400 ms transition returning the backdrop from the active
    /// submenu back to `main_center` (`0x46213e`–`0x46216d`).
    pub fn close_submenu(&mut self, now_ms: u32) {
        let cur = self.backdrop(now_ms);
        self.is_pop_in = false;
        self.backdrop_start_ms = now_ms;
        self.backdrop_from = cur;
        self.backdrop_to = BackdropState {
            center: self.main_center,
            radius: BACKDROP_RADIUS,
        };
    }

    /// Evaluates the circular radial backdrop's center and radius at `now_ms`
    /// (`0x46216e`–`0x46228d`).
    pub fn backdrop(&mut self, now_ms: u32) -> BackdropState {
        let elapsed = now_ms.saturating_sub(self.backdrop_start_ms);
        if elapsed >= POP_IN_MS {
            self.is_pop_in = false;
            self.backdrop_cur = self.backdrop_to;
        } else {
            let t = PieTransition::pop_in_progress(elapsed);
            self.backdrop_cur = BackdropState {
                center: [
                    PieTransition::lerp_i32(
                        self.backdrop_from.center[0],
                        self.backdrop_to.center[0],
                        t,
                    ),
                    PieTransition::lerp_i32(
                        self.backdrop_from.center[1],
                        self.backdrop_to.center[1],
                        t,
                    ),
                ],
                radius: PieTransition::lerp_i32(
                    self.backdrop_from.radius,
                    self.backdrop_to.radius,
                    t,
                ),
            };
        }
        self.backdrop_cur
    }

    /// Evaluates the 3D pie head's animated [`HeadPose`] for `look` at
    /// `now_ms`, applying both the 400 ms pop-in scale/`Y_OFFSET` ramp
    /// (`0x4625c1`–`0x46260d`) and the 400 ms direction-change ease-out
    /// (`0x462688`–`0x462822`).
    pub fn head_pose(&mut self, look: Look, now_ms: u32) -> HeadPose {
        let pop_elapsed = now_ms.saturating_sub(self.backdrop_start_ms);
        let (scale, y_offset) = if self.is_pop_in && pop_elapsed < POP_IN_MS {
            (
                PieTransition::head_scale(pop_elapsed),
                PieTransition::head_y_offset(pop_elapsed),
            )
        } else {
            if pop_elapsed >= POP_IN_MS {
                self.is_pop_in = false;
            }
            (1.0, Y_OFFSET)
        };

        let raw = look.pointer_angles();
        let target = look.angles();
        let next_dir = LastDirection::from_opt(look.direction);
        if next_dir != self.last_direction {
            self.last_direction = next_dir;
            if look.direction.is_some() {
                self.dir_start_ms = Some(now_ms);
                // 0x4626cf..0x4626d8: snapshot pointer-tracked angles without
                // the new compass turn as the ease-out start state.
                self.dir_from_angles = raw;
            } else {
                // With no highlighted direction, target == raw so the ease
                // curve is an exact identity; avoid spurious redraw ticks.
                self.dir_start_ms = None;
                self.dir_from_angles = raw;
            }
        }

        let [pitch, yaw] = if let Some(start_ms) = self.dir_start_ms {
            let elapsed = now_ms.saturating_sub(start_ms);
            if elapsed < DIRECTION_EASE_MS {
                let u = PieTransition::direction_ease(elapsed);
                let inv = 1.0 - u;
                [
                    inv * self.dir_from_angles[0] + u * target[0],
                    inv * self.dir_from_angles[1] + u * target[1],
                ]
            } else {
                // 0x4627d4..0x46281e: clear timer once 2 * 400 ms have passed.
                if elapsed >= DIRECTION_EASE_MS.saturating_mul(2) {
                    self.dir_start_ms = None;
                }
                target
            }
        } else {
            target
        };

        HeadPose {
            pitch,
            yaw,
            scale,
            y_offset,
            ambient: look.ambient(),
        }
    }

    /// Whether any 400 ms backdrop, pop-in, or direction-ease transition is
    /// still actively changing at `now_ms`.
    pub fn is_animating(&self, now_ms: u32) -> bool {
        let backdrop_active = now_ms.saturating_sub(self.backdrop_start_ms) < POP_IN_MS
            && (self.is_pop_in || self.backdrop_cur != self.backdrop_to);
        let dir_active = self
            .dir_start_ms
            .is_some_and(|start| now_ms.saturating_sub(start) < DIRECTION_EASE_MS);
        backdrop_active || dir_active
    }

    /// The parent pie menu's anchor center.
    pub const fn main_center(&self) -> [i32; 2] {
        self.main_center
    }
}

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
        // 0x5eb833: when distance < 1, the original returns early and leaves
        // the previous angle untouched (at the exact center distance == 0 so
        // r == 0 and any angle yields the same pose).
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

    /// Pointer-tracked `[pitch, yaw]` angles in radians before adding the
    /// 8-way compass offset (`0x4623c5`–`0x4623f3`, `0x462688`–`0x4626bf`).
    pub fn pointer_angles(&self) -> [f32; 2] {
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
        [cos_a * r + IDLE_BIAS, -sin_a * r + IDLE_BIAS]
    }

    /// The steady-state `[pitch, yaw]` angles in radians (`0x4623c5`–`0x4623f3`,
    /// `0x462688`–`0x462778`).
    pub fn angles(&self) -> [f32; 2] {
        let [mut pitch, mut yaw] = self.pointer_angles();
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
    project_scaled(point, origin, pitch, yaw, 1.0)
}

/// Like [`project`], scaling the HEAD-bone coordinates by `scale` (`0.0..=1.0`
/// during the 400 ms pop-in at `0x4625e1`–`0x46260d`).
pub fn project_scaled(point: Vec3, origin: [f32; 2], pitch: f32, yaw: f32, scale: f32) -> Vec3 {
    let v = rot_y(point, BASE_PITCH + pitch);
    let v = rot_z(v, FRAC_PI_2);
    let v = rot_y(v, yaw + PI);
    let v = rot_x(v, -FRAC_PI_6);
    let px_scale = SCALE_PIXELS * scale;
    Vec3::new(origin[0] + v.x * px_scale, origin[1] - v.y * px_scale, v.z)
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

/// A pie-menu 3D head rasterized at exact house-viewport coordinates and
/// clipped to the active house viewport and the native Direct3D 6 orthographic
/// near/far planes (`0x49b506`–`0x49b50b`, `0x462621` → `0x462985`), avoiding
/// any crop at the 201×201 face bitmap for tall or wide headwear.
#[derive(Debug, Clone)]
pub struct DrawnHead {
    /// Top-left screen pixel of [`DrawnHead::canvas`] in house-viewport
    /// coordinates.
    pub origin: [i32; 2],
    /// Tightly bounded RGBA pixels clipped to the house viewport and native
    /// near/far frustum.
    pub canvas: Canvas,
    /// Native 16-bit house z-buffer depth for each pixel of
    /// [`DrawnHead::canvas`] (`sims_render::depth::EMPTY` where transparent),
    /// for `D3DCMP_LESSEQUAL` depth testing against the house scene
    /// (`0x4a9500`, `0x5ce959`–`0x5ce977`).
    pub depth: Vec<i32>,
}

/// Target that rasterizes directly in full house-viewport coordinates while
/// storing only the tight bounding rectangle clamped to the viewport, applying
/// the house camera's native Direct3D 6 orthographic near/far clip planes
/// (`0x49b506`, `0x4623f5`–`0x462414`) and recording each winning pixel's
/// 16-bit screen depth (`0.0..=65535.0`).
struct ViewportPieTarget<'a> {
    viewport: (u32, u32),
    origin: (u32, u32),
    canvas: &'a mut Canvas,
    depth: &'a mut [i32],
}

impl Target for ViewportPieTarget<'_> {
    fn size(&self) -> (u32, u32) {
        self.viewport
    }

    fn passes(&self, x: u32, y: u32, z: f32) -> bool {
        if !(0.0..=65_535.0).contains(&z) {
            return false;
        }
        let Some(lx) = x.checked_sub(self.origin.0) else {
            return false;
        };
        let Some(ly) = y.checked_sub(self.origin.1) else {
            return false;
        };
        let Some(idx) = (ly as usize)
            .checked_mul(self.canvas.width as usize)
            .and_then(|row| row.checked_add(lx as usize))
        else {
            return false;
        };
        self.depth.get(idx).is_some_and(|&d| (z as i32) <= d)
    }

    fn write(&mut self, x: u32, y: u32, z: f32, texel: [u8; 4], s: f32) {
        if !(0.0..=65_535.0).contains(&z) {
            return;
        }
        let Some(lx) = x.checked_sub(self.origin.0) else {
            return;
        };
        let Some(ly) = y.checked_sub(self.origin.1) else {
            return;
        };
        let Some(idx) = (ly as usize)
            .checked_mul(self.canvas.width as usize)
            .and_then(|row| row.checked_add(lx as usize))
        else {
            return;
        };
        let zi = z as i32;
        let Some(slot) = self.depth.get_mut(idx) else {
            return;
        };
        if zi > *slot {
            return;
        }
        *slot = zi;
        let lit = |c: u8| (f32::from(c) * s + 0.5).clamp(0.0, 255.0) as u8;
        let [r, g, b, a] = texel;
        self.canvas
            .write(lx, ly, z, [lit(r), lit(g), lit(b), a], 1.0);
    }
}

/// Native 16-bit screen-depth mapping `(z_origin, z_per_unit)` for the active
/// house camera `zoom`, converting unscaled HEAD-bone `v.z` from [`project`]
/// into Direct3D 6's 16-bit z-buffer space `[0.0, 65535.0]` (`0x49b506`
/// `near = 0.0`, `0x49b124`–`0x49b187` `far`, `0x49b56d` `65535.0`,
/// `0x4623f5`–`0x462414` `z_screen = 0.5 * ((3000 + 10) << zoom)`,
/// `0x4625d9`–`0x46260d` `head_scale = (12.0 / (1 << zoom)) * 1.75`).
pub fn screen_depth_scale(zoom: sims_render::Zoom) -> (f32, f32) {
    const UNITS_PER_TILE: f32 = 3.0;
    let span = sims_render::camera::ARRAY_SIZE as f32 * UNITS_PER_TILE;
    let h = (255.0 / (2.0 * 6.0f32.sqrt())) * UNITS_PER_TILE + 68.0;
    let far = 3.0f32.sqrt() * (2.0f32.sqrt() * span) * 0.5 + h * 0.5;
    let zoom_factor = (zoom.tile_width() / 16) as f32;
    let z_origin = 0.5 * (3010.0 * zoom_factor);
    let head_scale = (12.0 / zoom_factor) * 1.75;
    (z_origin, (head_scale / far) * 65_535.0)
}

/// Native Direct3D 6 orthographic near/far frustum depth bounds `(near_z, far_z)`
/// in unscaled HEAD-bone units for the active house camera `zoom`
/// ([`screen_depth_scale`]).
///
/// Because [`project`] uses smaller (more negative) `z` for points closer to
/// the camera, pixels outside `near_z..=far_z` are clipped by Direct3D 6
/// (notably oversized headwear pitched down toward the viewer at far zoom).
pub fn depth_clip_bounds(zoom: sims_render::Zoom) -> (f32, f32) {
    let (z_origin, z_per_unit) = screen_depth_scale(zoom);
    (-z_origin / z_per_unit, (65_535.0 - z_origin) / z_per_unit)
}

fn skinned_head_parts(
    body: &Body,
) -> Result<Vec<(&crate::appearance::Part, crate::skin::Skinned)>, String> {
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
    Ok(parts)
}

/// Draws `body`'s pie-menu 3D head into an RGBA [`Canvas`] of `size` pixels
/// (typically `[201, 201]`), clear (`[0, 0, 0, 0]`) outside the head.
///
/// Returns an error if `size` exceeds [`MAX_SIDE`] on either axis, or if
/// `body`'s skeleton has no `HEAD` bone or no visible parts are bound to it.
pub fn draw_body(body: &Body, size: [u32; 2], look: Look) -> Result<Canvas, String> {
    draw_body_pose(body, size, HeadPose::steady(look))
}

/// Like [`draw_body`], rendering an animated [`HeadPose`] from
/// [`PieAnimState::head_pose`] (400 ms pop-in scale/`y_offset` and eased
/// `pitch`/`yaw`).
pub fn draw_body_pose(body: &Body, size: [u32; 2], pose: HeadPose) -> Result<Canvas, String> {
    let [width, height] = size;
    if width > MAX_SIDE || height > MAX_SIDE || width.checked_mul(height).is_none() {
        return Err(format!(
            "pie head size {width}x{height} exceeds {MAX_SIDE}x{MAX_SIDE}"
        ));
    }
    let parts = skinned_head_parts(body)?;
    let mut canvas = Canvas::new(width, height, [0; 4]);
    let scale = if pose.scale.is_finite() {
        pose.scale.max(0.0)
    } else {
        0.0
    };
    if scale == 0.0 {
        return Ok(canvas);
    }
    let pitch = if pose.pitch.is_finite() {
        pose.pitch
    } else {
        IDLE_BIAS
    };
    let yaw = if pose.yaw.is_finite() {
        pose.yaw
    } else {
        IDLE_BIAS
    };
    let y_offset = if pose.y_offset.is_finite() {
        pose.y_offset
    } else {
        Y_OFFSET
    };
    let ambient_level = if pose.ambient.is_finite() {
        pose.ambient.clamp(0.0, 1.0)
    } else {
        AMBIENT_IDLE
    };
    // 0x4625ae + 0x462621: bone origin sits `y_offset` px below the face center
    // (width/2, height/2), with (+0.5, +0.5) for D3D6 integer pixel centres.
    let origin = [
        (width / 2) as f32 + 0.5,
        (height / 2) as f32 + y_offset + 0.5,
    ];
    let ambient = Shade::Grey(ambient_level);
    let mut target = PieTarget(&mut canvas);
    for (part, skinned) in &parts {
        let shade = vec![ambient; skinned.positions.len()];
        let proj = |p| project_scaled(p, origin, pitch, yaw, scale);
        if part.texture_name.is_some() {
            let mut packed = Packed565(&mut target);
            draw_shaded_culled(&mut packed, part, skinned, &shade, proj);
        } else {
            draw_shaded_culled(&mut target, part, skinned, &shade, proj);
        }
    }
    Ok(canvas)
}

/// Draws `body`'s pie-menu 3D head positioned at the main pie menu's `center`
/// inside a house viewport of `viewport` `(width, height)` pixels (`0x462621`
/// → `0x462985`), clipping to `[0..viewport[0], 0..viewport[1]]` and the
/// house camera's native near/far frustum for `zoom` ([`depth_clip_bounds`])
/// so tall or wide headwear is never cropped by the 201×201 face bitmap while
/// still matching Direct3D 6 near-plane clipping.
pub fn draw_body_in_viewport(
    body: &Body,
    viewport: [u32; 2],
    center: [i32; 2],
    zoom: sims_render::Zoom,
    look: Look,
) -> Result<DrawnHead, String> {
    draw_body_in_viewport_pose(body, viewport, center, zoom, HeadPose::steady(look))
}

/// Like [`draw_body_in_viewport`], rendering an animated [`HeadPose`] from
/// [`PieAnimState::head_pose`] with `head_scale` applied to both screen
/// projection and 16-bit depth (`0x4625e1`–`0x46260d`).
pub fn draw_body_in_viewport_pose(
    body: &Body,
    viewport: [u32; 2],
    center: [i32; 2],
    zoom: sims_render::Zoom,
    pose: HeadPose,
) -> Result<DrawnHead, String> {
    let parts = skinned_head_parts(body)?;
    let [vp_w, vp_h] = viewport;
    let scale = if pose.scale.is_finite() {
        pose.scale.max(0.0)
    } else {
        0.0
    };
    if vp_w == 0 || vp_h == 0 || scale == 0.0 {
        return Ok(DrawnHead {
            origin: [0, 0],
            canvas: Canvas::new(0, 0, [0; 4]),
            depth: Vec::new(),
        });
    }
    let pitch = if pose.pitch.is_finite() {
        pose.pitch
    } else {
        IDLE_BIAS
    };
    let yaw = if pose.yaw.is_finite() {
        pose.yaw
    } else {
        IDLE_BIAS
    };
    let y_offset = if pose.y_offset.is_finite() {
        pose.y_offset
    } else {
        Y_OFFSET
    };
    let ambient_level = if pose.ambient.is_finite() {
        pose.ambient.clamp(0.0, 1.0)
    } else {
        AMBIENT_IDLE
    };
    let origin = [center[0] as f32 + 0.5, center[1] as f32 + y_offset + 0.5];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (_, skinned) in &parts {
        for &p in &skinned.positions {
            let v = project_scaled(p, origin, pitch, yaw, scale);
            if v.x.is_finite() && v.y.is_finite() {
                min_x = min_x.min(v.x);
                min_y = min_y.min(v.y);
                max_x = max_x.max(v.x);
                max_y = max_y.max(v.y);
            }
        }
    }
    if !min_x.is_finite() || !min_y.is_finite() {
        return Ok(DrawnHead {
            origin: [0, 0],
            canvas: Canvas::new(0, 0, [0; 4]),
            depth: Vec::new(),
        });
    }
    let rx0 = (min_x.floor() as i64)
        .saturating_sub(1)
        .clamp(0, i64::from(vp_w)) as u32;
    let ry0 = (min_y.floor() as i64)
        .saturating_sub(1)
        .clamp(0, i64::from(vp_h)) as u32;
    let rx1 = (max_x.ceil() as i64)
        .saturating_add(1)
        .clamp(0, i64::from(vp_w)) as u32;
    let ry1 = (max_y.ceil() as i64)
        .saturating_add(1)
        .clamp(0, i64::from(vp_h)) as u32;
    if rx0 >= rx1 || ry0 >= ry1 {
        return Ok(DrawnHead {
            origin: [0, 0],
            canvas: Canvas::new(0, 0, [0; 4]),
            depth: Vec::new(),
        });
    }
    let rw = rx1 - rx0;
    let rh = ry1 - ry0;
    if rw > MAX_SIDE || rh > MAX_SIDE || rw.checked_mul(rh).is_none() {
        return Err(format!(
            "pie head bounds {rw}x{rh} exceed {MAX_SIDE}x{MAX_SIDE}"
        ));
    }
    let (z_origin, z_per_unit) = screen_depth_scale(zoom);
    let scaled_z_per_unit = z_per_unit * scale;
    let mut canvas = Canvas::new(rw, rh, [0; 4]);
    let mut depth = vec![sims_render::depth::EMPTY; (rw * rh) as usize];
    let ambient = Shade::Grey(ambient_level);
    let mut target = ViewportPieTarget {
        viewport: (vp_w, vp_h),
        origin: (rx0, ry0),
        canvas: &mut canvas,
        depth: &mut depth,
    };
    for (part, skinned) in &parts {
        let shade = vec![ambient; skinned.positions.len()];
        let proj = |p| {
            let v = project_scaled(p, origin, pitch, yaw, scale);
            Vec3::new(v.x, v.y, z_origin + v.z * scaled_z_per_unit)
        };
        if part.texture_name.is_some() {
            let mut packed = Packed565(&mut target);
            draw_shaded_culled(&mut packed, part, skinned, &shade, proj);
        } else {
            draw_shaded_culled(&mut target, part, skinned, &shade, proj);
        }
    }
    Ok(DrawnHead {
        origin: [
            i32::try_from(rx0).unwrap_or(i32::MAX),
            i32::try_from(ry0).unwrap_or(i32::MAX),
        ],
        canvas,
        depth,
    })
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

    #[test]
    fn draw_body_in_viewport_handles_extreme_finite_coordinates_without_overflow() {
        use crate::appearance::Part;
        use crate::texture::Texture;
        use sims_formats::avatar::{Bone, Mesh, MeshBinding, Skeleton, Vertex, Wiggle};

        let body = Body {
            skeleton: Skeleton {
                name: "test".into(),
                bones: vec![Bone {
                    name: "HEAD".into(),
                    parent: None,
                    translation: [0.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    properties: Vec::new(),
                    flags: [0; 3],
                    wiggle: Wiggle::default(),
                }],
            },
            parts: vec![Part {
                appearance: "extreme".into(),
                bone: 0,
                bones: vec![0],
                mesh: Mesh {
                    name: "extreme-HEAD".into(),
                    texture: String::new(),
                    bones: vec!["HEAD".into()],
                    bindings: vec![MeshBinding {
                        bone: 0,
                        first_vertex: 0,
                        vertex_count: 3,
                        first_blend: 0,
                        blend_count: 0,
                    }],
                    triangles: vec![[0, 1, 2]],
                    vertices: vec![
                        Vertex {
                            position: [1e20, 1e20, 0.0],
                            normal: [0.0, 0.0, 1.0],
                            uv: [0.0, 0.0],
                            bone: 0,
                        },
                        Vertex {
                            position: [-1e20, -1e20, 0.0],
                            normal: [0.0, 0.0, 1.0],
                            uv: [1.0, 0.0],
                            bone: 0,
                        },
                        Vertex {
                            position: [0.0, 0.0, 1e20],
                            normal: [0.0, 0.0, 1.0],
                            uv: [0.0, 1.0],
                            bone: 0,
                        },
                    ],
                    blends: Vec::new(),
                },
                texture_name: None,
                texture: Texture::solid([200, 200, 200, 255]),
                censor: 0,
            }],
        };

        let drawn = draw_body_in_viewport(
            &body,
            [800, 500],
            [400, 250],
            sims_render::Zoom::Far,
            Look::neutral(),
        )
        .expect("extreme finite coordinates must clamp cleanly without overflow");
        assert!(drawn.canvas.width <= 800 && drawn.canvas.height <= 500);
    }

    #[test]
    fn pie_transition_400ms_pop_in_backdrop_and_submenu_glide_match_native() {
        let mut anim = PieAnimState::open([300, 200], 1000);
        assert!(anim.is_animating(1000));
        assert_eq!(
            anim.backdrop(1000),
            BackdropState {
                center: [300, 200],
                radius: 0,
            }
        );
        // Halfway through 400 ms pop-in: radius = 50, head scale = 0.5, y_offset = 7.
        assert_eq!(
            anim.backdrop(1200),
            BackdropState {
                center: [300, 200],
                radius: 50,
            }
        );
        let mid_pose = anim.head_pose(Look::neutral(), 1200);
        assert!((mid_pose.scale - 0.5).abs() < 1e-6);
        assert_eq!(mid_pose.y_offset, 7.0);

        // At 400 ms: pop-in settles cleanly at radius = 100, scale = 1.0, y_offset = 14.
        assert_eq!(
            anim.backdrop(1400),
            BackdropState {
                center: [300, 200],
                radius: BACKDROP_RADIUS,
            }
        );
        let settled_pose = anim.head_pose(Look::neutral(), 1400);
        assert_eq!(settled_pose.scale, 1.0);
        assert_eq!(settled_pose.y_offset, Y_OFFSET);

        // Open submenu at [420, 280]: glides linearly from [300, 200] over 400 ms
        // without re-triggering head pop-in scale ramp (`0x462117`).
        anim.open_submenu([420, 280], 2000);
        assert_eq!(
            anim.backdrop(2200),
            BackdropState {
                center: [360, 240],
                radius: BACKDROP_RADIUS,
            }
        );
        let sub_pose = anim.head_pose(Look::neutral(), 2200);
        assert_eq!(sub_pose.scale, 1.0);
        assert_eq!(sub_pose.y_offset, Y_OFFSET);

        // Close submenu mid-flight (at 2200 ms -> [360, 240]): glides back to [300, 200].
        anim.close_submenu(2200);
        assert_eq!(
            anim.backdrop(2400),
            BackdropState {
                center: [330, 220],
                radius: BACKDROP_RADIUS,
            }
        );
        assert_eq!(
            anim.backdrop(2600),
            BackdropState {
                center: [300, 200],
                radius: BACKDROP_RADIUS,
            }
        );
    }

    #[test]
    fn pie_transition_400ms_direction_ease_out_matches_sqrt_curve() {
        let mut anim = PieAnimState::open([300, 200], 0);
        // Settle initial pop-in at 500 ms with neutral look.
        let _ = anim.backdrop(500);
        let _ = anim.head_pose(Look::neutral(), 500);
        assert!(!anim.is_animating(500));

        // Hover north option (direction 0 -> pitch += 0.6) at t = 1000 ms.
        let north = Look {
            distance: 0.0,
            angle: 0.0,
            direction: Some(0),
        };
        let p0 = anim.head_pose(north, 1000);
        assert!(anim.is_animating(1000));
        let u0 = 0.1_f32.sqrt();
        assert!((p0.pitch - (IDLE_BIAS + u0 * CARDINAL_TURN)).abs() < 1e-5);
        assert!((p0.yaw - IDLE_BIAS).abs() < 1e-6);

        // At +200 ms (t = 0.5): u = sqrt(0.1 + 0.45) = sqrt(0.55).
        let p_mid = anim.head_pose(north, 1200);
        let u_mid = (0.1_f32 + 0.9_f32 * 0.5_f32).sqrt();
        assert!((p_mid.pitch - (IDLE_BIAS + u_mid * CARDINAL_TURN)).abs() < 1e-5);
        assert!(p_mid.pitch > p0.pitch);

        // At +400 ms: settles identically to steady-state Look::angles().
        let p_done = anim.head_pose(north, 1400);
        assert_eq!([p_done.pitch, p_done.yaw], north.angles());
        assert!(!anim.is_animating(1400));
    }

    #[test]
    fn draw_body_pose_handles_zero_scale_and_matches_steady_bit_for_bit() {
        use crate::appearance::Part;
        use crate::texture::Texture;
        use sims_formats::avatar::{Bone, Mesh, MeshBinding, Skeleton, Vertex, Wiggle};

        let body = Body {
            skeleton: Skeleton {
                name: "test".into(),
                bones: vec![Bone {
                    name: "HEAD".into(),
                    parent: None,
                    translation: [0.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    properties: Vec::new(),
                    flags: [0; 3],
                    wiggle: Wiggle::default(),
                }],
            },
            parts: vec![Part {
                appearance: "tri".into(),
                bone: 0,
                bones: vec![0],
                mesh: Mesh {
                    name: "tri-HEAD".into(),
                    texture: String::new(),
                    bones: vec!["HEAD".into()],
                    bindings: vec![MeshBinding {
                        bone: 0,
                        first_vertex: 0,
                        vertex_count: 3,
                        first_blend: 0,
                        blend_count: 0,
                    }],
                    triangles: vec![[0, 1, 2]],
                    vertices: vec![
                        Vertex {
                            position: [0.6, 0.0, 0.4],
                            normal: [0.0, 0.0, 1.0],
                            uv: [0.0, 0.0],
                            bone: 0,
                        },
                        Vertex {
                            position: [-0.3, -0.5, 0.4],
                            normal: [0.0, 0.0, 1.0],
                            uv: [1.0, 0.0],
                            bone: 0,
                        },
                        Vertex {
                            position: [-0.3, 0.5, 0.4],
                            normal: [0.0, 0.0, 1.0],
                            uv: [0.0, 1.0],
                            bone: 0,
                        },
                    ],
                    blends: Vec::new(),
                },
                texture_name: None,
                texture: Texture::solid([180, 140, 120, 255]),
                censor: 0,
            }],
        };

        let look = Look::from_menu([200, 200], Some([200, 120]), Some(0), 4);
        let steady_a = draw_body(&body, [128, 128], look).unwrap();
        let steady_b = draw_body_pose(&body, [128, 128], HeadPose::steady(look)).unwrap();
        assert_eq!(steady_a.rgba, steady_b.rgba);

        let vp_a = draw_body_in_viewport(
            &body,
            [400, 300],
            [200, 150],
            sims_render::Zoom::Medium,
            look,
        )
        .unwrap();
        let vp_b = draw_body_in_viewport_pose(
            &body,
            [400, 300],
            [200, 150],
            sims_render::Zoom::Medium,
            HeadPose::steady(look),
        )
        .unwrap();
        assert_eq!(vp_a.origin, vp_b.origin);
        assert_eq!(vp_a.canvas.rgba, vp_b.canvas.rgba);
        assert_eq!(vp_a.depth, vp_b.depth);

        // Zero scale (t = 0 on initial open) produces an empty transparent result.
        let zero_pose = HeadPose {
            scale: 0.0,
            y_offset: 0.0,
            ..HeadPose::steady(look)
        };
        let zero_canvas = draw_body_pose(&body, [128, 128], zero_pose).unwrap();
        assert!(zero_canvas.rgba.iter().all(|&b| b == 0));
        let zero_vp = draw_body_in_viewport_pose(
            &body,
            [400, 300],
            [200, 150],
            sims_render::Zoom::Medium,
            zero_pose,
        )
        .unwrap();
        assert_eq!((zero_vp.canvas.width, zero_vp.canvas.height), (0, 0));
    }
}
