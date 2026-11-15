//! A person drawn where the simulation has them, playing what it plays.
//!
//! The simulation is the source of truth. This module reads its
//! [`Animator`] and never changes it:
//!
//! - **Where.** `Animator::position` holds x and y in sixteenths of a tile
//!   and the direction in eighths of a turn. A tile is
//!   [`UNITS_PER_TILE`] model units (the walk clip's own stride, see
//!   below). The original's model matrix (0x592132, ported as
//!   `sims_sim::head::placement`) puts sim y on model X and sim x on
//!   model Z. Here both are negated as well, which is the original's lot
//!   turned half a turn about the vertical: with the house view's camera
//!   ([`crate::raster::Camera::house_view`]) sim +x then goes right and
//!   down the screen and sim +y left and down, as the lot drawing's
//!   projection has them (0x4b3c88, `sims_render::camera`). The ground is
//!   at Y = 0.
//! - **Which way.** The original turns the skeleton by (direction + 6)
//!   eighths about the vertical (0x592132); with the half turn above,
//!   that is θ + π/2 for θ the direction times 45 degrees. A skeleton at
//!   rest faces +Z (its toes are at +Z), so a person faces
//!   (sin θ, -cos θ) in sim x and y. That is the way the simulation walks
//!   them: person 259, walking through 82 ticks of the boot (`sims boot`
//!   of House05, ticks 226 to 333), moves along it with a mean difference
//!   of 0.2 degrees, while the direction itself swings by up to 12
//!   degrees from tick to tick. Direction 3 faces the viewer.
//! - **What.** The foreground layer (`Animator::foreground`) is drawn
//!   over the base layer, which plays the posture's loop. The simulation
//!   keeps whether the base layer exists (`Animator::base`) but not its
//!   clip or phase, so the clip comes from the saved person record and the
//!   phase from the animator's clock. Both are stand-ins until the
//!   simulation exposes them.
//!
//! The image is the original's, not its mirror image: see [`crate::rig`].

use crate::animation::{Clip, Library};
use crate::assets::Assets;
use crate::math::{Quat, Transform, Vec3};
use crate::rig::Pose;
use sims_formats::avatar::Skeleton;
use sims_formats::saved_objects::Person;
use sims_sim::animator::{Animator, UNSET};
use std::collections::BTreeMap;

/// Model units per tile. A walk step moves the person by the clip's
/// distance times 16/3 sixteenths of a tile (the sims-vm animator's walk step, from
/// 0x585502), so a clip's 3 units are one tile. An adult skeleton's head
/// is about 5 units up.
pub const UNITS_PER_TILE: f32 = 3.0;

/// The sine and cosine of a direction given in eighths of a turn. Only
/// additions and multiplications are used, so the result has the same
/// bits on every machine and renders made from it can be pinned.
pub fn heading(eighths: f32) -> [f32; 2] {
    if !eighths.is_finite() {
        return [0.0, 1.0];
    }
    let whole = eighths.round();
    let x = (eighths - whole) * std::f32::consts::FRAC_PI_4;
    let x2 = x * x;
    // Taylor series on |x| <= π/8: the error is below 1e-9.
    let s = x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0)));
    let c = 1.0 - x2 / 2.0 * (1.0 - x2 / 12.0 * (1.0 - x2 / 30.0));
    let h = 0.5f32.sqrt();
    let [sa, ca] = match (whole as i64).rem_euclid(8) {
        0 => [0.0, 1.0],
        1 => [h, h],
        2 => [1.0, 0.0],
        3 => [h, -h],
        4 => [0.0, -1.0],
        5 => [-h, -h],
        6 => [-1.0, 0.0],
        _ => [-h, h],
    };
    [sa * c + ca * s, ca * c - sa * s]
}

/// Where the simulation has the person, as the transform of the
/// skeleton's root: `None` while the animator is not placed.
pub fn placement(animator: &Animator) -> Option<Transform> {
    if animator.position.contains(&UNSET) {
        return None;
    }
    let [x, y, direction] = animator.position.map(|v| v.to_f32());
    // θ + π/2: two eighths more than the direction.
    let [s, c] = heading(direction + 2.0);
    Some(Transform {
        rotation: Quat::about_y(s, c),
        translation: Vec3::new(-y / 16.0 * UNITS_PER_TILE, 0.0, -x / 16.0 * UNITS_PER_TILE),
    })
}

/// The base layer's clip for a person: the clip name of the saved
/// record's base descriptor (`"a2o-standing-loop;-10;1000;..."`), or the
/// standing loop of the skeleton's age (`a2o` for adults, `c2o` for
/// children) when it is empty.
pub fn base_clip(person: &Person, skeleton: &str) -> String {
    let saved = person.animations[2].split(';').next().unwrap_or("").trim();
    if saved.is_empty() {
        format!("{}-standing-loop", age_prefix(skeleton))
    } else {
        saved.to_owned()
    }
}

/// The walking loop of the skeleton's age: `c2o-walking-loop` for a
/// child, `a2o-walking-loop` otherwise.
pub fn walk_clip(skeleton: &str) -> String {
    format!("{}-walking-loop", age_prefix(skeleton))
}

/// The prefix clip names give an age: `c2o` for a child's skeleton,
/// `a2o` for any other.
fn age_prefix(skeleton: &str) -> &'static str {
    if skeleton.eq_ignore_ascii_case("child") {
        "c2o"
    } else {
        "a2o"
    }
}

/// Plays the simulation's clips: loads each clip once and poses
/// skeletons from animators.
#[derive(Debug)]
pub struct Player<'a> {
    assets: &'a Assets,
    library: &'a Library,
    clips: BTreeMap<String, Clip>,
}

impl<'a> Player<'a> {
    pub fn new(assets: &'a Assets, library: &'a Library) -> Self {
        Self {
            assets,
            library,
            clips: BTreeMap::new(),
        }
    }

    /// A player that starts with the clips another one loaded
    /// ([`Player::into_clips`]).
    pub(crate) fn with_clips(
        assets: &'a Assets,
        library: &'a Library,
        clips: BTreeMap<String, Clip>,
    ) -> Self {
        Self {
            assets,
            library,
            clips,
        }
    }

    /// The clips loaded so far, to keep between frames.
    pub(crate) fn into_clips(self) -> BTreeMap<String, Clip> {
        self.clips
    }

    /// The clip named `name`, loaded on first use.
    pub fn clip(&mut self, name: &str) -> Result<&Clip, String> {
        let key = name.to_ascii_lowercase();
        if !self.clips.contains_key(&key) {
            let clip = self.library.load(self.assets, name)?;
            self.clips.insert(key.clone(), clip);
        }
        Ok(&self.clips[&key])
    }

    /// The pose of `skeleton` for `animator`: the rest pose, then the base
    /// clip `base` (when the animator has a base layer) repeating at the
    /// animator's clock, then the foreground layer at its phase and
    /// weight, then the carry layer the same way. A layer repeats when its
    /// end mode is 1. The layers go on in the order of their priorities
    /// (the base -10, the foreground 1 or 6, the carry 10), so a bone two
    /// clips move takes the higher layer's motion, as the skin's bone
    /// assignment does (0x5991fd); that blending is not modeled further.
    /// Last, the head turns as the simulation's head fields say
    /// ([`Pose::turn_head`]).
    pub fn pose(
        &mut self,
        skeleton: &Skeleton,
        animator: &Animator,
        base: &str,
    ) -> Result<Pose, String> {
        let mut pose = Pose::rest(skeleton);
        if animator.base {
            let clip = self.clip(base)?;
            let duration = f64::from(clip.animation.duration_ms);
            let phase = if duration > 0.0 {
                (f64::from(animator.clock).rem_euclid(duration) / duration) as f32
            } else {
                0.0
            };
            clip.apply(skeleton, &mut pose, phase, true, 1.0);
        }
        for layer in [&animator.foreground, &animator.carry]
            .into_iter()
            .flatten()
        {
            let clip = self.clip(&layer.clip)?;
            clip.apply(
                skeleton,
                &mut pose,
                layer.phase.to_f32(),
                layer.mode == 1,
                layer.weight.to_f32(),
            );
        }
        pose.turn_head(skeleton, animator.head.turn);
        Ok(pose)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_core::F32;

    /// A person record with nothing in it.
    fn blank_person() -> Person {
        let idle = sims_formats::saved_objects::Interaction {
            uid: 0,
            caller: 0,
            target: 0,
            icon: 0,
            table_index: -1,
            args: [0; 4],
            priority: 0,
            action: 0,
            attenuation: 0.0,
            flags: 0,
        };
        Person {
            animation_events: 0,
            engaged: 0,
            appearance: Default::default(),
            accessories: Vec::new(),
            animations: Default::default(),
            routing_state: 0,
            floats: [0.0; 9],
            previous_motives: [0.0; 16],
            motives: [0.0; 16],
            data: Vec::new(),
            routing_frames: 0,
            active: idle.clone(),
            last: idle,
            queue: Vec::new(),
            object_uses: Vec::new(),
            motive_deltas: Vec::new(),
        }
    }

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
    }

    #[test]
    fn heading_matches_the_sine_and_cosine() {
        for i in -40..=40 {
            let eighths = i as f32 * 0.37;
            let (s, c) = (eighths * std::f32::consts::FRAC_PI_4).sin_cos();
            assert!(close(heading(eighths), [s, c]), "{eighths}");
        }
        let h = 0.5f32.sqrt();
        assert_eq!(heading(3.0), [h, -h]);
        assert_eq!(heading(-2.0), [-1.0, 0.0]);
    }

    #[test]
    fn clips_follow_the_skeletons_age() {
        assert_eq!(walk_clip("adult"), "a2o-walking-loop");
        assert_eq!(walk_clip("Child"), "c2o-walking-loop");
        // Any other name, however short, is taken as an adult's.
        assert_eq!(walk_clip(""), "a2o-walking-loop");
        assert_eq!(walk_clip("a"), "a2o-walking-loop");
        let mut person = blank_person();
        assert_eq!(base_clip(&person, "child"), "c2o-standing-loop");
        person.animations[2] = "a;-10;1000".into();
        assert_eq!(base_clip(&person, "adult"), "a");
    }

    #[test]
    fn a_placed_person_faces_the_way_it_walks() {
        let mut animator = Animator::new(0);
        assert_eq!(placement(&animator), None);
        // Sim x is model -Z, sim y model -X.
        animator.position = [32.0, 48.0, 2.0].map(F32::from_f32);
        let root = placement(&animator).unwrap();
        assert_eq!(root.translation, Vec3::new(-9.0, 0.0, -6.0));
        // Direction 2 (a quarter turn): walking towards +x, model -Z.
        let ahead = root.vector(Vec3::new(0.0, 0.0, 1.0));
        assert!(
            (ahead - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-6,
            "{ahead:?}"
        );
        // Direction 0: walking towards -y, model +X.
        animator.position[2] = F32::ZERO;
        let ahead = placement(&animator)
            .unwrap()
            .vector(Vec3::new(0.0, 0.0, 1.0));
        assert!(
            (ahead - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-6,
            "{ahead:?}"
        );
    }

    #[test]
    fn the_pose_turns_the_head_the_simulation_turned() {
        let mut skeleton = crate::rig::tests::skeleton();
        skeleton.bones[1].name = "HEAD".into();
        let (assets, library) = (Assets::default(), Library::default());
        let mut player = Player::new(&assets, &library);
        // No base layer and no foreground: only the rest pose and the head.
        let mut animator = Animator::new(0);
        let still = player.pose(&skeleton, &animator, "unused").unwrap();
        assert_eq!(still, Pose::rest(&skeleton));
        animator.head.turn = F32::from_f32(1.0);
        let turned = player.pose(&skeleton, &animator, "unused").unwrap();
        let mut want = Pose::rest(&skeleton);
        want.turn_head(&skeleton, animator.head.turn);
        assert_eq!(turned, want);
        assert_ne!(turned, still);
    }

    #[test]
    fn the_lot_lies_on_the_screen_as_the_lot_drawing_has_it() {
        // The lot drawing (0x4b3c88): a tile's screen x grows with
        // x - y and its screen y (downwards) with x + y.
        let camera = crate::raster::Camera::house_view(1.0, Vec3::ZERO);
        let at = |x: f32, y: f32, direction: f32| {
            let mut animator = Animator::new(0);
            animator.position = [x, y, direction].map(F32::from_f32);
            placement(&animator).unwrap()
        };
        let screen = |x: f32, y: f32| camera.view(at(x, y, 0.0).translation);
        let (px, py) = (screen(16.0, 0.0), screen(0.0, 16.0));
        assert!(px.x > 0.0 && px.y < 0.0, "+x goes right and down: {px:?}");
        assert!(py.x < 0.0 && py.y < 0.0, "+y goes left and down: {py:?}");
        assert!((px.x + py.x).abs() < 1e-5 && (px.y - py.y).abs() < 1e-5);
        assert!((px.x + 2.0 * px.y).abs() < 1e-5, "twice as wide as deep");
        // Direction 3 faces the viewer: the toes point out of the screen.
        let toes = at(0.0, 0.0, 3.0).vector(Vec3::new(0.0, 0.0, 1.0));
        let forward = camera.forward();
        assert!(toes.dot(forward) < -0.8, "{toes:?} {forward:?}");
    }
}
