//! A route to an object's routing slot: the route context (0x48163e),
//! the places the person could stand (0x48235a, 0x483414), and the route
//! there (0x45c38e).
//!
//! Ported from the original's code. Only what the Goth house's routes to
//! update 261, and a player's snack order, reach is ported:
//!
//! - a slot with no flags in its low byte: one place, the slot's offset
//!   turned by the object's direction ([`transform`]; the diagonals
//!   through a rotation matrix, [`turn_by_matrix`]);
//! - a slot with flags in its low byte: rows of places searched by
//!   distance in each flagged direction
//!   ([`Interpreter::search_by_distance`]), each classified
//!   ([`Interpreter::classify`]), and with a sitting weight the seats
//!   beside each place ([`Interpreter::seat_beside`]);
//! - the last step of the route: toward the slot's object (facing -2),
//!   away from it (-1), on along the path (-3), or one tile in the
//!   object's direction turned by a set facing (0 to 7);
//! - for a place chosen beside a seat, the seat's Sit tree instead
//!   ([`SlotRoute::Sit`]).
//!
//! Every other branch fails loudly: a slot with flag 0x10000, a person of
//! a kind above 1, and a refusal of the place test whose placement error
//! was not read.

use crate::fault::{Fault, Trap};
use crate::interpreter::{Interpreter, Stream};
use crate::placement::{Refusal, capture, closed_directions, first_kept_on, off_lot};
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
/// The resolution of a search with seats: one tile (0x4836bf).
const TILE: i32 = 16;
/// The object function a seat has: Sit (callback 0x1a).
const SIT: usize = 26;
/// The object definition word whose bits say from which of the eight
/// places around it a seat may be sat in (OBJD +0x84).
const SEAT_ENTRIES: usize = 66;
/// Where each seat index looks for a seat from a place (jump table
/// 0x483d2b): the change to the place's first and second coordinates,
/// before the place is turned to its direction.
const SEAT_STEP: [[i32; 2]; 8] = [
    [0, -16],
    [16, -16],
    [16, 0],
    [16, 16],
    [0, 16],
    [-16, 16],
    [-16, 0],
    [-16, -16],
];

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
/// `[b, -a]`; any other turns through a matrix ([`turn_by_matrix`]). The
/// second coordinate of `pos` plus the first of the turned point, and the
/// first of `pos` plus the second, are each rounded to single precision
/// and truncated.
pub(crate) fn transform(pos: [i32; 2], a: F32, b: F32, dir: i32) -> [i32; 2] {
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
        _ => turn_by_matrix(a, b, dir),
    };
    let second = (Ext::from_i32(pos[1]) + a).single();
    let first = (b + Ext::from_i32(pos[0])).single();
    [first.to_i32_truncate(), second.to_i32_truncate()]
}

/// A slot place's point turned by a direction other than 0, 2, 4 and 6
/// (0x483d7c), as the original turns it, in world units through a
/// rotation matrix:
///
/// 1. The angle is `dir` times pi (0x673438) times 1/4 (0x666a48),
///    stored as a float (0x455070). Its cosine `c` and sine `s` are
///    stored as floats, and the matrix (0x5c3a20, axis 1) has the rows
///    `[c, 0, -s, 0]`, `[0, 1, 0, 0]`, `[s, 0, c, 0]` and `[0, 0, 0, 1]`.
/// 2. The point goes to world units (0x454ecc): the vector `[k b, h, k
///    a]` with `k` 3/16 (3 times 1/16, stored) and the height `h` 0.
/// 3. The vector times the matrix (0x5c39a0, 0x5c4510), divided by its
///    fourth coordinate (1 here): the first coordinate is `s` times the
///    third plus `c` times the first, the third is `-s` times the first
///    plus `c` times the third. The terms with the height and the
///    matrix's zeros add nothing to the value.
/// 4. Back to the lot's units (0x454fb9): each of the third and first
///    coordinates times 16/3 (stored), rounded half away from zero (0.5
///    added and floored, 0x636ac9, or taken away and ceiled, 0x636b98),
///    gives the new `a` and `b`.
///
/// Every add, multiply and divide is rounded to single precision.
fn turn_by_matrix(a: F32, b: F32, dir: i32) -> (Ext, Ext) {
    let (c, s) = facing_cos_sin(dir);
    // 0x454ecc.
    let v0 = to_world_units(b.ext());
    let v2 = to_world_units(a.ext());
    // 0x5c4510: the fourth coordinate is 1, so its reciprocal is 1.
    let first = ((s * v2).single() + (v0 * c).single()).single();
    let third = ((-s * v0).single() + (c * v2).single()).single();
    // 0x454fb9.
    (
        Ext::from_i32(to_lot_units(third)),
        Ext::from_i32(to_lot_units(first)),
    )
}

/// The cosine and sine of a facing `dir` as the original's rotation
/// matrix holds them (0x455070, 0x5c3a20 axis 1): the angle is `dir`
/// times pi (0x673438) times 1/4 (0x666a48), stored as a float, and its
/// cosine and sine are stored as floats. They come from libm (see
/// [`Ext::cos`] for the **unverified** caveat).
pub(crate) fn facing_cos_sin(dir: i32) -> (Ext, Ext) {
    const PI: F32 = F32::from_bits(0x4049_0fdb);
    const QUARTER: F32 = F32::from_bits(0x3e80_0000);
    let angle = ((Ext::from_i32(dir) * PI.ext()).single() * QUARTER.ext()).to_f32();
    (
        Ext::cos(angle).to_f32().ext(),
        Ext::sin(angle).to_f32().ext(),
    )
}

/// A coordinate in the lot's units (16 to a tile) in world units, as
/// 0x454ecc and 0x454f5c take it: times 3/16 (3 times 1/16, stored once
/// at 0x703f34 and 0x703f24), rounded to single precision and stored.
pub(crate) fn to_world_units(n: Ext) -> Ext {
    const THREE: F32 = F32::from_bits(0x4040_0000);
    let k = (THREE.ext() * SIXTEENTH.ext()).to_f32().ext();
    (k * n).to_f32().ext()
}

/// A coordinate in world units back in the lot's units (0x454fb9): times
/// 16/3 (stored once at 0x703f1c), rounded half away from zero (0.5
/// added and floored, 0x636ac9, or taken away and ceiled, 0x636b98) and
/// truncated to an integer (0x636128).
pub(crate) fn to_lot_units(v: Ext) -> i32 {
    const THREE: F32 = F32::from_bits(0x4040_0000);
    const HALF: F32 = F32::from_bits(0x3f00_0000);
    let scale = (SIXTEEN.ext() / THREE.ext()).to_f32().ext();
    let v = (scale * v).single();
    let whole = if v >= Ext::from_i32(0) {
        (v + HALF.ext()).single().floor()
    } else {
        -(-(v - HALF.ext()).single()).floor()
    };
    whole.to_i32_truncate()
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

    /// Byte 1: a seat was taken by someone else (0x483a3a).
    fn note_taken(&mut self) {
        self.0[1] = true;
    }

    /// The context's result when no place was found (0x483eb7). Byte 0
    /// starts set for a search with seats and no standing weight
    /// (0x4836d8); byte 1 notes a seat taken by another.
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

/// What a seat index finds beside a place (0x48391f to 0x483a51).
#[derive(Debug, PartialEq, Eq)]
enum Beside {
    /// No seat that may be sat in from the place.
    Nothing,
    /// A seat someone else is in: that person or object.
    Taken(u16),
    /// A free seat, or the person's own.
    Seat(u16),
}

/// Whether a seat index's point (`at`, before turning, in the search's
/// row and column units) lies inside the part of the search the row
/// covers (0x4838e7): on the row's own tile row, from the row to `last`;
/// on a row nearer the object, strictly between them; any row further
/// out passes.
fn seat_in_row(at: [i32; 2], row: i32, last: i32) -> bool {
    let (column, line) = (at[0] >> 4, at[1] >> 4);
    match line.cmp(&row) {
        std::cmp::Ordering::Equal => row <= column && column <= last,
        std::cmp::Ordering::Greater => row < column && column < last,
        std::cmp::Ordering::Less => true,
    }
}

/// One tile's step in a direction (0x42228a, table 0x4222bc): the
/// direction is taken modulo 8, and a tile is 16 units. Direction 0 is
/// toward a smaller first coordinate and the directions turn toward a
/// larger second coordinate.
pub(crate) fn step(dir: i32) -> [i32; 2] {
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
        Err(Refusal::OffLot | Refusal::Other) => (0, None),
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
    fn candidates(&self, world: &mut World, ctx: &mut Context, seats: bool) -> Result<(), Fault> {
        let person = ctx.person;
        let fail = |trap| Fault {
            object: person,
            at: None,
            trap,
        };
        drop_candidates(ctx);
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
            return self.search_by_distance(world, ctx, rot, pos, seats);
        }
        let [ox, oy, _] = ctx.slot.offset.map(F32::from_bits);
        let out = transform(pos, ox, oy, rot);
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

    /// The seat at a point, for a place approached from direction `dir`
    /// with seat index `index` (0x48391f to 0x483a51):
    ///
    /// 1. A point off the lot's inner tiles ([`off_lot`]) has none.
    /// 2. The object there is the first on its tile, at the level of the
    ///    slot's object (+0x110), that is not passed over
    ///    ([`first_kept_on`]).
    /// 3. It is a seat to sit in from the place if it has a Sit function
    ///    (callback 0x1a), faces back toward the place (its data word 1 is
    ///    `dir` + 4, modulo 8), and bit `index` of its definition's word 66
    ///    is set. (A test of the absolute value of 0 against 1 follows,
    ///    0x4839c9, and never refuses.)
    /// 4. The seat's first container slot (object +0x130, its second
    ///    record) holding an object other than the person makes it taken
    ///    by that object.
    fn seat_beside(
        &self,
        world: &World,
        ctx: &Context,
        point: [i32; 2],
        dir: i32,
        index: usize,
    ) -> Result<Beside, Trap> {
        if off_lot(world, point) {
            return Ok(Beside::Nothing);
        }
        let level = world
            .objects
            .get(&ctx.target)
            .ok_or_else(|| Trap::native(10, "missing object"))?
            .position[2];
        let Some(seat) = first_kept_on(world, [point[1], point[0]], level)? else {
            return Ok(Beside::Nothing);
        };
        if !self.has_function(world, seat, SIT) {
            return Ok(Beside::Nothing);
        }
        if i32::from(datum(world, seat, DIRECTION)?) != dir.wrapping_add(4) % 8 {
            return Ok(Beside::Nothing);
        }
        if self.seat_entries(world, seat)? & (1 << index) == 0 {
            return Ok(Beside::Nothing);
        }
        let object = world
            .objects
            .get(&seat)
            .ok_or_else(|| Trap::native(10, "missing object"))?;
        let occupant = object
            .slots
            .get(1)
            .and_then(|held| u16::try_from(held.object).ok())
            .filter(|id| *id > 0 && world.objects.contains_key(id));
        Ok(match occupant {
            Some(other) if other != ctx.person => Beside::Taken(other),
            _ => Beside::Seat(seat),
        })
    }

    /// The search by distance (0x483598). The sitting weight is 10 times
    /// slot +0x14 when `seats`, the call's argument (person word 65 at
    /// most 1, 0x4823d0), is set, and 0 otherwise (0x4835f8); the
    /// standing weight is
    /// 10 times the greater of slot +0x10 and +0x18. With a sitting
    /// weight the search goes tile by tile (resolution 16, not slot
    /// +0x38), and one row further in and out (0x48378b; the nearest row
    /// is at least one in). For each of the eight directions whose bit is
    /// set in the slot flags' low byte, taken from the slot's object's
    /// direction `rot`:
    ///
    /// 0. Without slot flag 0x800, a direction the object's tile is
    ///    closed off in ([`closed_directions`], turned to `rot`) is
    ///    refused as answer 3 would refuse it, and skipped.
    /// 1. The person turns for the direction ([`face`]).
    /// 2. Rows of places, one resolution apart, go from the nearest row
    ///    inside the least distance (slot +0x24) out to the farthest
    ///    inside the greatest (slot +0x28). A row at distance `r`
    ///    (counted in resolutions) spans from `-r` to `r`, one less at
    ///    the end when the direction two further on is also in the flags.
    /// 3. Each place is tried as index -1 to 8 (0x48386e), until one
    ///    gives a candidate:
    ///    - index -1 when the standing weight is at least the sitting
    ///      weight, else index 8: the place itself, with the standing
    ///      weight, its word +0xc 0 and its word +0xe the direction's bit
    ///      (0x483a5d);
    ///    - indices 0 to 7, with a sitting weight: the point one tile from
    ///      the place by [`SEAT_STEP`], if it lies in the row's part of
    ///      the search ([`seat_in_row`]), and a seat there
    ///      ([`Self::seat_beside`]) gives the sitting weight, word +0xc the
    ///      seat and word +0xe the index's bit. A seat taken by another
    ///      notes byte 1 of the refusals and the other in the context's
    ///      word +0x8c.
    /// 4. With a weight, the point tried (for a seat, the seat's point)
    ///    must lie between the least and greatest distances from the
    ///    object. It scores 100 plus the weight, less the distance from
    ///    the best (slot +0x2c, rounded down) times the gradient (slot
    ///    +0x30); 25 more when neither the direction nor the one on either
    ///    side of it is closed off (always with flag 0x800), and 20 more
    ///    when the point is where the person starts.
    /// 5. The place itself is classified ([`Self::classify`]). One the
    ///    person may stand at becomes a candidate, and the next place
    ///    follows. Answer 3 ends the direction at the end of the row.
    ///
    /// With slot flag 0x2000, each score also takes a draw from the active
    /// random stream (0x46f457) modulo 20, less 10, before the additions
    /// for the sides and the start (0x483b02).
    ///
    /// More candidates than the context's limit leave a random choice of
    /// them ([`choose_at_random`]).
    fn search_by_distance(
        &self,
        world: &mut World,
        ctx: &mut Context,
        rot: i32,
        pos: [i32; 2],
        seats: bool,
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
        // 0x4835f0: the weights; no sitting weight without seats
        // (0x4835f8).
        let sitting = if seats {
            slot.sitting.wrapping_mul(10)
        } else {
            0
        };
        let weight = slot.standing.max(slot.ground).wrapping_mul(10);
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
        // 0x4836b4: a search with seats goes tile by tile.
        let resolution = if sitting != 0 { TILE } else { slot.resolution };
        let gradient = F32::from_bits(slot.gradient);
        // 16 / resolution, in single precision (0x666a54).
        let scale = (SIXTEEN.ext() / Ext::from_i32(resolution)).single();
        let row_of = |distance: i32| {
            ((Ext::from_i32(distance.wrapping_neg()) * scale).single() * SIXTEENTH.ext())
                .single()
                .to_i32_truncate()
        };
        let (mut low, mut high) = (row_of(far), row_of(near));
        // 0x48378b: with seats, a row further out, and one further in that
        // is still at least one row in.
        if sitting > 0 {
            low = low.wrapping_sub(1);
            high = high.wrapping_add(1);
            if high >= 0 {
                high = -1;
            }
        }
        // 0x4836d8: byte 0 starts set for seats with no standing weight.
        let mut refusals = Refusals::default();
        refusals.0[0] = sitting > 0 && weight == 0;
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
                    let place = transform(pos, a.to_f32(), b.to_f32(), dir);
                    let (xi, yi) = (a.to_i32_truncate(), b.to_i32_truncate());
                    // 0x48386e: the standing place at index -1 or 8, the
                    // seats at 0 to 7; the first candidate ends the place.
                    let standing_at = if weight >= sitting { -1 } else { 8 };
                    for index in -1..=8i32 {
                        let (tried, weight, word_c, word_e) = if index == -1 || index == 8 {
                            // 0x483a5d.
                            if index != standing_at {
                                continue;
                            }
                            ([xi, yi], weight, 0, bit)
                        } else {
                            if sitting == 0 {
                                continue;
                            }
                            let index = index as usize;
                            let [dx, dy] = SEAT_STEP[index];
                            let at = [xi.wrapping_add(dx), yi.wrapping_add(dy)];
                            if !seat_in_row(at, row, last) {
                                continue;
                            }
                            let point = transform(
                                pos,
                                Ext::from_i32(at[0]).to_f32(),
                                Ext::from_i32(at[1]).to_f32(),
                                dir,
                            );
                            match self
                                .seat_beside(world, ctx, point, dir, index)
                                .map_err(fail)?
                            {
                                Beside::Nothing => continue,
                                Beside::Taken(other) => {
                                    // 0x483a3a.
                                    refusals.note_taken();
                                    ctx.word_8c = other as i16;
                                    continue;
                                }
                                Beside::Seat(seat) => (at, sitting, seat as i16, 1 << index),
                            }
                        };
                        if weight == 0 {
                            continue;
                        }
                        // 0x483a8d: the distances, compared as floats, are
                        // whole numbers well inside a float's precision.
                        let [ti, tj] = tried;
                        let squared = ti.wrapping_mul(ti).wrapping_add(tj.wrapping_mul(tj));
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
                            // 0x483b02: a draw from the active stream, as
                            // -10 to 9.
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
                            Ext::from_i32(ti).to_f32(),
                            Ext::from_i32(tj).to_f32(),
                            dir,
                        );
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
                                word_c,
                                word_e: Some(word_e as i16),
                            });
                            if score > ctx.best {
                                ctx.best = score;
                            }
                            break;
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

    /// Where the person starts its route (0x4832d4), in the context's
    /// +0x6c, +0x70 and +0x74: its own place (+0xf4, +0xf8) and 0.
    ///
    /// A person in a container with a Sit function (callback 0x1a; the
    /// container as `capture` gives it, 0x43265e) starts instead from a
    /// place beside the seat. For each bit `i` of the seat's definition
    /// word 66 ([`SEAT_ENTRIES`], OBJD +0x84) that is set, the place is
    /// one tile from the person's place (0x42228a) in the seat's direction
    /// (its data word 1, +0x4c) turned by `i`, modulo 8. Each is tested
    /// as the slot search tests a place ([`Self::classify`], 0x481439,
    /// approached from that direction, which may note an object in the
    /// context's word +0x8c). Of those that answer 6 or 5, the one
    /// nearest the slot's object (+0x48), by the squared distance in whole
    /// units, wins, the first of equals; the context's third word is then
    /// `1 << i`. With none the person's own place stands.
    fn start_place(&self, world: &mut World, ctx: &mut Context) -> Result<(), Fault> {
        let person = ctx.person;
        let fail = |trap| Fault {
            object: person,
            at: None,
            trap,
        };
        let native = |world: &World, id: u16| {
            world
                .objects
                .get(&id)
                .map(|o| [o.position[1], o.position[0]])
                .ok_or_else(|| fail(Trap::native(10, "missing object")))
        };
        let here = native(world, person)?;
        ctx.start = [here[0], here[1], 0];
        let Some(seat) = capture(world, person).map_err(fail)?.container else {
            return Ok(());
        };
        if !self.has_function(world, seat, SIT) {
            return Ok(());
        }
        let entries = self.seat_entries(world, seat).map_err(fail)?;
        let target = native(world, ctx.target)?;
        let mut nearest: Option<i32> = None;
        for i in 0..8 {
            let bit = 1 << i;
            if entries & bit == 0 {
                continue;
            }
            let rot = i32::from(datum(world, seat, DIRECTION).map_err(fail)?);
            let dir = rot.wrapping_add(i) & 7;
            let [dx, dy] = step(dir);
            let point = [here[0].wrapping_add(dx), here[1].wrapping_add(dy)];
            let answer = self.classify(world, ctx, point, dir)?;
            if answer != 6 && answer != 5 {
                continue;
            }
            let [a, b] = [
                target[0].wrapping_sub(point[0]),
                target[1].wrapping_sub(point[1]),
            ];
            let squared = a.wrapping_mul(a).wrapping_add(b.wrapping_mul(b));
            if nearest.is_none_or(|n| squared < n) {
                ctx.start = [point[0], point[1], bit];
                nearest = Some(squared);
            }
        }
        Ok(())
    }

    /// A seat's definition word 66 ([`SEAT_ENTRIES`], OBJD +0x84): from
    /// which of the eight places around it it may be sat in.
    fn seat_entries(&self, world: &World, seat: u16) -> Result<i32, Trap> {
        let object = world
            .objects
            .get(&seat)
            .ok_or_else(|| Trap::native(10, "missing object"))?;
        Ok(i32::from(
            self.catalog
                .object(object.guid)
                .and_then(|d| self.program.definition(d.resource, d.chunk))
                .ok_or_else(|| Trap::Invalid(format!("seat {seat} has no definition")))?
                [SEAT_ENTRIES],
        ))
    }

    /// Whether the person sits in the seat beside one of the context's
    /// candidates (0x48177b): a candidate's word +0xc names a seat that
    /// is there, has a Sit function (callback 0x1a), and is the person's
    /// container as `capture` gives it (0x43265e).
    pub(crate) fn seated_beside_candidate(
        &self,
        world: &mut World,
        ctx: &Context,
    ) -> Result<bool, Fault> {
        let person = ctx.person;
        for candidate in &ctx.candidates {
            let Some(seat) = u16::try_from(candidate.word_c)
                .ok()
                .filter(|id| *id > 0 && world.objects.contains_key(id))
            else {
                continue;
            };
            if !self.has_function(world, seat, SIT) {
                continue;
            }
            let container = capture(world, person)
                .map_err(|trap| Fault {
                    object: person,
                    at: None,
                    trap,
                })?
                .container;
            if container == Some(seat) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Fills a context's candidates (0x48235a): the start
    /// ([`Self::start_place`]), a limit of 8 in a room of more than 250
    /// tiles, and the search ([`Self::candidates`]), with seats only for
    /// a person of kind 1 or less (0x4823d0). Person word 73 holds the
    /// context's word +0x94 during the search, and the person's direction
    /// is put back after it.
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
        self.start_place(world, ctx)?;
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
        // 0x4823d0: only a person of kind 1 or less (not a pet) is
        // offered seats.
        let seats = person_word(world, person, KIND).map_err(fail)? <= 1;
        let found = self.candidates(world, ctx, seats);
        set_person_word(world, person, WORD_73, 0).map_err(fail)?;
        set_datum(world, person, DIRECTION, facing).map_err(fail)?;
        found
    }

    /// Plans the route to the chosen candidate and adds the last step to
    /// the slot (0x45c38e):
    ///
    /// 1. A slot with a sitting weight, for a person of a kind above 1
    ///    (person word 65), loses its seats first (0x46de84); that fails
    ///    loudly.
    /// 2. The route is planned. With no way found the answer is 0
    ///    (0x45c4c7).
    /// 3. Person word 37 takes the chosen candidate's word +0xe.
    /// 4. A candidate beside a seat (word +0xc, the seat's ID): a seat that
    ///    is gone or has no Sit function (callback 0x1a), or a person of a
    ///    kind above 1, sets the context's result to 5 and the answer is 0
    ///    (0x45c4a8). A person already in the seat (its container,
    ///    0x43265e) gives 1. Otherwise the seat's Sit tree is to be pushed
    ///    on the person's thread ([`SlotRoute::Sit`]).
    /// 5. Otherwise the last step is added to the path: the answer is 2,
    ///    or 0 when that step is the path's end itself.
    ///
    /// Every answer but 2 drops the candidates, with the chosen and best
    /// candidates -1 (0x45c4c9, 0x481898).
    pub(crate) fn route_to_slot(
        &self,
        world: &mut World,
        ctx: &mut Context,
        path: &mut Vec<[i32; 2]>,
    ) -> Result<SlotRoute, Fault> {
        let person = ctx.person;
        let fail = |trap| Fault {
            object: person,
            at: None,
            trap,
        };
        // 0x45c3a4: the slot's seats are dropped for a kind above 1.
        let kind = person_word(world, person, KIND).map_err(fail)?;
        if ctx.slot.sitting != 0 && kind > 1 {
            return Err(fail(unsupported(
                "0x45c3c5",
                "dropping the seats of a slot for a person of kind above 1 (0x46de84)",
            )));
        }
        let room = datum(world, person, ROOM).map_err(fail)?;
        if !world.rooms.rooms.contains_key(&room) {
            return Err(fail(unsupported("0x45c3b9", "a route from no room")));
        }
        path.clear();
        if !self.plan(world, ctx, path)? {
            // 0x45c4c7: no way; the candidates are dropped (0x481898) and
            // the answer is 0.
            drop_candidates(ctx);
            return Ok(SlotRoute::Answer(0));
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
            // 0x45c430: the seat beside the place.
            let answer = self
                .sit_in(world, person, chosen.word_c, kind)
                .map_err(fail)?;
            drop_candidates(ctx);
            if answer == SlotRoute::Answer(0) {
                ctx.result = 5;
            }
            return Ok(answer);
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
                    return Ok(SlotRoute::Answer(finish(path, last, next)));
                }
                i32::from(datum(world, person, DIRECTION).map_err(fail)?)
            }
            // 0x45c587: toward the slot's object (-2) or away from it (-1);
            // with flag 0x100 and flags in the low byte, to the object's
            // own place.
            -1 | -2 => {
                let toward = if low_flags && ctx.slot.flags & 0x100 != 0 {
                    return Ok(SlotRoute::Answer(finish(path, last, to)));
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
        Ok(SlotRoute::Answer(finish(
            path,
            last,
            [last[0].wrapping_add(dx), last[1].wrapping_add(dy)],
        )))
    }

    /// The answer for a candidate beside a seat (0x45c430 to 0x45c4a6),
    /// before the candidates are dropped. See [`Self::route_to_slot`].
    fn sit_in(
        &self,
        world: &mut World,
        person: u16,
        seat: i16,
        kind: i16,
    ) -> Result<SlotRoute, Trap> {
        // 0x40743b: the seat by its ID.
        let Some(seat) = u16::try_from(seat)
            .ok()
            .filter(|id| *id > 0 && world.objects.contains_key(id))
        else {
            return Ok(SlotRoute::Answer(0));
        };
        let tree = self.function_tree(world, seat, SIT);
        if tree == 0 {
            return Ok(SlotRoute::Answer(0));
        }
        if capture(world, person)?.container == Some(seat) {
            return Ok(SlotRoute::Answer(1));
        }
        if kind > 1 {
            return Ok(SlotRoute::Answer(0));
        }
        Ok(SlotRoute::Sit { seat, tree })
    }
}

/// What planning a route to a slot asks of the route
/// ([`Interpreter::route_to_slot`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotRoute {
    /// The answer: 0 for no way, 1 for a person already in the chosen
    /// seat, 2 for a route ready to walk.
    Answer(i32),
    /// The person is to sit in `seat`: its Sit tree `tree` is pushed on
    /// the person's thread, as the person's virtual function 0x34 does
    /// with no arguments and no flag (0x45c49d). Pushed, the answer is 3;
    /// a push that fails gives the context's result 5 and the answer 0.
    Sit { seat: u16, tree: u16 },
}

/// Drops a context's candidates; the chosen and best become -1 (0x481898).
fn drop_candidates(ctx: &mut Context) {
    ctx.candidates.clear();
    ctx.chosen = -1;
    ctx.best = -1;
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
        assert_eq!(transform(pos, f(3.0), f(5.0), 0), [105, 203]);
        // 2 gives [-b, a], with a truncated.
        assert_eq!(transform(pos, f(3.7), f(5.0), 2), [103, 195]);
        assert_eq!(transform(pos, f(3.0), f(5.0), 4), [95, 197]);
        assert_eq!(transform(pos, f(3.7), f(5.0), 6), [97, 205]);
        // The sums are truncated toward zero.
        assert_eq!(transform(pos, f(-0.5), f(0.5), 0), [100, 199]);
        // A diagonal turns through the matrix in world units and rounds
        // half away from zero: (3, 5) by an eighth of a turn is about
        // (-1.41, 5.66), so (-1, 6).
        assert_eq!(transform(pos, f(3.0), f(5.0), 1), [106, 199]);
        assert_eq!(transform(pos, f(3.0), f(5.0), 3), [99, 194]);
        assert_eq!(transform(pos, f(3.0), f(5.0), -1), [101, 206]);
        // A tile along the first axis, turned an eighth, is about
        // (11.31, 11.31).
        assert_eq!(transform([0, 0], f(16.0), f(0.0), 1), [11, 11]);
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
