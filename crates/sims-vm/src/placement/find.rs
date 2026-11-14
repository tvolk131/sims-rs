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
//! the running game only where `docs/decisions.md` says so.
//!
//! The placement error the original writes on a refusal (global 0x6e4acc,
//! and the object it collided with in 0x6e4ad0) is not kept: nothing the
//! simulation holds reads it on these paths. The comments name it, so the
//! reason for each refusal is on record.
//!
//! Only what an ordinary object on the ground needs is ported. A person, a
//! tile of a group, a place inside a container, the upper level's floor
//! test, an object that must stand against a wall (data word 13 nonzero)
//! and two objects that are both not solid on the same tile fail loudly.

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
}

/// Person data word 68; bit 0 set means the person takes no room.
const PERSON_NO_ROOM: usize = 68;

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

/// The index of a tile's cell in the lot's 64 by 64 arrays.
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
            spiral_step(&mut dx, &mut dy);
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
        // 0x478f76: a pool tile, on the ground level of a lot with a pool
        // array.
        if level == 1 && world.arrays.contains_key(&POOL) {
            let pool = array(world, POOL, 1).map_err(fail)?;
            if pool[cell(a, b).map_err(fail)?] < 0x10 {
                return Ok(false);
            }
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
    ///    ([`Self::against_walls`], 0x476f5a).
    /// 4. Above the ground level without placement flag 0x200: the upper
    ///    level's floor test, which is not ported.
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
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let object = world.objects.get(&id).ok_or_else(|| fail(missing()))?;
        if object.person.is_some() {
            return Err(fail(Trap::Unsupported(
                "the place test of a person is not ported".into(),
            )));
        }
        if object.multipart.is_some() {
            return Err(fail(Trap::Unsupported(
                "the place test of a tile of a group (0x422abb) is not ported".into(),
            )));
        }
        let read =
            |world: &World, index: usize| datum(world, id, index).map(|w| w as u16).map_err(fail);
        let placement = read(world, word::PLACEMENT)?;
        let levels = read(world, word::LEVELS)?;
        let [a, b, _] = World::tile([position[0], position[1], level]);
        if !inside(world, a, b) || !(0..=3).contains(&level) {
            return Ok(false);
        }
        if level > 1 && placement & 0x1000 != 0 {
            return Ok(false);
        }
        if !self
            .against_walls(world, [a, b], level, read(world, word::WALL_RULES)?)
            .map_err(fail)?
        {
            return Ok(false);
        }
        let index = level_index(level).map_err(fail)?;
        if placement & 0x200 == 0 && level > 1 {
            return Err(fail(Trap::Unsupported(
                "the upper level's floor test (0x4330b7) is not ported".into(),
            )));
        }
        let at = cell(a, b).map_err(fail)?;
        let water = level == 1 && array(world, WATER, 1).map_err(fail)?[at] != 0xff;
        if water {
            if placement & 4 == 0 {
                let on_tile =
                    super::standing_on(world, [a as u8, b as u8, level as u8]).map_err(fail)?;
                let mut floats = false;
                for other in on_tile {
                    floats |= datum(world, other, word::FLAGS).map_err(fail)? as u16 & 0x1000 != 0;
                }
                if !floats {
                    return Ok(false);
                }
            }
        } else if placement & 0x2000 != 0 {
            return Ok(false);
        }
        let pool = array(world, POOL, 1).map_err(fail)?[at];
        if level == 1 {
            if placement & 0x800 == 0 && pool < 0x10 {
                return Ok(false);
            }
            if placement & 0x1000 != 0 && pool >= 0x10 {
                return Ok(false);
            }
        }
        if placement & 0x100 == 0 {
            let heights = &array(world, HEIGHTS, 4).map_err(fail)?[at * 4..at * 4 + 4];
            if heights.iter().any(|&h| h != heights[0]) {
                return Ok(false);
            }
        }
        let floor = {
            let floors = array(world, FLOORS[index], 2).map_err(fail)?;
            u16::from_le_bytes([floors[at * 2], floors[at * 2 + 1]])
        };
        if placement & 1 == 0 && (floor != 0 || (level == 1 && pool == 0x10)) {
            return Ok(false);
        }
        if placement & 2 == 0 && level == 1 && floor == 0 {
            return Ok(false);
        }
        let rule = read(world, word::TILE_FLAG_RULE)?;
        if rule != 0 {
            let flag = array(world, FLAGS[index], 1).map_err(fail)?[at] & 4 != 0;
            if (rule == 1 && !flag) || (rule == 2 && flag) {
                return Ok(false);
            }
        }
        if !self.clear_of_objects(world, id, position, level)? {
            return Ok(false);
        }
        Ok(levels & 1 != 0)
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

    /// Whether an object at a position collides with nothing (0x4337f6,
    /// for an object that is not a person). Its footprint there is worked
    /// out as the lot stores footprints. On its tile and the eight around
    /// it, every object linked from the tile's grid cell through slot 0 is
    /// looked at, the object itself skipped:
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
    ) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let me = |world: &World, index: usize| datum(world, id, index).map(|w| w as u16);
        let flags = me(world, word::FLAGS).map_err(fail)?;
        let not_solid = flags & 4 != 0;
        let exclusive = me(world, word::EXCLUSIVE).map_err(fail)?;
        let facing = datum(world, id, data::FACING).map_err(fail)?;
        let words = self.words(world, id).map_err(fail)?;
        let bounds =
            footprint([position[0], position[1], level], words, facing, None).map_err(fail)?;
        let tile = World::tile([position[0], position[1], level]);
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
                if other != id && self.in_way(world, id, other, not_solid, flags)? {
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
                            return Ok(false);
                        }
                    }
                } else if other != id
                    && index == 8
                    && self.exclusive_clash(world, id, other, not_solid, exclusive)?
                {
                    return Ok(false);
                }
                current = slot(world, other, 0).map_err(fail)?;
            }
        }
        if me(world, word::PLACEMENT).map_err(fail)? & 0x400 == 0
            && crosses_wall(world, bounds, level).map_err(fail)?
        {
            return Ok(false);
        }
        Ok(true)
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
    use super::{WALLS, crosses_wall, spiral_step};
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
}
