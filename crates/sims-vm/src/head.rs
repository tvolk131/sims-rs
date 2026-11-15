//! Where a person's head looks, as the animator's tick and its draw keep
//! it: the head's step (0x590768), the measurement of the two headings
//! (0x5904d2) and the turn taken off again (0x590b27). The arithmetic is
//! in [`sims_sim::head`]; this gathers what it reads.
//!
//! The step (0x590768), with the animator's clock not yet seen by it
//! (+0x164):
//!
//! 1. Person word 45 goes down by 1 when above 0; reaching 0 sets word 42
//!    to 8.
//! 2. With bit 0 of word 42 set the turn moves towards the target
//!    ([`sims_sim::head::Head::follow`]); a turn stopped at 1.5 either way
//!    sets word 42 to 5 when word 44 is 0, or to 4 when it is 1. With bit
//!    0 clear the turn shrinks ([`sims_sim::head::Head::relax`]).
//!
//! Then, at every call: a turn under 0.01 either way becomes 0 and the
//! step ends ([`sims_sim::head::Head::settle`]). Any other turn turns
//! the bone "HEAD" (entry 31 of the skin's list, 0x59910a) relative to
//! its parent ([`sims_sim::head::turned`]) and places that bone again.
//!
//! The measurement (0x5904d2) runs when the animator has its head anchors
//! (+0x144 and +0x14c, bound from the type's body strings, 0x5924a4) and
//! bit 0 of word 42 is set:
//!
//! 1. With person word 41 not 0, the object it names is looked up
//!    (0x40743b). One that is missing clears word 41 and sets word 42 to
//!    0x10. For a person, the target becomes the point on top of its head
//!    (0x59293d with 1, its +0x144 anchor). For any other object the
//!    target is its position in world units, at height 0 (0x59058f,
//!    0x454f5c): native +0xf4 (our second coordinate) first, then +0xf8
//!    (our first), each times 3/16.
//!    Then a target on another level than the person (object +0x110)
//!    clears word 41 and sets word 42 to 0x10.
//! 2. The head's heading: from the point on top of the head to the point
//!    100 units in front of it ([`sims_sim::head::heading`]).
//! 3. The target's heading: from the point on top of the head to the
//!    target.
//!
//! The original then takes the turn off the bone again (0x590b27): it
//! puts back the rotation it saved (+0x154) and places the bone again.
//! Here the pose is never kept: it is worked out from the layers when it
//! is needed ([`Interpreter::posed`]), so there is nothing to put back.
//!
//! The pose, as the skin's update left it (0x5992b3, 0x597c4c): each
//! layer's motions sampled at its phase, by priority (0x5991fd): the base
//! layer (-10, kept in [`World::base_layers`]) first, then the foreground
//! and the carry layer in the order the skin's list holds them. Each
//! layer at weight 1 or more replaces what the bones held with its
//! motions; one with nothing bound, no frames or no weight samples
//! nothing, and one below weight 1, which blends (0x597c4c), is not
//! ported. The skin does not put the bones back at rest first, so a bone
//! on the way from the root to a bone measured or turned whose rotation
//! no layer playing now writes holds what an earlier layer left, which is
//! not kept and fails; a translation no layer writes is taken at rest,
//! which is unverified. The bones
//! are then placed from the root (0x5994c3), which stands at the origin.
//!
//! The person's model matrix (0x592132) turns the skeleton to the
//! person's direction and moves it to its position and the terrain's
//! height there ([`Interpreter::model`]).

use crate::fault::Trap;
use crate::interpreter::Interpreter;
use crate::routing::slot::to_world_units;
use sims_core::{Ext, F32};
use sims_sim::World;
use sims_sim::animator::Animator;
use sims_sim::head::{self, Anchor, Matrix};
use sims_sim::pose;

/// Person data words.
const TARGET: usize = 41;
const HEAD_MODE: usize = 42;
const AT_LIMIT: usize = 44;
const HEAD_COUNTDOWN: usize = 45;
/// The bone the turn moves: entry 31 of the skin's list of bones
/// (0x5908d4, 0x59910a). That list follows the game's table of bone names
/// (0x6d1aa0), whose entry 31 is "HEAD", not the skeleton file's order
/// (legacy/crates/sims-sim/src/head_look/rig.rs; the table is filled when
/// the game starts and was not read here).
const TURNED_BONE: &str = "HEAD";
/// The lot array of the terrain's corner heights (Arry 0; the altitude
/// array, legacy/crates/sims-sim/src/native_world/arrays.rs).
const ALTITUDE: u16 = 0;
/// The size of the lot's arrays, whatever the lot's size.
const FULL: usize = 64;

/// One of a person's anchors: a bone, by place in the skeleton, and the
/// named point on it.
type Bound = (usize, Anchor);

fn unsupported(at: &str, what: &str) -> Trap {
    Trap::Unsupported(format!("{what} ({at}), which is not ported"))
}

fn person_data(world: &mut World, id: u16) -> Result<&mut Vec<i16>, Trap> {
    Ok(&mut world
        .objects
        .get_mut(&id)
        .and_then(|o| o.person.as_mut())
        .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?
        .data)
}

fn word(data: &[i16], index: usize) -> i16 {
    data.get(index).copied().unwrap_or(0)
}

fn set(data: &mut [i16], index: usize, value: i16) -> Result<(), Trap> {
    *data
        .get_mut(index)
        .ok_or_else(|| Trap::Invalid(format!("person data has no word {index}")))? = value;
    Ok(())
}

impl Interpreter<'_> {
    /// The head's step, measurement and restoration for person `id`,
    /// whose animator is `a` (the module documentation). The animator's
    /// tick calls it at the end, and so does the animator's draw.
    pub(crate) fn look(&self, world: &mut World, id: u16, a: &mut Animator) -> Result<(), Trap> {
        // 0x590768: the step, once per clock.
        if a.head_clock != a.clock {
            let data = person_data(world, id)?;
            let countdown = word(data, HEAD_COUNTDOWN);
            if countdown > 0 {
                set(data, HEAD_COUNTDOWN, countdown - 1)?;
                if countdown == 1 {
                    set(data, HEAD_MODE, 8)?;
                }
            }
            if word(data, HEAD_MODE) & 1 != 0 {
                if a.head.follow() {
                    match word(data, AT_LIMIT) {
                        0 => set(data, HEAD_MODE, 5)?,
                        1 => set(data, HEAD_MODE, 4)?,
                        _ => {}
                    }
                }
            } else {
                a.head.relax();
            }
            a.head_clock = a.clock;
        }
        let turned = a.head.settle().then_some(a.head.turn);

        // 0x5904d2: the measurement. The original checks the anchors
        // first, then bit 0; both are only reads, so the order does not
        // matter, and checking bit 0 first leaves the anchors unread (and
        // the person type's profile unneeded) while the head is not set
        // to follow.
        if word(person_data(world, id)?, HEAD_MODE) & 1 == 0 {
            return Ok(());
        }
        let rig = self.anchors(world, id)?;
        let (Some(top), Some(front)) = (rig.top, rig.front) else {
            return Ok(());
        };
        let target = word(person_data(world, id)?, TARGET);
        if target != 0 {
            let found = u16::try_from(target)
                .ok()
                .and_then(|t| world.objects.get(&t).map(|o| (t, o)));
            match found {
                None => {
                    let data = person_data(world, id)?;
                    set(data, TARGET, 0)?;
                    set(data, HEAD_MODE, 0x10)?;
                }
                Some((t, object)) => {
                    if object.person.is_none() {
                        // 0x59058f: the object's position in world units
                        // (0x454f5c, with height 0).
                        a.head.target = Some(world_point(object.position));
                    } else if t == id {
                        return Err(Trap::Unsupported(format!(
                            "person {id} looks at itself (0x59293d on its own animator, with \
                             its head turned): not ported"
                        )));
                    }
                    let level = World::tile(object.position)[2];
                    if object.person.is_some()
                        && let Some(point) = self.top_of(world, t)?
                    {
                        a.head.target = Some(point);
                    }
                    let own = world
                        .objects
                        .get(&id)
                        .map(|o| World::tile(o.position)[2])
                        .ok_or_else(|| Trap::native(10, "missing object"))?;
                    if level != own {
                        let data = person_data(world, id)?;
                        set(data, TARGET, 0)?;
                        set(data, HEAD_MODE, 0x10)?;
                    }
                }
            }
        }
        let bones = self.posed(world, id, a, &[top.0, front.0], turned)?;
        let model = self.model(world, id, a)?;
        let at = head::anchor_point(bones[top.0], top.1, &model);
        let ahead = head::anchor_point(bones[front.0], front.1, &model);
        if let Some(heading) = head::heading(
            head::difference(ahead[0], at[0]),
            head::difference(ahead[2], at[2]),
        ) {
            a.head.heading = heading;
        }
        let target = a.head.target.ok_or_else(|| {
            Trap::Unsupported(format!(
                "person {id}'s head measures its target before any target was written \
                 (animator +0x21c, 0x5904d2): the original reads what memory held"
            ))
        })?;
        if let Some(heading) = head::heading(
            head::difference(target[0], at[0]),
            head::difference(target[2], at[2]),
        ) {
            a.head.target_heading = heading;
        }
        Ok(())
    }

    /// The point on top of person `id`'s head in the lot (0x59293d with
    /// 1: its +0x144 anchor, 0x59640d), from its animator as the world
    /// holds it; None when it has no such anchor.
    ///
    /// A person queued for removal fails: it left the lot without its
    /// animator's refresh, which the original makes only if the view drew
    /// it (`Interpreter::take_off_to_remove`, 0x58da12), so its pose may
    /// differ from the original's.
    fn top_of(&self, world: &World, id: u16) -> Result<Option<[F32; 3]>, Trap> {
        if world.removals.contains(&id) {
            return Err(Trap::Unsupported(format!(
                "a head looks at person {id}, which is queued for removal: its animator's \
                 refresh as it left the lot (0x58da12) depends on whether the view drew it \
                 (animator byte +0x109), which is not kept"
            )));
        }
        let Some(top) = self.anchors(world, id)?.top else {
            return Ok(None);
        };
        let a = world.animators.get(&id).ok_or_else(|| {
            Trap::Unsupported(format!(
                "person {id} is looked at before it has an animator: not ported"
            ))
        })?;
        let bones = self.posed(world, id, a, &[top.0], None)?;
        let model = self.model(world, id, a)?;
        Ok(Some(head::anchor_point(bones[top.0], top.1, &model)))
    }

    /// Person `id`'s head anchors (0x5924a4): +0x144, the body strings'
    /// bone 3 at their point 4 (counting from 0), and +0x14c, the same
    /// bone at "HeadFake"; neither when bone 3 is empty. An empty or
    /// absent point 4 passes no name, so +0x144 is the bone's own origin
    /// (0x5a199b binds the identity, 0x6fe720).
    fn anchors(&self, world: &World, id: u16) -> Result<Rig, Trap> {
        let guid = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::native(10, "missing object"))?
            .guid;
        let profile = self.program.people.profiles.get(&guid).ok_or_else(|| {
            Trap::Unsupported(format!(
                "no profile was supplied for the person type 0x{guid:08X}"
            ))
        })?;
        let [bone, point, _, _] = &profile.rig;
        let Some(bone) = bone.as_deref().filter(|b| !b.is_empty()) else {
            return Ok(Rig {
                top: None,
                front: None,
            });
        };
        let place = self
            .skeleton(guid)?
            .bones
            .iter()
            .position(|b| b.name == bone)
            .ok_or_else(|| {
                unsupported(
                    "0x5924a4",
                    &format!("binds an anchor to a bone {bone:?} its skeleton does not have"),
                )
            })?;
        let named = |name: &str| {
            Anchor::named(name).ok_or_else(|| {
                unsupported("0x5a1631", &format!("binds an anchor of the name {name:?}"))
            })
        };
        let top = match point.as_deref().filter(|p| !p.is_empty()) {
            Some(name) => Some((place, named(name)?)),
            None => Some((place, Anchor::Origin)),
        };
        Ok(Rig {
            top,
            front: Some((place, Anchor::HeadFake)),
        })
    }

    fn skeleton(&self, guid: u32) -> Result<&sims_content::Skeleton, Trap> {
        self.program.people.skeletons.get(&guid).ok_or_else(|| {
            Trap::Unsupported(format!(
                "no skeleton was supplied for the person type 0x{guid:08X}"
            ))
        })
    }

    /// Person `id`'s bones placed relative to the skeleton's root, as the
    /// skin's last update left them (the module documentation), with
    /// the bone "HEAD" turned by `turn` when given. `needed` are the bones
    /// the caller reads; the rotation of each bone on the way from the
    /// root to each, and to the turned bone, must be one a layer playing
    /// now writes.
    fn posed(
        &self,
        world: &World,
        id: u16,
        a: &Animator,
        needed: &[usize],
        turn: Option<F32>,
    ) -> Result<Vec<pose::Bone>, Trap> {
        let guid = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::native(10, "missing object"))?
            .guid;
        let skeleton = self.skeleton(guid)?;
        // A carry clip chosen whose layer has not started yet is not
        // ported.
        if a.carry_selection != -1 && a.carry.is_none() {
            return Err(unsupported(
                "0x5992b3",
                &format!("person {id}'s pose with a carry clip but no carry layer"),
            ));
        }
        let parents: Vec<Option<usize>> = skeleton.bones.iter().map(|b| b.parent).collect();
        let place = |bone: &str| skeleton.bones.iter().position(|b| b.name == bone);
        let turned = match turn {
            Some(turn) => {
                let bone = place(TURNED_BONE).ok_or_else(|| {
                    unsupported(
                        "0x59910a",
                        &format!("person {id}'s skeleton has no bone {TURNED_BONE:?} to turn"),
                    )
                })?;
                Some((bone, turn))
            }
            None => None,
        };

        // The bones from the root to each needed one.
        let mut chain = vec![false; parents.len()];
        for &bone in needed.iter().chain(turned.as_ref().map(|(b, _)| b)) {
            let mut at = Some(bone);
            let mut steps = 0;
            while let Some(b) = at {
                let seen = chain.get_mut(b).ok_or_else(|| {
                    Trap::Invalid(format!("person {id}'s skeleton has no bone {b}"))
                })?;
                *seen = true;
                at = parents[b];
                steps += 1;
                if steps > parents.len() {
                    return Err(Trap::Invalid("a skeleton's bones form a loop".into()));
                }
            }
        }

        // The skin's layers in the order its update samples them: by
        // priority, the base layer (-10) first (0x5991fd).
        let base = if a.base.is_filled() {
            Some(world.base_layers.get(&id).ok_or_else(|| {
                Trap::Invalid(format!(
                    "person {id}'s base slot is filled but has no layer"
                ))
            })?)
        } else {
            None
        };
        let (first, second) = if crate::animator::carry_first(a) {
            (a.carry.as_ref(), a.foreground.as_ref())
        } else {
            (a.foreground.as_ref(), a.carry.as_ref())
        };
        let layers = [base, first, second];

        let bad = |e: pose::Error| unsupported("0x5992b3", &format!("cannot pose: {}", e.0));
        let mut locals: Vec<pose::Bone> = skeleton
            .bones
            .iter()
            .map(|b| pose::Bone {
                translation: b.translation.map(F32::from_bits),
                rotation: b.rotation.map(F32::from_bits),
            })
            .collect();
        let mut rotated = vec![false; parents.len()];
        for layer in layers.into_iter().flatten() {
            if a.skin_dirty || layer.current != a.skin_clock {
                return Err(Trap::Unsupported(format!(
                    "person {id}'s pose is read while its skin waits for its update (0x5992b3)"
                )));
            }
            // 0x597c4c samples nothing for a layer with nothing bound, no
            // frames or no weight.
            if !layer.bound || layer.frames == 0 || layer.weight.ext() <= Ext::ZERO {
                continue;
            }
            if layer.weight.ext() < Ext::from_i32(1) {
                return Err(unsupported(
                    "0x597c4c",
                    &format!(
                        "person {id}'s pose with its layer of {:?} below weight 1: the blend \
                         with what the bones held",
                        layer.clip
                    ),
                ));
            }
            let clip = self.program.animations.clip(&layer.clip).map_err(|e| {
                unsupported("0x59dc32", &format!("finds no clip {:?}: {e}", layer.clip))
            })?;
            let samples =
                self.program.animations.samples(clip).map_err(|e| {
                    unsupported("0x5992b3", &format!("cannot read its samples: {e}"))
                })?;
            let translations: Vec<[F32; 3]> = samples
                .translations
                .iter()
                .map(|v| v.map(F32::from_f32))
                .collect();
            let rotations: Vec<[F32; 4]> = samples
                .rotations
                .iter()
                .map(|v| v.map(F32::from_f32))
                .collect();
            let elapsed = Ext::from_i32(layer.current.wrapping_sub(layer.previous))
                .single()
                .to_f32();
            let at = pose::Selection::at(
                layer.phase,
                layer.frames,
                layer.duration,
                elapsed,
                layer.mode == 1,
            )
            .map_err(bad)?;
            for channel in &clip.channels {
                let Some(bone) = place(&channel.bone) else {
                    continue;
                };
                let from = |start: Option<u32>, len: usize| {
                    start.map(|s| s as usize).filter(|&s| s <= len)
                };
                let motion = pose::Motion {
                    translations: from(channel.translation, translations.len())
                        .map(|s| &translations[s..]),
                    rotations: from(channel.rotation, rotations.len()).map(|s| &rotations[s..]),
                };
                if motion.rotations.is_some() {
                    rotated[bone] = true;
                }
                pose::sample(&mut locals[bone], motion, at).map_err(bad)?;
            }
        }
        // 0x5992b3 does not put the bones back at rest before it samples:
        // a rotation no layer playing now writes is what the last layer
        // that wrote it left, which is not kept.
        if let Some(bone) = (0..parents.len()).find(|&b| chain[b] && !rotated[b]) {
            return Err(unsupported(
                "0x5992b3",
                &format!(
                    "person {id}'s bone {:?}, on the way to a bone read, is turned by no layer \
                     playing now: its rotation is the last layer's that turned it",
                    skeleton.bones[bone].name
                ),
            ));
        }
        let mut placed = pose::absolute(&parents, &locals).map_err(bad)?;
        if let Some((turned, turn)) = turned {
            // 0x590a3c: only the turned bone is placed again (0x59615b).
            if parents.contains(&Some(turned)) {
                return Err(unsupported(
                    "0x5a15de",
                    &format!("person {id}'s turned head bone has bones hanging from it"),
                ));
            }
            let mut bone = locals[turned];
            bone.rotation = head::turned(bone.rotation, turn);
            let parent = parents[turned].map_or(pose::ORIGIN, |p| placed[p]);
            placed[turned] = pose::compose(bone, parent);
        }
        Ok(placed)
    }

    /// Person `id`'s model matrix (0x592132; [`head::placement`]) on the
    /// ground level. The terrain's height is that at the position rounded
    /// to the nearest 16th (0x478dfe; [`head::altitude_at`]): the tile is
    /// the rounded position shifted right by 4, the offsets in it the low
    /// 4 bits.
    ///
    /// The original builds the point with the rounded second coordinate
    /// first (0x592234, 0x592242). It finds the cell through a table of
    /// rows indexed by the point's second word, which is the first
    /// coordinate's tile, so the cell is at the first coordinate's tile
    /// times 64 plus the second's, as the pool test reads it
    /// ([`crate::placement`]). The offsets go the same way: the first one
    /// [`head::altitude_at`] takes is the second coordinate's. The capture
    /// pins both. At tick 334 person 16 looks at person 259, whose tile
    /// has corners of height 3 that way and 6 the other way, and the
    /// target's height the original stored is the one of 3. From tick 356
    /// on person 259 walks over a sloped tile, and the heights the
    /// original stored fit this order of the offsets and of the corner
    /// bytes, and no other, at each of eleven points.
    fn model(&self, world: &World, id: u16, a: &Animator) -> Result<Matrix, Trap> {
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::native(10, "missing object"))?;
        let level = World::tile(object.position)[2];
        if level != 1 {
            return Err(unsupported(
                "0x5922ad",
                &format!("person {id}'s model matrix on level {level}"),
            ));
        }
        let half = Ext::from_f64_bits(0x3fe0_0000_0000_0000);
        let round = |v: F32| (v.ext() + half).single().to_i32_truncate();
        let point = [round(a.position[0]), round(a.position[1])];
        let size = i32::from(world.size);
        let [first, second] = point.map(|p| (p >> 4) as i8 as i32);
        if !(0..size).contains(&first) || !(0..size).contains(&second) {
            return Err(unsupported(
                "0x45ee1d",
                &format!("person {id}'s model matrix off the lot"),
            ));
        }
        let altitude = world
            .arrays
            .get(&ALTITUDE)
            .filter(|a| a.stride == 4 && a.data.len() == FULL * FULL * 4)
            .ok_or_else(|| Trap::Invalid("the lot's altitude array is missing".into()))?;
        let at = (first as usize * FULL + second as usize) * 4;
        let corners = [0, 1, 2, 3].map(|i| altitude.data[at + i]);
        // 0x592242 hands 0x478dfe the rounded second coordinate, then the
        // rounded first: its first offset is the second coordinate's.
        let height = head::terrain_height(head::altitude_at(corners, point[1] & 15, point[0] & 15));
        Ok(head::placement(a.position, height))
    }
}

/// A person's head anchors: +0x144, the point on top of the head, and
/// +0x14c, the point 100 units in front of it.
struct Rig {
    top: Option<Bound>,
    front: Option<Bound>,
}

/// A position in the lot as a point in world units (0x454f5c), as the
/// head's measurement passes it (0x59058f): the input is native +0xf4 then
/// +0xf8, which are our second coordinate then our first. So the point is
/// the second coordinate times 3/16, the height 0 (0x454f29 of 0, times
/// 3), then the first coordinate times 3/16, each stored as a float.
fn world_point(position: [i32; 3]) -> [F32; 3] {
    [
        to_world_units(Ext::from_i32(position[1])).to_f32(),
        F32::from_bits(0),
        to_world_units(Ext::from_i32(position[0])).to_f32(),
    ]
}
