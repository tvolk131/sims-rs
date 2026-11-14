//! A route to an object's routing slot: the route context (0x48163e),
//! the places the person could stand (0x48235a, 0x483414), and the route
//! there (0x45c38e).
//!
//! Ported from the original's code. Only what the Goth house's first
//! route reaches is ported: a slot with no flags in its low byte (the
//! place is the slot's offset turned by the object's direction, for
//! direction 2), on the object's own tile, facing the slot's object
//! (facing -2). Every other branch fails.

use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::placement::capture;
use sims_core::{Ext, F32};
use sims_formats::execution::Slot;
use sims_sim::World;
use sims_sim::route::{Candidate, Context, RouteSlot};

/// Object data words.
const DIRECTION: usize = 1;
const ROOM: usize = 29;
/// Person data words.
const WORD_37: usize = 37;
const KIND: usize = 65;
const WORD_72: usize = 72;
const WORD_73: usize = 73;
/// The lot's height array (Arry 0): four corner heights per tile.
const HEIGHTS: u16 = 0;

/// A candidate's score when the slot's own place is free (0x483574).
const FREE: i32 = 100;

fn unsupported(at: &str, what: &str) -> Trap {
    Trap::Unsupported(format!("{what} ({at}), which is not ported"))
}

/// A slot record as the object's slot table holds it: the type's entry
/// with its number (index plus one) and the flags the table now has.
pub(crate) fn route_slot(slot: &Slot, index: usize, flags: Option<i32>) -> RouteSlot {
    RouteSlot {
        offset: slot.offset.map(f32::to_bits),
        number: index as i32 + 1,
        standing: slot.standing,
        sitting: slot.sitting,
        ground: slot.ground,
        flags: flags.unwrap_or(slot.flags as i32),
        snap_target: slot.snap_target,
        proximity: slot.proximity,
        gradient: slot.gradient.to_bits(),
        facing: slot.facing,
        resolution: slot.resolution,
    }
}

/// A new route context for a person and a slot of an object (0x48163e,
/// 0x481693).
pub(crate) fn new_context(world: &World, person: u16, target: u16, slot: RouteSlot) -> Context {
    let (position, word_94) = world.objects.get(&person).map_or(([0; 3], 0), |o| {
        (
            o.position,
            o.person
                .as_ref()
                .and_then(|p| p.data.get(WORD_72).copied())
                .unwrap_or(0),
        )
    });
    Context {
        candidates: Vec::new(),
        slot,
        target,
        person,
        chosen: -1,
        best: -1,
        word_58: 0,
        time: 0,
        saved: [position[1], position[0]],
        word_68: 0,
        start: [0; 3],
        byte_78: true,
        objects: [0; 2],
        byte_84: false,
        result: 0,
        word_8c: 0,
        byte_8e: false,
        word_94,
        limit: 0x30,
    }
}

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

fn set_datum(world: &mut World, id: u16, index: usize, value: i16) -> Result<(), Trap> {
    let word = world
        .objects
        .get_mut(&id)
        .ok_or_else(|| Trap::native(10, "missing object"))?
        .data
        .get_mut(index)
        .ok_or_else(|| Trap::Invalid(format!("object data has no word {index}")))?;
    *word = value;
    Ok(())
}

fn person_word(world: &World, id: u16, index: usize) -> Result<i16, Trap> {
    world
        .objects
        .get(&id)
        .and_then(|o| o.person.as_ref())
        .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?
        .data
        .get(index)
        .copied()
        .ok_or_else(|| Trap::Invalid(format!("person data has no word {index}")))
}

fn set_person_word(world: &mut World, id: u16, index: usize, value: i16) -> Result<(), Trap> {
    let word = world
        .objects
        .get_mut(&id)
        .and_then(|o| o.person.as_mut())
        .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?
        .data
        .get_mut(index)
        .ok_or_else(|| Trap::Invalid(format!("person data has no word {index}")))?;
    *word = value;
    Ok(())
}

/// A direction put into 0 to 7 (0x483e98): a negative one is raised by
/// whole turns first.
fn wrap_direction(mut v: i32) -> i16 {
    if (v as i16) < 0 {
        let below = (7i32.wrapping_sub(i32::from(v as i16)) as u32 >> 3) as i32;
        v = v.wrapping_add(below.wrapping_mul(8));
    }
    (v & 7) as i16
}

/// The direction from one point to another (0x4520e9), for what is
/// ported: the second coordinate's difference at least half the first's
/// and the first's not negative.
fn direction(from: [i32; 2], to: [i32; 2]) -> Result<i32, Trap> {
    let a = to[0].wrapping_sub(from[0]);
    let b = to[1].wrapping_sub(from[1]);
    if a < 0 {
        return Err(unsupported(
            "0x4520e9",
            "a direction with a negative first difference",
        ));
    }
    if a.wrapping_abs() > b.wrapping_abs().wrapping_mul(2) {
        return Err(unsupported(
            "0x4520e9",
            "a direction mostly along the first coordinate",
        ));
    }
    let dir = 2 + i32::from(a.wrapping_abs().wrapping_mul(2) >= b.wrapping_abs());
    if b >= 0 {
        return Err(unsupported(
            "0x4520e9",
            "a direction with a positive second difference",
        ));
    }
    if dir != 2 {
        return Err(unsupported("0x4520e9", "a diagonal direction"));
    }
    Ok(6)
}

impl Interpreter<'_> {
    /// How a person could stand at a point to use the slot (0x481439): 6
    /// when it may stand there. Only that answer is ported.
    fn classify(
        &self,
        world: &mut World,
        ctx: &Context,
        point: [i32; 2],
        dir: i32,
    ) -> Result<(), Fault> {
        let person = ctx.person;
        let fail = |trap| Fault {
            object: person,
            at: None,
            trap,
        };
        let [first, second] = [point[1], point[0]];
        let (a, b) = ((first >> 4) as i8, (second >> 4) as i8);
        let size = i32::from(world.size);
        let ok = |n: i8| n >= 1 && i32::from(n) < size - 1;
        if !ok(a) || !ok(b) {
            return Err(fail(unsupported(
                "0x481439",
                "a place outside the lot's border",
            )));
        }
        if ctx.slot.flags & 0x800 != 0 {
            return Err(fail(unsupported("0x481451", "a slot with flag 0x800")));
        }
        let me = world
            .objects
            .get(&person)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        let level = me.position[2];
        let room = datum(world, person, ROOM).map_err(fail)?;
        let map = usize::try_from(level - 1)
            .ok()
            .and_then(|l| world.rooms.map.get(l))
            .ok_or_else(|| fail(Trap::Invalid("the person is on no level".into())))?;
        let cell = (a as usize) * 64 + b as usize;
        if map.get(cell).copied() != Some(room) {
            return Err(fail(unsupported(
                "0x481439",
                "a place in another room than the person",
            )));
        }
        if dir != -1 {
            return Err(fail(unsupported(
                "0x481439",
                "a place approached from a direction",
            )));
        }
        if ctx.slot.flags & 0x8000 != 0 {
            return Err(fail(unsupported("0x481439", "a slot with flag 0x8000")));
        }
        let heights = world
            .arrays
            .get(&HEIGHTS)
            .filter(|a| a.stride == 4 && a.data.len() == 64 * 64 * 4)
            .ok_or_else(|| fail(Trap::Invalid("the lot has no height array".into())))?;
        let target = world
            .objects
            .get(&ctx.target)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?
            .position;
        let tile = |x: i32, y: i32| {
            let at = ((x >> 4) as usize * 64 + (y >> 4) as usize) * 4;
            heights.data.get(at..at + 4).map(<[u8]>::to_vec)
        };
        let here = tile(first, second);
        let there = tile(target[0], target[1]);
        if here.is_none()
            || here != there
            || here.as_ref().is_some_and(|h| h.iter().any(|&v| v != h[0]))
        {
            return Err(fail(unsupported(
                "0x481439",
                "a place on sloped ground or at another height",
            )));
        }
        if !self.fits(world, person, [first, second], level)? {
            return Err(fail(unsupported(
                "0x481439",
                "a place the person does not fit",
            )));
        }
        Ok(())
    }

    /// The places a person could stand to use a slot (0x483414): here,
    /// the slot's offset from its object.
    fn candidates(&self, world: &mut World, ctx: &mut Context) -> Result<(), Fault> {
        let person = ctx.person;
        let fail = |trap| Fault {
            object: person,
            at: None,
            trap,
        };
        ctx.candidates.clear();
        ctx.chosen = -1;
        ctx.best = -1;
        let flags = ctx.slot.flags;
        let mut dir = i32::from(datum(world, ctx.target, DIRECTION).map_err(fail)?);
        if flags & 0x200 != 0 {
            dir = 0;
        }
        let target = world
            .objects
            .get(&ctx.target)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?
            .position;
        let pos = [target[1], target[0]];
        if flags & 0x1_0000 != 0 {
            return Err(fail(unsupported("0x483479", "a slot with flag 0x10000")));
        }
        if flags & 0xff != 0 {
            return Err(fail(unsupported(
                "0x483598",
                "a slot searched by distance (flags in its low byte)",
            )));
        }
        // 0x483d4b: the offset turned by the direction.
        if dir != 2 {
            return Err(fail(unsupported(
                "0x483d4b",
                "a slot offset turned by a direction other than 2",
            )));
        }
        let [ox, oy, _] = ctx.slot.offset.map(F32::from_bits);
        let t = ox.ext().to_i32_truncate();
        let second = (Ext::from_i32(pos[1]) + (-oy.ext()).single()).single();
        let first = (Ext::from_i32(t) + Ext::from_i32(pos[0])).single();
        let out = [first.to_i32_truncate(), second.to_i32_truncate()];
        let same_tile =
            out == pos || ((out[1] ^ pos[1]) & !0xf == 0 && (out[0] ^ pos[0]) & !0xf == 0);
        if !same_tile {
            return Err(fail(unsupported(
                "0x4834ea",
                "a slot place on another tile than its object",
            )));
        }
        // 0x483e63: the person faces the slot's object.
        let facing = wrap_direction(dir.wrapping_add(4));
        set_datum(world, person, DIRECTION, facing).map_err(fail)?;
        self.classify(world, ctx, out, -1)?;
        ctx.candidates.push(Candidate {
            position: out,
            score: FREE,
            word_c: 0,
            word_e: None,
        });
        if FREE > ctx.best {
            ctx.best = FREE;
        }
        Ok(())
    }

    /// Fills a context's candidates (0x48235a).
    pub(crate) fn find_candidates(
        &self,
        world: &mut World,
        ctx: &mut Context,
    ) -> Result<(), Fault> {
        let person = ctx.person;
        let fail = |trap| Fault {
            object: person,
            at: None,
            trap,
        };
        // 0x4832d4: where the person starts.
        if capture(world, person).map_err(fail)?.container.is_some() {
            return Err(fail(unsupported(
                "0x4832d4",
                "a route from inside a container",
            )));
        }
        let position = world
            .objects
            .get(&person)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?
            .position;
        ctx.start = [position[1], position[0], 0];
        let room = datum(world, person, ROOM).map_err(fail)?;
        if world
            .rooms
            .rooms
            .get(&room)
            .is_some_and(|r| r.half_tiles / 2 > 250)
        {
            ctx.limit = 8;
        }
        let facing = datum(world, person, DIRECTION).map_err(fail)?;
        set_person_word(world, person, WORD_73, ctx.word_94).map_err(fail)?;
        if person_word(world, person, KIND).map_err(fail)? > 1 {
            return Err(fail(unsupported(
                "0x48235a",
                "a slot search for a person of kind above 1",
            )));
        }
        let found = self.candidates(world, ctx);
        set_person_word(world, person, WORD_73, 0).map_err(fail)?;
        set_datum(world, person, DIRECTION, facing).map_err(fail)?;
        found
    }

    /// Plans the route to the chosen candidate and adds the last step to
    /// the slot (0x45c38e). Returns 2: the route is ready.
    pub(crate) fn route_to_slot(
        &self,
        world: &mut World,
        ctx: &mut Context,
        path: &mut Vec<[i32; 2]>,
    ) -> Result<i32, Fault> {
        let person = ctx.person;
        let fail = |trap| Fault {
            object: person,
            at: None,
            trap,
        };
        if ctx.slot.sitting != 0 {
            return Err(fail(unsupported("0x45c3a4", "a route to a slot to sit in")));
        }
        let room = datum(world, person, ROOM).map_err(fail)?;
        if !world.rooms.rooms.contains_key(&room) {
            return Err(fail(unsupported("0x45c3b9", "a route from no room")));
        }
        path.clear();
        if !self.plan(world, ctx, path)? {
            return Err(fail(unsupported(
                "0x45c3f0",
                "a route the planner finds no way for",
            )));
        }
        let chosen = usize::try_from(ctx.chosen)
            .ok()
            .and_then(|i| ctx.candidates.get(i))
            .copied()
            .ok_or_else(|| fail(Trap::Invalid("the planner chose no candidate".into())))?;
        // 0x45c418: person word 37 takes the candidate's word +0xe, which
        // the slot search never wrote.
        match chosen.word_e {
            Some(word) => {
                set_person_word(world, person, WORD_37, word).map_err(fail)?;
                world.routing.unknown_words.remove(&(person, WORD_37));
            }
            None => {
                set_person_word(world, person, WORD_37, 0).map_err(fail)?;
                world.routing.unknown_words.insert((person, WORD_37));
            }
        }
        if chosen.word_c != 0 {
            return Err(fail(unsupported(
                "0x45c425",
                "a candidate with word +0xc set",
            )));
        }
        let Some(&last) = path.last().filter(|_| path.len() >= 2) else {
            return Err(fail(unsupported(
                "0x45c437",
                "a path of fewer than two points",
            )));
        };
        match ctx.slot.facing {
            -2 => {}
            -1 => return Err(fail(unsupported("0x45c587", "a slot with facing -1"))),
            _ => return Err(fail(unsupported("0x45c44d", "a slot with a set facing"))),
        }
        if ctx.slot.flags & 0xff != 0 {
            return Err(fail(unsupported(
                "0x45c591",
                "a slot with flags in its low byte",
            )));
        }
        let target = world
            .objects
            .get(&ctx.target)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?
            .position;
        let to = [target[1], target[0]];
        if to[1] == last[1] {
            return Err(fail(unsupported(
                "0x45c5a6",
                "a path ending level with the slot's object",
            )));
        }
        let dir = direction(last, to).map_err(fail)?;
        // 0x4222bc: direction 6 is a step of one tile back along the
        // second coordinate.
        if dir != 6 {
            return Err(fail(unsupported(
                "0x4222bc",
                "a last step in another direction",
            )));
        }
        let step = [last[0], last[1] - 16];
        path.push(step);
        Ok(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_directions_wrap_into_a_turn() {
        assert_eq!(wrap_direction(6), 6);
        assert_eq!(wrap_direction(9), 1);
        assert_eq!(wrap_direction(-1), 7);
        assert_eq!(wrap_direction(-9), 7);
    }

    #[test]
    fn a_point_beyond_its_target_along_the_second_coordinate_faces_direction_6() {
        assert_eq!(direction([408, 429], [408, 424]).unwrap(), 6);
        assert!(direction([408, 429], [380, 424]).is_err());
        assert!(direction([408, 420], [408, 424]).is_err());
    }
}
