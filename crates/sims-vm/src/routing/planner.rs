//! The planner (0x4823fa): from a route context's candidates to the
//! person's path.
//!
//! 1. The person's room is the room of the slot's object. A free
//!    rectangle found while searching costs 3 in a room outdoors on the
//!    ground, else 0.
//! 2. The obstacles of the objects in the room (0x482599 to 0x4827b9):
//!    each object on the lot's list, but the person, one in another room
//!    and one [`Interpreter::ignored`] skips, gives its footprint as a
//!    rectangle, hard unless its data word 10 is set (then cost 10). One
//!    that overlaps the person's own footprint is left out (Wine's
//!    IntersectRect). The others are grown by 3 when they are people
//!    (type 2) or cost 8 or more, which is all of them.
//! 3. The room's own rectangles ([`Interpreter::room_rects`]).
//! 4. The goals (0x482861): each candidate above score 0 as a one-unit
//!    rectangle at its position, costing 16 for each point below the best
//!    score.
//! 5. The search (0x482a31 to 0x482b2a): a room of at least 100 tiles
//!    whose objects are fewer than a fifth of its half tiles searches
//!    with rectangles at most 128 wide ([`bounded`]: the ratio is rounded
//!    to single precision, so exactly a fifth is too many); any other
//!    room without that limit.
//!    The step limit is the tiles times the goals plus 50, at most 1000.
//!    A room with the width limit and more than 256 tiles first searches
//!    without it. If that pass finds no path, the context's result is 3
//!    and the planner answers false; otherwise the pass is thrown away.
//! 6. The chosen candidate is the goal reached (context +0x50), and the
//!    path's points, coordinates swapped, are the person's path.
//!
//! A person with bit 0 of person word 84 or 68 set goes through
//! everything ([`through`]): no slot in another room fails it, and it has
//! no obstacles in steps 2 and 3, and the width limit in step 5.
//! **Unverified**: no capture covers it; the first is the Goth house's
//! tick 30321 (person 36, made at tick 30320 by object 243).
//!
//! Ported from the original's code, with the search policy of the old
//! prototype (legacy/crates/sims-sim/src/lot_route/search.rs:
//! `Config::prepare`, `Config::steps`, `Config::run`, `goals`;
//! lot_route/geometry.rs: `dynamic`). Every branch the Goth house's first
//! route does not take fails.

use super::regions::{HARD, Region};
use super::search::{Query, search};
use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::placement::capture;
use sims_core::Ext;
use sims_sim::World;
use sims_sim::route::{Candidate, Context};

/// Object data words.
const FLAGS: usize = 8;
const COST: usize = 10;
/// Set, the planner leaves out what the ground of an outdoor room on
/// level 1 costs: free rectangles cost 0 and the room's terrain is not
/// read (0x48245e, 0x482505, 0x4827f6).
const PACE: usize = 17;
const ROOM: usize = 29;
const WALL_MASK: usize = 53;
const DIRECTION: usize = 1;
/// Person data words: bit 0 of either makes the person go through
/// everything ([`through`]).
const NO_ROOM: usize = 68;
const CARRIED: usize = 84;

/// The context's result when the first, unbounded search finds no path.
const NO_WAY: i32 = 3;

/// The function an object is asked whether a person may pass it.
const ALLOW_OVERLAP: usize = 5;
/// Object types.
const PERSON: i16 = 2;
const DOOR: i16 = 8;

/// Half the lot's person padding (0x6c297c, 6).
const PADDING: i32 = 3;
/// The distance below which another person is in the way (0x666ccc).
const NEAR: i32 = 3;
/// The density of objects above which the search is not bounded (a
/// double, 0x667ac8).
const DENSE: u64 = 0x3fc9_9999_9999_999a;

fn datum(world: &World, id: u16, index: usize) -> Result<i16, Trap> {
    world
        .objects
        .get(&id)
        .ok_or_else(|| Trap::native(10, "missing object"))?
        .data
        .get(index)
        .copied()
        .ok_or_else(|| Trap::Invalid(format!("object data has no word {index}")))
}

fn unsupported(what: &str) -> Trap {
    Trap::Unsupported(format!(
        "the planner (0x4823fa): {what}, which is not ported"
    ))
}

/// An object's obstacle (0x482599 to 0x4827b9): its rectangle (initially its
/// footprint, or replaced by a door edge strip at `0x482625..0x482758`),
/// left out when it overlaps the person's footprint (Wine's IntersectRect,
/// `0x48276f`), and grown by [`PADDING`] when the object is a person or the
/// cost is 8 or more (`0x482779..0x48279f`).
fn obstacle(rect: Region, cost: i32, kind: i16, actor: [i32; 4]) -> Option<Region> {
    let a = rect.0;
    let b = actor;
    if a[0].max(b[0]) < a[2].min(b[2]) && a[1].max(b[1]) < a[3].min(b[3]) {
        return None;
    }
    Some(if kind == PERSON || cost >= 8 {
        rect.inflate(PADDING)
    } else {
        rect
    })
}

/// Pre-check for [`Interpreter::ignored`]: handles all fast checks (never
/// mutates `world.order`) and tells [`Interpreter::plan`] when an
/// `ALLOW_OVERLAP` callback is needed.
enum IgnoreCheck {
    Ignored,
    Obstacle,
    CheckOverlap,
}

impl Interpreter<'_> {
    fn check_ignored_fast(
        &self,
        world: &mut World,
        ctx: &Context,
        id: u16,
    ) -> Result<IgnoreCheck, Fault> {
        let fail = |trap| Fault {
            object: ctx.person,
            at: None,
            trap,
        };
        if capture(world, id).map_err(fail)?.container.is_some() {
            return Ok(IgnoreCheck::Ignored);
        }
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        let flags = object
            .data
            .get(FLAGS)
            .copied()
            .ok_or_else(|| fail(Trap::Invalid(format!("object data has no word {FLAGS}"))))?
            as u16;
        let passable = if flags & 4 != 0 {
            flags & 2 == 0
        } else {
            flags & 0x10 != 0
        };
        if passable {
            let cost =
                object.data.get(COST).copied().ok_or_else(|| {
                    fail(Trap::Invalid(format!("object data has no word {COST}")))
                })?;
            if cost == 0 {
                return Ok(IgnoreCheck::Ignored);
            }
        }
        if self.kind(world, id).map_err(fail)? != PERSON {
            return Ok(IgnoreCheck::CheckOverlap);
        }
        if object.person.is_none() {
            return Err(fail(unsupported(
                "an object of the person type that is not a person",
            )));
        }
        if ctx.objects[0] == id {
            return Ok(IgnoreCheck::Ignored);
        }
        let from = world
            .objects
            .get(&ctx.person)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?
            .position;
        let distance = crate::primitives::distance(from, object.position, false);
        // 0x48188a: a person within 3 tiles (or at a distance that is not
        // a number) is in the way.
        Ok(if distance > Ext::from_i32(NEAR) {
            IgnoreCheck::Ignored
        } else {
            IgnoreCheck::Obstacle
        })
    }

    /// Whether the planner leaves an object out of its obstacles
    /// (0x4817e9): one in a container; one a person may pass by its data
    /// word 8 (with bit 2, when bit 1 is clear; else when bit 4 is set)
    /// and whose data word 10 is 0; another person farther than 3 tiles
    /// (one nearer is an obstacle),
    /// or the one the context names; and any other object that answers
    /// its function 5 true with the person as the stack object.
    pub(crate) fn ignored(&self, world: &mut World, ctx: &Context, id: u16) -> Result<bool, Fault> {
        match self.check_ignored_fast(world, ctx, id)? {
            IgnoreCheck::Ignored => Ok(true),
            IgnoreCheck::Obstacle => Ok(false),
            IgnoreCheck::CheckOverlap => self.callback(world, id, ALLOW_OVERLAP, ctx.person as i16),
        }
    }

    /// Plans the route of a context: fills `path` and the context's
    /// chosen candidate, and returns whether a path was found.
    pub(crate) fn plan(
        &self,
        world: &mut World,
        ctx: &mut Context,
        path: &mut Vec<[i32; 2]>,
    ) -> Result<bool, Fault> {
        let person = ctx.person;
        let fail = |trap| Fault {
            object: person,
            at: None,
            trap,
        };
        // What comes before this in the original only sets flags for later.
        if gives_up(&ctx.candidates, &mut ctx.result) {
            return Ok(false);
        }
        let me = world
            .objects
            .get(&person)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        let words = me
            .person
            .as_ref()
            .ok_or_else(|| fail(unsupported("a route for an object that is not a person")))?;
        let through = through(&words.data);
        // 0x482458: data word 17 set, the ground is not costed.
        let ground = me
            .data
            .get(PACE)
            .copied()
            .ok_or_else(|| fail(Trap::Invalid(format!("object data has no word {PACE}"))))?
            == 0;
        let room = me
            .data
            .get(ROOM)
            .copied()
            .ok_or_else(|| fail(Trap::Invalid(format!("object data has no word {ROOM}"))))?;
        let actor = me.footprint;
        if !through && datum(world, ctx.target, ROOM).map_err(fail)? != room {
            return Err(fail(unsupported("a slot in another room than the person")));
        }
        let (outdoors, half, objects) = match world.rooms.rooms.get(&room) {
            Some(r) => (r.outdoors, r.half_tiles, r.objects),
            None => return Err(fail(unsupported("a person in no room"))),
        };
        let free_cost = free_cost(room, outdoors, ground);

        // The objects' obstacles; none for a person going through
        // everything (0x48258f). Borrow `&world.order` until the first object
        // actually requires an `ALLOW_OVERLAP` callback, then snapshot only
        // the remaining suffix so semantics remain identical to an upfront
        // clone even if a callback mutates `world.order`.
        let mut dynamic = Vec::new();
        if !through {
            let mut idx = 0usize;
            let mut suffix: Option<Vec<u16>> = None;
            loop {
                let id = if let Some(rest) = suffix.as_ref() {
                    if idx >= rest.len() {
                        break;
                    }
                    let id = rest[idx];
                    idx += 1;
                    id
                } else {
                    if idx >= world.order.len() {
                        break;
                    }
                    let id = world.order[idx];
                    idx += 1;
                    id
                };
                if id == person {
                    continue;
                }
                let Some(object) = world.objects.get(&id) else {
                    continue;
                };
                let obj_room = object.data.get(ROOM).copied().ok_or_else(|| {
                    fail(Trap::Invalid(format!("object data has no word {ROOM}")))
                })?;
                if obj_room != room {
                    continue;
                }
                match self.check_ignored_fast(world, ctx, id)? {
                    IgnoreCheck::Ignored => continue,
                    IgnoreCheck::Obstacle => {}
                    IgnoreCheck::CheckOverlap => {
                        if suffix.is_none() {
                            suffix = Some(world.order[idx..].to_vec());
                            idx = 0;
                        }
                        if self.callback(world, id, ALLOW_OVERLAP, ctx.person as i16)? {
                            continue;
                        }
                    }
                }
                let object = world
                    .objects
                    .get(&id)
                    .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
                let fp = object.footprint;
                let cost_word = object.data.get(COST).copied().ok_or_else(|| {
                    fail(Trap::Invalid(format!("object data has no word {COST}")))
                })?;
                let mut cost = if cost_word != 0 { 10 } else { HARD };
                let mut rect = Region([fp[0], fp[1], fp[2], fp[3], cost]);
                let kind = self.kind(world, id).map_err(fail)?;
                if kind == DOOR {
                    // 0x4359ce: the door's walls, turned by its direction.
                    // 0x482620..0x482758: each matching wall edge bit replaces
                    // the obstacle rectangle (`[ebp - 0x58]`) with a 1-unit strip
                    // along that footprint edge at cost 10000 and inflates it by
                    // `PADDING` (3); `0x48276f` and `0x48279f` then run on the
                    // resulting rectangle and cost.
                    let mask = i32::from(object.data.get(WALL_MASK).copied().ok_or_else(|| {
                        fail(Trap::Invalid(format!(
                            "object data has no word {WALL_MASK}"
                        )))
                    })?);
                    if mask != 0 {
                        let dir = (object.data.get(DIRECTION).copied().ok_or_else(|| {
                            fail(Trap::Invalid(format!(
                                "object data has no word {DIRECTION}"
                            )))
                        })? as u8
                            & 31) as u32;
                        let turned = (mask as u32).wrapping_shl(dir);
                        let bits = (turned & 0xff) | ((turned >> 8) & 0xff);
                        const DOOR_EDGE_COST: i32 = 10_000;
                        if bits & 0x01 != 0 {
                            cost = DOOR_EDGE_COST;
                            rect = Region([fp[0], fp[1], fp[2], fp[1].wrapping_add(1), cost])
                                .inflate(PADDING);
                        }
                        if bits & 0x04 != 0 {
                            cost = DOOR_EDGE_COST;
                            rect = Region([fp[2].wrapping_sub(1), fp[1], fp[2], fp[3], cost])
                                .inflate(PADDING);
                        }
                        if bits & 0x10 != 0 {
                            cost = DOOR_EDGE_COST;
                            rect = Region([fp[0], fp[3].wrapping_sub(1), fp[2], fp[3], cost])
                                .inflate(PADDING);
                        }
                        if bits & 0x40 != 0 {
                            cost = DOOR_EDGE_COST;
                            rect = Region([fp[0], fp[1], fp[0].wrapping_add(1), fp[3], cost])
                                .inflate(PADDING);
                        }
                    }
                }
                dynamic.extend(obstacle(rect, cost, kind, actor));
            }
        }

        // 0x4827f6: the room's own rectangles (0x46795a, with terrain), or,
        // for an outdoor room on level 1 with the ground not costed, the
        // room's rectangles made again without terrain (0x4818af with 0).
        // A person going through everything has none (0x4827e3: the
        // list stays null, which the search reads as empty, 0x469733).
        let terrain = ground || !(super::rooms::room_level(room) == 1 && outdoors);
        let fixed = if through {
            Vec::new()
        } else {
            self.room_rects(world, room, terrain).map_err(fail)?
        };

        // The goals.
        let mut goals = Vec::new();
        let mut map = Vec::new();
        for (index, c) in ctx.candidates.iter().enumerate() {
            if c.score > 0 {
                let [x, y] = c.position;
                goals.push(Region([
                    y,
                    x,
                    y.wrapping_add(1),
                    x.wrapping_add(1),
                    ctx.best.wrapping_sub(c.score).wrapping_mul(16),
                ]));
                map.push(index);
            }
        }

        let tiles = half / 2;
        // 0x4828ba: a person going through everything always keeps the
        // width limit.
        let bounded = through || bounded(objects, half);
        let limit = tiles
            .wrapping_mul(goals.len() as i32)
            .wrapping_add(50)
            .min(1000);
        let steps = if limit < 0 { 0 } else { limit as usize + 1 };
        let start = [ctx.start[1], ctx.start[0]];
        let query = |maximum| Query {
            start,
            goals: &goals,
            dynamic: &dynamic,
            fixed: &fixed,
            maximum,
            free_cost,
            steps,
        };
        if tiles > 256 && bounded {
            let first = search(query(None)).map_err(fail)?;
            if first.points.is_empty() {
                // 0x482abc to 0x482af5: no way; the context's result is 3,
                // and nothing else is chosen.
                ctx.result = NO_WAY;
                return Ok(false);
            }
        }
        let found = search(query(bounded.then_some(128))).map_err(fail)?;
        let Some(goal) = found.goal else {
            // 0x482b35: no goal reached, so no path. The chosen candidate
            // is forgotten (0x481745), and with no path the context's
            // result is 3 and the planner answers false (0x482e6d).
            forget_chosen(&mut ctx.candidates, &mut ctx.chosen, &mut ctx.best);
            ctx.result = NO_WAY;
            return Ok(false);
        };
        ctx.chosen = map[goal] as i32;
        if found.points.is_empty() {
            return Err(fail(unsupported("a search that finds no path")));
        }
        path.extend(found.points.iter().map(|p| [p[1], p[0]]));
        Ok(true)
    }
}

/// Whether a person goes through everything (0x482413 to 0x482451): bit
/// 0 of its person word 84 (+0x634) or 68 (+0x614). The planner then
/// takes the person's own room whatever the slot's (0x4824a0), leaves
/// out the objects' obstacles (0x48258f) and the room's own rectangles,
/// walls included (0x4827e3), and searches with the width limit
/// (0x4828ba). A person with no person words does not.
fn through(words: &[i16]) -> bool {
    let bit = |i: usize| words.get(i).is_some_and(|w| w & 1 != 0);
    bit(CARRIED) || bit(NO_ROOM)
}

/// 0x481745: a context with a chosen candidate forgets it: that
/// candidate's score (+8) becomes 0 when the index is one of the
/// candidates', and the chosen index and best score (+0x50, +0x54)
/// become -1. With none chosen (-1) nothing changes.
fn forget_chosen(candidates: &mut [Candidate], chosen: &mut i32, best: &mut i32) {
    if *chosen == -1 {
        return;
    }
    if let Some(candidate) = usize::try_from(*chosen)
        .ok()
        .and_then(|i| candidates.get_mut(i))
    {
        candidate.score = 0;
    }
    *chosen = -1;
    *best = -1;
}

/// What a free rectangle found while searching costs (0x4824d0 to
/// 0x48253e): 3 in a room outdoors on the ground when the ground is
/// costed (person data word 17 is 0), else 0.
fn free_cost(room: i16, outdoors: bool, ground: bool) -> i32 {
    if ground && super::rooms::room_level(room) == 1 && outdoors {
        3
    } else {
        0
    }
}

/// Whether the search keeps rectangles at most 128 wide (0x4828ba): not
/// in a room crowded with objects (0x468103: the objects over the half
/// tiles, rounded to single precision, above the double 0.2, so exactly
/// a fifth is crowded; a ratio that is not a number is not) or of fewer
/// than 100 tiles (0x46810a).
fn bounded(objects: i32, half: i32) -> bool {
    let density = (Ext::from_i32(objects) / Ext::from_i32(half)).single();
    let crowded = density > Ext::from_f64_bits(DENSE);
    !crowded && half / 2 >= 100
}

/// Whether the planner gives up at once (0x482462): with no candidates
/// there is no path, and a result of 1 (no place found yet) becomes 7.
fn gives_up(candidates: &[Candidate], result: &mut i32) -> bool {
    if !candidates.is_empty() {
        return false;
    }
    if *result == 1 {
        *result = 7;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_person_goes_through_everything_with_bit_0_of_person_word_84_or_68() {
        let with = |word: usize, value: i16| {
            let mut words = vec![0; 90];
            words[word] = value;
            through(&words)
        };
        assert!(with(84, 1));
        assert!(with(68, 1));
        assert!(with(68, -1));
        // Only bit 0 counts, and only those words.
        assert!(!with(84, 2));
        assert!(!with(68, 0x7ffe));
        assert!(!with(74, 1));
        assert!(!through(&[0; 90]));
        // A person with no person words that far does not.
        assert!(!through(&[1; 68]));
    }

    #[test]
    fn a_search_with_no_goal_forgets_the_chosen_candidate() {
        let place = |score| Candidate {
            position: [8, 8],
            score,
            word_c: 0,
            word_e: None,
        };
        let mut candidates = vec![place(5), place(7)];
        let (mut chosen, mut best) = (1, 7);
        forget_chosen(&mut candidates, &mut chosen, &mut best);
        assert_eq!((candidates[0].score, candidates[1].score), (5, 0));
        assert_eq!((chosen, best), (-1, -1));
        // An index past the candidates, or below 0 but not -1, is still
        // forgotten; no candidate changes.
        for index in [2, -2] {
            let (mut chosen, mut best) = (index, 7);
            let mut candidates = vec![place(5)];
            forget_chosen(&mut candidates, &mut chosen, &mut best);
            assert_eq!(candidates[0].score, 5);
            assert_eq!((chosen, best), (-1, -1));
        }
        // With none chosen, the best score stays.
        let (mut chosen, mut best) = (-1, 7);
        forget_chosen(&mut candidates, &mut chosen, &mut best);
        assert_eq!((chosen, best), (-1, 7));
    }

    #[test]
    fn with_no_candidates_the_planner_gives_up() {
        let mut result = 1;
        assert!(gives_up(&[], &mut result));
        assert_eq!(result, 7);
        let mut result = 4;
        assert!(gives_up(&[], &mut result));
        assert_eq!(result, 4);
        let one = Candidate {
            position: [8, 8],
            score: 1,
            word_c: 0,
            word_e: None,
        };
        let mut result = 1;
        assert!(!gives_up(&[one], &mut result));
        assert_eq!(result, 1);
    }

    #[test]
    fn an_obstacle_keeps_the_footprint_order_and_grows_when_costly() {
        let actor = [0, 0, 4, 4];
        // A hard obstacle (cost 8 or more) grows by 3 on each side; the
        // footprint's words stay in order.
        assert_eq!(
            obstacle(Region([10, 20, 30, 40, HARD]), HARD, 4, actor),
            Some(Region([7, 17, 33, 43, HARD]))
        );
        // A cheap one stays as it is, unless it is a person.
        assert_eq!(
            obstacle(Region([10, 20, 30, 40, 5]), 5, 4, actor),
            Some(Region([10, 20, 30, 40, 5]))
        );
        assert_eq!(
            obstacle(Region([10, 20, 30, 40, 5]), 5, PERSON, actor),
            Some(Region([7, 17, 33, 43, 5]))
        );
    }

    #[test]
    fn an_obstacle_over_the_person_is_left_out() {
        let actor = [8, 8, 12, 12];
        assert_eq!(
            obstacle(Region([10, 10, 20, 20, HARD]), HARD, 4, actor),
            None
        );
        // Touching edges do not overlap; growing comes after the test.
        assert_eq!(
            obstacle(Region([12, 0, 20, 20, HARD]), HARD, 4, actor),
            Some(Region([9, -3, 23, 23, HARD]))
        );
    }

    #[test]
    fn door_edge_strip_grows_twice_at_cost_10000_when_disjoint_from_actor() {
        // 0x482625..0x48279f: edge strip at cost 10000 gets InflateRect(3, 3)
        // when built, then a second InflateRect(3, 3) in obstacle() because
        // cost >= 8.
        let fp = [32, 48, 48, 64];
        let strip = Region([fp[0], fp[1], fp[2], fp[1] + 1, 10_000]).inflate(PADDING);
        assert_eq!(strip, Region([29, 45, 51, 52, 10_000]));
        assert_eq!(
            obstacle(strip, 10_000, DOOR, [0, 0, 4, 4]),
            Some(Region([26, 42, 54, 55, 10_000]))
        );
        // If the pre-inflated strip overlaps the actor, IntersectRect drops it.
        assert_eq!(obstacle(strip, 10_000, DOOR, [30, 46, 34, 50]), None);
    }

    #[test]
    fn a_free_rectangle_costs_3_only_outdoors_on_the_ground() {
        assert_eq!(free_cost(0, true, true), 3);
        assert_eq!(free_cost(0, true, false), 0);
        assert_eq!(free_cost(5, false, true), 0);
        // Room 1024 is on the second level.
        assert_eq!(free_cost(1024, true, true), 0);
    }

    #[test]
    fn a_small_or_crowded_room_is_searched_without_the_width_limit() {
        // 100 tiles (200 half tiles) with fewer than a fifth of them in
        // objects. Exactly a fifth is crowded: the ratio is rounded to
        // single precision, which is above the double 0.2 it is compared
        // with.
        assert!(bounded(39, 200));
        assert!(!bounded(40, 200));
        assert!(!bounded(0, 198));
        // A room of no tiles is unbounded; 0/0 is not crowded but the
        // room is too small.
        assert!(!bounded(0, 0));
        assert!(!bounded(1, 0));
    }
}
