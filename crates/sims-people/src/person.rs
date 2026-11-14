//! A person drawn where the simulation has them, playing what it plays.
//!
//! The simulation is the source of truth. This module reads its
//! [`Animator`] and never changes it:
//!
//! - **Where.** `Animator::position` holds x and y in sixteenths of a tile
//!   and the direction in eighths of a turn. A tile is
//!   [`UNITS_PER_TILE`] model units (the walk clip's own stride, see
//!   below). Sim x becomes model X and sim y becomes model Z; the ground
//!   is at Y = 0.
//! - **Which way.** Person 259, walking through 82 ticks of the boot
//!   (`sims boot` of House05, ticks 226 to 333), moves along
//!   (sin θ, -cos θ) in sim x and y, for θ its direction times 45
//!   degrees: the difference averages 0.2 degrees, while the direction
//!   itself swings by up to 12 degrees from tick to tick. A skeleton at rest
//!   faces +Z (its toes are at +Z), so it is turned by π - θ about the
//!   vertical. The old prototype turned people the same way
//!   (legacy/crates/sims-render/src/lib.rs, `actor_pose`).
//! - **What.** The foreground layer (`Animator::foreground`) is drawn
//!   over the base layer, which plays the posture's loop. The simulation
//!   keeps whether the base layer exists (`Animator::base`) but not its
//!   clip or phase, so the clip comes from the saved person record and the
//!   phase from the animator's clock. Both are stand-ins until the
//!   simulation exposes them.
//!
//! Whether the result is the original's image or its mirror image is
//! **unverified** (see [`crate::rig`]).

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
    let [s, c] = heading(direction);
    Some(Transform {
        rotation: Quat::about_y(s, -c),
        translation: Vec3::new(x / 16.0 * UNITS_PER_TILE, 0.0, y / 16.0 * UNITS_PER_TILE),
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
    /// weight. A foreground layer repeats when its end mode is 1.
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
        if let Some(layer) = &animator.foreground {
            let clip = self.clip(&layer.clip)?;
            clip.apply(
                skeleton,
                &mut pose,
                layer.phase.to_f32(),
                layer.mode == 1,
                layer.weight.to_f32(),
            );
        }
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
        // Direction 2 (a quarter turn): walking towards +x.
        animator.position = [32.0, 48.0, 2.0].map(F32::from_f32);
        let root = placement(&animator).unwrap();
        assert_eq!(root.translation, Vec3::new(6.0, 0.0, 9.0));
        let ahead = root.vector(Vec3::new(0.0, 0.0, 1.0));
        assert!(
            (ahead - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-6,
            "{ahead:?}"
        );
        // Direction 0: walking towards -y.
        animator.position[2] = F32::ZERO;
        let ahead = placement(&animator)
            .unwrap()
            .vector(Vec3::new(0.0, 0.0, 1.0));
        assert!(
            (ahead - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-6,
            "{ahead:?}"
        );
    }
}
