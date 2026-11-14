//! A room as rectangles for the planner (0x4818af): hard rectangles for
//! the room's border, the tiles a person of the room cannot stand on and
//! the walls, then soft rectangles for the ground's covering.
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

/// The level a room is on, from its ID (0x468756).
pub(crate) fn room_level(room: i16) -> i32 {
    i32::from((room as u16 >> 10) & 3) + 1
}

impl Interpreter<'_> {
    /// The other side of a door (0x4636ae): the next part of the door's
    /// group, round the ring, that has the door function. Doors that pair
    /// by graphic (definition word 69 is 1) are not ported.
    fn partner(&self, world: &World, door: u16) -> Result<u16, Trap> {
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
    /// whose other side is in the room; and the tile has no water.
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
        // 0x478f4a: a tile with water needs an object that makes it
        // standable.
        if level == 1 && array(world, WATER, 1)?[x * FULL + y] != 0xff {
            return Err(Trap::Unsupported(
                "a tile with water in a room the planner reads (0x482301) is not ported".into(),
            ));
        }
        Ok(true)
    }

    /// The room's rectangles (0x4818af with terrain asked for), the hard
    /// ones first, each hard one grown by 7.
    pub(crate) fn room_rects(&self, world: &World, room: i16) -> Result<Vec<Region>, Trap> {
        let level = room_level(room);
        if level != 1 {
            return Err(Trap::Unsupported(
                "routing in a room above the ground (0x4818af) is not ported".into(),
            ));
        }
        let size = usize::try_from(world.size)
            .ok()
            .filter(|s| (3..=FULL).contains(s))
            .ok_or_else(|| Trap::Invalid("the lot's size is out of range".into()))?;
        let map = &world.rooms.map[0];
        if map.len() != FULL * FULL {
            return Err(Trap::Invalid("the lot has no room map".into()));
        }
        let walls = array(world, WALLS[0], WALL_CELL)?;
        let floors = array(world, FLOORS, 2)?;
        let flags = array(world, FLAGS, 1)?;
        let index = |x: usize, y: usize| x * FULL + y;
        let outdoors = world
            .rooms
            .rooms
            .get(&room)
            .ok_or_else(|| Trap::Invalid(format!("room {room} is not on the lot")))?
            .outdoors;
        // 0x40745f: the lot's zoning; terrain counts on a residential lot.
        let lot = world.globals.get(LOT).copied().unwrap_or(0);
        let residential = lot == 0 || self.program.zoning.get(&lot) == Some(&1);
        let terrain = residential && outdoors;

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
