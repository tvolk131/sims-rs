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
const PACE: usize = 17;
const ROOM: usize = 29;
const WALL_MASK: usize = 53;
const DIRECTION: usize = 1;
/// Person data words: bit 0 of either sends the planner down paths that
/// are not ported.
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

/// An object's obstacle (0x482599 to 0x4827b9): its footprint, in the
/// object's own word order (the emulated original's obstacles at the
/// Goth house's first route show that order), with the cost, left out when it
/// overlaps the person's footprint (Wine's IntersectRect, on the
/// footprint before growing), and grown by [`PADDING`] when the object is
/// a person or the cost is 8 or more.
fn obstacle(footprint: [i32; 4], cost: i32, kind: i16, actor: [i32; 4]) -> Option<Region> {
    let a = footprint;
    let b = actor;
    if a[0].max(b[0]) < a[2].min(b[2]) && a[1].max(b[1]) < a[3].min(b[3]) {
        return None;
    }
    let rect = Region([a[0], a[1], a[2], a[3], cost]);
    Some(if kind == PERSON || cost >= 8 {
        rect.inflate(PADDING)
    } else {
        rect
    })
}

impl Interpreter<'_> {
    /// Whether the planner leaves an object out of its obstacles
    /// (0x4817e9): one in a container; one a person may pass by its data
    /// word 8 (with bit 2, when bit 1 is clear; else when bit 4 is set)
    /// and whose data word 10 is 0; another person farther than 3 tiles
    /// (one nearer is an obstacle),
    /// or the one the context names; and any other object that answers
    /// its function 5 true with the person as the stack object.
    pub(crate) fn ignored(&self, world: &mut World, ctx: &Context, id: u16) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: ctx.person,
            at: None,
            trap,
        };
        if capture(world, id).map_err(fail)?.container.is_some() {
            return Ok(true);
        }
        let flags = datum(world, id, FLAGS).map_err(fail)? as u16;
        let passable = if flags & 4 != 0 {
            flags & 2 == 0
        } else {
            flags & 0x10 != 0
        };
        if passable && datum(world, id, COST).map_err(fail)? == 0 {
            return Ok(true);
        }
        if self.kind(world, id).map_err(fail)? != PERSON {
            return self.callback(world, id, ALLOW_OVERLAP, ctx.person as i16);
        }
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        if object.person.is_none() {
            return Err(fail(unsupported(
                "an object of the person type that is not a person",
            )));
        }
        if ctx.objects[0] == id {
            return Ok(true);
        }
        let from = world
            .objects
            .get(&ctx.person)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?
            .position;
        let distance = crate::primitives::distance(from, object.position, false);
        // 0x48188a: a person within 3 tiles (or at a distance that is not
        // a number) is in the way.
        Ok(distance > Ext::from_i32(NEAR))
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
        let bit = |i: usize| words.data.get(i).is_some_and(|w| w & 1 != 0);
        if bit(CARRIED) || bit(NO_ROOM) {
            return Err(fail(unsupported(
                "a person with bit 0 of person word 84 or 68",
            )));
        }
        if datum(world, person, PACE).map_err(fail)? != 0 {
            return Err(fail(unsupported("a person whose data word 17 is set")));
        }
        let room = datum(world, person, ROOM).map_err(fail)?;
        if datum(world, ctx.target, ROOM).map_err(fail)? != room {
            return Err(fail(unsupported("a slot in another room than the person")));
        }
        let (outdoors, half, objects) = match world.rooms.rooms.get(&room) {
            Some(r) => (r.outdoors, r.half_tiles, r.objects),
            None => return Err(fail(unsupported("a person in no room"))),
        };
        let free_cost = free_cost(room, outdoors);
        let actor = me.footprint;

        // The objects' obstacles.
        let mut dynamic = Vec::new();
        let order = world.order.clone();
        for id in order {
            if id == person || !world.objects.contains_key(&id) {
                continue;
            }
            if datum(world, id, ROOM).map_err(fail)? != room {
                continue;
            }
            if self.ignored(world, ctx, id)? {
                continue;
            }
            let object = world
                .objects
                .get(&id)
                .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
            let fp = object.footprint;
            let cost = if datum(world, id, COST).map_err(fail)? != 0 {
                10
            } else {
                HARD
            };
            let kind = self.kind(world, id).map_err(fail)?;
            if kind == DOOR {
                // 0x4359ce: the door's walls, turned by its direction.
                let mask = i32::from(datum(world, id, WALL_MASK).map_err(fail)?);
                if mask != 0 {
                    let dir = (datum(world, id, DIRECTION).map_err(fail)? as u8 & 31) as u32;
                    let turned = (mask as u32).wrapping_shl(dir);
                    let bits = (turned & 0xff) | ((turned >> 8) & 0xff);
                    if bits & 0x55 != 0 {
                        return Err(fail(unsupported(
                            "a door with walls on its edges (0x482625)",
                        )));
                    }
                }
            }
            dynamic.extend(obstacle(fp, cost, kind, actor));
        }

        let fixed = self.room_rects(world, room).map_err(fail)?;

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
        let bounded = bounded(objects, half);
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
        let goal = found
            .goal
            .ok_or_else(|| fail(unsupported("a search that reaches no goal")))?;
        ctx.chosen = map[goal] as i32;
        if found.points.is_empty() {
            return Err(fail(unsupported("a search that finds no path")));
        }
        path.extend(found.points.iter().map(|p| [p[1], p[0]]));
        Ok(true)
    }
}

/// What a free rectangle found while searching costs (0x4824d0 to
/// 0x48253e): 3 in a room outdoors on the ground, else 0.
fn free_cost(room: i16, outdoors: bool) -> i32 {
    if super::rooms::room_level(room) == 1 && outdoors {
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
            obstacle([10, 20, 30, 40], HARD, 4, actor),
            Some(Region([7, 17, 33, 43, HARD]))
        );
        // A cheap one stays as it is, unless it is a person.
        assert_eq!(
            obstacle([10, 20, 30, 40], 5, 4, actor),
            Some(Region([10, 20, 30, 40, 5]))
        );
        assert_eq!(
            obstacle([10, 20, 30, 40], 5, PERSON, actor),
            Some(Region([7, 17, 33, 43, 5]))
        );
    }

    #[test]
    fn an_obstacle_over_the_person_is_left_out() {
        let actor = [8, 8, 12, 12];
        assert_eq!(obstacle([10, 10, 20, 20], HARD, 4, actor), None);
        // Touching edges do not overlap; growing comes after the test.
        assert_eq!(
            obstacle([12, 0, 20, 20], HARD, 4, actor),
            Some(Region([9, -3, 23, 23, HARD]))
        );
    }

    #[test]
    fn a_free_rectangle_costs_3_only_outdoors_on_the_ground() {
        assert_eq!(free_cost(0, true), 3);
        assert_eq!(free_cost(5, false), 0);
        // Room 1024 is on the second level.
        assert_eq!(free_cost(1024, true), 0);
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
