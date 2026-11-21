//! A room as rectangles for the planner (0x4818af): hard rectangles for
//! the room's border, the tiles a person of the room cannot stand on and
//! the walls, then soft rectangles for the ground's covering.
//!
//! It is the same on both levels: the room's ID gives the level
//! (0x468756), which picks the room map (0x481926) and the walls
//! (0x4795c5, array 2 or 102), and the tile test (0x4821f3) looks at the
//! objects on that level. The ground's covering counts only on level 1,
//! and so does water (0x478f4a).
//!
//! The original keeps the result in the room (0x46795a) and works it out
//! again only when the room changes. It is worked out on each use here:
//! nothing ported changes walls, floors, tile flags or doors between two
//! uses, so the two agree. That a room's kept rectangles were never
//! worked out from an older lot is **unverified**.
//!
//! Ported from the old prototype (legacy/crates/sims-sim/src/lot_route.rs:
//! `tile_available`, `prepare`; lot_route/geometry.rs: `finalize`;
//! lot_walls.rs: `Grid::expanded`; portals.rs: `partner`), which compared
//! these with the original's code run alone, and checked against the
//! original's code (listed in the comments).

use super::regions::{HARD, Region};
use crate::fault::Trap;
use crate::interpreter::Interpreter;
use crate::placement::standing_on;
use sims_sim::World;
use sims_sim::rooms::NO_ROOM;

const FULL: usize = 64;
const FLOORS: u16 = 1;
const WALLS: [u16; 2] = [2, 102];
const FLAGS: u16 = 8;
const WATER: u16 = 10;
const WALL_CELL: usize = 14;

/// Wall styles a person walks through (0x4818af's table): the same as
/// the placement's.
const CROSSABLE: [u16; 5] = [3, 5, 6, 15, 23];

/// The function a door's tile has (definition callback 15).
const PORTAL_FUNCTION: usize = 15;
/// The object type of a door.
const DOOR: i16 = 8;
/// Definition word 69: how a door finds its other side.
const PAIRING: usize = 0x8a / 2;
/// Object data word 29: the room.
const ROOM: usize = 29;
/// Object data word 8, the object's flags (+0x5a).
const OBJECT_FLAGS: usize = 8;
/// The flag of data word 8 that makes a tile with water one a person can
/// stand on (0x482321).
const ON_WATER: i16 = 0x1000;
/// Global 10: the lot's number.
const LOT: usize = 10;

fn array(world: &World, id: u16, stride: usize) -> Result<&[u8], Trap> {
    let array = world
        .arrays
        .get(&id)
        .ok_or_else(|| Trap::Unsupported(format!("the lot has no array {id}")))?;
    if array.stride != stride || array.data.len() != FULL * FULL * stride {
        return Err(Trap::Invalid(format!("lot array {id} has the wrong shape")));
    }
    Ok(&array.data)
}

/// Whether a wall crosses a footprint (0x476dbc), given in the original's
/// order (`[greatest +0xf4, greatest +0xf8, least +0xf4, least +0xf8]`)
/// on `level`. Each tile it covers, `+0xf4` tiles in the outer loop and
/// `+0xf8` tiles in the inner (each from the least coordinate's tile to
/// the tile of the greatest less one), is read as 0x4795c5 expands it:
/// its segments are the cell's first byte, the style of segment 1 its
/// first style word and that of segments 2, 0x10 and 0x20 its second
/// (0x53b98f). From the lowest segment the tile has, through each higher
/// one below 0x20, a segment whose style is not one a person crosses
/// ([`CROSSABLE`]) crosses the footprint when its line lies strictly
/// between the footprint's two values for it:
///
/// - segment 1, the line `+0xf8` = 16 × the `+0xf8` tile, against the
///   least and greatest `+0xf8`;
/// - segment 2, `+0xf4` = 16 × the `+0xf4` tile, against the `+0xf4`s;
/// - segment 0x10, `+0xf4 + +0xf8` = 16 × (the two tiles' sum + 1),
///   against the sums of the greatest and of the least;
/// - segment 0x20, `+0xf4 - +0xf8` = 16 × (the tiles' difference),
///   against least `+0xf4` less greatest `+0xf8` and greatest less least.
///
/// Segments 4 and 8 are never tested, and 0x20 only when it is the
/// tile's lowest (the search for the next segment stops at 0x20).
pub(crate) fn walls_cross(world: &World, footprint: [i32; 4], level: i32) -> Result<bool, Trap> {
    let floor = usize::try_from(level - 1)
        .ok()
        .filter(|&f| f < WALLS.len())
        .ok_or_else(|| {
            Trap::Unsupported(format!(
                "a wall test on level {level}, whose walls the original reads past its arrays \
                 (0x4795c5)"
            ))
        })?;
    let walls = array(world, WALLS[floor], WALL_CELL)?;
    let [top_a, top_b, low_a, low_b] = footprint;
    let between = |line: i32, one: i32, other: i32| {
        (one < line && line < other) || (other < line && line < one)
    };
    for a in (low_a >> 4)..=((top_a - 1) >> 4) {
        for b in (low_b >> 4)..=((top_b - 1) >> 4) {
            let (x, y) = (b as i8, a as i8);
            let inside = |n: i8| (0..FULL as i8).contains(&n);
            if !inside(x) || !inside(y) {
                return Err(Trap::Unsupported(format!(
                    "a wall test over tile ({x}, {y}), off the lot's arrays (0x4795c5)"
                )));
            }
            let cell = (x as usize * FULL + y as usize) * WALL_CELL;
            let mask = u32::from(walls[cell]);
            let style = |word: usize| {
                u16::from_le_bytes([walls[cell + 2 + word * 2], walls[cell + 3 + word * 2]])
            };
            if mask == 0 {
                continue;
            }
            let mut bit = mask & mask.wrapping_neg();
            loop {
                let line = match bit {
                    1 => Some((style(0), i32::from(x) << 4, low_b, top_b)),
                    2 => Some((style(1), i32::from(y) << 4, low_a, top_a)),
                    0x10 => Some((
                        style(1),
                        (i32::from(y) + i32::from(x) + 1) << 4,
                        top_a + top_b,
                        low_a + low_b,
                    )),
                    0x20 => Some((
                        style(1),
                        (i32::from(y) - i32::from(x)) << 4,
                        low_a - top_b,
                        top_a - low_b,
                    )),
                    _ => None,
                };
                if let Some((style, line, one, other)) = line
                    && !CROSSABLE.contains(&style)
                    && between(line, one, other)
                {
                    return Ok(true);
                }
                loop {
                    bit <<= 1;
                    if bit & mask == bit || bit >= 0x20 {
                        break;
                    }
                }
                if bit >= 0x20 {
                    break;
                }
            }
        }
    }
    Ok(false)
}

/// The level a room is on, from its ID (0x468756).
pub(crate) fn room_level(room: i16) -> i32 {
    i32::from((room as u16 >> 10) & 3) + 1
}

impl Interpreter<'_> {
    /// The other side of a door (0x4636ae): the next part of the door's
    /// group, round the ring, that has the door function. Doors that pair
    /// by graphic (definition word 69 is 1) are not ported.
    pub(crate) fn partner(&self, world: &World, door: u16) -> Result<u16, Trap> {
        if self.words(world, door)?[PAIRING] == 1 {
            return Err(Trap::Unsupported(
                "a door that pairs with its other side by graphic (0x4636c5) is not ported".into(),
            ));
        }
        let parts = self.parts(world, door)?;
        if parts.len() < 2 {
            return Err(Trap::Unsupported(
                "a door of one tile: the original's partner search reads through a null pointer \
                 (0x463705)"
                    .into(),
            ));
        }
        let at = parts.iter().position(|&p| p == door).unwrap_or(0);
        for step in 1..parts.len() {
            let part = parts[(at + step) % parts.len()];
            // The original casts each part to its door class (0x635abe);
            // every part of a door's group is taken to be one.
            if self.kind(world, part)? != DOOR {
                return Err(Trap::Unsupported(
                    "a door's group has a part of another type; the original's class cast \
                     (0x463724) is not modeled"
                        .into(),
                ));
            }
            if self.has_function(world, part, PORTAL_FUNCTION) {
                return Ok(part);
            }
        }
        Err(Trap::Unsupported(
            "a door whose group has no other door tile: the original's partner search goes \
             round the ring again (0x46374b), which is not ported"
                .into(),
        ))
    }

    /// Whether a person of room `room` may stand on a tile (0x4821f3):
    /// the tile is the room's, split by a diagonal wall, or holds a door
    /// whose other side is in the room; and the tile has no water, or an
    /// object on it with flag 0x1000 of data word 8.
    fn available(
        &self,
        world: &World,
        x: usize,
        y: usize,
        level: i32,
        room: i16,
    ) -> Result<bool, Trap> {
        let map = &world.rooms.map[(level - 1) as usize];
        let primary = map
            .get(x * FULL + y)
            .copied()
            .ok_or_else(|| Trap::Invalid("the lot has no room map".into()))?;
        let mut connected = primary == room || primary == NO_ROOM;
        if !connected {
            for id in standing_on(world, [x as u8, y as u8, level as u8])? {
                if self.kind(world, id)? == DOOR && self.has_function(world, id, PORTAL_FUNCTION) {
                    let partner = self.partner(world, id)?;
                    let other = world
                        .objects
                        .get(&partner)
                        .and_then(|o| o.data.get(ROOM).copied())
                        .ok_or_else(|| Trap::Invalid("a door's other side has no room".into()))?;
                    if other == room {
                        connected = true;
                    }
                }
            }
            if !connected {
                return Ok(false);
            }
        }
        // 0x478f4a: a tile with water (array 10 not 0xff, on the ground)
        // is one only when an object on it, as the lot links them
        // (0x43f331, 0x43f379), has flag 0x1000 of data word 8 (+0x5a,
        // 0x48231d).
        if level == 1 && array(world, WATER, 1)?[x * FULL + y] != 0xff {
            for id in standing_on(world, [x as u8, y as u8, 1])? {
                let flags = world
                    .objects
                    .get(&id)
                    .and_then(|o| o.data.get(OBJECT_FLAGS).copied())
                    .ok_or_else(|| Trap::Invalid(format!("object {id} has no data word 8")))?;
                if flags & ON_WATER != 0 {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        Ok(true)
    }

    /// The room's rectangles (0x4818af), the hard ones first, each hard
    /// one grown by 7. `asked` is the call's third argument: without it
    /// the terrain is left out (0x4818e0).
    pub(crate) fn room_rects(
        &self,
        world: &World,
        room: i16,
        asked: bool,
    ) -> Result<Vec<Region>, Trap> {
        // The level indexes the lot's room maps and wall arrays (0x481926,
        // 0x4795ea); the lot has two.
        let level = room_level(room);
        if level > 2 {
            return Err(Trap::Invalid(format!(
                "room {room} is on level {level}, and the lot has two"
            )));
        }
        let floor = (level - 1) as usize;
        let size = usize::try_from(world.size)
            .ok()
            .filter(|s| (3..=FULL).contains(s))
            .ok_or_else(|| Trap::Invalid("the lot's size is out of range".into()))?;
        let map = &world.rooms.map[floor];
        if map.len() != FULL * FULL {
            return Err(Trap::Invalid("the lot has no room map".into()));
        }
        let walls = array(world, WALLS[floor], WALL_CELL)?;
        let floors = array(world, FLOORS, 2)?;
        let flags = array(world, FLAGS, 1)?;
        let index = |x: usize, y: usize| x * FULL + y;
        let outdoors = world
            .rooms
            .rooms
            .get(&room)
            .ok_or_else(|| Trap::Invalid(format!("room {room} is not on the lot")))?
            .outdoors;
        // 0x4818e0: terrain counts only when asked for, on the ground,
        // outdoors, on a residential lot (the lot's zoning, 0x40745f).
        let lot = world.globals.get(LOT).copied().unwrap_or(0);
        let residential = lot == 0 || self.program.zoning.get(&lot) == Some(&1);
        let terrain = asked && level == 1 && residential && outdoors;

        let mut cache = vec![None; FULL * FULL];
        let mut available = |x: usize, y: usize| -> Result<bool, Trap> {
            if let Some(v) = cache[index(x, y)] {
                return Ok(v);
            }
            let v = self.available(world, x, y, level, room)?;
            cache[index(x, y)] = Some(v);
            Ok(v)
        };

        let mut bounds: Option<[usize; 4]> = None;
        for y in 1..size - 1 {
            for x in 1..size - 1 {
                if map[index(x, y)] == room {
                    let b = bounds.get_or_insert([x, y, x, y]);
                    b[0] = b[0].min(x);
                    b[1] = b[1].min(y);
                    b[2] = b[2].max(x);
                    b[3] = b[3].max(y);
                }
            }
        }
        let [left, top, right, bottom] = bounds.ok_or_else(|| {
            Trap::Unsupported("a room with no tile of its own in the room map".into())
        })?;
        let (left, top) = (left.saturating_sub(1).max(1), top.saturating_sub(1).max(1));
        let (right, bottom) = ((right + 1).min(size - 2), (bottom + 1).min(size - 2));
        let rect = |a: usize, b: usize, d: usize, e: usize, cost| {
            Region([
                a as i32 * 16,
                b as i32 * 16,
                d as i32 * 16,
                e as i32 * 16,
                cost,
            ])
        };
        let mut rects = vec![
            rect(left - 1, top - 1, left, bottom + 2, HARD),
            rect(right + 1, top - 1, right + 2, bottom + 2, HARD),
            rect(left - 1, top - 1, right + 2, top, HARD),
            rect(left - 1, bottom + 1, right + 2, bottom + 2, HARD),
        ];
        let overlap = |rs: &[Region], seed: Region| rs.iter().find(|r| seed.overlaps(**r)).copied();
        // Grows a seed right while the row accepts, then down while every
        // tile of the row below accepts; the row past the bound is tested
        // before the bound.
        let grow = |x: usize,
                    y: usize,
                    accepts: &mut dyn FnMut(usize, usize) -> Result<bool, Trap>|
         -> Result<(usize, usize), Trap> {
            let mut end_x = x + 1;
            while end_x <= right && accepts(end_x, y)? {
                end_x += 1;
            }
            let mut end_y = y + 1;
            loop {
                let mut all = true;
                for xx in x..end_x {
                    if !accepts(xx, end_y)? {
                        all = false;
                        break;
                    }
                }
                if !(all && end_y <= bottom) {
                    break;
                }
                end_y += 1;
            }
            Ok((end_x, end_y))
        };
        let style = |cell: usize, which: usize| {
            let at = cell * WALL_CELL + 2 + which * 2;
            u16::from_le_bytes([walls[at], walls[at + 1]])
        };
        for y in top..=bottom {
            let mut x = left;
            while x <= right {
                if !available(x, y)? {
                    let seed = rect(x, y, x + 1, y + 1, HARD);
                    if let Some(r) = overlap(&rects, seed) {
                        let end = (r.0[2] / 16) as usize;
                        if end != x {
                            x = end - 1;
                        }
                    } else {
                        let (end_x, end_y) =
                            grow(x, y, &mut |xx, yy| available(xx, yy).map(|a| !a))?;
                        rects.push(rect(x, y, end_x, end_y, HARD));
                    }
                } else {
                    // 0x4795c5: the tile's segments; the east and south
                    // styles belong to the neighbours' west and north.
                    let segments = walls[index(x, y) * WALL_CELL];
                    let east = (walls[index(x + 1, y) * WALL_CELL] & 1 != 0)
                        .then(|| style(index(x + 1, y), 0));
                    let south = (walls[index(x, y + 1) * WALL_CELL] & 2 != 0)
                        .then(|| style(index(x, y + 1), 1));
                    if (segments & 8 != 0 && south.is_none())
                        || (segments & 4 != 0 && east.is_none())
                    {
                        return Err(Trap::Unsupported(
                            "a wall segment whose neighbour has no matching segment; the \
                             original reads a style it never set (0x4795c5)"
                                .into(),
                        ));
                    }
                    let blocking = |s: Option<u16>| s.is_some_and(|s| !CROSSABLE.contains(&s));
                    if segments & 8 != 0 && blocking(south) && available(x, y + 1)? {
                        rects.push(rect(x, y + 1, x + 1, y + 1, HARD));
                    }
                    if segments & 4 != 0 && blocking(east) && available(x + 1, y)? {
                        rects.push(rect(x + 1, y, x + 1, y + 1, HARD));
                    }
                    if segments & 0x30 != 0 {
                        let (a, b) = if segments & 0x10 != 0 {
                            (12, 4)
                        } else {
                            (4, 12)
                        };
                        let (xx, yy) = (x as i32 * 16, y as i32 * 16);
                        rects.push(Region([xx + 4, yy + a, xx + 4, yy + a, HARD]));
                        rects.push(Region([xx + 12, yy + b, xx + 12, yy + b, HARD]));
                    }
                }
                x += 1;
            }
        }
        let hard = rects.len();
        if terrain {
            for y in top..=bottom {
                let mut x = left;
                while x <= right {
                    let covered = |xx: usize, yy: usize| {
                        let at = index(xx, yy) * 2;
                        u16::from_le_bytes([floors[at], floors[at + 1]]) != 0
                    };
                    if covered(x, y) {
                        // Global 0x6e4ad4, which would drop the marker's
                        // cost, is taken to be 0.
                        let cost = if flags[index(x, y)] & 0x20 != 0 {
                            10
                        } else {
                            0
                        };
                        let seed = rect(x, y, x + 1, y + 1, cost);
                        if let Some(r) = overlap(&rects, seed) {
                            let end = (r.0[2] / 16) as usize;
                            if end != x {
                                x = end - 1;
                            }
                        } else {
                            let (end_x, end_y) =
                                grow(
                                    x,
                                    y,
                                    &mut |xx, yy| Ok(covered(xx, yy) && available(xx, yy)?),
                                )?;
                            rects.push(rect(x, y, end_x, end_y, cost));
                        }
                    }
                    x += 1;
                }
            }
        }
        Ok(rects
            .into_iter()
            .enumerate()
            .map(|(i, r)| if i < hard { r.inflate(7) } else { r })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    //! A synthetic lot, built in memory: no game data. Ten flat tiles a
    //! side with no walls or water, all one outdoor room on the ground
    //! (room 0), on a residential lot (global 10 is 0), with a floor
    //! covering on the tiles 3 to 4 by 4 to 6.
    use super::*;
    use crate::{Program, Stream};
    use sims_content::Catalog;
    use sims_formats::lot::Array;
    use std::collections::BTreeMap;

    fn array(stride: usize, fill: u8) -> Array {
        Array {
            width: 64,
            height: 64,
            stride,
            encoding: 0,
            data: vec![fill; FULL * FULL * stride],
        }
    }

    fn program() -> Program {
        Program {
            global: 0,
            semi_globals: BTreeMap::new(),
            libraries: BTreeMap::new(),
            careers: None,
            career_texts_end: false,
            zoning: BTreeMap::new(),
            animations: Default::default(),
            people: Default::default(),
            icon_frames: None,
            wardrobe: None,
            character_files: Default::default(),
            expansions: 0,
            characters: Default::default(),
        }
    }

    fn catalog() -> Catalog {
        Catalog {
            global: 0,
            objects: Vec::new(),
            by_guid: Default::default(),
            semi_globals: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }

    fn world() -> World {
        let mut world = World::empty();
        world.size = 10;
        world.arrays.insert(0, array(4, 0));
        for (floors, walls, flags) in [(1, 2, 8), (101, 102, 108)] {
            world.arrays.insert(floors, array(2, 0));
            world.arrays.insert(walls, array(WALL_CELL, 0));
            world.arrays.insert(flags, array(1, 0));
        }
        world.arrays.insert(3, array(2, 0));
        world.arrays.insert(103, array(2, 0));
        world.arrays.insert(9, array(1, 0xff));
        world.arrays.insert(WATER, array(1, 0xff));
        let floors = &mut world.arrays.get_mut(&FLOORS).unwrap().data;
        for x in 3..5 {
            for y in 4..7 {
                floors[(x * FULL + y) * 2] = 1;
            }
        }
        world.rebuild_rooms().unwrap();
        world
    }

    #[test]
    fn terrain_is_left_out_unless_asked_for() {
        // 0x4818e0: on the ground, outdoors, on a residential lot, the
        // covered tiles are a soft rectangle after the hard ones when the
        // call's third argument asks for terrain (as 0x46795a does). The
        // planner asks without it for a person whose data word 17 is set
        // in an outdoor room on level 1 (0x4827f6).
        let (program, catalog) = (program(), catalog());
        let interpreter = Interpreter::new(&program, &catalog, Stream::Lot);
        let world = world();
        assert!(world.rooms.rooms[&0].outdoors);
        let with = interpreter.room_rects(&world, 0, true).unwrap();
        let without = interpreter.room_rects(&world, 0, false).unwrap();
        // The room's border: four hard rectangles, grown by 7.
        assert_eq!(without.len(), 4);
        assert!(without.iter().all(|r| r.0[4] == HARD));
        assert_eq!(with[..4], without[..]);
        // The covering, tiles 3 to 4 by 4 to 6, costs nothing (no flag
        // 0x20) and is not grown.
        assert_eq!(with[4..], [Region([48, 64, 80, 112, 0])]);
    }

    /// Sets the wall cell of tile (`x`, `y`) (the `+0xf8` tile first) on
    /// `level`: its segments, and its two style words.
    fn wall(world: &mut World, level: usize, [x, y]: [usize; 2], segments: u8, styles: [u16; 2]) {
        let walls = &mut world.arrays.get_mut(&WALLS[level - 1]).unwrap().data;
        let cell = (x * FULL + y) * WALL_CELL;
        walls[cell] = segments;
        walls[cell + 2..cell + 4].copy_from_slice(&styles[0].to_le_bytes());
        walls[cell + 4..cell + 6].copy_from_slice(&styles[1].to_le_bytes());
    }

    /// A footprint in the original's order from its `+0xf4` (`a`) and
    /// `+0xf8` (`b`) ranges.
    fn footprint(a: [i32; 2], b: [i32; 2]) -> [i32; 4] {
        [a[1], b[1], a[0], b[0]]
    }

    #[test]
    fn a_wall_crosses_a_footprint_when_its_line_lies_strictly_inside() {
        // 0x476dbc. Segment 1 of tile (3, 1): the line +0xf8 = 48, styled
        // by the first style word.
        let mut lot = world();
        wall(&mut lot, 1, [3, 1], 1, [0, 3]);
        assert_eq!(
            walls_cross(&lot, footprint([21, 27], [45, 51]), 1),
            Ok(true)
        );
        // On the line's edge, or on another level, it does not.
        assert_eq!(
            walls_cross(&lot, footprint([21, 27], [48, 54]), 1),
            Ok(false)
        );
        assert_eq!(
            walls_cross(&lot, footprint([21, 27], [45, 51]), 2),
            Ok(false)
        );
        // Style 3 is one a person crosses.
        wall(&mut lot, 1, [3, 1], 1, [3, 0]);
        assert_eq!(
            walls_cross(&lot, footprint([21, 27], [45, 51]), 1),
            Ok(false)
        );
        // Segment 2 of tile (2, 2): the line +0xf4 = 32, styled by the
        // second style word.
        let mut lot = world();
        wall(&mut lot, 2, [2, 2], 2, [3, 0]);
        let across = footprint([29, 35], [37, 43]);
        assert_eq!(walls_cross(&lot, across, 2), Ok(true));
        wall(&mut lot, 2, [2, 2], 2, [0, 3]);
        assert_eq!(walls_cross(&lot, across, 2), Ok(false));
    }

    #[test]
    fn the_diagonals_compare_sums_and_differences() {
        // Segment 0x10 of tile (3, 1): +0xf4 + +0xf8 = 16 x (1 + 3 + 1) =
        // 80, against 21 + 53 and 27 + 59.
        let mut lot = world();
        wall(&mut lot, 1, [3, 1], 0x10, [3, 0]);
        let centred = footprint([21, 27], [53, 59]);
        assert_eq!(walls_cross(&lot, centred, 1), Ok(true));
        assert_eq!(
            walls_cross(&lot, footprint([17, 23], [49, 55]), 1),
            Ok(false)
        );
        // Segment 0x20 of tile (3, 5): +0xf4 - +0xf8 = 16 x (5 - 3) = 32,
        // against 85 - 59 and 91 - 53.
        let mut lot = world();
        wall(&mut lot, 1, [3, 5], 0x20, [3, 0]);
        let centred = footprint([85, 91], [53, 59]);
        assert_eq!(walls_cross(&lot, centred, 1), Ok(true));
        assert_eq!(
            walls_cross(&lot, footprint([81, 87], [57, 63]), 1),
            Ok(false)
        );
    }

    #[test]
    fn segments_4_and_8_are_never_tested_and_0x20_only_when_lowest() {
        let mut lot = world();
        let centred = footprint([85, 91], [53, 59]);
        // 4 and 8 would mirror 1 and 2 on the next tile; they are not
        // read.
        wall(&mut lot, 1, [3, 5], 4 | 8, [0, 0]);
        assert_eq!(walls_cross(&lot, centred, 1), Ok(false));
        // The search for the next segment stops at 0x20 (0x476f0e), so
        // after segment 1 (whose line, 48, is outside 53 to 59) the
        // crossing diagonal is not read.
        wall(&mut lot, 1, [3, 5], 1 | 0x20, [0, 0]);
        assert_eq!(walls_cross(&lot, centred, 1), Ok(false));
        wall(&mut lot, 1, [3, 5], 0x20, [0, 0]);
        assert_eq!(walls_cross(&lot, centred, 1), Ok(true));
        // 0x10 after 1 is read.
        let mut lot = world();
        wall(&mut lot, 1, [3, 1], 1 | 0x10, [0, 0]);
        assert_eq!(
            walls_cross(&lot, footprint([21, 27], [53, 59]), 1),
            Ok(true)
        );
    }

    #[test]
    fn a_wall_test_off_the_walls_arrays_fails_loudly() {
        let lot = world();
        for level in [0, 3] {
            let result = walls_cross(&lot, footprint([21, 27], [45, 51]), level);
            assert!(
                matches!(&result, Err(Trap::Unsupported(m)) if m.contains("0x4795c5")),
                "{level}: {result:?}"
            );
        }
        let result = walls_cross(&lot, footprint([-5, 3], [45, 51]), 1);
        assert!(matches!(&result, Err(Trap::Unsupported(m)) if m.contains("off the lot")));
    }

    /// A world with water on tile (5, 5), and, when `flags` is given, an
    /// object standing there with those flags in data word 8.
    fn watered(flags: Option<i16>) -> World {
        let mut world = world();
        world.arrays.get_mut(&WATER).unwrap().data[5 * FULL + 5] = 0;
        if let Some(flags) = flags {
            let mut data = vec![0; 72];
            data[OBJECT_FLAGS] = flags;
            world.objects.insert(
                40,
                sims_sim::Object {
                    id: 40,
                    guid: 0x7001,
                    footprint: [0; 4],
                    position: [5 * 16 + 8, 5 * 16 + 8, 1],
                    unknown_short: 0,
                    attributes: Vec::new(),
                    temps: vec![0; 8],
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
            world.order.push(40);
            // The object grid (array 3) names it first on the tile.
            let cell = (5 * FULL + 5) * 2;
            world.arrays.get_mut(&3).unwrap().data[cell..cell + 2]
                .copy_from_slice(&40u16.to_le_bytes());
        }
        world
    }

    #[test]
    fn a_tile_with_water_needs_an_object_that_lets_a_person_stand_there() {
        // 0x4822f8 to 0x482332: a tile with water (0x478f4a) is the room's
        // only when an object on it has flag 0x1000 of data word 8 (+0x5a).
        let (program, catalog) = (program(), catalog());
        let interpreter = Interpreter::new(&program, &catalog, Stream::Lot);
        let dry = interpreter.room_rects(&world(), 0, false).unwrap();
        // Water alone: the tile is hard, one more rectangle round it.
        let wet = interpreter.room_rects(&watered(None), 0, false).unwrap();
        assert_eq!(wet.len(), dry.len() + 1, "{wet:?}");
        let tile = wet.iter().find(|r| !dry.contains(r)).unwrap();
        assert_eq!(tile.0[4], HARD);
        assert!(tile.0[0] <= 5 * 16 && tile.0[2] >= 6 * 16, "{tile:?}");
        // An object there without the flag changes nothing.
        let other = interpreter
            .room_rects(&watered(Some(0x0fff)), 0, false)
            .unwrap();
        assert_eq!(other, wet);
        // With it, the tile is as if dry.
        let standable = interpreter
            .room_rects(&watered(Some(0x1000)), 0, false)
            .unwrap();
        assert_eq!(standable, dry);
    }
}
