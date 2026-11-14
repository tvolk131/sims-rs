//! The planner (0x4823fa): from a route context's candidates to the
//! person's path.
//!
//! 1. The person's room is the room of the slot's object; the room must
//!    be on the ground and outdoors here (a free rectangle found while
//!    searching costs 3 there).
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
//!    whose objects are at most a fifth of its half tiles searches with
//!    rectangles at most 128 wide, and a step limit of the tiles times
//!    the goals plus 50, at most 1000. A room of more than 256 tiles
//!    first searches without the width limit; that pass must find a path
//!    and is then thrown away.
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
use sims_sim::route::Context;

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
    /// and whose data word 10 is 0; another person farther than 3 tiles,
    /// or the one the context names; and any other object that answers
    /// its function 5 true with the person as the stack object.
    fn ignored(&self, world: &mut World, ctx: &Context, id: u16) -> Result<bool, Fault> {
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
        if distance > Ext::from_i32(NEAR) {
            return Ok(true);
        }
        Err(fail(unsupported("another person within 3 tiles")))
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
        if ctx.candidates.is_empty() {
            return Err(fail(unsupported("a context with no candidates")));
        }
        let room = datum(world, person, ROOM).map_err(fail)?;
        if datum(world, ctx.target, ROOM).map_err(fail)? != room {
            return Err(fail(unsupported("a slot in another room than the person")));
        }
        let (outdoors, half, objects) = match world.rooms.rooms.get(&room) {
            Some(r) => (r.outdoors, r.half_tiles, r.objects),
            None => return Err(fail(unsupported("a person in no room"))),
        };
        if super::rooms::room_level(room) != 1 || !outdoors {
            return Err(fail(unsupported(
                "a room that is not outdoors on the ground",
            )));
        }
        let free_cost = 3;
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

        let density = (Ext::from_i32(objects) / Ext::from_i32(half)).single();
        if density > Ext::from_f64_bits(DENSE) {
            return Err(fail(unsupported("a room crowded with objects")));
        }
        let tiles = half / 2;
        if tiles < 100 {
            return Err(fail(unsupported("a room of fewer than 100 tiles")));
        }
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
        if tiles > 256 {
            let first = search(query(None)).map_err(fail)?;
            if first.points.is_empty() {
                return Err(fail(unsupported("a first search that finds no path")));
            }
        }
        let found = search(query(Some(128))).map_err(fail)?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
