//! The lot's rooms: which room each tile is in, and what the simulation
//! knows about each room.
//!
//! The original works the rooms out from the walls (0x479d9a). It does so
//! whenever the lot's walls or floors may have changed; in a load, right
//! after the saved objects are read. A room is a set of tiles that connect
//! without crossing a wall. A diagonal wall cuts its tile into two
//! triangles, which may belong to two rooms. Each level has its own rooms:
//! the ground level's are numbered from 0 and the upper level's from 1024.
//!
//! [`World::rebuild_rooms`] is that function: the partition, and the pass
//! over the tile flags that marks which tiles are outside every enclosure.
//! [`World::room_at`] is the room an object at a position is in
//! (0x433bc5). [`World::update_rooms`] counts each room's size and writes
//! the lot's indoor area to global 27; it is the part of the original's
//! room update (0x466c3c) that needs only the walls and floors.
//!
//! Ported from the old prototype (legacy/crates/sims-sim/src/lot_rooms.rs,
//! lot_flags.rs, lot_walls.rs, lot_geometry.rs, `room_outdoors` in
//! spatial.rs and `Totals::add_tile` in lot_environment.rs), which
//! compared each piece with the original's code run alone. For the Goth house the results were also compared with a
//! trace of the original loading it; see `docs/decisions.md`.

use crate::load::{FLOORS, WALLS};
use crate::{FULL_SIZE, World};
use serde::{Deserialize, Serialize};
use sims_formats::lot::Array;
use std::collections::BTreeMap;

/// The room of an object that is in no room, and the map's mark for a tile
/// that a diagonal wall splits between two rooms.
pub const NO_ROOM: i16 = -5;

/// The map's value for a tile no room took: every tile outside the lot's
/// size.
pub const UNSEEN: i16 = -1;

/// Global 27: the area of the lot's indoor rooms, in tiles.
const INDOOR_AREA: usize = 27;

/// The lot arrays the rooms are worked out from, besides the floors and
/// walls: terrain heights, the tile flags, and the pool.
const HEIGHTS: u16 = 0;
const FLAGS: [u16; 2] = [8, 108];
const POOL: u16 = 9;

/// Tile flag bits (arrays 8 and 108).
mod flag {
    /// The tile is level and inside the lot's border, so it can be built
    /// on; upstairs it also needs support from below.
    pub const BUILDABLE: u8 = 1;
    /// The flood from the lot's corner reached the tile.
    pub const REACHED: u8 = 2;
    /// The tile is outside every enclosure.
    pub const OUTSIDE: u8 = 4;
    /// The tile is covered: something upstairs rests on it.
    pub const COVERED: u8 = 8;
    /// The two bits a rebuild keeps.
    pub const KEPT: u8 = 0x28;
}

/// Wall segment bits, the first byte of a wall cell.
mod wall {
    pub const WEST: u8 = 1;
    pub const NORTH: u8 = 2;
    pub const EAST: u8 = 4;
    pub const SOUTH: u8 = 8;
    /// The diagonal that leaves the tile's north-west and south-east
    /// corners on either side.
    pub const DESCENDING: u8 = 0x10;
    /// The diagonal through the tile's north-west and south-east corners.
    pub const ASCENDING: u8 = 0x20;
    pub const DIAGONAL: u8 = 0x30;
}

/// Why the rooms could not be worked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomError {
    /// The lot lacks an array the rooms are worked out from.
    MissingArray(u16),
    /// An array that is not 64 by 64 cells of the size its use needs.
    ArrayShape(u16),
    /// The lot's size is not 2 to 64 tiles.
    Size(i16),
    /// A level is neither 1 nor 2.
    Level(i32),
    /// A wall cell has segment bits beyond the six known ones.
    Segments { level: u8, x: usize, y: usize },
    /// A tile has both diagonal walls. What the original does then is not
    /// known.
    CrossingDiagonals { level: u8, x: usize, y: usize },
    /// A tile's west or north wall is not mirrored as its neighbour's east
    /// or south wall. The original keeps the two in step.
    Asymmetric { level: u8, x: usize, y: usize },
    /// A diagonal wall that blocks on the lot's border. The original reads
    /// outside its arrays there.
    BorderDiagonal { level: u8, x: usize, y: usize },
    /// More than 1024 rooms on one level. The room IDs of the two levels
    /// would overlap.
    TooManyRooms { level: u8 },
    /// The rooms have not been worked out yet.
    NoRooms,
    /// A step that needs all 64 globals ran on fewer.
    NoContext,
}

impl std::fmt::Display for RoomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingArray(array) => write!(f, "the lot has no array {array}"),
            Self::ArrayShape(array) => write!(
                f,
                "lot array {array} does not have the shape the rooms are worked out from"
            ),
            Self::Size(size) => write!(f, "a lot of {size} tiles has no rooms"),
            Self::Level(level) => write!(f, "level {level} has no rooms"),
            Self::Segments { level, x, y } => {
                write!(f, "unknown wall segments at {x}, {y} on level {level}")
            }
            Self::CrossingDiagonals { level, x, y } => write!(
                f,
                "two diagonal walls cross at {x}, {y} on level {level}; that is not ported"
            ),
            Self::Asymmetric { level, x, y } => write!(
                f,
                "the wall at {x}, {y} on level {level} is not mirrored in its neighbour"
            ),
            Self::BorderDiagonal { level, x, y } => write!(
                f,
                "a diagonal wall on the lot's border at {x}, {y} on level {level}; that is \
                 not ported"
            ),
            Self::TooManyRooms { level } => write!(f, "more than 1024 rooms on level {level}"),
            Self::NoRooms => write!(f, "the lot's rooms have not been worked out"),
            Self::NoContext => write!(f, "the load context has not been applied"),
        }
    }
}

impl std::error::Error for RoomError {}

/// The lot's rooms. See [`World::rooms`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rooms {
    /// For each level, the room of each tile: 64 by 64 cells, column by
    /// column, like the lot arrays. [`UNSEEN`] for a tile outside the
    /// lot's size, [`NO_ROOM`] for a tile a diagonal wall splits between
    /// two rooms; [`Rooms::halves`] has those. Empty until the rooms are
    /// first worked out.
    pub map: [Vec<i16>; 2],
    /// The split tiles, by level, column and row: the two triangles and
    /// the room of each.
    pub halves: BTreeMap<[u8; 3], [Half; 2]>,
    /// The rooms by ID.
    pub rooms: BTreeMap<i16, Room>,
}

/// One triangle of a tile that a diagonal wall splits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Half {
    /// Which triangle, by the corner it holds: 1 south-west, 2 south-east,
    /// 3 north-east, 4 north-west.
    pub triangle: u8,
    /// Its room, or [`UNSEEN`] if no room took it.
    pub room: i16,
}

/// One room.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Room {
    /// The room's tiles as column, row and level, in the order the
    /// partition reached them. A tile reached more than once is listed
    /// more than once, and the original counts it each time.
    pub tiles: Vec<[u8; 3]>,
    /// Whether the room is outdoors (0x46876f): whether the first listed
    /// tile that the map gives to this room is outside every enclosure. A
    /// room with no such tile, one made only of triangles, is outdoors if
    /// it lists exactly one tile.
    ///
    /// The original works this out when it is first asked and keeps the
    /// answer. Here it is worked out with the rooms. Nothing between the
    /// two changes what it is worked out from.
    pub outdoors: bool,
    /// Set when the room is made, cleared when [`World::update_rooms`] has
    /// counted it.
    pub dirty: bool,
    /// The room's size in half tiles, zero until it is counted. A listed
    /// tile counts 2, or 1 if it has a diagonal wall and the room is
    /// indoors.
    pub half_tiles: i32,
    /// How many of those half tiles have a floor covering. Zero outdoors.
    pub covered_half_tiles: i32,
    /// The wall segments along the room's listed tiles whose style can
    /// take a wall covering, and how many of them have one. Zero outdoors.
    pub walls: i32,
    pub covered_walls: i32,
}

/// One level's arrays, checked to be 64 by 64 cells of the right size.
struct Level<'a> {
    level: u8,
    walls: &'a [u8],
    floors: &'a [u8],
}

const FULL: usize = FULL_SIZE as usize;
const WALL_CELL: usize = 14;

fn at(x: usize, y: usize) -> usize {
    x * FULL + y
}

fn array(arrays: &BTreeMap<u16, Array>, id: u16, stride: usize) -> Result<&[u8], RoomError> {
    let array = arrays.get(&id).ok_or(RoomError::MissingArray(id))?;
    if array.width != FULL
        || array.height != FULL
        || array.stride != stride
        || array.data.len() != FULL * FULL * stride
    {
        return Err(RoomError::ArrayShape(id));
    }
    Ok(&array.data)
}

impl Level<'_> {
    fn segments(&self, cell: usize) -> u8 {
        self.walls[cell * WALL_CELL]
    }

    /// One of a wall cell's six words: two styles, then four patterns.
    fn word(&self, cell: usize, word: usize) -> u16 {
        let at = cell * WALL_CELL + 2 + word * 2;
        u16::from_le_bytes([self.walls[at], self.walls[at + 1]])
    }

    fn floor(&self, cell: usize) -> u16 {
        u16::from_le_bytes([self.floors[cell * 2], self.floors[cell * 2 + 1]])
    }

    /// The style of one of a tile's four straight walls, by its bit
    /// (0x4795c5). A tile owns its west and north walls: their styles are
    /// its first and second style words. Its east wall is the west wall of
    /// the next tile east, and its south wall the north wall of the next
    /// tile south. None if that neighbour has no such wall.
    fn style(&self, x: usize, y: usize, bit: u8) -> Option<u16> {
        match bit {
            wall::WEST => Some(self.word(at(x, y), 0)),
            wall::NORTH => Some(self.word(at(x, y), 1)),
            wall::EAST => (x + 1 < FULL && self.segments(at(x + 1, y)) & wall::WEST != 0)
                .then(|| self.word(at(x + 1, y), 0)),
            _ => (y + 1 < FULL && self.segments(at(x, y + 1)) & wall::NORTH != 0)
                .then(|| self.word(at(x, y + 1), 1)),
        }
    }

    /// The pattern on the side of a straight wall that faces this tile, in
    /// the order the original keeps them after it has expanded the cell
    /// (0x4795c5): west, north, east, south.
    fn pattern(&self, cell: usize, bit: u8) -> u16 {
        self.word(
            cell,
            match bit {
                wall::WEST => 2,
                wall::NORTH => 3,
                wall::EAST => 5,
                _ => 4,
            },
        )
    }
}

/// Whether a wall of this style closes a room off from the outside
/// (0x479d9a). Fences and the like do not.
fn encloses(style: u16) -> bool {
    !matches!(style, 2 | 12 | 13 | 14)
}

/// Whether a wall of this style can take a wall covering (0x467b74).
fn decorated(style: u16) -> bool {
    matches!(style, 1 | 3 | 5 | 6 | 7 | 8 | 9 | 15 | 22 | 23)
}

/// The two triangles of a tile with a diagonal wall, in the order the
/// partition takes them, or zeros for a whole tile.
fn triangles(segments: u8) -> [u8; 2] {
    match segments & wall::DIAGONAL {
        wall::DESCENDING => [4, 2],
        wall::ASCENDING => [3, 1],
        _ => [0, 0],
    }
}

/// Whether a triangle touches the tile's edge in a direction: 0 north,
/// 1 south, 2 west, 3 east. A whole tile touches all four.
fn touches(triangle: u8, direction: usize) -> bool {
    match triangle {
        1 => matches!(direction, 1 | 2),
        2 => matches!(direction, 1 | 3),
        3 => matches!(direction, 0 | 3),
        4 => matches!(direction, 0 | 2),
        _ => true,
    }
}

/// The map's working values while a room spreads: one triangle of the tile
/// is waiting (the one on its west side, or the one on its east side), or
/// the whole tile is.
const WAITING_WEST: i16 = -2;
const WAITING_EAST: i16 = -3;
const WAITING_WHOLE: i16 = -4;

/// One level's partition: the map, the split tiles by cell, and each
/// room's tiles.
type Partition = (
    Vec<i16>,
    BTreeMap<usize, [Half; 2]>,
    Vec<(i16, Vec<[u8; 3]>)>,
);

/// The first pass of 0x479d9a for one level: gives every tile inside the
/// lot's size a room.
///
/// The tiles are scanned column by column. A tile no room has taken starts
/// a new room, which spreads to the four neighbours it can reach without
/// crossing a wall, depth first. Only the wall bits count, not the wall
/// styles. Upstairs a room does not spread between a tile with a floor and
/// one without. On the ground it does not spread across the edge of a
/// pool (pool array values below 16).
///
/// A triangle that the scan leaves untaken, because the tile's other
/// triangle was the one the scan started from and no room reached around
/// to it, stays untaken ([`UNSEEN`] in [`Half::room`]). The original
/// scans each tile once.
fn partition(
    level: &Level<'_>,
    active: usize,
    pool: Option<&[u8]>,
) -> Result<Partition, RoomError> {
    let floor = level.level;
    for x in 0..active {
        for y in 0..active {
            let bits = level.segments(at(x, y));
            let fail = |error: fn(u8, usize, usize) -> RoomError| Err(error(floor, x, y));
            if bits & !0x3f != 0 {
                return fail(|level, x, y| RoomError::Segments { level, x, y });
            }
            if bits & wall::DIAGONAL == wall::DIAGONAL {
                return fail(|level, x, y| RoomError::CrossingDiagonals { level, x, y });
            }
            let mirrored = |own: u8, x: usize, y: usize, theirs: u8| {
                (bits & own != 0) == (level.segments(at(x, y)) & theirs != 0)
            };
            if (x > 0 && !mirrored(wall::WEST, x - 1, y, wall::EAST))
                || (y > 0 && !mirrored(wall::NORTH, x, y - 1, wall::SOUTH))
            {
                return fail(|level, x, y| RoomError::Asymmetric { level, x, y });
            }
        }
    }
    let in_pool = |cell: usize| floor == 1 && pool.is_some_and(|pool| pool[cell] < 16);
    let mut map = vec![UNSEEN; FULL * FULL];
    let mut split: BTreeMap<usize, [Half; 2]> = BTreeMap::new();
    let mut rooms = Vec::new();
    let mut next = 0u16;
    for x in 0..active {
        for y in 0..active {
            let seed = at(x, y);
            let open = matches!(map[seed], UNSEEN | WAITING_WEST | WAITING_EAST)
                || (map[seed] == NO_ROOM && split[&seed][1].room == UNSEEN);
            if !open {
                continue;
            }
            if next >= 1024 {
                return Err(RoomError::TooManyRooms { level: floor });
            }
            let room = ((u16::from(floor - 1) << 10) | next) as i16;
            next += 1;
            let covered = level.floor(seed) != 0;
            let pool_side = in_pool(seed);
            let mut stack = vec![seed];
            let mut tiles = Vec::new();
            let tile = |cell: usize| [(cell / FULL) as u8, (cell % FULL) as u8, floor];
            while let Some(cell) = stack.pop() {
                if map[cell] == room {
                    continue;
                }
                let triangle = match map[cell] {
                    UNSEEN => {
                        let triangle = triangles(level.segments(cell))[0];
                        map[cell] = match triangle {
                            4 => WAITING_WEST,
                            3 => WAITING_EAST,
                            _ => WAITING_WHOLE,
                        };
                        stack.push(cell);
                        tiles.push(tile(cell));
                        triangle
                    }
                    WAITING_WHOLE => {
                        map[cell] = room;
                        0
                    }
                    state @ (WAITING_WEST | WAITING_EAST | NO_ROOM) => {
                        let descending = level.segments(cell) & wall::DESCENDING != 0;
                        let triangle = if state == NO_ROOM {
                            let second = split[&cell][1];
                            if second.room == room {
                                continue;
                            }
                            if second.room == UNSEEN {
                                tiles.push(tile(cell));
                            }
                            second.triangle
                        } else if state == WAITING_WEST {
                            if descending { 4 } else { 1 }
                        } else if descending {
                            2
                        } else {
                            3
                        };
                        if let Some(record) = split.get_mut(&cell) {
                            if record[0].room == room {
                                // Both triangles are in one room after
                                // all.
                                split.remove(&cell);
                                map[cell] = room;
                            } else {
                                record[1] = Half { triangle, room };
                                map[cell] = NO_ROOM;
                            }
                        } else {
                            // The opposite triangle.
                            let other = (triangle + 1) % 4 + 1;
                            split.insert(
                                cell,
                                [
                                    Half { triangle, room },
                                    Half {
                                        triangle: other,
                                        room: UNSEEN,
                                    },
                                ],
                            );
                            map[cell] = NO_ROOM;
                        }
                        triangle
                    }
                    _ => 0,
                };
                let (x, y) = (cell / FULL, cell % FULL);
                // North, south, west, east, each with the wall bit of the
                // neighbour that would block the way.
                let ways = [
                    (0, -1, wall::SOUTH),
                    (0, 1, wall::NORTH),
                    (-1, 0, wall::EAST),
                    (1, 0, wall::WEST),
                ];
                for (direction, (dx, dy, blocking)) in ways.into_iter().enumerate() {
                    if !touches(triangle, direction) {
                        continue;
                    }
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= active as i32 || ny >= active as i32 {
                        continue;
                    }
                    let target = at(nx as usize, ny as usize);
                    if !(NO_ROOM..=UNSEEN).contains(&map[target])
                        || level.segments(target) & blocking != 0
                        || (floor != 1 && (level.floor(target) != 0) != covered)
                        || in_pool(target) != pool_side
                    {
                        continue;
                    }
                    // The neighbour's triangle on the side we come from.
                    let both = triangles(level.segments(target));
                    let entered = both[usize::from(!touches(both[0], direction ^ 1))];
                    let waiting = match entered {
                        1 | 4 => WAITING_WEST,
                        2 | 3 => WAITING_EAST,
                        _ => WAITING_WHOLE,
                    };
                    let state = map[target];
                    map[target] = if entered == 0 {
                        WAITING_WHOLE
                    } else if state == UNSEEN {
                        waiting
                    } else if matches!(
                        (state, waiting),
                        (WAITING_WEST, WAITING_EAST) | (WAITING_EAST, WAITING_WEST)
                    ) {
                        WAITING_WHOLE
                    } else if state == NO_ROOM
                        && split[&target]
                            .iter()
                            .any(|half| half.triangle == entered && half.room == UNSEEN)
                    {
                        NO_ROOM
                    } else {
                        continue;
                    };
                    stack.push(target);
                    tiles.push(tile(target));
                }
            }
            if tiles.is_empty() {
                next -= 1;
            } else {
                rooms.push((room, tiles));
            }
        }
    }
    Ok((map, split, rooms))
}

/// The rest of 0x479d9a for one level: rebuilds the tile flags.
///
/// Every cell of the array, also outside the lot's size, keeps only bits
/// 8 and 0x20. Then a flood from the lot's corner marks the tiles outside
/// every enclosure with bits 2 and 4: it crosses a tile's edge unless a
/// wall of an enclosing style is there, and it never enters a tile with
/// an enclosing diagonal wall. Last, each level tile inside the lot's
/// border gets bit 1; upstairs only if the tile below is enclosed, or is
/// covered, or the tile has a floor beside it and support near it.
///
/// The original also has a branch for a flood that stands on a tile with
/// a diagonal wall, which sets bit 0x40 or 0x80. A flood can only stand on
/// one if it starts there, at the lot's corner, and a diagonal wall on the
/// border is refused here. So that branch is not ported.
///
/// `flags` holds the level's saved flags and is rewritten. `below` is the
/// ground level's rebuilt flags, for the upper level.
fn rebuild_flags(
    level: &Level<'_>,
    active: usize,
    heights: &[u8],
    flags: &mut [u8],
    below: Option<&[u8]>,
) -> Result<(), RoomError> {
    let floor = level.level;
    let diagonal =
        |cell: usize| level.segments(cell) & wall::DIAGONAL != 0 && encloses(level.word(cell, 1));
    for x in 0..active {
        for y in 0..active {
            if (x == 0 || y == 0 || x + 1 == FULL || y + 1 == FULL) && diagonal(at(x, y)) {
                return Err(RoomError::BorderDiagonal { level: floor, x, y });
            }
        }
    }
    for cell in flags.iter_mut() {
        *cell &= flag::KEPT;
    }
    let neighbor = |x: usize, y: usize, dx: i32, dy: i32| {
        let (x, y) = (x as i32 + dx, y as i32 + dy);
        (x >= 0 && y >= 0 && x < active as i32 && y < active as i32)
            .then_some((x as usize, y as usize))
    };
    let mut stack = vec![(0usize, 0usize)];
    while let Some((x, y)) = stack.pop() {
        let cell = at(x, y);
        if flags[cell] & flag::REACHED != 0 {
            continue;
        }
        flags[cell] |= flag::REACHED | flag::OUTSIDE;
        let ways = [
            (-1, 0, wall::WEST),
            (1, 0, wall::EAST),
            (0, -1, wall::NORTH),
            (0, 1, wall::SOUTH),
        ];
        for (dx, dy, bit) in ways {
            let Some((nx, ny)) = neighbor(x, y, dx, dy) else {
                continue;
            };
            let next = at(nx, ny);
            if flags[next] & flag::REACHED != 0 {
                continue;
            }
            let blocked = level.segments(cell) & bit != 0
                && encloses(level.style(x, y, bit).ok_or(RoomError::Asymmetric {
                    level: floor,
                    x,
                    y,
                })?);
            if !blocked && !diagonal(next) {
                stack.push((nx, ny));
            }
        }
    }
    for x in 1..active.saturating_sub(1) {
        for y in 1..active.saturating_sub(1) {
            let cell = at(x, y);
            let corners = &heights[cell * 4..cell * 4 + 4];
            if corners.iter().any(|&height| height != corners[0]) {
                continue;
            }
            let supported = match below {
                None => true,
                Some(below) => {
                    let enclosed = |cell: usize| below[cell] & flag::OUTSIDE == 0;
                    let covered = |cell: usize| below[cell] & flag::COVERED != 0;
                    enclosed(cell)
                        || covered(cell)
                        || ([(0, -1), (-1, 0), (1, 0), (0, 1)]
                            .into_iter()
                            .filter_map(|(dx, dy)| neighbor(x, y, dx, dy))
                            .any(|(nx, ny)| level.floor(at(nx, ny)) != 0)
                            && (-2i32..=2).any(|dy| {
                                (-2i32..=2).any(|dx| {
                                    (dx, dy) != (0, 0)
                                        && neighbor(x, y, dx, dy).is_some_and(|(nx, ny)| {
                                            let near = at(nx, ny);
                                            enclosed(near)
                                                || (covered(near) && dx.abs() <= 1 && dy.abs() <= 1)
                                        })
                                })
                            }))
                }
            };
            if supported {
                flags[cell] |= flag::BUILDABLE;
            }
        }
    }
    Ok(())
}

impl World {
    /// Works out the lot's rooms from its walls and floors, both levels
    /// (0x479d9a), and rebuilds both levels' tile flags (arrays 8 and
    /// 108). The rooms replace [`World::rooms`]; each starts uncounted
    /// ([`Room::dirty`]). On an error the lot is unchanged.
    ///
    /// The original runs this when the lot's architecture may have
    /// changed. In a load that is after the saved objects are read and the
    /// controllers reconciled, and before every object is given its room.
    /// It then tells the game's other parts that the rooms changed, which
    /// is how load step 10 comes to run; the caller does that.
    ///
    /// For the Goth house the result was compared with a trace of the
    /// original loading it, and was equal: the room IDs, each room's
    /// number of listed tiles, the tile lists of the 8 rooms whose tiles
    /// the trace holds, the upper level's map in the columns the trace
    /// holds at that moment and both levels' maps at the end of the load,
    /// whether each room is outdoors, and every cell of both levels'
    /// flags. See `docs/decisions.md` for what the trace does not hold.
    ///
    /// Not modeled: two booleans the original keeps about the upper level
    /// (whether it holds anything, and whether anything on it is
    /// supported), the roofs, and the redrawing the original does here.
    /// Not ported, and failing with an error that says so: a tile with two
    /// crossing diagonal walls, a wall that its neighbour does not mirror,
    /// an enclosing diagonal wall on the border of the arrays, and more
    /// than 1024 rooms on a level.
    pub fn rebuild_rooms(&mut self) -> Result<(), RoomError> {
        let active = usize::try_from(self.size)
            .ok()
            .filter(|size| (2..=FULL).contains(size))
            .ok_or(RoomError::Size(self.size))?;
        let heights = array(&self.arrays, HEIGHTS, 4)?;
        // The pool array is optional in a house file.
        let pool = match self.arrays.contains_key(&POOL) {
            true => Some(array(&self.arrays, POOL, 1)?),
            false => None,
        };
        let mut rooms = Rooms::default();
        let mut rebuilt: Vec<Vec<u8>> = Vec::new();
        for index in 0..2 {
            let level = Level {
                level: index as u8 + 1,
                walls: array(&self.arrays, WALLS[index], WALL_CELL)?,
                floors: array(&self.arrays, FLOORS[index], 2)?,
            };
            let (map, split, listed) = partition(&level, active, pool)?;
            let mut flags = array(&self.arrays, FLAGS[index], 1)?.to_vec();
            rebuild_flags(
                &level,
                active,
                heights,
                &mut flags,
                rebuilt.first().map(Vec::as_slice),
            )?;
            for (id, tiles) in listed {
                let own = tiles
                    .iter()
                    .map(|tile| at(usize::from(tile[0]), usize::from(tile[1])))
                    .find(|&cell| map[cell] == id);
                let outdoors = match own {
                    Some(cell) => flags[cell] & flag::OUTSIDE != 0,
                    None => tiles.len() == 1,
                };
                rooms.rooms.insert(
                    id,
                    Room {
                        tiles,
                        outdoors,
                        dirty: true,
                        ..Room::default()
                    },
                );
            }
            for (cell, halves) in split {
                let tile = [level.level, (cell / FULL) as u8, (cell % FULL) as u8];
                rooms.halves.insert(tile, halves);
            }
            rooms.map[index] = map;
            rebuilt.push(flags);
        }
        for (id, flags) in FLAGS.into_iter().zip(rebuilt) {
            if let Some(array) = self.arrays.get_mut(&id) {
                array.data = flags;
            }
        }
        self.rooms = rooms;
        Ok(())
    }

    /// The room an object at `position` is in (0x433bc5), or [`NO_ROOM`].
    ///
    /// The object's tile is its position divided by 16 ([`World::tile`]).
    /// A tile on the lot's outermost ring, or outside the lot's size, is
    /// in no room. On a tile that a diagonal wall splits, the original
    /// does not look up the triangle's room. It takes the room of the
    /// tile beside it, on the side of the diagonal the position is on:
    /// first the neighbour in the first coordinate, and if that is split
    /// too, the neighbour in the second.
    ///
    /// For the Goth house every object's room was compared with a trace
    /// of the original. None of its objects stands on a split tile, so the
    /// rule for split tiles is **unverified** against the running game. It
    /// follows the old prototype's `object_room_in_active`
    /// (legacy/crates/sims-sim/src/lot_rooms.rs), which was compared with
    /// the original's 0x433bc5 run alone on 32,780 positions.
    pub fn room_at(&self, position: [i32; 3]) -> Result<i16, RoomError> {
        let level = usize::try_from(position[2] - 1)
            .ok()
            .filter(|&level| level < 2)
            .ok_or(RoomError::Level(position[2]))?;
        let map = &self.rooms.map[level];
        if map.len() != FULL * FULL {
            return Err(RoomError::NoRooms);
        }
        let [x, y, _] = Self::tile(position).map(i32::from);
        let size = i32::from(self.size);
        let inside = |n: i32| n > 0 && n < size - 1 && n < FULL as i32;
        if !inside(x) || !inside(y) {
            return Ok(NO_ROOM);
        }
        let cell = |x: i32, y: i32| at(x as usize, y as usize);
        let mut room = map[cell(x, y)];
        if room != NO_ROOM {
            return Ok(room);
        }
        let walls = array(&self.arrays, WALLS[level], WALL_CELL)?;
        let segments = walls[cell(x, y) * WALL_CELL];
        let (fx, fy) = (position[0] & 15, position[1] & 15);
        let (dx, dy) = if segments & wall::DESCENDING != 0 {
            if fx + fy < 16 { (-1, -1) } else { (1, 1) }
        } else if segments & wall::ASCENDING != 0 {
            if fx < fy { (-1, 1) } else { (1, -1) }
        } else {
            (0, 0)
        };
        // If the first neighbour is off the lot, the second is not tried.
        if inside(x + dx) {
            room = map[cell(x + dx, y)];
            if room == NO_ROOM && inside(y + dy) {
                room = map[cell(x, y + dy)];
            }
        }
        Ok(room)
    }

    /// Counts every room that has not been counted and writes the lot's
    /// indoor area to global 27. This is the part of the original's room
    /// update (0x467094, 0x466c3c) that needs only the walls and floors
    /// (0x467bb1). Returns the IDs of the rooms counted.
    ///
    /// For each listed tile of an uncounted room, repeats included: an
    /// outdoor room counts 2 half tiles and nothing else. An indoor room
    /// counts 2 half tiles for a tile with no diagonal wall, both covered
    /// if the tile has a floor, and each of the tile's four walls whose
    /// style can take a covering, with whether it has one. A tile with a
    /// diagonal wall counts 1 half tile, covered if the wall cell's first
    /// style or first pattern word is nonzero (they hold the two half
    /// floors), and the diagonal wall itself.
    ///
    /// The indoor area is the sum, over the indoor rooms, of the half
    /// tiles divided by 2, rounding each room down.
    ///
    /// In the load the original does this after every object has been
    /// given its room (step 10) and before the removal queue is emptied
    /// (step 11). For the Goth house the four counts of all 11 rooms and
    /// global 27 were compared with a trace of the original at that
    /// moment and were equal.
    ///
    /// Not modeled, though the original's update does it for each room it
    /// counts: what the objects standing in the room add (their number,
    /// their light and their effect on the room's score), the room's
    /// light and score, the lot's outdoor score, the people in the room,
    /// and the lighting of the tiles. The trace shows no object, global
    /// or random stream changing in this update other than global 27.
    pub fn update_rooms(&mut self) -> Result<Vec<i16>, RoomError> {
        if self.globals.len() != 64 {
            return Err(RoomError::NoContext);
        }
        let mut counted = Vec::new();
        let mut totals = Vec::new();
        for (&id, room) in &self.rooms.rooms {
            if !room.dirty {
                continue;
            }
            let index = usize::from((id as u16 >> 10) & 3);
            if index > 1 {
                return Err(RoomError::Level(index as i32 + 1));
            }
            let level = Level {
                level: index as u8 + 1,
                walls: array(&self.arrays, WALLS[index], WALL_CELL)?,
                floors: array(&self.arrays, FLOORS[index], 2)?,
            };
            let mut total = [0i32; 4];
            for tile in &room.tiles {
                let (x, y) = (usize::from(tile[0]), usize::from(tile[1]));
                let cell = at(x, y);
                let segments = level.segments(cell);
                let [half_tiles, covered, walls, covered_walls] = &mut total;
                if room.outdoors {
                    *half_tiles = half_tiles.wrapping_add(2);
                } else if segments & wall::DIAGONAL == 0 {
                    *half_tiles = half_tiles.wrapping_add(2);
                    if level.floor(cell) != 0 {
                        *covered = covered.wrapping_add(2);
                    }
                    for bit in [wall::WEST, wall::NORTH, wall::EAST, wall::SOUTH] {
                        if segments & bit == 0 {
                            continue;
                        }
                        let style = level.style(x, y, bit).ok_or(RoomError::Asymmetric {
                            level: level.level,
                            x,
                            y,
                        })?;
                        if decorated(style) {
                            *walls = walls.wrapping_add(1);
                            if level.pattern(cell, bit) != 0 {
                                *covered_walls = covered_walls.wrapping_add(1);
                            }
                        }
                    }
                } else {
                    *half_tiles = half_tiles.wrapping_add(1);
                    // A diagonal wall's style is the cell's second style
                    // word, and its two sides are the last two pattern
                    // words.
                    if decorated(level.word(cell, 1)) {
                        *walls = walls.wrapping_add(1);
                        if level.word(cell, 4) != 0 || level.word(cell, 5) != 0 {
                            *covered_walls = covered_walls.wrapping_add(1);
                        }
                    }
                    if level.word(cell, 0) != 0 || level.word(cell, 2) != 0 {
                        *covered = covered.wrapping_add(1);
                    }
                }
            }
            counted.push(id);
            totals.push(total);
        }
        for (id, total) in counted.iter().zip(totals) {
            if let Some(room) = self.rooms.rooms.get_mut(id) {
                [
                    room.half_tiles,
                    room.covered_half_tiles,
                    room.walls,
                    room.covered_walls,
                ] = total;
                room.dirty = false;
            }
        }
        let indoor = self
            .rooms
            .rooms
            .values()
            .filter(|room| !room.outdoors)
            .fold(0i32, |sum, room| sum.wrapping_add(room.half_tiles / 2));
        self.globals[INDOOR_AREA] = indoor as i16;
        Ok(counted)
    }
}
