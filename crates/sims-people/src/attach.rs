//! Where a person draws what it holds: its animator's attach points.
//!
//! The original draws a Sim's held objects from the Sim's own draw, not
//! from the generic slot walk, which skips person holders (`0x437c56`:
//! definition `+0x12 == 2`). The person's draw (`0x459dc0`..`0x45a131`)
//! draws the object in each container slot at one of the animator's
//! attach points, then the Sim (`0x4378bc`), all depth-tested:
//! `0x59185d(ctx, k, object)` takes attach point `k` (`0x59293d`), puts its
//! position and the person's level into the draw context (`0x4b966b`) and
//! calls the object's draw. Slot 0 (the carry slot) and slot 4 use point 0,
//! slot 2 point 3, slot 3 point 1 (`sims_render::PersonAttach::for_slot`).
//!
//! The animator binds its points when its body is made (`0x5924a4`, from
//! `0x5884cc`), from the person type's body strings (`STR# 200`, counting
//! from 1, through `0x4704fa`; read at `0x5924db`..`0x592585`): point 0
//! (`+0x140`) is the bone named by entry 6 at the named point of entry 7;
//! points 1 and 2 (`+0x144`) the bone of entry 4 at the point of entry 5;
//! point 3 (`+0x148`) is `PELVIS` with no named point. A bone the skeleton
//! lacks (an empty entry, a name in another case, or a pet's skeleton) or
//! a point name the original does not know leaves the point unbound, and
//! the slot draw skips an unbound point and what it holds (`0x591874`). Nearly every character file has
//! entries 4 to 7 as `HEAD`, `Top`, `R_HAND`, `Palm`; the Side Show
//! Assistant (`NPC_SideShowAssistTop.iff`) leaves 6 and 7 empty, so it
//! has no point 0.
//!
//! A named point is a fixed matrix on the bone (`0x5a1631`): a move, then
//! a turn. "Palm" moves (0.5, 0, 0); "Top" (0.8, 0, 0); "HeadFake"
//! (0.8, 0, 100); "Shoulder" (0, 0, 1); no name, none (the bone itself,
//! `0x5a199b`). The point's position is the translation of
//! point · bone · skeleton (`0x59640d`, `0x59638f`, with row vectors:
//! `0x49b87c(out, a, b)` is `a · b`), so the turn after the move does not
//! change it: the point is the bone-local move put through the posed bone.
//! Posed bones here land exactly where the original's composition puts
//! them ([`crate::rig`]).

use crate::math::{Transform, Vec3};
use sims_formats::avatar::Skeleton;

/// Point 3's bone (`0x6c45ac`, set at `0x5925b4`): its origin.
pub const PELVIS: &str = "PELVIS";

/// The bone-local move of the named point `name` (`0x5a1631`), or None
/// for a name the original does not know. An empty name is the bone
/// itself.
pub fn named_point(name: &str) -> Option<Vec3> {
    Some(match name {
        "" => Vec3::ZERO,
        "Palm" => Vec3::new(0.5, 0.0, 0.0),
        "Top" => Vec3::new(0.8, 0.0, 0.0),
        "HeadFake" => Vec3::new(0.8, 0.0, 100.0),
        "Shoulder" => Vec3::new(0.0, 0.0, 1.0),
        _ => return None,
    })
}

/// The bone named exactly `name`. The original finds the binding's bone
/// by a case-sensitive hash and byte compare, so unlike
/// [`crate::rig::bone`] a name in another case binds nothing.
fn bone(skeleton: &Skeleton, name: &str) -> Option<usize> {
    skeleton.bones.iter().position(|b| b.name == name)
}

/// A posed person's attach points, in the drawing frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Attach {
    /// Point 0: body strings 6 and 7 (`R_HAND`'s "Palm").
    pub palm: Option<Vec3>,
    /// Points 1 and 2: body strings 4 and 5 (`HEAD`'s "Top").
    pub head_top: Option<Vec3>,
    /// Point 3: `PELVIS`.
    pub pelvis: Option<Vec3>,
}

/// The attach points of a skeleton posed as `bones` (each bone's world
/// transform, in the skeleton's order), bound by the person type's body
/// strings `rows` (in their order: `rows[0]` is entry 1).
pub fn attach(skeleton: &Skeleton, bones: &[Transform], rows: &[String]) -> Attach {
    let at = |bone: &str, point: &str| {
        let local = named_point(point)?;
        let bone = bones.get(self::bone(skeleton, bone)?)?;
        Some(bone.point(local))
    };
    let entry = |n: usize| rows.get(n - 1).map(String::as_str);
    let bound = |bone: usize, point: usize| at(entry(bone)?, entry(point).unwrap_or(""));
    Attach {
        palm: bound(6, 7),
        head_top: bound(4, 5),
        pelvis: at(PELVIS, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_points_are_the_originals_moves() {
        assert_eq!(named_point("Palm"), Some(Vec3::new(0.5, 0.0, 0.0)));
        assert_eq!(named_point("Top"), Some(Vec3::new(0.8, 0.0, 0.0)));
        assert_eq!(named_point("HeadFake"), Some(Vec3::new(0.8, 0.0, 100.0)));
        assert_eq!(named_point("Shoulder"), Some(Vec3::new(0.0, 0.0, 1.0)));
        assert_eq!(named_point(""), Some(Vec3::ZERO));
        assert_eq!(named_point("Elbow"), None);
    }

    /// The body strings bind the points: entries 6 and 7 the palm, 4 and 5
    /// the head top; PELVIS is always bound. Empty entries 6 and 7 (the
    /// Side Show Assistant's) leave the palm unbound, and so does a bone
    /// the skeleton lacks or a point name the original does not know.
    #[test]
    fn the_body_strings_bind_the_points() {
        use crate::rig::Pose;
        let mut skeleton = crate::rig::tests::skeleton();
        skeleton.bones[0].name = "PELVIS".into();
        skeleton.bones[1].name = "R_HAND".into();
        let bones = Pose::rest(&skeleton).world(&skeleton, Transform::IDENTITY);
        let rows = |e4: &str, e5: &str, e6: &str, e7: &str| -> Vec<String> {
            ["a", "b", "c", e4, e5, e6, e7]
                .into_iter()
                .map(String::from)
                .collect()
        };
        let hand = bones[1].point(Vec3::new(0.5, 0.0, 0.0));
        let points = attach(&skeleton, &bones, &rows("HEAD", "Top", "R_HAND", "Palm"));
        assert_eq!(points.palm, Some(hand));
        assert_eq!(points.head_top, None, "no HEAD bone");
        assert_eq!(points.pelvis, Some(bones[0].point(Vec3::ZERO)));
        let side_show = attach(&skeleton, &bones, &rows("HEAD", "Top", "", ""));
        assert_eq!(side_show.palm, None);
        assert!(side_show.pelvis.is_some());
        let head = attach(&skeleton, &bones, &rows("R_HAND", "", "R_HAND", "Elbow"));
        assert_eq!(head.head_top, Some(bones[1].point(Vec3::ZERO)));
        assert_eq!(head.palm, None);
        assert_eq!(attach(&skeleton, &bones, &[]).palm, None);
        // Names bind in their exact case only.
        let lower = attach(&skeleton, &bones, &rows("HEAD", "Top", "r_hand", "Palm"));
        assert_eq!(lower.palm, None);
    }
}
