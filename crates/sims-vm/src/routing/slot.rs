//! A route to an object's routing slot: the route context (0x48163e),
//! the places the person could stand (0x48235a, 0x483414), and the route
//! there (0x45c38e).
//!
//! Ported from the original's code. Only what the Goth house's routes to
//! update 261 reach is ported:
//!
//! - a slot with no flags in its low byte: one place, the slot's offset
//!   turned by the object's direction (0, 2, 4 or 6; [`transform`]);
//! - a slot with flags in its low byte, for a person standing: rows of
//!   places searched by distance in each flagged direction
//!   ([`Interpreter::search_by_distance`]), each classified
//!   ([`Interpreter::classify`]);
//! - the last step of the route: toward the slot's object (facing -2),
//!   away from it (-1), on along the path (-3), or one tile in the
//!   object's direction turned by a set facing (0 to 7).
//!
//! Every other branch fails loudly: seats, a random part of a score,
//! more places than the context's limit, a diagonal turn of a place,
//! and a place the person does not fit at (whose answer depends on why).

use crate::fault::{Fault, Trap};
use crate::interpreter::{Interpreter, Stream};
use crate::placement::{Refusal, capture, closed_directions};
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

/// A candidate's score when the slot's own place is free (0x483574),
/// and the base of every score in a search (0x483af3).
const FREE: i32 = 100;
/// The slot flag that adds a random part to each score (0x483aed).
const JITTER: i32 = 0x2000;
/// 16 as a float (0x666a54) and 1/16 (0x667278).
const SIXTEEN: F32 = F32::from_bits(0x4180_0000);
const SIXTEENTH: F32 = F32::from_bits(0x3d80_0000);

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
        action: 0,
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

/// The direction from one point to another (0x4520e9), in eighths of a
/// turn as [`step`] numbers them. With `a` and `b` the differences of
/// the first and second coordinates: mostly along the first (|a| above
/// 2|b|) is 4; otherwise 3 where 2|a| is at least |b| and 2 where it is
/// not. A negative `b` then turns 2 into 6 and 3 into 5, and a negative
/// `a` turns 3 into 1, 4 into 0 and 5 into 7. Two equal points give 3.
pub(crate) fn direction(from: [i32; 2], to: [i32; 2]) -> i32 {
    let a = to[0].wrapping_sub(from[0]);
    let b = to[1].wrapping_sub(from[1]);
    let (x, y) = (a.wrapping_abs(), b.wrapping_abs());
    let mut dir = if x > y.wrapping_add(y) {
        4
    } else {
        2 + i32::from(x.wrapping_add(x) >= y)
    };
    if b < 0 {
        dir = match dir {
            2 => 6,
            3 => 5,
            d => d,
        };
    }
    if a < 0 {
        dir = match dir {
            3 => 1,
            4 => 0,
            5 => 7,
            d => d,
        };
    }
    dir
}

/// A slot's place: the point `[a, b]` from the slot's object at `pos`,
/// turned by the direction `dir` (0x483d4b). Direction 2 truncates `a`
/// and gives `[-b, a]`; 4 gives `[-a, -b]`; 6 truncates `a` and gives
/// `[b, -a]`. The second coordinate of `pos` plus the first of the turned
/// point, and the first of `pos` plus the second, are each rounded to
/// single precision and truncated. The diagonals (a rotation matrix,
/// 0x455070) are not ported.
pub(crate) fn transform(pos: [i32; 2], a: F32, b: F32, dir: i32) -> Result<[i32; 2], Trap> {
    let (a, b) = match dir {
        0 => (a.ext(), b.ext()),
        2 => (
            (-b.ext()).single(),
            Ext::from_i32(a.ext().to_i32_truncate()),
        ),
        4 => ((-a.ext()).single(), -b.ext()),
        6 => (
            b.ext(),
            Ext::from_i32(a.ext().to_i32_truncate().wrapping_neg()),
        ),
        _ => {
            return Err(unsupported(
                "0x483d7c",
                &format!("a slot place turned by direction {dir}"),
            ));
        }
    };
    let second = (Ext::from_i32(pos[1]) + a).single();
    let first = (b + Ext::from_i32(pos[0])).single();
    Ok([first.to_i32_truncate(), second.to_i32_truncate()])
}

/// The person turns for a slot place (0x483e63): with no flags in the
/// slot's low byte, toward the slot's object (`rot` + 4); otherwise by
/// the slot's facing: -1 the place's direction `dir`, -2 its opposite,
/// -3 not at all, and any other the object's direction turned by it.
/// The direction goes into 0 to 7 ([`wrap_direction`]).
fn face(world: &mut World, person: u16, rot: i32, dir: i32, slot: &RouteSlot) -> Result<(), Trap> {
    let v = if slot.flags & 0xff == 0 {
        rot.wrapping_add(4)
    } else {
        match slot.facing {
            -1 => dir,
            -2 => dir.wrapping_add(4),
            -3 => return Ok(()),
            facing => rot.wrapping_add(facing),
        }
    };
    set_datum(world, person, DIRECTION, wrap_direction(v))
}

/// Why places were refused (bytes 0 to 6 at 0x483507 and 0x4836e5),
/// by the classification's answer.
#[derive(Default)]
struct Refusals([bool; 7]);

impl Refusals {
    /// Notes an answer of the classification ([`Interpreter::classify`]).
    fn note(&mut self, answer: i32) {
        let byte = match answer {
            0 => 2,
            1 => 3,
            2 | 3 => 5,
            4 => 6,
            5 => 4,
            _ => return,
        };
        self.0[byte] = true;
    }

    /// The context's result when no place was found (0x483eb7). Bytes 0
    /// and 1, which only the search with seats sets, are kept for the
    /// order.
    fn result(&self) -> i32 {
        let b = self.0;
        if b[1] {
            9
        } else if b[4] {
            13
        } else if b[3] {
            8
        } else if b[5] {
            11
        } else if b[6] {
            12
        } else if b[0] {
            10
        } else {
            0
        }
    }
}

/// One tile's step in a direction (0x42228a, table 0x4222bc): the
/// direction is taken modulo 8, and a tile is 16 units. Direction 0 is
/// toward a smaller first coordinate and the directions turn toward a
/// larger second coordinate.
fn step(dir: i32) -> [i32; 2] {
    let [x, y] = match dir & 7 {
        0 => [-1, 0],
        1 => [-1, 1],
        2 => [0, 1],
        3 => [1, 1],
        4 => [1, 0],
        5 => [1, -1],
        6 => [0, -1],
        _ => [-1, -1],
    };
    [x * 16, y * 16]
}

/// The answer for a place from the place test's verdict (0x481585 to
/// 0x4815dd), and the object to note in the context's word +0x8c: 6 where
/// the person fits; 3 for a wall in the way (7); for a collision (11), 6
/// when no object is named or it is `named` (the context's +0x7c), else 5
/// for a person and 1 for anything else, noting the object; 0 for any
/// other error.
fn place_answer(
    test: Result<(), Refusal>,
    named: u16,
    is_person: impl FnOnce(u16) -> Result<bool, Trap>,
) -> Result<(i32, Option<u16>), Trap> {
    Ok(match test {
        Ok(()) => (6, None),
        Err(Refusal::Wall) => (3, None),
        Err(Refusal::Collision(None)) => (6, None),
        Err(Refusal::Collision(Some(other))) if other == named => (6, None),
        Err(Refusal::Collision(Some(other))) => {
            (if is_person(other)? { 5 } else { 1 }, Some(other))
        }
        Err(Refusal::Other) => (0, None),
        Err(Refusal::Unrecorded) => {
            return Err(unsupported(
                "0x48158c",
                "a refusal of the place test whose placement error was not read",
            ));
        }
    })
}

impl Interpreter<'_> {
    /// How a person could stand at a point to use the slot (0x481439),
    /// approaching it from direction `dir` (-1 for none). In order:
    ///
    /// 1. A point outside the lot's border is 0.
    /// 2. Without slot flag 0x800: a point in another room than the
    ///    person's (object data word 29) is 2. A point approached from a
    ///    direction is 3 if its tile is closed off ([`closed_directions`],
    ///    0x4359ea, turned to the direction opposite `dir`) straight
    ///    ahead, or, for a diagonal `dir`, just to either side.
    /// 3. Without slot flag 0x8000: a point whose tile's four corner
    ///    heights differ, or differ from the slot object's tile's, is 4.
    /// 4. If the person fits at the point (virtual function 0x54, after
    ///    0x6e4ad0 is cleared) the answer is 6: it may stand there. Where
    ///    it does not, the placement error decides (0x48158c): a wall in
    ///    the way (7) is 3. A collision (11) with no object named, or
    ///    with the context's object at +0x7c, is 6. A collision with
    ///    another object writes that object's ID to the context's word
    ///    +0x8c, and is 5 if it is a person (type 2) and 1 if not. Any
    ///    other error is 0.
    fn classify(
        &self,
        world: &mut World,
        ctx: &mut Context,
        point: [i32; 2],
        dir: i32,
    ) -> Result<i32, Fault> {
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
            return Ok(0);
        }
        let me = world
            .objects
            .get(&person)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        let level = me.position[2];
        if ctx.slot.flags & 0x800 == 0 {
            let room = datum(world, person, ROOM).map_err(fail)?;
            let map = usize::try_from(level - 1)
                .ok()
                .and_then(|l| world.rooms.map.get(l))
                .ok_or_else(|| fail(Trap::Invalid("the person is on no level".into())))?;
            let cell = (a as usize) * 64 + b as usize;
            if map.get(cell).copied() != Some(room) {
                return Ok(2);
            }
            // 0x4814e6: approached from a direction, the place may not
            // be closed off toward the target's side.
            if dir != -1 {
                let closed = closed_directions(world, [a, b, level as i8], dir.wrapping_sub(4) & 7)
                    .map_err(fail)?;
                if closed & 1 != 0 || (dir & 1 != 0 && closed & 0x82 != 0) {
                    return Ok(3);
                }
            }
        }
        if ctx.slot.flags & 0x8000 == 0 {
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
                return Ok(4);
            }
        }
        let test = self.place_test(world, person, [first, second], level)?;
        let (answer, blocker) = place_answer(test, ctx.objects[0], |other| {
            Ok(self.kind(world, other)? == 2)
        })
        .map_err(fail)?;
        if let Some(other) = blocker {
            ctx.word_8c = other as i16;
        }
        Ok(answer)
    }

    /// The places a person could stand to use a slot (0x483414). The
    /// slot's object's direction (data word 1, or 0 with slot flag 0x200)
    /// turns the slot. A slot with no flags in its low byte has one place:
    /// its offset from the object ([`transform`]); one with flags there is
    /// searched by distance ([`Self::search_by_distance`]). A place the person may not
    /// stand at leaves the context's result (+0x88) saying why
    /// ([`Refusals`]) if no place is found.
    fn candidates(&self, world: &mut World, ctx: &mut Context) -> Result<(), Fault> {
        let person = ctx.person;
        let fail = |trap| Fault {
            object: person,
            at: None,
            trap,
        };
        // 0x481898.
        ctx.candidates.clear();
        ctx.chosen = -1;
        ctx.best = -1;
        let flags = ctx.slot.flags;
        let mut rot = i32::from(datum(world, ctx.target, DIRECTION).map_err(fail)?);
        if flags & 0x200 != 0 {
            rot = 0;
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
            return self.search_by_distance(world, ctx, rot, pos);
        }
        let [ox, oy, _] = ctx.slot.offset.map(F32::from_bits);
        let out = transform(pos, ox, oy, rot).map_err(fail)?;
        // 0x4834c8: a place on another tile is approached from the
        // object's side.
        let same_tile =
            out == pos || ((out[1] ^ pos[1]) & !0xf == 0 && (out[0] ^ pos[0]) & !0xf == 0);
        let dir = if same_tile { -1 } else { direction(pos, out) };
        face(world, person, rot, 0, &ctx.slot).map_err(fail)?;
        let mut refusals = Refusals::default();
        let answer = self.classify(world, ctx, out, dir)?;
        refusals.note(answer);
        if answer != 6 {
            ctx.result = refusals.result();
            return Ok(());
        }
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

    /// The search by distance (0x483598), for a person standing. For each
    /// of the eight directions whose bit is set in the slot flags' low
    /// byte, taken from the slot's object's direction `rot`:
    ///
    /// 0. Without slot flag 0x800, a direction the object's tile is
    ///    closed off in ([`closed_directions`], turned to `rot`) is
    ///    refused as answer 3 would refuse it, and skipped.
    /// 1. The person turns for the direction ([`face`]).
    /// 2. Rows of points, one resolution (slot +0x38) apart, go from the
    ///    nearest row inside the least distance (slot +0x24) out to the
    ///    farthest inside the greatest (slot +0x28). A row at distance
    ///    `r` (counted in resolutions) spans from `-r` to `r`, one less at
    ///    the end when the direction two further on is also in the flags.
    /// 3. A point whose distance from the object is outside the least and
    ///    greatest is skipped. Otherwise it scores 100 plus 10 times the
    ///    greater of the standing and ground weights (slot +0x10, +0x18),
    ///    less the distance from the best (slot +0x2c, rounded down) times
    ///    the gradient (slot +0x30); 25 more when neither the direction
    ///    nor the one on either side of it is closed off (always with
    ///    flag 0x800), and 20 more when the place is where the person
    ///    starts.
    /// 4. The point is classified ([`Self::classify`]). A place the person
    ///    may stand at becomes a candidate, its word +0xe the direction's
    ///    bit; then the next point. Answer 3 ends the direction at the end
    ///    of the row.
    ///
    /// Each point is scored and classified once (0x483a5d: the loop over
    /// the standing place, the eight seats and the standing place again
    /// takes the place at the first or the last index, whichever weight is
    /// the larger; seats fail loudly, so the sitting weight is 0).
    ///
    /// With slot flag 0x2000, each score also takes a draw from the active
    /// random stream (0x46f457) modulo 20, less 10, before the additions
    /// for the sides and the start (0x483b02).
    ///
    /// More candidates than the context's limit leave a random choice of
    /// them ([`choose_at_random`]).
    ///
    /// Fails loudly for a slot with a sitting weight (seats).
    fn search_by_distance(
        &self,
        world: &mut World,
        ctx: &mut Context,
        rot: i32,
        pos: [i32; 2],
    ) -> Result<(), Fault> {
        let person = ctx.person;
        let fail = |trap| Fault {
            object: person,
            at: None,
            trap,
        };
        let slot = ctx.slot;
        let [near, far, best] = slot.proximity;
        if !(1..=512).contains(&near)
            || !(1..=512).contains(&far)
            || far < near
            || best < near
            || best > far
        {
            return Ok(());
        }
        if slot.sitting.wrapping_mul(10) != 0 {
            return Err(fail(unsupported("0x4835f0", "a slot search with seats")));
        }
        // 0x483660: the directions the target's tile is closed off in,
        // turned to the target's facing, and those beside them.
        let closed = if slot.flags & 0x800 == 0 {
            let target = world
                .objects
                .get(&ctx.target)
                .ok_or_else(|| fail(Trap::native(10, "missing object")))?
                .position;
            closed_directions(world, World::tile(target), rot).map_err(fail)?
        } else {
            0
        };
        let mut beside = 0;
        for i in 0..8 {
            if closed & (1 << i) != 0 {
                beside |= (1 << i) | (1 << ((i + 7) % 8)) | (1 << ((i + 1) % 8));
            }
        }
        let jitter = slot.flags & JITTER != 0;
        let weight = slot.standing.max(slot.ground).wrapping_mul(10);
        let resolution = slot.resolution;
        let gradient = F32::from_bits(slot.gradient);
        // 16 / resolution, in single precision (0x666a54).
        let scale = (SIXTEEN.ext() / Ext::from_i32(resolution)).single();
        let row_of = |distance: i32| {
            ((Ext::from_i32(distance.wrapping_neg()) * scale).single() * SIXTEENTH.ext())
                .single()
                .to_i32_truncate()
        };
        let (low, high) = (row_of(far), row_of(near));
        let mut refusals = Refusals::default();
        for rel in 0..8 {
            let bit = 1 << rel;
            if slot.flags & bit == 0 {
                continue;
            }
            if closed & bit != 0 {
                // 0x48371f: refused as a wall would refuse it.
                refusals.note(3);
                continue;
            }
            let dir = rot.wrapping_add(rel) % 8;
            face(world, person, rot, dir, &slot).map_err(fail)?;
            let shorter = slot.flags & (1 << ((rel + 2) % 8)) != 0;
            let mut stop = false;
            let mut row = high;
            while row >= low && !stop {
                let last = row.wrapping_neg() - i32::from(shorter);
                let y = Ext::from_i32(row.wrapping_mul(resolution)).to_f32();
                for column in row..=last {
                    let x = Ext::from_i32(column.wrapping_mul(resolution)).to_f32();
                    let a = (x.ext() + F32::from_bits(slot.offset[0]).ext()).single();
                    let b = (y.ext() + F32::from_bits(slot.offset[1]).ext()).single();
                    let place = transform(pos, a.to_f32(), b.to_f32(), dir).map_err(fail)?;
                    let (xi, yi) = (a.to_i32_truncate(), b.to_i32_truncate());
                    // 0x483a5d: the place is scored once, as the standing
                    // place (index -1) or, when the sitting weight is the
                    // larger, after the seats (index 8). Seats fail loudly
                    // above, so the sitting weight is 0 and either index
                    // gives the same single pass.
                    if weight == 0 {
                        continue;
                    }
                    let squared = xi.wrapping_mul(xi).wrapping_add(yi.wrapping_mul(yi));
                    if squared > far.wrapping_mul(far) || squared < near.wrapping_mul(near) {
                        continue;
                    }
                    let radial = Ext::from_i32(squared).sqrt().single().to_i32_truncate();
                    let off = radial.wrapping_sub(best).wrapping_abs();
                    let penalty = (Ext::from_i32(off) * gradient.ext())
                        .single()
                        .to_i32_truncate();
                    let mut score = weight.wrapping_add(FREE).wrapping_sub(penalty);
                    if jitter {
                        // 0x483b02: a draw from the active stream, as -10
                        // to 9.
                        let draw = match self.stream {
                            Stream::Lot => world.lot_rng.next_u32(),
                            Stream::Global => world.global_rng.next_u32(),
                        };
                        score = score.wrapping_add((draw % 20) as i32 - 10);
                    }
                    if beside & bit == 0 {
                        score = score.wrapping_add(25);
                    }
                    let there = transform(
                        pos,
                        Ext::from_i32(xi).to_f32(),
                        Ext::from_i32(yi).to_f32(),
                        dir,
                    )
                    .map_err(fail)?;
                    if there == [ctx.start[0], ctx.start[1]] {
                        score = score.wrapping_add(20);
                    }
                    let answer = self.classify(world, ctx, place, dir)?;
                    refusals.note(answer);
                    if answer == 3 {
                        stop = true;
                    }
                    if answer == 6 {
                        ctx.candidates.push(Candidate {
                            position: place,
                            score,
                            word_c: 0,
                            word_e: Some(bit as i16),
                        });
                        if score > ctx.best {
                            ctx.best = score;
                        }
                    }
                }
                row -= 1;
            }
        }
        // The count is compared with the limit unsigned (0x483c4b).
        if ctx.candidates.len() > ctx.limit as u32 as usize {
            let stream = self.stream;
            choose_at_random(&mut ctx.candidates, ctx.limit, || match stream {
                Stream::Lot => world.lot_rng.next_u32(),
                Stream::Global => world.global_rng.next_u32(),
            });
        }
        if ctx.candidates.is_empty() {
            ctx.result = refusals.result();
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
    /// the slot (0x45c38e). Returns 2: the route is ready; or 0 when the
    /// planner finds no way, with the candidates dropped and the chosen
    /// and best candidates -1 (0x45c4c7, 0x481898).
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
            // 0x45c4c7: no way; the candidates are dropped (0x481898) and
            // the answer is 0.
            ctx.candidates.clear();
            ctx.chosen = -1;
            ctx.best = -1;
            return Ok(0);
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
        let target = world
            .objects
            .get(&ctx.target)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?
            .position;
        let to = [target[1], target[0]];
        let facing = ctx.slot.facing;
        let low_flags = ctx.slot.flags & 0xff != 0;
        let dir = match facing {
            // 0x45c516: on along the path's last segment, or, for a last
            // segment of no length, in the person's direction.
            -3 => {
                let before = path[path.len() - 2];
                let along = [
                    last[0].wrapping_sub(before[0]),
                    last[1].wrapping_sub(before[1]),
                ];
                if along != [0, 0] {
                    let next = [
                        last[0].wrapping_add(along[0]),
                        last[1].wrapping_add(along[1]),
                    ];
                    return Ok(finish(path, last, next));
                }
                i32::from(datum(world, person, DIRECTION).map_err(fail)?)
            }
            // 0x45c587: toward the slot's object (-2) or away from it (-1);
            // with flag 0x100 and flags in the low byte, to the object's
            // own place.
            -1 | -2 => {
                let toward = if low_flags && ctx.slot.flags & 0x100 != 0 {
                    return Ok(finish(path, last, to));
                } else if !low_flags && to == last {
                    i32::from(datum(world, ctx.target, DIRECTION).map_err(fail)?)
                } else {
                    direction(last, to)
                };
                if facing == -1 {
                    toward.wrapping_add(4) % 8
                } else {
                    toward
                }
            }
            // 0x45c56f: a set facing turns the slot's object's direction
            // (its data word 1, +0x4c) by the facing.
            facing => {
                i32::from(datum(world, ctx.target, DIRECTION).map_err(fail)?).wrapping_add(facing)
            }
        };
        // 0x45c5fb: the last step is one tile from the path's end in that
        // direction (0x42228a).
        let [dx, dy] = step(dir);
        Ok(finish(
            path,
            last,
            [last[0].wrapping_add(dx), last[1].wrapping_add(dy)],
        ))
    }
}

/// The end of planning a route to a slot (0x45c60e): a last point other
/// than the path's end is added (0x45f659) and the route is ready (2);
/// the path's end itself gives 0.
fn finish(path: &mut Vec<[i32; 2]>, last: [i32; 2], next: [i32; 2]) -> i32 {
    if next == last {
        return 0;
    }
    path.push(next);
    2
}

/// Keeps a random choice of the candidates when there are more than the
/// context's limit (0x483c57 to 0x483cee):
///
/// 1. The candidates are copied and cleared. Each with word +0xc set is
///    kept first, in order, and its score in the copy becomes 0.
/// 2. Then, as many times as the kept count is below the limit (signed,
///    counted from the count after step 1): a draw from the active stream,
///    modulo the number copied, names a candidate, and the first from
///    there on whose score in the copy is not 0 is kept and its score
///    there becomes 0. A draw that finds none keeps nothing and still
///    counts.
///
/// A score of 0 marks a candidate as taken, so one that scored 0 is never
/// chosen. The context's best score stays as it was.
fn choose_at_random(candidates: &mut Vec<Candidate>, limit: i32, mut draw: impl FnMut() -> u32) {
    let mut copy = std::mem::take(candidates);
    if copy.is_empty() {
        return;
    }
    for candidate in &mut copy {
        if candidate.word_c != 0 {
            candidates.push(*candidate);
            candidate.score = 0;
        }
    }
    let mut kept = candidates.len() as i32;
    while kept < limit {
        let start = (draw() % copy.len() as u32) as usize;
        if let Some(candidate) = copy[start..].iter_mut().find(|c| c.score != 0) {
            candidates.push(*candidate);
            candidate.score = 0;
        }
        kept += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_place_is_answered_by_the_placement_error() {
        let person = |id: u16| Ok(id == 21);
        assert_eq!(place_answer(Ok(()), 0, person).unwrap(), (6, None));
        assert_eq!(
            place_answer(Err(Refusal::Wall), 0, person).unwrap(),
            (3, None)
        );
        assert_eq!(
            place_answer(Err(Refusal::Other), 0, person).unwrap(),
            (0, None)
        );
        // A collision with nothing named, or with the context's object,
        // still lets the person stand there.
        assert_eq!(
            place_answer(Err(Refusal::Collision(None)), 0, person).unwrap(),
            (6, None)
        );
        assert_eq!(
            place_answer(Err(Refusal::Collision(Some(21))), 21, person).unwrap(),
            (6, None)
        );
        // Another person in the way is 5, anything else 1; both noted.
        assert_eq!(
            place_answer(Err(Refusal::Collision(Some(21))), 0, person).unwrap(),
            (5, Some(21))
        );
        assert_eq!(
            place_answer(Err(Refusal::Collision(Some(40))), 0, person).unwrap(),
            (1, Some(40))
        );
        assert!(place_answer(Err(Refusal::Unrecorded), 0, person).is_err());
    }

    #[test]
    fn a_step_is_one_tile_in_the_direction_taken_modulo_8() {
        assert_eq!(step(0), [-16, 0]);
        assert_eq!(step(2), [0, 16]);
        assert_eq!(step(4), [16, 0]);
        assert_eq!(step(6), [0, -16]);
        assert_eq!(step(7), [-16, -16]);
        assert_eq!(step(12), [16, 0]);
        assert_eq!(step(-1), [-16, -16]);
    }

    #[test]
    fn negative_directions_wrap_into_a_turn() {
        assert_eq!(wrap_direction(6), 6);
        assert_eq!(wrap_direction(9), 1);
        assert_eq!(wrap_direction(-1), 7);
        assert_eq!(wrap_direction(-9), 7);
    }

    #[test]
    fn a_direction_is_the_step_that_goes_most_nearly_toward_the_point() {
        // Each step's own direction comes back.
        for dir in 0..8 {
            assert_eq!(
                direction([400, 400], {
                    let [x, y] = step(dir);
                    [400 + x, 400 + y]
                }),
                dir
            );
        }
        assert_eq!(direction([408, 429], [408, 424]), 6);
        // The cut-offs are at a ratio of 2: exactly 2 to 1 is diagonal.
        assert_eq!(direction([0, 0], [32, 16]), 3);
        assert_eq!(direction([0, 0], [33, 16]), 4);
        assert_eq!(direction([0, 0], [16, 32]), 3);
        assert_eq!(direction([0, 0], [16, 33]), 2);
        assert_eq!(direction([0, 0], [-33, -16]), 0);
        assert_eq!(direction([0, 0], [-16, -32]), 7);
        // Two equal points.
        assert_eq!(direction([5, 5], [5, 5]), 3);
    }

    #[test]
    fn a_slot_place_is_turned_by_the_direction() {
        let f = F32::from_f32;
        let pos = [100, 200];
        // Direction 0 adds the point as it is: the first coordinate to
        // the second of the position.
        assert_eq!(transform(pos, f(3.0), f(5.0), 0), Ok([105, 203]));
        // 2 gives [-b, a], with a truncated.
        assert_eq!(transform(pos, f(3.7), f(5.0), 2), Ok([103, 195]));
        assert_eq!(transform(pos, f(3.0), f(5.0), 4), Ok([95, 197]));
        assert_eq!(transform(pos, f(3.7), f(5.0), 6), Ok([97, 205]));
        // The sums are truncated toward zero.
        assert_eq!(transform(pos, f(-0.5), f(0.5), 0), Ok([100, 199]));
        // A diagonal fails loudly.
        assert!(transform(pos, f(3.0), f(5.0), 1).is_err());
    }

    #[test]
    fn the_first_refusal_in_the_order_gives_the_result() {
        let result = |answers: &[i32]| {
            let mut r = Refusals::default();
            for &a in answers {
                r.note(a);
            }
            r.result()
        };
        assert_eq!(result(&[]), 0);
        // 6 (may stand) and unknown answers note nothing.
        assert_eq!(result(&[6, 9]), 0);
        assert_eq!(result(&[0]), 0);
        assert_eq!(result(&[1]), 8);
        assert_eq!(result(&[2]), 11);
        assert_eq!(result(&[3]), 11);
        assert_eq!(result(&[4]), 12);
        assert_eq!(result(&[5]), 13);
        // Answer 5 comes before 1, 1 before 2 and 3, and those before 4.
        assert_eq!(result(&[4, 2, 1, 5]), 13);
        assert_eq!(result(&[4, 3, 1]), 8);
        assert_eq!(result(&[4, 2]), 11);
    }

    #[test]
    fn a_route_to_a_slot_ends_with_a_point_other_than_its_end() {
        let mut path = vec![[0, 0], [16, 0]];
        assert_eq!(finish(&mut path, [16, 0], [16, 0]), 0);
        assert_eq!(path, [[0, 0], [16, 0]]);
        assert_eq!(finish(&mut path, [16, 0], [16, 16]), 2);
        assert_eq!(path, [[0, 0], [16, 0], [16, 16]]);
    }

    fn place(x: i32, score: i32, word_c: i16) -> Candidate {
        Candidate {
            position: [x, 0],
            score,
            word_c,
            word_e: None,
        }
    }

    #[test]
    fn a_random_choice_keeps_marked_places_first_then_draws() {
        let mut places = vec![
            place(0, 5, 0),
            place(1, 7, 1),
            place(2, 0, 0),
            place(3, 9, 0),
            place(4, 4, 0),
        ];
        // Draws name places 1 (taken by its mark), 2 (scored 0), then 0.
        let mut draws = [6, 2, 5].into_iter();
        choose_at_random(&mut places, 4, || draws.next().unwrap());
        let kept: Vec<i32> = places.iter().map(|c| c.position[0]).collect();
        // 1 by its mark; draw 6 % 5 = 1 is taken, so the next untaken is
        // 3; draw 2 is scored 0, so 3 is taken and then 4; draw 5 % 5 = 0.
        assert_eq!(kept, [1, 3, 4, 0]);
        assert!(draws.next().is_none());
    }

    #[test]
    fn a_draw_past_every_untaken_place_keeps_nothing_and_still_counts() {
        let mut places = vec![place(0, 5, 0), place(1, 3, 0), place(2, 0, 0)];
        let mut draws = [2, 2, 1].into_iter();
        choose_at_random(&mut places, 2, || draws.next().unwrap());
        // Draw 2 finds only the place that scored 0: nothing is kept.
        // Draw 2 again: nothing. The limit is reached after two draws.
        assert!(places.is_empty());
        assert_eq!(draws.next(), Some(1));
    }
}
