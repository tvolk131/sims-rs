//! Whether an object may stand on a tile, and the search for a tile where
//! it may: the services Find Location For (primitive 16) uses.
//!
//! - [`Interpreter::fits`] is an ordinary object's place test, virtual
//!   function 0x54 (0x433311). It asks 0x4329c1 to test the place without
//!   moving anything. That checks the tile, the ground, the water, the
//!   slope, the tile flags (0x4329c1), the walls the object needs or
//!   cannot stand against (0x476f5a), the objects on the tile and the
//!   eight around it (0x4337f6), and the walls its footprint would cross
//!   (0x476dbc).
//! - [`Interpreter::search`] is the lot's search for a free tile
//!   (0x4347f0): a square spiral out from a tile, taking the first tile
//!   that passes the lot's own tests and the place test.
//!
//! Recovered from the original's code. The old prototype ported the same
//! routines (legacy/crates/sims-sim/src/placement.rs: `Spiral`,
//! `search_tile_allowed`; legacy/crates/sims-sim/src/lot_acceptance.rs)
//! and compared them with the original's code run alone; the ports here
//! were written from the original's code and checked against a trace of
//! the running game only where `docs/decisions/` says so.
//!
//! The placement error the original writes on a refusal (global 0x6e4acc,
//! and the object it collided with in 0x6e4ad0) is kept only as far as a
//! caller reads it: [`Refusal`] tells a wall in the way (7) and a
//! collision (11, with the object hit if there is one) from the rest. The
//! comments name each refusal's code, so the reason is on record.
//!
//! Only what an ordinary object or a person on the ground needs is
//! ported, and a group's own test ([`Interpreter::fits`] of a tile of a
//! group, 0x422abb) as far as its tiles leave codes. A person whose person
//! word 84 has bit 0, a place inside a person, an object that must stand against a wall (data
//! word 13 nonzero) and two objects that are both not solid on the same
//! tile fail loudly.

use super::{FLAGS, FULL, GRIDS, data, datum, footprint, missing, registered, slot};
use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use sims_sim::World;

/// The lot's arrays these tests read, by ID, with their cell sizes.
const HEIGHTS: u16 = 0;
const FLOORS: [u16; 2] = [1, 101];
const WALLS: [u16; 2] = [2, 102];
const POOL: u16 = 9;
const WATER: u16 = 10;
const WALL_CELL: usize = 14;

/// The callback an object is asked whether another may overlap it, with
/// the other object as the stack object (function 5).
const ALLOW_OVERLAP: usize = 5;

/// Why the place test refused a place: the placement error it leaves in
/// global 0x6e4acc, as far as a caller tells the codes apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Code 7: the footprint crosses a wall (0x433b7e).
    Wall,
    /// Code 11: another object is in the way. The object is the one whose
    /// footprint overlaps (0x433b42, written to 0x6e4ad0); an exclusive
    /// clash on the tile leaves 0x6e4ad0 as the caller set it (0x433b4a),
    /// None here.
    Collision(Option<u16>),
    /// Code 1: the place is off the lot (0x432a68), or a group's origin
    /// is (0x422adc).
    OffLot,
    /// One of the other codes the steps of [`Interpreter::fits`] name.
    Other,
    /// A refusal whose code was not read from the original: the wall
    /// rules of an object that is not a person (0x476f5a).
    Unrecorded,
}

/// Data words of the object being placed.
mod word {
    /// The levels it may stand at; bit 0 is the ground or a floor.
    pub const LEVELS: usize = 4;
    /// Bit 2: not solid. Bit 1 and bit 4 decide whether a person on the
    /// tile is in the way. Bit 0x1000 (of another object): it makes water
    /// standable.
    pub const FLAGS: usize = 8;
    /// How it stands against walls: 0x40 needs a diagonal wall on its
    /// tile, 0x80 allows one; any other nonzero value turns on tests that
    /// are not ported.
    pub const WALL_RULES: usize = 13;
    /// 1: the tile must have flag 4; 2: it must not.
    pub const TILE_FLAG_RULE: usize = 30;
    /// What it may stand on (see [`super::Interpreter::fits`]).
    pub const PLACEMENT: usize = 42;
    /// Bit 0, in both objects: they cannot share a tile.
    pub const EXCLUSIVE: usize = 63;
    /// Its weight, and what a container may hold (+0x80, +0x82).
    pub const WEIGHT: usize = 27;
    pub const CAPACITY: usize = 28;
}

/// The SLOT entry kind of a container slot.
const CONTAINER: u16 = 0;

/// The global the place test reads as the game's mode (the simulator's
/// +0x36, which a capture of the original names its mode; 0 in live
/// mode).
const MODE: usize = 19;

/// A container slot as the place test reads it (a record of 0x24 bytes).
struct Held {
    /// Whether its offset is 0 across (record +0 and +4).
    centre: bool,
    /// Its height (record +0x14).
    height: i32,
    /// The object it holds, or 0 (record +0x10).
    occupant: i16,
}

/// The heights of the container slots a person adds after its
/// definition's (0x45962b): the slot it carries in, its head, and three
/// more. All are at its centre.
const PERSON_RECORDS: [i32; crate::objects::PERSON_SLOTS] = [6, 0, 1, 1, 1];

/// The person type in definition word 9.
const PERSON_TYPE: i16 = 2;

/// Person data word 68; bit 0 set means the person takes no room.
const PERSON_NO_ROOM: usize = 68;

/// Person data word 84; bit 0 sends the collision test through steps
/// that are not ported.
const PERSON_CARRIED: usize = 84;

/// The tiles around a tile that the collision test visits, as offsets of
/// the first and second coordinates, the tile itself last (the tables at
/// 0x6c2934 and 0x6c2958).
const AROUND: [(i8, i8); 9] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, 0),
];

/// The wall styles the footprint test lets an object cross (0x476dbc).
const CROSSABLE_STYLES: [u16; 5] = [3, 5, 6, 0xf, 0x17];

/// What the search asks of a tile besides the place test (search struct
/// bytes 0x14 to 0x16, 0x4347d1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rules {
    /// The tile must be in the room of the tile the search starts from.
    pub same_room: bool,
    /// No object may be on the tile.
    pub empty: bool,
    /// The tile may not have ground flag 0x20.
    pub unmarked: bool,
}

fn array(world: &World, id: u16, stride: usize) -> Result<&[u8], Trap> {
    let array = world
        .arrays
        .get(&id)
        .ok_or_else(|| Trap::Unsupported(format!("the lot has no array {id}")))?;
    let size = FULL as usize * FULL as usize * stride;
    if array.stride != stride || array.data.len() != size {
        return Err(Trap::Invalid(format!("lot array {id} has the wrong shape")));
    }
    Ok(&array.data)
}

/// Whether a tile is a pool's (0x478f76): on the ground level of a lot
/// with a pool array, a pool byte below 0x10.
pub(crate) fn pool_tile(world: &World, [a, b]: [i8; 2], level: i32) -> Result<bool, Trap> {
    if level != 1 || !world.arrays.contains_key(&POOL) {
        return Ok(false);
    }
    Ok(array(world, POOL, 1)?[cell(a, b)?] < 0x10)
}

/// Object data word 53: the directions an object closes off, as eight
/// bits for directions 0 to 7 relative to the object's facing.
const CLOSED: usize = 53;

/// The directions a tile is closed off in, by walls and by the objects
/// on it, as eight bits for directions 0 to 7, turned so that bit 0 is
/// direction `rot` (0x4359ea). The tile is column, row and level.
///
/// 1. A tile outside the lot's border (0x433087) has none.
/// 2. Its wall segments (byte 0 of the wall cell, [`segments`]) give
///    bit 0x40 for segment 1, bit 1 for segment 2, bit 4 for segment 4
///    and bit 0x10 for segment 8.
/// 3. Each object on the tile, with everything it holds
///    ([`super::standing_on`]), adds its data word 53, sign-extended and
///    turned left by the low byte of its direction (data word 1)
///    (0x4359ce): the bits shifted past bit 7 come back at bit 0.
/// 4. If `rot` is not 0, the bits are turned right by `rot` the same way.
///
/// Read from the original's code.
pub(crate) fn closed_directions(world: &World, tile: [i8; 3], rot: i32) -> Result<i32, Trap> {
    let [a, b, level] = tile;
    if !inside(world, a, b) {
        return Ok(0);
    }
    let walls = segments(world, a, b, i32::from(level))?;
    let mut bits = 0i32;
    for (segment, bit) in [(1, 0x40), (2, 1), (4, 4), (8, 0x10)] {
        if walls & segment != 0 {
            bits |= bit;
        }
    }
    for id in super::standing_on(world, [a as u8, b as u8, level as u8])? {
        let closed = i32::from(datum(world, id, CLOSED)?);
        if closed != 0 {
            let facing = datum(world, id, data::FACING)? as u8;
            bits |= turn(closed, u32::from(facing));
        }
    }
    if rot != 0 {
        bits = turn(bits, 8u32.wrapping_sub(rot as u32));
    }
    Ok(bits)
}

/// A left shift as SHL does it (the count taken mod 32), then the byte
/// above the lowest folded back into it: an eight-bit turn left for a
/// count below 8.
fn turn(bits: i32, count: u32) -> i32 {
    let shifted = bits.wrapping_shl(count & 31);
    (shifted & 0xff) | ((shifted >> 8) & 0xff)
}

fn cell(a: i8, b: i8) -> Result<usize, Trap> {
    let (a, b) = (i32::from(a), i32::from(b));
    if !(0..FULL).contains(&a) || !(0..FULL).contains(&b) {
        return Err(Trap::Unsupported(format!(
            "tile ({a}, {b}) is outside the lot's arrays"
        )));
    }
    Ok((a * FULL + b) as usize)
}

/// Whether a tile is inside the lot's border (0x433087 and the search's
/// own test).
fn inside(world: &World, a: i8, b: i8) -> bool {
    let size = i32::from(world.size);
    let ok = |n: i8| n >= 1 && i32::from(n) < size - 1;
    ok(a) && ok(b)
}

/// The level's index in the arrays kept per level, 1 or 2.
fn level_index(level: i32) -> Result<usize, Trap> {
    usize::try_from(level - 1)
        .ok()
        .filter(|&l| l < 2)
        .ok_or_else(|| Trap::Unsupported(format!("placing an object on level {level}")))
}

/// The wall segments a tile has: byte 0 of its wall cell (0x4795c5).
fn segments(world: &World, a: i8, b: i8, level: i32) -> Result<u8, Trap> {
    let walls = array(world, WALLS[level_index(level)?], WALL_CELL)?;
    Ok(walls[cell(a, b)? * WALL_CELL])
}

/// The ID word of a tile's object grid cell, 0 outside the lot's size or
/// off levels 1 and 2 (0x432500).
fn grid_word(world: &World, a: i8, b: i8, level: i32) -> Result<i16, Trap> {
    let size = i32::from(world.size);
    let ok = |n: i8| n >= 0 && i32::from(n) < size;
    if !ok(a) || !ok(b) || !(1..=2).contains(&level) {
        return Ok(0);
    }
    let grid = array(world, GRIDS[level_index(level)?], 2)?;
    let at = cell(a, b)? * 2;
    Ok(i16::from_le_bytes([grid[at], grid[at + 1]]))
}

/// A tile's room, from the level's room map.
fn room(world: &World, a: i8, b: i8, level: i32) -> Result<i16, Trap> {
    let map = &world.rooms.map[level_index(level)?];
    map.get(cell(a, b)?)
        .copied()
        .ok_or_else(|| Trap::Invalid("the lot has no room map".into()))
}

/// One step of the search's square spiral (0x434ba9 with direction -1):
/// from the start tile, then out ring by ring.
fn spiral_step(dx: &mut i32, dy: &mut i32) {
    if *dy <= -*dx {
        if *dy <= *dx {
            *dx -= 1;
        } else if *dy < -*dx {
            *dy += 1;
        } else {
            *dx += 1;
        }
    } else if *dy <= *dx {
        *dy -= 1;
    } else {
        *dx += 1;
    }
}

/// The next step of a search along a line (0x434be9 to 0x434c24): the
/// step of the coordinate the direction names goes from n to -n when n is
/// positive, and to 1 - n otherwise.
fn line_step(direction: i32, dx: &mut i32, dy: &mut i32) {
    let alternate = |n: &mut i32| {
        *n = if *n > 0 {
            n.wrapping_neg()
        } else {
            1i32.wrapping_sub(*n)
        }
    };
    match direction {
        0 | 4 => alternate(dy),
        2 | 6 => alternate(dx),
        _ => {}
    }
}

/// Whether a footprint crosses a wall on the tiles it covers (0x476dbc).
/// The footprint is in the lot's order: least first and second
/// coordinates, then the greatest. On each tile, the wall segments are
/// visited from the lowest bit up; a diagonal segment 0x20 is visited only
/// when it is the lowest. A segment whose style is one of
/// [`CROSSABLE_STYLES`] is skipped. A wall on the tile's first edge
/// (segment 1) is crossed when the footprint's first coordinates lie on
/// both sides of the edge, one on the second edge (segment 2) likewise,
/// and a diagonal when the footprint has corners on both sides of it.
/// Segments 4 and 8 are never crossed.
fn crosses_wall(world: &World, bounds: [i32; 4], level: i32) -> Result<bool, Trap> {
    let walls = array(world, WALLS[level_index(level)?], WALL_CELL)?;
    // The original's names: r0 the greatest second coordinate, r1 the
    // greatest first, r2 the least second, r3 the least first.
    let [r3, r2, r1, r0] = bounds;
    let style = |cell: &[u8], index: usize| u16::from_le_bytes([cell[index], cell[index + 1]]);
    for second in (r2 >> 4)..=((r0 - 1) >> 4) {
        for first in (r3 >> 4)..=((r1 - 1) >> 4) {
            let (a, b) = (first as i8, second as i8);
            let at = cell(a, b)? * WALL_CELL;
            let cell = &walls[at..at + WALL_CELL];
            let segments = u32::from(cell[0]);
            if segments == 0 {
                continue;
            }
            let mut bit = 1u32;
            while bit & segments != bit {
                bit <<= 1;
            }
            loop {
                // Segment 1 has the first style word; 2 and the
                // diagonals the second (0x53b98f, 0x43517c). Segments 4
                // and 8 are never crossed whatever their style.
                let style = match bit {
                    1 => Some(style(cell, 2)),
                    2 | 0x10 | 0x20 => Some(style(cell, 4)),
                    _ => None,
                };
                if let Some(style) = style
                    && !CROSSABLE_STYLES.contains(&style)
                {
                    let (a, b) = (i32::from(a), i32::from(b));
                    let (line, low, high) = match bit {
                        1 => (a << 4, r3, r1),
                        2 => (b << 4, r2, r0),
                        0x10 => ((a + b + 1) << 4, r0 + r1, r3 + r2),
                        _ => ((b - a) << 4, r2 - r1, r0 - r3),
                    };
                    if (low < line && high > line) || (low > line && high < line) {
                        return Ok(true);
                    }
                }
                bit <<= 1;
                while bit & segments != bit && bit < 0x20 {
                    bit <<= 1;
                }
                if bit >= 0x20 {
                    break;
                }
            }
        }
    }
    Ok(false)
}

impl Interpreter<'_> {
    /// The lot's search for a tile where an object may stand (0x4347f0),
    /// with no direction and no random start: the spiral starts at the
    /// tile of `from`, clamped inside the lot's border, and visits up to
    /// 4096 tiles. It gives up once it has stepped outside the border
    /// eight times the lot's size in a row. A tile inside the border is
    /// taken when, in this order: it has no diagonal wall, it is empty if
    /// that is asked, it is in the start tile's room if that is asked, it
    /// is not a pool tile, it lacks ground flag 0x20 if that is asked, and
    /// the place test passes there ([`Interpreter::fits`]). The place is
    /// the middle of the tile.
    ///
    /// The ground flag rule depends on global 0x6e4ad4, which the
    /// simulation does not hold; it is taken to be 0, as it is in the
    /// trace (**unverified** elsewhere). The ground flags read are always
    /// the ground level's, as in the original.
    pub(crate) fn search(
        &self,
        world: &mut World,
        id: u16,
        from: [i32; 2],
        level: i32,
        rules: Rules,
    ) -> Result<Option<[i32; 2]>, Fault> {
        self.search_toward(world, id, from, level, rules, None)
    }

    /// [`Interpreter::search`] along a line instead of a spiral (search
    /// record +0x10 not -1, 0x434be9): for a direction of 0 or 4 the second
    /// coordinate's step goes 0, 1, -1, 2, -2 and so on, and for 2 or 6
    /// the first's does. A direction of -1 is the spiral's. Any other
    /// direction does not step, so the start tile is tried until the
    /// search gives up. The rest is the spiral's.
    pub(crate) fn search_line(
        &self,
        world: &mut World,
        id: u16,
        from: [i32; 2],
        level: i32,
        rules: Rules,
        direction: i32,
    ) -> Result<Option<[i32; 2]>, Fault> {
        self.search_toward(world, id, from, level, rules, Some(direction))
    }

    fn search_toward(
        &self,
        world: &mut World,
        id: u16,
        from: [i32; 2],
        level: i32,
        rules: Rules,
        direction: Option<i32>,
    ) -> Result<Option<[i32; 2]>, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let size = i32::from(world.size);
        if size > FULL {
            return Err(fail(Trap::Unsupported(format!("a lot {size} tiles wide"))));
        }
        let clamp = |n: i32| {
            let mut tile = (n >> 4) as i8;
            if i32::from(tile) >= size - 1 {
                tile = (size - 2) as i8;
            }
            tile.max(1)
        };
        let start = [clamp(from[0]), clamp(from[1])];
        let start_room = room(world, start[0], start[1], level).map_err(fail)?;
        let limit = size * 8;
        if limit <= 0 {
            return Ok(None);
        }
        let (mut dx, mut dy) = (0i32, 0i32);
        let mut outside = 0;
        let mut count = 0;
        loop {
            if count >= 0x1000 {
                return Ok(None);
            }
            let a = start[0].wrapping_add(dx as i8);
            let b = start[1].wrapping_add(dy as i8);
            if !inside(world, a, b) {
                outside += 1;
            } else {
                outside = 0;
                if self.open(world, id, [a, b], level, start_room, rules)? {
                    return Ok(Some([(i32::from(a) << 4) | 8, (i32::from(b) << 4) | 8]));
                }
            }
            // The record keeps the raw direction (0x434803 to 0x434817), and
            // -1 there is the spiral (0x434ba9): a line search from an
            // object facing -1 (or -3 turned by 2) spirals.
            match direction {
                None | Some(-1) => spiral_step(&mut dx, &mut dy),
                Some(direction) => line_step(direction, &mut dx, &mut dy),
            }
            count += 1;
            if outside >= limit {
                return Ok(None);
            }
        }
    }

    /// The search's tests of one tile inside the border.
    fn open(
        &self,
        world: &mut World,
        id: u16,
        [a, b]: [i8; 2],
        level: i32,
        start_room: i16,
        rules: Rules,
    ) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        if segments(world, a, b, level).map_err(fail)? & 0x30 != 0 {
            return Ok(false);
        }
        if rules.empty && grid_word(world, a, b, level).map_err(fail)? != 0 {
            return Ok(false);
        }
        if rules.same_room && room(world, a, b, level).map_err(fail)? != start_room {
            return Ok(false);
        }
        if pool_tile(world, [a, b], level).map_err(fail)? {
            return Ok(false);
        }
        if rules.unmarked {
            let flags = array(world, FLAGS[0], 1).map_err(fail)?;
            if flags[cell(a, b).map_err(fail)?] & 0x20 != 0 {
                return Ok(false);
            }
        }
        let point = [(i32::from(a) << 4) | 8, (i32::from(b) << 4) | 8];
        self.fits(world, id, point, level)
    }

    /// Whether an object may stand at a position on the ground, outside
    /// any container (virtual function 0x54, 0x433311, testing through
    /// 0x4329c1 without moving anything). In order, refusing at the first
    /// that fails (the placement error is in brackets):
    ///
    /// 1. The tile is inside the lot's border (1) and the level is 0 to 3
    ///    (2).
    /// 2. Above the ground level, placement flag 0x1000 is clear (0x1f).
    /// 3. The tile's diagonal wall is one the object may stand against
    ///    ([`Self::against_walls`], 0x476f5a). A person skips this.
    /// 4. Above the ground level without placement flag 0x200, the tile
    ///    has a floor (0x4330b7; 0x12 or 0x29).
    /// 5. On water (the ground level, water byte not 0xff) without
    ///    placement flag 4, an object on the tile has data word 8 flag
    ///    0x1000 (0x1d). Off water, placement flag 0x2000 is clear (0x2f).
    /// 6. On the ground level: a pool tile (pool byte below 0x10) needs
    ///    placement flag 0x800 (0x1d), and placement flag 0x1000 needs a
    ///    pool tile (0x1e).
    /// 7. Without placement flag 0x100, the tile's four corner heights are
    ///    equal (0x11).
    /// 8. Without placement flag 1, the tile has no floor and is not a
    ///    pool's edge (pool byte 0x10) on the ground level (0x13 or 0x2a).
    /// 9. Without placement flag 2, a tile on the ground level has a floor
    ///    (0x14).
    /// 10. Data word 30: 1 needs tile flag 4 (9), 2 needs it clear (0xa).
    /// 11. The objects around do not collide with it
    ///     ([`Self::clear_of_objects`], 0x4337f6).
    /// 12. Data word 4 has bit 0 (0x18).
    pub(crate) fn fits(
        &self,
        world: &mut World,
        id: u16,
        position: [i32; 2],
        level: i32,
    ) -> Result<bool, Fault> {
        Ok(self.place_test(world, id, position, level)?.is_ok())
    }

    /// Whether an object may go into slot `slot` (as scripts count it) of
    /// object `container`, which stands at `position` on `level` (the same
    /// virtual function, 0x433311, with a container in the record
    /// 0x432679 builds). The steps of [`Interpreter::fits`] run with the
    /// container's position, except that steps 7 to 11 are skipped, and
    /// step 3 refuses nothing (0x432aeb; it is skipped here, as its answer
    /// is not read). In place of steps 11 and 12 come the container's own
    /// ([`Self::fits_container`]).
    pub(crate) fn fits_in(
        &self,
        world: &mut World,
        id: u16,
        container: u16,
        slot: i32,
        position: [i32; 2],
        level: i32,
    ) -> Result<bool, Fault> {
        Ok(self
            .test_place(world, id, position, level, Some((container, slot)))?
            .is_ok())
    }

    /// [`Interpreter::fits`], with the reason for a refusal.
    pub(crate) fn place_test(
        &self,
        world: &mut World,
        id: u16,
        position: [i32; 2],
        level: i32,
    ) -> Result<Result<(), Refusal>, Fault> {
        self.test_place(world, id, position, level, None)
    }

    /// [`Interpreter::place_test`], with the object the test last wrote
    /// to 0x6e4ad0 as collided with (0x433b42), or None if it wrote none
    /// and left it as the caller set it. For one object that is the
    /// object of a [`Refusal::Collision`]; for a tile of a group, whose
    /// test goes on through every tile, it is the last tile's collision,
    /// whatever code the group keeps ([`Self::test_group`]).
    pub(crate) fn place_test_collided(
        &self,
        world: &mut World,
        id: u16,
        position: [i32; 2],
        level: i32,
    ) -> Result<(Result<(), Refusal>, Option<u16>), Fault> {
        let object = world.objects.get(&id).ok_or_else(|| Fault {
            object: id,
            at: None,
            trap: missing(),
        })?;
        if object.multipart.is_some() {
            return self.group_test(world, id, position, level);
        }
        let test = self.test_tile(world, id, position, level, None)?;
        let written = match test {
            Err(Refusal::Collision(other)) => other,
            _ => None,
        };
        Ok((test, written))
    }

    /// [`Interpreter::fits_in`], with the reason for a refusal.
    pub(crate) fn place_test_in(
        &self,
        world: &mut World,
        id: u16,
        container: u16,
        slot: i32,
        position: [i32; 2],
        level: i32,
    ) -> Result<Result<(), Refusal>, Fault> {
        self.test_place(world, id, position, level, Some((container, slot)))
    }

    /// The place test on the ground, or in a container's slot.
    fn test_place(
        &self,
        world: &mut World,
        id: u16,
        position: [i32; 2],
        level: i32,
        container: Option<(u16, i32)>,
    ) -> Result<Result<(), Refusal>, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let object = world.objects.get(&id).ok_or_else(|| fail(missing()))?;
        if object.multipart.is_some() {
            // A tile of a group refuses any container at once (0x422ac8,
            // code 0x10).
            if container.is_some() {
                return Ok(Err(Refusal::Other));
            }
            return self.test_group(world, id, position, level);
        }
        self.test_tile(world, id, position, level, container)
    }

    /// The place test of a group on the ground, asked of any of its tiles
    /// (virtual function 0x54 of a group, 0x422abb), at the place
    /// `position` and `level` for the tile `id`:
    ///
    /// 1. The group's origin is the place less the tile's offset from the
    ///    lead (0x422022 makes sixteenths of it; the level offset is in
    ///    levels). An origin off the lot's inner tiles (0x422632) refuses
    ///    (code 1).
    /// 2. Each tile, from the lead along the list, is tested at the
    ///    origin plus its own offset with the ordinary place test
    ///    (0x433311, [`Interpreter::fits`]'s steps).
    /// 3. The group refuses when a tile does. The code it leaves is the
    ///    first tile's refusal, replaced by any later one of 0x1e or 0x1f
    ///    (0x422b9e to 0x422bb3). The tiles after a refusal are still
    ///    tested (0x422c08), and each collision among them writes the
    ///    object collided with (0x6e4ad0, 0x433b42) over the last: a
    ///    [`Refusal::Collision`] of the group names the last one written
    ///    by any tile, which is not always the tile whose code is kept.
    ///
    /// A tile that refuses without leaving a code ([`Refusal::Unrecorded`])
    /// fails loudly: the group then reads whatever code an earlier test
    /// left (0x422ba2), and a code of 0 would send it on to compare the
    /// tiles' heights (0x422bbb to 0x422c01), which is not ported. So does
    /// a later tile refusing with [`Refusal::Other`] after an earlier one
    /// that is not: whether its code is 0x1e or 0x1f, which would replace
    /// the earlier, is not told apart.
    fn test_group(
        &self,
        world: &mut World,
        id: u16,
        position: [i32; 2],
        level: i32,
    ) -> Result<Result<(), Refusal>, Fault> {
        let (kept, written) = self.group_test(world, id, position, level)?;
        Ok(kept.map_err(|refusal| match refusal {
            Refusal::Collision(_) => Refusal::Collision(written),
            refusal => refusal,
        }))
    }

    /// [`Self::test_group`]: the refusal of the tile whose code the group
    /// keeps, and the object the tiles' tests last wrote as collided with.
    fn group_test(
        &self,
        world: &mut World,
        id: u16,
        position: [i32; 2],
        level: i32,
    ) -> Result<(Result<(), Refusal>, Option<u16>), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let offset = |world: &World, part: u16| {
            world
                .objects
                .get(&part)
                .and_then(|o| o.multipart.as_ref())
                .map(|m| m.offset)
                .ok_or_else(|| fail(missing()))
        };
        let selected = offset(world, id)?;
        let origin = [
            position[0].wrapping_sub(selected[0].wrapping_mul(16)),
            position[1].wrapping_sub(selected[1].wrapping_mul(16)),
        ];
        let origin_level = level.wrapping_sub(selected[2]);
        if super::off_lot(world, origin) {
            return Ok((Err(Refusal::OffLot), None));
        }
        let mut refusal: Option<Refusal> = None;
        let mut written = None;
        for part in self.parts(world, id).map_err(fail)? {
            let at = offset(world, part)?;
            let place = [
                origin[0].wrapping_add(at[0].wrapping_mul(16)),
                origin[1].wrapping_add(at[1].wrapping_mul(16)),
            ];
            let part_level = origin_level.wrapping_add(at[2]);
            let test = self.test_tile(world, part, place, part_level, None)?;
            if let Err(Refusal::Collision(Some(other))) = test {
                written = Some(other);
            }
            match test {
                Ok(()) => {}
                Err(Refusal::Unrecorded) => {
                    return Err(fail(Trap::Unsupported(format!(
                        "the place test of a group: tile {part} refuses without leaving a \
                         code, so the group reads the code an earlier test left (0x422ba2), \
                         which is not kept"
                    ))));
                }
                Err(later) => match refusal {
                    None => refusal = Some(later),
                    Some(first) if later == Refusal::Other && first != Refusal::Other => {
                        return Err(fail(Trap::Unsupported(format!(
                            "the place test of a group: tile {part} refuses after an earlier \
                             tile did, and whether its code (0x1e or 0x1f) replaces the \
                             earlier one's (0x422ba9) is not told apart"
                        ))));
                    }
                    Some(_) => {}
                },
            }
        }
        Ok((refusal.map_or(Ok(()), Err), written))
    }

    /// The ordinary place test of one object (0x433311 and 0x4329c1).
    fn test_tile(
        &self,
        world: &mut World,
        id: u16,
        position: [i32; 2],
        level: i32,
        container: Option<(u16, i32)>,
    ) -> Result<Result<(), Refusal>, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let object = world.objects.get(&id).ok_or_else(|| fail(missing()))?;
        let person = object.person.is_some();
        let read =
            |world: &World, index: usize| datum(world, id, index).map(|w| w as u16).map_err(fail);
        let placement = read(world, word::PLACEMENT)?;
        let levels = read(world, word::LEVELS)?;
        let [a, b, _] = World::tile([position[0], position[1], level]);
        // Code 1, then code 2 for a level outside 1 to 3 (0x433058).
        if !inside(world, a, b) {
            return Ok(Err(Refusal::OffLot));
        }
        if !(1..=3).contains(&level) {
            return Ok(Err(Refusal::Other));
        }
        // Code 0x1f.
        if level > 1 && placement & 0x1000 != 0 {
            return Ok(Err(Refusal::Other));
        }
        // A person skips the wall rules (0x432ac5), and a place in a
        // container is not refused by them (0x432aeb).
        if !person
            && container.is_none()
            && !self
                .against_walls(world, [a, b], level, read(world, word::WALL_RULES)?)
                .map_err(fail)?
        {
            return Ok(Err(Refusal::Unrecorded));
        }
        let index = level_index(level).map_err(fail)?;
        let at = cell(a, b).map_err(fail)?;
        let floor = |world: &World| -> Result<u16, Fault> {
            let floors = array(world, FLOORS[index], 2).map_err(fail)?;
            Ok(u16::from_le_bytes([floors[at * 2], floors[at * 2 + 1]]))
        };
        // Codes 0x12 and 0x29: which of the two depends on the level the
        // player looks at (0x4a6757), which no caller here tells apart.
        if placement & 0x200 == 0 && level > 1 && floor(world)? == 0 {
            return Ok(Err(Refusal::Other));
        }
        let water = level == 1 && array(world, WATER, 1).map_err(fail)?[at] != 0xff;
        if water {
            if placement & 4 == 0 {
                let on_tile =
                    super::standing_on(world, [a as u8, b as u8, level as u8]).map_err(fail)?;
                let mut floats = false;
                for other in on_tile {
                    floats |= datum(world, other, word::FLAGS).map_err(fail)? as u16 & 0x1000 != 0;
                }
                // Code 0x1d.
                if !floats {
                    return Ok(Err(Refusal::Other));
                }
            }
        } else if placement & 0x2000 != 0 {
            // Code 0x2f.
            return Ok(Err(Refusal::Other));
        }
        let pool = array(world, POOL, 1).map_err(fail)?[at];
        if level == 1 {
            // Codes 0x1d and 0x1e.
            if placement & 0x800 == 0 && pool < 0x10 {
                return Ok(Err(Refusal::Other));
            }
            if placement & 0x1000 != 0 && pool >= 0x10 {
                return Ok(Err(Refusal::Other));
            }
        }
        if let Some((container, slot)) = container {
            return self
                .fits_container(world, id, container, slot)
                .map_err(fail);
        }
        if placement & 0x100 == 0 {
            let heights = &array(world, HEIGHTS, 4).map_err(fail)?[at * 4..at * 4 + 4];
            // Code 0x11.
            if heights.iter().any(|&h| h != heights[0]) {
                return Ok(Err(Refusal::Other));
            }
        }
        let floor = floor(world)?;
        // Codes 0x13 and 0x2a, then 0x14.
        if placement & 1 == 0 && (floor != 0 || (level == 1 && pool == 0x10)) {
            return Ok(Err(Refusal::Other));
        }
        if placement & 2 == 0 && level == 1 && floor == 0 {
            return Ok(Err(Refusal::Other));
        }
        let rule = read(world, word::TILE_FLAG_RULE)?;
        if rule != 0 {
            let flag = array(world, FLAGS[index], 1).map_err(fail)?[at] & 4 != 0;
            // Codes 9 and 0xa.
            if (rule == 1 && !flag) || (rule == 2 && flag) {
                return Ok(Err(Refusal::Other));
            }
        }
        if let Some(refusal) = self.clear_of_objects(world, id, position, level)? {
            return Ok(Err(refusal));
        }
        // Code 0x18.
        Ok(if levels & 1 != 0 {
            Ok(())
        } else {
            Err(Refusal::Other)
        })
    }

    /// The place test's own steps for a place in slot `slot` (as scripts
    /// count it) of object `container` (0x432d62 to 0x432fb8). In order,
    /// refusing at the first that fails (the placement error is in
    /// brackets):
    ///
    /// 1. Neither the object nor the container has data word 8 flag 4
    ///    (0xc).
    /// 2. The container has the slot (0xd).
    /// 3. The slot holds nothing, or the object itself (11).
    /// 4. In mode 2 (global 19, the simulator's +0x36), an object that is
    ///    not a person and has no data word 8 flag 0x2000 needs a slot at
    ///    the container's centre (an offset of 0 across), or a container
    ///    with only that slot (3).
    /// 5. A container whose definition is a person's (word 9 is 2) takes
    ///    nothing outside mode 0 (0xd).
    /// 6. For each slot at the same height holding another object, from
    ///    the last slot to the first, unless the object is a person: if
    ///    that slot is at the centre, its object needs data word 8 flag
    ///    0x2000 (11); and if the slot asked for is at the centre, the
    ///    object needs that flag itself (11).
    /// 7. The container's data word 28 is at least the object's data word
    ///    27 (0xe).
    /// 8. The object is not the container, nor anything the container is
    ///    in, however deep ([`super::capture`], 0x4325b5; 11).
    /// 9. The slot's height is not 0 and the object's data word 4 has the
    ///    bit for it, bit height minus 1 (0x18).
    ///
    /// A slot's offset and height come from the container's definition
    /// ([`Self::container_slots`]) and what it holds from the container's
    /// slots. Step 3 names no object it collided with (0x6e4ad0 is left
    /// as it was), so neither does any refusal here.
    fn fits_container(
        &self,
        world: &mut World,
        id: u16,
        container: u16,
        slot: i32,
    ) -> Result<Result<(), Refusal>, Trap> {
        let flags = datum(world, id, word::FLAGS)? as u16;
        let person = world.objects.get(&id).ok_or_else(missing)?.person.is_some();
        // Code 0xc.
        if flags & 4 != 0 || datum(world, container, word::FLAGS)? as u16 & 4 != 0 {
            return Ok(Err(Refusal::Other));
        }
        let slots = self.container_slots(world, container)?;
        // Code 0xd.
        let Some(target) = usize::try_from(slot).ok().and_then(|i| slots.get(i)) else {
            return Ok(Err(Refusal::Other));
        };
        // Code 11.
        if target.occupant != 0 && target.occupant != id as i16 {
            return Ok(Err(Refusal::Collision(None)));
        }
        let unlinked = flags & 0x2000 == 0;
        let mode = *world
            .globals
            .get(MODE)
            .ok_or_else(|| Trap::Invalid(format!("the simulator has no global {MODE}")))?;
        // Code 3.
        if !person && unlinked && mode == 2 && !target.centre && slots.len() > 1 {
            return Ok(Err(Refusal::Other));
        }
        // Code 0xd: the container's definition type (+0x118, word 9) is
        // read, not whether it has person data (0x432e57).
        if self.kind(world, container)? == PERSON_TYPE && mode != 0 {
            return Ok(Err(Refusal::Other));
        }
        for other in slots.iter().rev() {
            if other.height != target.height || other.occupant == 0 || other.occupant == id as i16 {
                continue;
            }
            if other.centre && !person {
                let occupant = u16::try_from(other.occupant)
                    .ok()
                    .filter(|o| world.objects.contains_key(o))
                    .ok_or_else(|| {
                        Trap::Invalid(format!(
                            "a slot of object {container} holds object {}, which is not there",
                            other.occupant
                        ))
                    })?;
                // Code 11.
                if datum(world, occupant, word::FLAGS)? as u16 & 0x2000 == 0 {
                    return Ok(Err(Refusal::Collision(None)));
                }
            }
            // Code 11.
            if target.centre && !person && unlinked {
                return Ok(Err(Refusal::Collision(None)));
            }
        }
        // Code 0xe.
        if datum(world, container, word::CAPACITY)? < datum(world, id, word::WEIGHT)? {
            return Ok(Err(Refusal::Other));
        }
        let mut around = Some(container);
        let mut seen = std::collections::BTreeSet::new();
        while let Some(holder) = around {
            // Code 11.
            if holder == id {
                return Ok(Err(Refusal::Collision(None)));
            }
            if !seen.insert(holder) {
                return Err(Trap::Invalid(format!(
                    "the containers around object {container} form a loop"
                )));
            }
            around = super::capture(world, holder)?.container;
        }
        // Code 0x18. The shift takes the low five bits, as the original's.
        let levels = i32::from(datum(world, id, word::LEVELS)?) as u32;
        let bit = 1u32.wrapping_shl(target.height.wrapping_sub(1) as u32);
        Ok(if target.height != 0 && levels & bit != 0 {
            Ok(())
        } else {
            Err(Refusal::Other)
        })
    }

    /// An object's container slots as scripts count them (object +0x130,
    /// past its first record, which is the tile's chain): one for each
    /// SLOT entry of kind 0 of its definition, in order (0x46e2d1), each
    /// with the entry's offset and height (0x46debb), and the object the
    /// object's slot one further on holds. A height of 0 becomes 1 for an
    /// entry at height 0 and 5 for any other (0x46def8).
    ///
    /// A person has five more after its definition's (its post-constructor
    /// 0x45962b, after the base's 0x431aaf): records made by 0x46de93,
    /// which sets an offset of 0 and a height of 0, with only the height
    /// or the flags changed and none of them converted. In order: height
    /// 6, the slot a person carries in; height 0 with flag 2 and the name
    /// "HEAD" (0x6c4528); and three at height 1 ([`PERSON_RECORDS`]).
    fn container_slots(&self, world: &World, container: u16) -> Result<Vec<Held>, Trap> {
        let object = world.objects.get(&container).ok_or_else(missing)?;
        let own: &[i32] = if object.person.is_some() {
            &PERSON_RECORDS
        } else {
            &[]
        };
        let definition = self
            .catalog
            .object(object.guid)
            .ok_or_else(|| Trap::Invalid("object has no definition".into()))?;
        let entries: Vec<_> = self
            .program
            .slots(definition.resource, definition.chunk)
            .iter()
            .filter(|entry| entry.kind == CONTAINER)
            .collect();
        if entries.len() + own.len() + 1 != object.slots.len() {
            return Err(Trap::Invalid(format!(
                "object {container} keeps {} slots, but its SLOT has {} containers and it \
                 adds {} of its own",
                object.slots.len(),
                entries.len(),
                own.len()
            )));
        }
        let records = entries
            .iter()
            .map(|entry| {
                (
                    entry.offset[0] == 0.0 && entry.offset[1] == 0.0,
                    match entry.height {
                        0 if entry.offset[2] == 0.0 => 1,
                        0 => 5,
                        height => height,
                    },
                )
            })
            .chain(own.iter().map(|&height| (true, height)));
        Ok(records
            .zip(&object.slots[1..])
            .map(|((centre, height), held)| Held {
                centre,
                height,
                occupant: held.object,
            })
            .collect())
    }

    /// Whether the tile's diagonal wall suits an object's wall rules
    /// (0x476f5a): with rule 0x40 the tile must have a diagonal wall, and
    /// without rule 0x80 it must not. With no rules that is all; any rule
    /// turns on further tests of the walls around, which are not ported.
    fn against_walls(
        &self,
        world: &World,
        [a, b]: [i8; 2],
        level: i32,
        rules: u16,
    ) -> Result<bool, Trap> {
        let diagonal = segments(world, a, b, level)? & 0x30 != 0;
        if rules & 0x40 != 0 && !diagonal {
            return Ok(false);
        }
        if rules & 0x80 == 0 && diagonal {
            return Ok(false);
        }
        if rules == 0 {
            return Ok(true);
        }
        Err(Trap::Unsupported(format!(
            "an object with wall rules {rules:#x} (data word 13) needs the wall tests of \
             0x476f5a, which are not ported"
        )))
    }

    /// Whether an object at a position collides with something (0x4337f6),
    /// and if so why.
    /// Its footprint there is worked out as the lot stores footprints. On
    /// its tile and the eight around it, every object linked from the
    /// tile's grid cell through slot 0 is looked at, the object itself
    /// skipped (virtual function 0x5c): for a tile of a multi-tile object,
    /// every tile of its group (0x42314e).
    ///
    /// For a person being placed: one whose person data word 68 has bit 0
    /// collides with nothing; one whose word 84 has bit 0 goes on to steps
    /// that are not ported and fails. Any other object is in the way as
    /// [`Self::in_way_of_person`] says, and there is no test of data word
    /// 63. For any other object:
    ///
    /// - A person whose person data word 68 has bit 0 is skipped. Any
    ///   other person is in the way only when the object is solid and has
    ///   no data word 8 flag 0x10, or is not solid and has no flag 2.
    /// - Another object is in the way when both are solid. Otherwise, on
    ///   the object's own tile only, the two may not both have bit 0 of
    ///   data word 63 (0xb); two objects that are both not solid go on to
    ///   tests that are not ported.
    ///
    /// One in the way collides unless either answers its callback 5 with
    /// true, asked with the other as the stack object, or their footprints
    /// do not overlap (0xb). Last, without placement flag 0x400, the
    /// footprint may not cross a wall ([`crosses_wall`], 7).
    fn clear_of_objects(
        &self,
        world: &mut World,
        id: u16,
        position: [i32; 2],
        level: i32,
    ) -> Result<Option<Refusal>, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let me = |world: &World, index: usize| datum(world, id, index).map(|w| w as u16);
        let person = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(missing()))?
            .person
            .as_ref()
            .map(|p| p.data.clone());
        if let Some(person) = &person {
            let bit = |index: usize| person.get(index).is_some_and(|w| w & 1 != 0);
            if bit(PERSON_NO_ROOM) {
                return Ok(None);
            }
            if bit(PERSON_CARRIED) {
                return Err(fail(Trap::Unsupported(
                    "the collision test of a person whose person word 84 has bit 0 (0x43383f) \
                     is not ported"
                        .into(),
                )));
            }
        }
        let flags = me(world, word::FLAGS).map_err(fail)?;
        let not_solid = flags & 4 != 0;
        let exclusive = me(world, word::EXCLUSIVE).map_err(fail)?;
        let facing = datum(world, id, data::FACING).map_err(fail)?;
        let words = self.words(world, id).map_err(fail)?;
        let bounds = footprint(
            [position[0], position[1], level],
            words,
            facing,
            person.as_deref(),
        )
        .map_err(fail)?;
        let tile = World::tile([position[0], position[1], level]);
        // What counts as the object itself (virtual function 0x5c): the
        // object alone (0x433ba9), or every tile of its group (0x42314e).
        let own = self.parts(world, id).map_err(fail)?;
        for (index, (da, db)) in AROUND.iter().enumerate() {
            let a = tile[0].wrapping_add(*da);
            let b = tile[1].wrapping_add(*db);
            if !inside(world, a, b) {
                continue;
            }
            let mut current = registered(world, grid_word(world, a, b, level).map_err(fail)?);
            let mut seen = 0usize;
            while let Some(other) = current {
                seen += 1;
                if seen > world.objects.len() {
                    return Err(fail(Trap::Invalid(
                        "the objects on a tile link in a loop".into(),
                    )));
                }
                let in_way = if person.is_some() {
                    self.in_way_of_person(world, id, other)?
                } else {
                    self.in_way(world, id, other, not_solid, flags)?
                };
                let itself = own.contains(&other);
                if !itself && in_way {
                    if !self.callback(world, id, ALLOW_OVERLAP, other as i16)?
                        && !self.callback(world, other, ALLOW_OVERLAP, id as i16)?
                    {
                        let theirs = world
                            .objects
                            .get(&other)
                            .ok_or_else(|| fail(missing()))?
                            .footprint;
                        // Both in the lot's order: least first and second
                        // coordinates, then the greatest.
                        let overlap = theirs[1] < bounds[3]
                            && theirs[3] > bounds[1]
                            && theirs[0] < bounds[2]
                            && theirs[2] > bounds[0];
                        if overlap {
                            return Ok(Some(Refusal::Collision(Some(other))));
                        }
                    }
                } else if !itself
                    && person.is_none()
                    && index == 8
                    && self.exclusive_clash(world, id, other, not_solid, exclusive)?
                {
                    return Ok(Some(Refusal::Collision(None)));
                }
                current = slot(world, other, 0).map_err(fail)?;
            }
        }
        if me(world, word::PLACEMENT).map_err(fail)? & 0x400 == 0
            && crosses_wall(world, bounds, level).map_err(fail)?
        {
            return Ok(Some(Refusal::Wall));
        }
        Ok(None)
    }

    /// Whether another object is in the way of the one being placed and
    /// has to be tested for overlap (the first half of 0x4337f6's test of
    /// each object).
    fn in_way(
        &self,
        world: &World,
        id: u16,
        other: u16,
        not_solid: bool,
        flags: u16,
    ) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let object = world.objects.get(&other).ok_or_else(|| fail(missing()))?;
        if let Some(person) = &object.person {
            let no_room = person.data.get(PERSON_NO_ROOM).copied().ok_or_else(|| {
                fail(Trap::Invalid(format!(
                    "a person has no data word {PERSON_NO_ROOM}"
                )))
            })?;
            if no_room & 1 != 0 {
                return Ok(false);
            }
            let passes = if not_solid {
                flags & 2 == 0
            } else {
                flags & 0x10 != 0
            };
            return Ok(!passes);
        }
        let other_not_solid = datum(world, other, word::FLAGS).map_err(fail)? as u16 & 4 != 0;
        Ok(!not_solid && !other_not_solid)
    }

    /// Whether another object is in the way of a person being placed
    /// (0x433975): when it is not solid, if it has data word 8 flag 2;
    /// when it is solid, if it has no flag 0x10. Another person is tested
    /// by the same flags.
    fn in_way_of_person(&self, world: &World, id: u16, other: u16) -> Result<bool, Fault> {
        let flags = datum(world, other, word::FLAGS).map_err(|trap| Fault {
            object: id,
            at: None,
            trap,
        })? as u16;
        Ok(if flags & 4 != 0 {
            flags & 2 != 0
        } else {
            flags & 0x10 == 0
        })
    }

    /// The test of an object on the placed object's own tile when one of
    /// the two is not solid (0x433a7f): both with bit 0 of data word 63
    /// clash. Two objects that are both not solid go on to a comparison of
    /// their routing data (0x4358c3, 0x45728d), which is not ported.
    fn exclusive_clash(
        &self,
        world: &World,
        id: u16,
        other: u16,
        not_solid: bool,
        exclusive: u16,
    ) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        if world
            .objects
            .get(&other)
            .ok_or_else(|| fail(missing()))?
            .person
            .is_some()
        {
            return Ok(false);
        }
        let theirs = datum(world, other, word::EXCLUSIVE).map_err(fail)? as u16;
        if theirs & exclusive & 1 != 0 {
            return Ok(true);
        }
        let other_not_solid = datum(world, other, word::FLAGS).map_err(fail)? as u16 & 4 != 0;
        if not_solid && other_not_solid {
            return Err(fail(Trap::Unsupported(format!(
                "objects {id} and {other} are both not solid on one tile; the test that \
                 follows (0x4358c3, 0x45728d) is not ported"
            ))));
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::{WALLS, closed_directions, crosses_wall, spiral_step, turn};
    use sims_formats::lot::Array;
    use sims_sim::World;

    #[test]
    fn the_spiral_goes_round_each_ring_before_the_next() {
        let (mut dx, mut dy) = (0, 0);
        let mut seen = vec![(dx, dy)];
        for _ in 0..24 {
            spiral_step(&mut dx, &mut dy);
            seen.push((dx, dy));
        }
        assert_eq!(
            seen[..10],
            [
                (0, 0),
                (-1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
                (1, 0),
                (1, -1),
                (0, -1),
                (-1, -1),
                (-2, -1),
            ]
        );
        // The first ring covers the 3 by 3 square, the second the 5 by 5.
        let mut ring: Vec<_> = seen[..9].to_vec();
        ring.sort();
        ring.dedup();
        assert_eq!(ring.len(), 9);
        assert!(ring.iter().all(|&(x, y)| x.abs() <= 1 && y.abs() <= 1));
        let mut square = seen.clone();
        square.sort();
        square.dedup();
        assert_eq!(square.len(), 25);
        assert!(square.iter().all(|&(x, y)| x.abs() <= 2 && y.abs() <= 2));
    }

    /// A lot whose only wall is `segments` on tile (2, 3), with both
    /// style words `style`.
    fn walled(segments: u8, style: u8) -> World {
        let mut world = World::empty();
        let mut data = vec![0; 64 * 64 * 14];
        let cell = (2 * 64 + 3) * 14;
        data[cell] = segments;
        data[cell + 2] = style;
        data[cell + 4] = style;
        world.arrays.insert(
            WALLS[0],
            Array {
                width: 64,
                height: 64,
                stride: 14,
                encoding: 0,
                data,
            },
        );
        world
    }

    #[test]
    fn a_footprint_crosses_a_wall_on_either_side_of_its_edge() {
        // Tile (2, 3)'s first edge is at first coordinate 32.
        let world = walled(1, 1);
        assert_eq!(crosses_wall(&world, [24, 48, 40, 64], 1), Ok(true));
        // A footprint that only touches the edge does not cross it.
        assert_eq!(crosses_wall(&world, [32, 48, 48, 64], 1), Ok(false));
        // Its second edge is at second coordinate 48.
        let world = walled(2, 1);
        assert_eq!(crosses_wall(&world, [32, 40, 48, 56], 1), Ok(true));
        assert_eq!(crosses_wall(&world, [32, 48, 48, 64], 1), Ok(false));
    }

    #[test]
    fn some_walls_are_never_crossed() {
        // A style that may be crossed, such as 3.
        let world = walled(1, 3);
        assert_eq!(crosses_wall(&world, [24, 48, 40, 64], 1), Ok(false));
        // Segments 4 and 8.
        for segments in [4, 8] {
            let world = walled(segments, 1);
            assert_eq!(crosses_wall(&world, [24, 40, 56, 72], 1), Ok(false));
        }
    }

    #[test]
    fn a_diagonal_is_crossed_by_corners_on_both_sides() {
        // Segment 0x10 on tile (2, 3) runs along first + second = 96.
        let world = walled(0x10, 1);
        assert_eq!(crosses_wall(&world, [32, 48, 48, 64], 1), Ok(true));
        // Segment 0x20 runs along second - first = 16.
        let world = walled(0x20, 1);
        assert_eq!(crosses_wall(&world, [32, 48, 48, 64], 1), Ok(true));
        // A footprint on the tile wholly on one side of it does not.
        let world = walled(0x10, 1);
        assert_eq!(crosses_wall(&world, [32, 48, 40, 52], 1), Ok(false));
    }

    #[test]
    fn a_turn_folds_the_bits_shifted_past_bit_7_back_to_bit_0() {
        assert_eq!(turn(0x81, 1), 0x03);
        assert_eq!(turn(0x41, 6), 0x50);
        assert_eq!(turn(0x41, 0), 0x41);
        // A word with its sign set fills the byte.
        assert_eq!(turn(-1, 2), 0xff);
    }

    #[test]
    fn a_tile_is_closed_off_by_its_walls_and_the_objects_on_it() {
        let mut world = walled(1 | 2, 1);
        world.size = 10;
        // Segment 1 closes direction 6, segment 2 direction 0.
        assert_eq!(closed_directions(&world, [2, 3, 1], 0), Ok(0x41));
        // Turned to direction 2: bit 0 is then direction 2.
        assert_eq!(closed_directions(&world, [2, 3, 1], 2), Ok(0x50));
        // A tile outside the lot's border is closed off in none.
        assert_eq!(closed_directions(&world, [0, 3, 1], 0), Ok(0));
        // An object on the tile closes its data word 53's directions,
        // turned by its direction (data word 1).
        let mut grid = vec![0; 64 * 64 * 2];
        grid[(2 * 64 + 3) * 2] = 5;
        world.arrays.insert(
            3,
            Array {
                width: 64,
                height: 64,
                stride: 2,
                encoding: 0,
                data: grid,
            },
        );
        let mut data = vec![0; 72];
        data[1] = 2;
        data[53] = 1;
        world.objects.insert(
            5,
            sims_sim::Object {
                id: 5,
                guid: 0,
                footprint: [0; 4],
                position: [32, 48, 1],
                unknown_short: 0,
                attributes: Vec::new(),
                temps: Vec::new(),
                data,
                obsolete_word: 0,
                stack_capacity_bytes: 0,
                stack: Vec::new(),
                relationship_flag: -1,
                relationships: Vec::new(),
                slots: Vec::new(),
                dynamic_sprites: Vec::new(),
                multipart: None,
                portal: None,
                person: None,
                registry_word: None,
                last_result: false,
                removal_skips_resets: false,
                selected: false,
            },
        );
        assert_eq!(closed_directions(&world, [2, 3, 1], 0), Ok(0x45));
        world.objects.get_mut(&5).unwrap().data[53] = -1;
        assert_eq!(closed_directions(&world, [2, 3, 1], 0), Ok(0xff));
    }
}
